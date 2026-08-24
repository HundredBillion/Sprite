//! What a pane is holding in the way of images.
//!
//! Checkpoint 4 Task 1 needs this in order to *test* its own limits: "the file
//! was never read" and "the storage limit is what is enforced" are claims about
//! what is stored, and a claim about storage that nothing can observe is not a
//! claim anyone should believe. So this is the smallest projection that makes
//! those assertions possible — identities, sizes, and placements, and no pixels
//! at all.
//!
//! Task 3 extends it with decoded pixels and full placement geometry. Nothing
//! here copies image data, which is why a pane with images still pays nothing
//! for a capture that does not ask for them.

use std::sync::Arc;

use libghostty_vt::Terminal;
use libghostty_vt::kitty::graphics::PlacementIterator;

use crate::SessionError;

/// One image the terminal is holding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageSummary {
    pub id: u32,
    /// Changes when the image's content changes, which is half of its identity.
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    /// How many bytes of image data the terminal is holding for it.
    pub byte_len: usize,
}

/// One place an image is shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementSummary {
    pub image: u32,
    pub placement: u32,
    /// Virtual placements are addressed by text rather than drawn directly.
    pub is_virtual: bool,
}

/// What one pane is holding, without any of the image data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphicsSnapshot {
    /// The image storage's generation, which advances as images change.
    pub generation: u64,
    pub images: Vec<ImageSummary>,
    pub placements: Vec<PlacementSummary>,
}

impl GraphicsSnapshot {
    /// Whether the terminal is holding an image under this id.
    pub fn holds(&self, id: u32) -> bool {
        self.images.iter().any(|image| image.id == id)
    }

    /// The bytes of image data the terminal is holding in total.
    pub fn stored_bytes(&self) -> usize {
        self.images.iter().map(|image| image.byte_len).sum()
    }
}

/// Where a placement sits relative to the cell's background and its text.
///
/// Ghostty classifies the protocol's signed z-index into three bands; Sprite
/// draws in this order rather than sorting by a raw number.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Layer {
    /// Behind the cell background, so text and background both cover it.
    BelowBackground,
    /// Over the background but under the text.
    #[default]
    BelowText,
    /// Over the text.
    AboveText,
}

/// One image's pixels, owned.
///
/// Held behind an `Arc` and shared between captures: the pixels are copied once
/// per image *generation*, not once per frame. A still image on screen through
/// a thousand frames is copied once.
#[derive(Debug, Eq, PartialEq)]
pub struct ImagePixels {
    pub id: u32,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    /// The format the image was transmitted in, before decoding.
    pub transmitted: TransmittedFormat,
    /// The stored pixels, exactly as libghostty holds them.
    pub pixels: Vec<u8>,
}

impl ImagePixels {
    /// Bytes per pixel as stored, derived rather than assumed.
    pub fn bytes_per_pixel(&self) -> usize {
        let pixels = (self.width as usize) * (self.height as usize);
        if pixels == 0 {
            return 0;
        }
        self.pixels.len() / pixels
    }
}

/// How an image arrived, which is not necessarily how it is stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransmittedFormat {
    Rgb,
    #[default]
    Rgba,
    Png,
    Gray,
    GrayAlpha,
    /// A format this version of Sprite does not know.
    ///
    /// The binding's format enum can gain variants, and a projection that
    /// guessed would be reporting a fact it does not have. The pixels are still
    /// carried; only the name of how they arrived is unknown.
    Unknown,
}

