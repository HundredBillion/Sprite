//! Textures for the images a pane is showing.
//!
//! **Why Sprite keeps its own cache.** GPUI's `RenderImage` mints a fresh
//! identity on every construction, so its own identity says nothing about
//! whether two textures hold the same picture. Sprite therefore keys on the
//! terminal's identity — image id plus content generation — which is the pair
//! that actually changes when a picture changes.
//!
//! **And why the conversion lives here.** GPUI wants BGRA; libghostty produces
//! RGBA. Converting at the point of caching means it happens once per image
//! generation rather than once per frame, which for a still image on screen is
//! the difference between one conversion and sixty a second.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::RenderImage;
use sprite_term::ImagePixels;

/// How much decoded image a pane may hold on the GPU side.
///
/// Independent of the terminal's own storage limit, as the PRD requires: the
/// terminal holds what a program transmitted, and this holds what is actually
/// being drawn. They bound different things and are exceeded at different
/// times.
pub const DEFAULT_BUDGET_BYTES: usize = 128 * 1024 * 1024;

struct Entry {
    generation: u64,
    texture: Arc<RenderImage>,
    bytes: usize,
    /// When this was last asked for, so the least recently *placed* image is
    /// the one evicted — not the least recently transmitted.
    last_used: u64,
}

/// One pane's textures.
pub struct GraphicsCache {
    entries: HashMap<u32, Entry>,
    budget: usize,
    used: usize,
    clock: u64,
}

impl Default for GraphicsCache {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET_BYTES)
    }
}

impl GraphicsCache {
    pub fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            used: 0,
            clock: 0,
        }
    }

    /// The texture for these pixels, converting only if this is new content.
    ///
    /// Returns `None` when the image cannot be represented — a size that does
    /// not match its pixels, or one too large for the whole budget. A pane
    /// missing one image still draws its text and its other images.
    pub fn texture(&mut self, pixels: &ImagePixels) -> Option<Arc<RenderImage>> {
        self.clock += 1;
        let now = self.clock;

        if let Some(entry) = self.entries.get_mut(&pixels.id)
            && entry.generation == pixels.generation
        {
            entry.last_used = now;
            return Some(Arc::clone(&entry.texture));
        }

        let bgra = to_bgra(pixels)?;
        let bytes = bgra.len();
        if bytes > self.budget {
            // Refused rather than admitted and then evicting everything else to
            // make room for something that still would not fit.
            return None;
        }

        let buffer = image::RgbaImage::from_raw(pixels.width, pixels.height, bgra)?;
        let texture = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));

        // Replacing an entry releases the old texture, which is what makes a
        // generation change reclaim rather than accumulate.
        if let Some(previous) = self.entries.remove(&pixels.id) {
            self.used = self.used.saturating_sub(previous.bytes);
        }
        self.make_room(bytes, pixels.id);

        self.entries.insert(
            pixels.id,
            Entry {
                generation: pixels.generation,
                texture: Arc::clone(&texture),
                bytes,
                last_used: now,
            },
        );
        self.used += bytes;
        Some(texture)
    }

    /// The texture already built for this image, if it is the current content.
    ///
    /// Drawing looks up rather than builds: textures are made when a snapshot
    /// arrives, so a frame that asked to build one would be converting during
    /// paint.
    pub fn get(&self, id: u32, generation: u64) -> Option<Arc<RenderImage>> {
        self.entries
            .get(&id)
            .filter(|entry| entry.generation == generation)
            .map(|entry| Arc::clone(&entry.texture))
    }

    /// Drops every texture whose image is no longer being shown.
    ///
    /// Called with the images of the current frame, so a pane that has shown a
    /// thousand images over an hour holds textures for the few on screen.
    pub fn retain(&mut self, shown: &[u32]) {
        self.entries.retain(|id, entry| {
            let keep = shown.contains(id);
            if !keep {
                self.used = self.used.saturating_sub(entry.bytes);
            }
            keep
        });
    }

    /// Releases everything, for a pane that is closing.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Changes the budget, evicting immediately if the new one is smaller.
    ///
    /// Immediately rather than at the next image: a person who lowers the limit
    /// has asked for the memory back now, and a pane showing nothing new would
    /// otherwise hold the old textures until it did.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        // `keep` names an image that must survive; nothing is being admitted
        // here, so no id is exempt. Zero is not special-cased because an image
        // id of zero is a real id — it is simply as evictable as any other.
        self.make_room(0, u32::MAX);
    }

    /// Evicts least-recently-placed entries until `wanted` bytes will fit.
    fn make_room(&mut self, wanted: usize, keep: u32) {
        while self.used + wanted > self.budget {
            let Some(victim) = self
                .entries
                .iter()
                .filter(|(id, _)| **id != keep)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| *id)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.used = self.used.saturating_sub(entry.bytes);
            }
        }
    }
}

