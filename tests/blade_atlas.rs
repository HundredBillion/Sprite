#![cfg(test)]

use etagere::{BucketedAtlasAllocator, size2};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    ops,
    sync::atomic::{AtomicUsize, Ordering},
};

type AtlasKey = u32;

#[derive(Clone, Copy)]
struct AtlasTextureId {
    kind: usize,
    index: u32,
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: etagere::AllocId,
}

struct Texture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    live_atlas_keys: usize,
}

impl Texture {
    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }
    fn is_unreferenced(&self) -> bool {
        self.live_atlas_keys == 0
    }
    fn destroy(&self, gpu: &AtomicUsize) {
        gpu.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct TextureList {
    textures: Vec<Option<Texture>>,
    free_list: Vec<usize>,
}

struct Storage([TextureList; 2]);
impl ops::Index<usize> for Storage {
    type Output = TextureList;
    fn index(&self, kind: usize) -> &TextureList {
        &self.0[kind]
    }
}
impl ops::IndexMut<usize> for Storage {
    fn index_mut(&mut self, kind: usize) -> &mut TextureList {
        &mut self.0[kind]
    }
}

struct State {
    tiles_by_key: HashMap<AtlasKey, AtlasTile>,
    storage: Storage,
    gpu: AtomicUsize,
}
struct BladeAtlas(Mutex<State>);

impl BladeAtlas {
    // PRODUCTION_REMOVE
}

fn atlas(kind: usize) -> BladeAtlas {
    let mut storage = Storage(Default::default());
    storage[kind].textures.push(Some(Texture {
        id: AtlasTextureId { kind, index: 0 },
        allocator: BucketedAtlasAllocator::new(size2(1024, 1024)),
        live_atlas_keys: 0,
    }));
    BladeAtlas(Mutex::new(State {
        tiles_by_key: HashMap::new(),
        storage,
        gpu: AtomicUsize::new(0),
    }))
}

fn insert(atlas: &BladeAtlas, kind: usize, key: AtlasKey) {
    let mut state = atlas.0.lock();
    let texture = state.storage[kind].textures[0].as_mut().unwrap();
    let allocation = texture
        .allocator
        .allocate(size2(512, 512))
        .expect("a removed image's space must be reusable");
    texture.live_atlas_keys += 1;
    let tile = AtlasTile {
        texture_id: texture.id,
        tile_id: allocation.id,
    };
    assert!(state.tiles_by_key.insert(key, tile).is_none());
}

#[test]
fn removed_images_reuse_space_while_another_image_stays_live() {
    for kind in 0..2 {
        let atlas = atlas(kind);
        insert(&atlas, kind, 0);
        for key in 1..=60 {
            insert(&atlas, kind, key);
            atlas.remove(&key);
            atlas.remove(&key);
            let state = atlas.0.lock();
            assert_eq!(state.storage[kind].textures.len(), 1);
            assert_eq!(
                state.storage[kind].textures[0]
                    .as_ref()
                    .unwrap()
                    .live_atlas_keys,
                1
            );
            assert_eq!(state.gpu.load(Ordering::SeqCst), 0);
        }
        atlas.remove(&0);
        atlas.remove(&0);
        atlas.remove(&999);
        let state = atlas.0.lock();
        assert!(state.tiles_by_key.is_empty());
        assert!(state.storage[kind].textures[0].is_none());
        assert_eq!(state.storage[kind].free_list, vec![0]);
        assert_eq!(state.gpu.load(Ordering::SeqCst), 1);
    }
}