/// One placement, with everything needed to draw it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placement {
    pub image: u32,
    pub placement: u32,
    /// Virtual placements are addressed by text rather than drawn directly, so
    /// a renderer skips them.
    pub is_virtual: bool,
    pub layer: Layer,
    /// The part of the image to draw, in image pixels, already resolved and
    /// clamped by libghostty.
    pub source: Rectangle,
    /// Rendered size in pixels.
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// Cells the placement occupies.
    pub columns: u32,
    pub rows: u32,
    /// Where the placement's top-left cell sits relative to the viewport.
    /// Negative when it is partly scrolled off the top or left.
    pub viewport_column: i32,
    pub viewport_row: i32,
    /// False when the placement is entirely off screen, or virtual.
    pub visible: bool,
    /// Pixel offsets within the first cell.
    pub x_offset: u32,
    pub y_offset: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rectangle {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// What an observer may learn about an image.
///
/// **Metadata only, and deliberately no way to reach the picture.** The
/// observation exclusion list bans transmitted bytes, decoded pixels, and
/// source filenames; this type carries none of them and has no field that could
/// hold one. A client learns that an image occupies terminal space, how much of
/// it, and in what order it draws — enough to understand a screen, and nothing
/// that reproduces the image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementMetadata {
    pub image: u32,
    pub placement: u32,
    pub is_virtual: bool,
    pub layer: Layer,
    /// How the image arrived, which is a fact about the protocol rather than
    /// about the picture.
    pub format: TransmittedFormat,
    /// The image's own pixel dimensions.
    pub image_width: u32,
    pub image_height: u32,
    /// The cells the placement covers.
    pub columns: u32,
    pub rows: u32,
    pub viewport_column: i32,
    pub viewport_row: i32,
    pub visible: bool,
}

/// Everything a renderer needs to draw one pane's images.
#[derive(Debug, Default)]
pub struct GraphicsFrame {
    /// The image storage's generation when this was taken.
    pub generation: u64,
    pub images: Vec<Arc<ImagePixels>>,
    pub placements: Vec<Placement>,
}

impl GraphicsFrame {
    pub fn image(&self, id: u32) -> Option<&Arc<ImagePixels>> {
        self.images.iter().find(|image| image.id == id)
    }
}

impl PartialEq for GraphicsFrame {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.placements == other.placements
            && self.images.len() == other.images.len()
            && self
                .images
                .iter()
                .zip(other.images.iter())
                .all(|(left, right)| Arc::ptr_eq(left, right) || left == right)
    }
}

impl Eq for GraphicsFrame {}

/// Pixels already copied, so a still image is not copied again every frame.
#[derive(Default)]
pub(crate) struct PixelCache {
    images: std::collections::HashMap<u32, Arc<ImagePixels>>,
}

/// Reads what the terminal is holding.
///
/// Images are discovered through their placements, because the binding offers
/// no way to enumerate storage directly. An image that was transmitted but
/// never placed is therefore invisible here — which is worth knowing when
/// reading a test, and is why the tests place what they transmit.
pub(crate) fn capture_graphics(
    terminal: &Terminal<'_, '_>,
    placements: &mut PlacementIterator<'_>,
) -> Result<Arc<GraphicsSnapshot>, SessionError> {
    let vt = |what: &'static str| move |error| SessionError::new(what, error);

    let graphics = terminal.kitty_graphics().map_err(vt("kitty_graphics"))?;
    let generation = graphics.generation().map_err(vt("graphics_generation"))?;

    let mut images: Vec<ImageSummary> = Vec::new();
    let mut placed: Vec<PlacementSummary> = Vec::new();

    {
        let mut iteration = placements
            .update(&graphics)
            .map_err(vt("placement_iterator"))?;
        while iteration.next().is_some() {
            let image_id = iteration.image_id().map_err(vt("placement_image_id"))?;
            placed.push(PlacementSummary {
                image: image_id,
                placement: iteration.placement_id().map_err(vt("placement_id"))?,
                is_virtual: iteration.is_virtual().map_err(vt("placement_virtual"))?,
            });

            if images.iter().any(|image| image.id == image_id) {
                continue;
            }
            let Some(image) = graphics.image(image_id) else {
                continue;
            };
            images.push(ImageSummary {
                id: image_id,
                generation: image.generation().map_err(vt("image_generation"))?,
                width: image.width().map_err(vt("image_width"))?,
                height: image.height().map_err(vt("image_height"))?,
                // The length only. Copying the data is Task 3's job, and doing
                // it here would make every probe as expensive as a decode.
                byte_len: image.data().map_err(vt("image_data"))?.len(),
            });
        }
    }

    images.sort_by_key(|image| image.id);
    placed.sort_by_key(|placement| (placement.image, placement.placement));

    Ok(Arc::new(GraphicsSnapshot {
        generation,
        images,
        placements: placed,
    }))
}