/// Converts what libghostty stored into what GPUI draws.
///
/// Every branch produces four bytes a pixel in blue, green, red, alpha order.
/// Getting this wrong swaps red and blue, which looks like a plausible picture
/// and is therefore exactly the kind of defect that survives a review.
fn to_bgra(pixels: &ImagePixels) -> Option<Vec<u8>> {
    let width = pixels.width as usize;
    let height = pixels.height as usize;
    let count = width.checked_mul(height)?;
    if count == 0 {
        return None;
    }
    let stride = pixels.bytes_per_pixel();
    // A declared size that disagrees with the data is not something to guess
    // about: the pixels came from a program that can print anything.
    if stride == 0 || count.checked_mul(stride)? != pixels.pixels.len() {
        return None;
    }

    let mut out = Vec::with_capacity(count * 4);
    for chunk in pixels.pixels.chunks_exact(stride) {
        let (red, green, blue, alpha) = match stride {
            4 => (chunk[0], chunk[1], chunk[2], chunk[3]),
            3 => (chunk[0], chunk[1], chunk[2], 0xff),
            2 => (chunk[0], chunk[0], chunk[0], chunk[1]),
            1 => (chunk[0], chunk[0], chunk[0], 0xff),
            _ => return None,
        };
        out.extend_from_slice(&[blue, green, red, alpha]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sprite_term::TransmittedFormat;

    fn image(id: u32, generation: u64, size: u32, value: u8) -> ImagePixels {
        ImagePixels {
            id,
            generation,
            width: size,
            height: size,
            transmitted: TransmittedFormat::Rgba,
            pixels: vec![value; (size * size * 4) as usize],
        }
    }

    /// A red/blue swap produces a picture that looks fine at a glance, so the
    /// channel order is asserted against a pixel whose channels all differ.
    #[test]
    fn pixels_reach_the_gpu_in_blue_green_red_alpha_order() {
        let pixels = ImagePixels {
            id: 1,
            generation: 1,
            width: 1,
            height: 1,
            transmitted: TransmittedFormat::Rgba,
            pixels: vec![0x10, 0x20, 0x30, 0x40],
        };

        assert_eq!(
            to_bgra(&pixels).expect("converted"),
            vec![0x30, 0x20, 0x10, 0x40],
            "blue, green, red, alpha — not the order it arrived in"
        );
    }

    #[test]
    fn stored_formats_other_than_rgba_are_expanded() {
        let rgb = ImagePixels {
            id: 1,
            generation: 1,
            width: 1,
            height: 1,
            transmitted: TransmittedFormat::Rgb,
            pixels: vec![0x10, 0x20, 0x30],
        };
        assert_eq!(
            to_bgra(&rgb).expect("converted"),
            vec![0x30, 0x20, 0x10, 0xff],
            "opaque, since no alpha was sent"
        );

        let gray = ImagePixels {
            id: 2,
            generation: 1,
            width: 1,
            height: 1,
            transmitted: TransmittedFormat::Gray,
            pixels: vec![0x77],
        };
        assert_eq!(
            to_bgra(&gray).expect("converted"),
            vec![0x77, 0x77, 0x77, 0xff]
        );

        let gray_alpha = ImagePixels {
            id: 3,
            generation: 1,
            width: 1,
            height: 1,
            transmitted: TransmittedFormat::GrayAlpha,
            pixels: vec![0x55, 0x80],
        };
        assert_eq!(
            to_bgra(&gray_alpha).expect("converted"),
            vec![0x55, 0x55, 0x55, 0x80]
        );
    }

    #[test]
    fn pixels_that_do_not_match_their_declared_size_are_refused() {
        let lying = ImagePixels {
            id: 1,
            generation: 1,
            width: 8,
            height: 8,
            transmitted: TransmittedFormat::Rgba,
            pixels: vec![0; 12],
        };
        assert!(to_bgra(&lying).is_none());

        let empty = ImagePixels {
            id: 2,
            generation: 1,
            width: 0,
            height: 0,
            transmitted: TransmittedFormat::Rgba,
            pixels: Vec::new(),
        };
        assert!(to_bgra(&empty).is_none());
    }

    /// The point of the cache: one conversion per generation, however many
    /// frames ask for it.
    #[test]
    fn a_still_image_is_converted_once_however_often_it_is_drawn() {
        let mut cache = GraphicsCache::default();
        let pixels = image(1, 7, 4, 0x40);

        let first = cache.texture(&pixels).expect("a texture");
        for _ in 0..10 {
            let again = cache.texture(&pixels).expect("a texture");
            assert!(
                Arc::ptr_eq(&first, &again),
                "the same texture, so nothing was converted again"
            );
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn new_content_under_one_id_replaces_the_texture_and_releases_the_old() {
        let mut cache = GraphicsCache::default();

        let first = cache.texture(&image(1, 1, 4, 0x10)).expect("a texture");
        let bytes_after_first = cache.used_bytes();
        let second = cache.texture(&image(1, 2, 4, 0x20)).expect("a texture");

        assert!(
            !Arc::ptr_eq(&first, &second),
            "a new generation is a new texture"
        );
        assert_eq!(
            cache.len(),
            1,
            "and the old one is gone, not kept beside it"
        );
        assert_eq!(
            cache.used_bytes(),
            bytes_after_first,
            "so the memory is reclaimed rather than doubled"
        );
    }

    /// A pane that has shown a thousand images holds textures for the few it is
    /// showing now.
    #[test]
    fn images_no_longer_shown_are_evicted() {
        let mut cache = GraphicsCache::default();
        for id in 0..1000 {
            cache.texture(&image(id, 1, 4, 0x30)).expect("a texture");
            // Each frame shows only the newest image.
            cache.retain(&[id]);
        }

        assert_eq!(cache.len(), 1, "one image on screen, one texture held");
        assert_eq!(cache.used_bytes(), 4 * 4 * 4);
    }

    #[test]
    fn closing_a_pane_releases_every_texture() {
        let mut cache = GraphicsCache::default();
        cache.texture(&image(1, 1, 8, 0x10));
        cache.texture(&image(2, 1, 8, 0x20));
        assert_eq!(cache.len(), 2);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.used_bytes(), 0);
    }

    /// A lowered limit is a request for the memory back now, not at whatever
    /// moment the pane happens to show its next image.
    #[test]
    fn lowering_the_budget_releases_textures_at_once() {
        let mut cache = GraphicsCache::with_budget(4096);
        cache.texture(&image(1, 1, 8, 0x10));
        cache.texture(&image(2, 1, 8, 0x20));
        let before = cache.used_bytes();
        assert!(before > 0, "two images are held");

        cache.set_budget(before / 2);
        assert!(
            cache.used_bytes() <= before / 2,
            "the memory is given back when the limit falls, not at the next image"
        );
    }

    /// The GPU-side ceiling is its own limit, and is enforced by evicting what
    /// was placed longest ago rather than whatever the map happens to yield.
    #[test]
    fn the_budget_evicts_the_least_recently_placed() {
        // Room for two 8x8 textures (256 bytes each) and not three.
        let mut cache = GraphicsCache::with_budget(600);

        cache.texture(&image(1, 1, 8, 0x10)).expect("a texture");
        cache.texture(&image(2, 1, 8, 0x20)).expect("a texture");
        // Touching image 1 makes image 2 the least recently placed.
        cache.texture(&image(1, 1, 8, 0x10)).expect("a texture");
        cache.texture(&image(3, 1, 8, 0x30)).expect("a texture");

        assert!(cache.used_bytes() <= 600, "inside the budget");
        assert_eq!(cache.len(), 2);
        assert!(
            cache.entries.contains_key(&1),
            "the recently drawn image survived"
        );
        assert!(
            !cache.entries.contains_key(&2),
            "and the one nobody had drawn for longest went"
        );
    }

    #[test]
    fn an_image_larger_than_the_whole_budget_is_refused_rather_than_emptying_it() {
        let mut cache = GraphicsCache::with_budget(1024);
        cache.texture(&image(1, 1, 8, 0x10)).expect("a texture");

        // 32x32 RGBA is 4 KiB: larger than the entire budget.
        assert!(
            cache.texture(&image(2, 1, 32, 0x20)).is_none(),
            "refused, because admitting it would evict everything and still not fit"
        );
        assert_eq!(
            cache.len(),
            1,
            "the image already being drawn was not sacrificed for it"
        );
    }
}
