//! Sprite's PNG decoder for the Kitty graphics protocol.
//!
//! **Why Sprite has its own.** `libghostty-vt` ships a `RustPngDecoder` behind
//! its `png` feature, and it cannot be used: the struct has a private field and
//! neither a constructor nor a `Default`, so nothing outside that crate can
//! build one. Its `decode_png` is also wrong — it reserves buffer *capacity*
//! and never sets the buffer's length, then hands `next_frame` a zero-length
//! slice — so it would decode nothing even if it could be constructed. Both are
//! recorded in `DEPENDENCIES.md`, and both are worth reporting upstream.
//!
//! **This is a parser for hostile input.** Every byte reaching it was printed by
//! an arbitrary child. It therefore refuses more than it accepts: anything
//! larger than the pane's own storage limit is rejected before a buffer is
//! allocated for it, and every failure is a `None` rather than a panic.

use libghostty_vt::alloc::{Allocator, Bytes};
use libghostty_vt::kitty::graphics::{DecodePng, DecodedImage};

/// The decoder a pane that shows no images installs.
///
/// Installed rather than clearing the decoder, because clearing is not the
/// thread-local act it looks like. `set_png_decoder(None)` writes the
/// thread-local *and* calls `ghostty_sys_set(GHOSTTY_SYS_OPT_DECODE_PNG, null)`,
/// which is a library-wide option: one disabled pane would turn PNG decoding
/// off for every other pane in the process. Refusing here leaves that callback
/// registered for the panes that want it, while this thread declines — which is
/// what a disabled pane means.
///
/// The defence is unchanged. No PNG parser runs: this returns before looking at
/// a byte, and the pane's storage limit is zero besides.
pub(crate) struct RefusingDecoder;

impl DecodePng for RefusingDecoder {
    fn decode_png<'alloc>(
        &mut self,
        _alloc: &'alloc Allocator<'_>,
        _data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        None
    }
}

/// Decodes PNG transmissions into the RGBA pixels libghostty expects.
pub(crate) struct PngDecoder {
    /// The most decoded bytes this decoder will produce for one image.
    ///
    /// Matches the pane's storage limit: an image too large to be *kept* should
    /// never be decoded, because decoding is where the memory is actually spent.
    limit: usize,
    /// Reused between images so a pane showing many does not reallocate for
    /// each one.
    buffer: Vec<u8>,
}

impl PngDecoder {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            buffer: Vec::new(),
        }
    }
}

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        // libghostty accepts RGBA8 only, so paletted and grayscale images are
        // expanded and 16-bit channels are reduced. Without this a legitimate
        // PNG in any other form would be rejected rather than shown.
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);

        let mut reader = decoder.read_info().ok()?;
        let needed = reader.output_buffer_size()?;

        // Checked against the limit *before* allocating: the declared size of a
        // PNG is attacker-controlled, and a decoder that allocates first and
        // checks afterwards is a decoder that can be asked for a gigabyte.
        if needed == 0 || needed > self.limit {
            return None;
        }

        // `resize`, not `reserve`: `next_frame` writes into the slice the
        // buffer's *length* describes, and a reserved-but-empty buffer is a
        // zero-length slice. This is the upstream bug this decoder exists to
        // avoid repeating.
        self.buffer.resize(needed, 0);
        let info = reader.next_frame(&mut self.buffer).ok()?;

        let produced = info.buffer_size();
        if produced == 0 || produced > self.limit || produced > self.buffer.len() {
            return None;
        }

        // The buffer must come from libghostty's allocator: it takes ownership
        // and frees it with the same allocator.
        let mut bytes = Bytes::new_with_alloc(alloc, produced).ok()?;
        bytes.copy_from_slice(&self.buffer[..produced]);
        reader.finish().ok()?;

        Some(DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PNG, built by the same crate that reads it back.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("write the header");
            let pixels = vec![0x80_u8; (width * height * 4) as usize];
            writer.write_image_data(&pixels).expect("write the pixels");
        }
        out
    }

    /// Decodes with a real libghostty allocator, since the decoder must produce
    /// its buffer from one.
    fn decode(decoder: &mut PngDecoder, data: &[u8]) -> Option<(u32, u32, usize)> {
        let allocator = Allocator::GLOBAL;
        decoder
            .decode_png(&allocator, data)
            .map(|image| (image.width, image.height, image.data.len()))
    }

    #[test]
    fn a_png_decodes_to_rgba_pixels() {
        let mut decoder = PngDecoder::new(1024 * 1024);
        let decoded = decode(&mut decoder, &png_bytes(4, 3)).expect("a valid PNG decodes");

        assert_eq!(decoded, (4, 3, 4 * 3 * 4), "four bytes a pixel");
    }

    /// The defect that made the upstream decoder useless: a buffer reserved but
    /// never resized is a zero-length slice, and every image fails.
    #[test]
    fn decoding_twice_works_as_well_as_once() {
        let mut decoder = PngDecoder::new(1024 * 1024);

        assert!(decode(&mut decoder, &png_bytes(2, 2)).is_some());
        assert_eq!(
            decode(&mut decoder, &png_bytes(8, 8)),
            Some((8, 8, 8 * 8 * 4)),
            "a reused buffer grows for a larger image"
        );
        assert_eq!(
            decode(&mut decoder, &png_bytes(1, 1)),
            Some((1, 1, 4)),
            "and reports the right length for a smaller one afterwards"
        );
    }

    #[test]
    fn an_image_larger_than_the_limit_is_refused_before_it_is_decoded() {
        // Room for far less than a 64x64 RGBA image.
        let mut decoder = PngDecoder::new(1024);
        assert!(decode(&mut decoder, &png_bytes(64, 64)).is_none());
        assert!(
            decoder.buffer.is_empty(),
            "refused before a buffer was allocated for it"
        );

        // The same decoder still accepts something that fits, so the limit
        // refuses an image rather than disabling the decoder.
        assert!(decode(&mut decoder, &png_bytes(4, 4)).is_some());
    }

    /// Hostile input must produce `None`, never a panic: this runs on the
    /// thread that owns the terminal, and a panic there takes the pane with it.
    #[test]
    fn malformed_input_is_refused_without_panicking() {
        let mut decoder = PngDecoder::new(1024 * 1024);
        let valid = png_bytes(4, 4);

        for (name, bytes) in [
            ("empty", Vec::new()),
            ("not a png", b"this is not a png at all".to_vec()),
            ("truncated header", valid[..8].to_vec()),
            ("truncated body", valid[..valid.len() / 2].to_vec()),
            ("header only", valid[..valid.len().min(33)].to_vec()),
            (
                "trailing garbage removed",
                valid[..valid.len() - 4].to_vec(),
            ),
            ("one byte", vec![0x89]),
            ("all zeroes", vec![0_u8; 128]),
        ] {
            assert!(
                decode(&mut decoder, &bytes).is_none(),
                "{name} must be refused"
            );
        }

        // And the decoder still works afterwards.
        assert!(decode(&mut decoder, &valid).is_some());
    }

    /// A PNG whose header claims a size it does not deliver.
    #[test]
    fn a_png_that_lies_about_its_size_is_refused() {
        let mut valid = png_bytes(4, 4);
        // The IHDR width lives at a fixed offset; claiming a much larger image
        // leaves the data too short for what the header promises.
        valid[16..20].copy_from_slice(&40_000_u32.to_be_bytes());

        let mut decoder = PngDecoder::new(1024 * 1024);
        assert!(decode(&mut decoder, &valid).is_none());
    }
}