/// Builds the owned frame a renderer draws from.
///
/// Returns `None` when the pane has no images to show, which is the common
/// case and is deliberately close to free: the storage generation is one call,
/// and a pane that never received an image stops there.
///
/// Every value is copied out before this returns, so nothing the renderer holds
/// borrows from the terminal — the same rule the text projection follows, for
/// the same reason: the next byte of child output may move or free any of it.
pub(crate) fn capture_frame(
    terminal: &Terminal<'_, '_>,
    placements: &mut PlacementIterator<'_>,
    cache: &mut PixelCache,
) -> Result<Option<Arc<GraphicsFrame>>, SessionError> {
    let vt = |what: &'static str| move |error| SessionError::new(what, error);

    let graphics = terminal.kitty_graphics().map_err(vt("kitty_graphics"))?;
    let generation = graphics.generation().map_err(vt("graphics_generation"))?;
    if generation == 0 && cache.images.is_empty() {
        // Nothing has ever been transmitted to this pane. One call, no
        // allocation, no iteration: a pane showing text pays this and no more.
        return Ok(None);
    }

    let mut found: Vec<Placement> = Vec::new();

    // Three passes, one per layer band, because the binding classifies
    // placements by filtering rather than by reporting a z-index. Each pass is
    // proportional to the number of placements, never to cells on screen.
    for layer in [
        (
            libghostty_vt::kitty::graphics::Layer::BelowBg,
            Layer::BelowBackground,
        ),
        (
            libghostty_vt::kitty::graphics::Layer::BelowText,
            Layer::BelowText,
        ),
        (
            libghostty_vt::kitty::graphics::Layer::AboveText,
            Layer::AboveText,
        ),
    ] {
        let mut iteration = placements
            .update(&graphics)
            .map_err(vt("placement_iterator"))?;
        iteration
            .set_layer(layer.0)
            .map_err(vt("placement_layer"))?;

        while iteration.next().is_some() {
            let image_id = iteration.image_id().map_err(vt("placement_image_id"))?;
            let Some(image) = graphics.image(image_id) else {
                // A placement whose image has gone is not drawable.
                continue;
            };

            // One call for pixel size, grid size, viewport position and source
            // rectangle together. The binding offers four separate calls and
            // says outright that this exists to avoid them, which matters here
            // because this runs on every capture.
            let info = iteration
                .placement_render_info(&image, terminal)
                .map_err(vt("placement_render_info"))?;

            found.push(Placement {
                image: image_id,
                placement: iteration.placement_id().map_err(vt("placement_id"))?,
                is_virtual: iteration.is_virtual().map_err(vt("placement_virtual"))?,
                layer: layer.1,
                source: Rectangle {
                    x: info.source_x,
                    y: info.source_y,
                    width: info.source_width,
                    height: info.source_height,
                },
                pixel_width: info.pixel_width,
                pixel_height: info.pixel_height,
                columns: info.grid_cols,
                rows: info.grid_rows,
                viewport_column: info.viewport_col,
                viewport_row: info.viewport_row,
                visible: info.viewport_visible,
                x_offset: iteration.x_offset().map_err(vt("placement_x_offset"))?,
                y_offset: iteration.y_offset().map_err(vt("placement_y_offset"))?,
            });
        }
    }

    if found.is_empty() {
        // Images may still be stored, but none is on screen. Holding their
        // pixels for a pane that is not showing them is memory spent on
        // nothing.
        cache.images.clear();
        return Ok(None);
    }

    let mut images: Vec<Arc<ImagePixels>> = Vec::new();
    for placement in &found {
        if images.iter().any(|image| image.id == placement.image) {
            continue;
        }
        let Some(image) = graphics.image(placement.image) else {
            continue;
        };
        let image_generation = image.generation().map_err(vt("image_generation"))?;

        // The heart of this task: pixels are copied when an image's generation
        // is new, and shared by reference otherwise. A still image on screen
        // through a thousand frames is copied once.
        let cached = match cache.images.get(&placement.image) {
            Some(existing) if existing.generation == image_generation => Arc::clone(existing),
            _ => {
                let pixels = Arc::new(ImagePixels {
                    id: placement.image,
                    generation: image_generation,
                    width: image.width().map_err(vt("image_width"))?,
                    height: image.height().map_err(vt("image_height"))?,
                    transmitted: match image.format().map_err(vt("image_format"))? {
                        libghostty_vt::kitty::graphics::ImageFormat::Rgb => TransmittedFormat::Rgb,
                        libghostty_vt::kitty::graphics::ImageFormat::Rgba => {
                            TransmittedFormat::Rgba
                        }
                        libghostty_vt::kitty::graphics::ImageFormat::Png => TransmittedFormat::Png,
                        libghostty_vt::kitty::graphics::ImageFormat::Gray => {
                            TransmittedFormat::Gray
                        }
                        libghostty_vt::kitty::graphics::ImageFormat::GrayAlpha => {
                            TransmittedFormat::GrayAlpha
                        }
                        _ => TransmittedFormat::Unknown,
                    },
                    pixels: image.data().map_err(vt("image_data"))?.to_vec(),
                });
                cache.images.insert(placement.image, Arc::clone(&pixels));
                pixels
            }
        };
        images.push(cached);
    }

    // Anything no longer placed stops being held.
    cache
        .images
        .retain(|id, _| images.iter().any(|image| image.id == *id));

    images.sort_by_key(|image| image.id);
    found.sort_by_key(|placement| (placement.layer, placement.image, placement.placement));

    Ok(Some(Arc::new(GraphicsFrame {
        generation,
        images,
        placements: found,
    })))
}

