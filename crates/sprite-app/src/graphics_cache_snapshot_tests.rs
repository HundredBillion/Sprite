use super::*;
use sprite_term::TransmittedFormat;

fn image(id: u32, generation: u64, size: u32) -> Arc<ImagePixels> {
    Arc::new(ImagePixels {
        id,
        generation,
        width: size,
        height: size,
        transmitted: TransmittedFormat::Rgba,
        pixels: vec![127; size as usize * size as usize * 4],
    })
}

#[test]
fn snapshot_pressure_preserves_retained_textures_without_conversion() {
    let images: Vec<_> = (1..=3).map(|id| image(id, 1, 1)).collect();
    let mut cache = GraphicsCache::with_budget(8);
    cache.refresh(&images);
    let retained: Vec<_> = images
        .iter()
        .filter_map(|image| {
            cache
                .get(image.id, image.generation)
                .map(|texture| (image.id, texture))
        })
        .collect();
    assert_eq!(retained.len(), 2);
    CONVERSIONS.set(0);
    for _ in 0..5 {
        cache.refresh(&images);
        for (id, texture) in &retained {
            assert!(Arc::ptr_eq(
                texture,
                &cache.get(*id, 1).expect("retained texture")
            ));
        }
        assert_eq!(cache.used_bytes(), 8);
    }
    assert_eq!(CONVERSIONS.get(), 0);
}

#[test]
fn snapshot_budget_changes_release_and_restore_quiet_images() {
    let images: Vec<_> = (1..=3).map(|id| image(id, 1, 1)).collect();
    let mut cache = GraphicsCache::with_budget(12);
    cache.refresh(&images);
    cache.set_budget(4);
    assert_eq!(cache.used_bytes(), 4);
    let (survivor_id, survivor) = images
        .iter()
        .find_map(|image| cache.get(image.id, 1).map(|texture| (image.id, texture)))
        .expect("one retained image");
    CONVERSIONS.set(0);
    cache.refresh(&images);
    assert!(Arc::ptr_eq(&survivor, &cache.get(survivor_id, 1).unwrap()));
    assert_eq!(CONVERSIONS.get(), 0);
    cache.set_budget(12);
    cache.refresh(&images);
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.used_bytes(), 12);
    assert_eq!(CONVERSIONS.get(), 2);
    assert!(Arc::ptr_eq(&survivor, &cache.get(survivor_id, 1).unwrap()));
    cache.set_budget(0);
    cache.refresh(&images);
    assert!(cache.is_empty());
    assert_eq!(cache.used_bytes(), 0);
    assert_eq!(CONVERSIONS.get(), 2);
}

#[test]
fn snapshot_content_changes_drop_stale_generations_even_when_refused() {
    let mut images: Vec<_> = (1..=3).map(|id| image(id, 1, 1)).collect();
    let mut cache = GraphicsCache::with_budget(8);
    cache.refresh(&images);
    let changed_id = images
        .iter()
        .find(|image| cache.get(image.id, 1).is_some())
        .unwrap()
        .id;
    let changed_index = images
        .iter()
        .position(|image| image.id == changed_id)
        .unwrap();
    let first = cache.get(changed_id, 1).unwrap();
    images[changed_index] = image(changed_id, 2, 1);
    cache.refresh(&images);
    assert!(cache.get(changed_id, 1).is_none());
    let current = cache.get(changed_id, 2).expect("new content admitted");
    assert!(!Arc::ptr_eq(&first, &current));
    images[changed_index] = image(changed_id, 3, 2);
    cache.refresh(&images);
    assert!(cache.get(changed_id, 1).is_none());
    assert!(cache.get(changed_id, 2).is_none());
    assert!(cache.get(changed_id, 3).is_none());
    assert!(cache.used_bytes() <= 8);
    cache.refresh(&[]);
    assert!(cache.is_empty());
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn snapshot_refusals_do_not_convert_or_retain_old_content() {
    let mut cache = GraphicsCache::with_budget(8);
    let kept = image(2, 1, 1);
    let old = cache.texture(&kept).unwrap();
    let mut invalid = image(3, 1, 1);
    Arc::get_mut(&mut invalid).unwrap().pixels.clear();
    let images = [image(1, 1, 2), kept, invalid, image(4, 1, 1)];
    CONVERSIONS.set(0);
    assert_eq!(cache.refresh(&images), vec![1, 3]);
    assert_eq!(CONVERSIONS.get(), 1);
    assert!(Arc::ptr_eq(&old, &cache.get(2, 1).unwrap()));
    assert_eq!(cache.used_bytes(), 8);
    assert_eq!(cache.refresh(&images), vec![1, 3]);
    assert_eq!(CONVERSIONS.get(), 1);
    let mut invalid = image(2, 2, 1);
    Arc::get_mut(&mut invalid).unwrap().pixels.clear();
    assert_eq!(cache.refresh(&[invalid]), vec![2]);
    assert!(cache.get(2, 1).is_none());
    assert!(cache.is_empty());
    assert_eq!(cache.used_bytes(), 0);
    assert_eq!(CONVERSIONS.get(), 1);
}

#[test]
fn changing_snapshot_images_keeps_storage_bounded() {
    let mut cache = GraphicsCache::with_budget(8);
    for id in 0..128 {
        let images = [image(id, 1, 1), image(id + 1, 1, 1), image(u32::MAX, 1, 1)];
        cache.refresh(&images);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used_bytes(), 8);
    }
    cache.set_budget(0);
    assert!(cache.is_empty());
    assert_eq!(cache.used_bytes(), 0);
}
