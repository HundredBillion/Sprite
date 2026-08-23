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