/// Reads placement metadata for the observation path.
///
/// Never touches `Image::data`, so no pixel can reach an answer even by
/// accident: the bytes are not read, not copied, and not reachable from the
/// value this returns.
pub(crate) fn capture_placements(
    terminal: &Terminal<'_, '_>,
    placements: &mut PlacementIterator<'_>,
) -> Result<Vec<PlacementMetadata>, SessionError> {
    let vt = |what: &'static str| move |error| SessionError::new(what, error);

    let graphics = terminal.kitty_graphics().map_err(vt("kitty_graphics"))?;
    let mut found = Vec::new();

    for layer in [
        (
            libghostty_vt::kitty::graphics::Layer::BelowBg,
            Layer::BelowBackground,
        ),
        (
            libghostty_vt::kitty::graphics::Layer::BelowText,
            Layer::BelowText,
        ),
        (
            libghostty_vt::kitty::graphics::Layer::AboveText,
            Layer::AboveText,
        ),
    ] {
        let mut iteration = placements
            .update(&graphics)
            .map_err(vt("placement_iterator"))?;
        iteration
            .set_layer(layer.0)
            .map_err(vt("placement_layer"))?;

        while iteration.next().is_some() {
            let image_id = iteration.image_id().map_err(vt("placement_image_id"))?;
            let Some(image) = graphics.image(image_id) else {
                continue;
            };
            let info = iteration
                .placement_render_info(&image, terminal)
                .map_err(vt("placement_render_info"))?;

            found.push(PlacementMetadata {
                image: image_id,
                placement: iteration.placement_id().map_err(vt("placement_id"))?,
                is_virtual: iteration.is_virtual().map_err(vt("placement_virtual"))?,
                layer: layer.1,
                format: match image.format().map_err(vt("image_format"))? {
                    libghostty_vt::kitty::graphics::ImageFormat::Rgb => TransmittedFormat::Rgb,
                    libghostty_vt::kitty::graphics::ImageFormat::Rgba => TransmittedFormat::Rgba,
                    libghostty_vt::kitty::graphics::ImageFormat::Png => TransmittedFormat::Png,
                    libghostty_vt::kitty::graphics::ImageFormat::Gray => TransmittedFormat::Gray,
                    libghostty_vt::kitty::graphics::ImageFormat::GrayAlpha => {
                        TransmittedFormat::GrayAlpha
                    }
                    _ => TransmittedFormat::Unknown,
                },
                image_width: image.width().map_err(vt("image_width"))?,
                image_height: image.height().map_err(vt("image_height"))?,
                columns: info.grid_cols,
                rows: info.grid_rows,
                viewport_column: info.viewport_col,
                viewport_row: info.viewport_row,
                visible: info.viewport_visible,
            });
        }
    }

    found.sort_by_key(|placement| (placement.layer, placement.image, placement.placement));
    Ok(found)
}
