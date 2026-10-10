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
///
/// Holds no pixels between images: each decode allocates what that image
/// needs and releases it before returning, so one large image does not pin its
/// size for as long as the pane lives.
pub(crate) struct PngDecoder {
    /// The most decoded bytes this decoder will produce for one image.
    ///
    /// Matches the pane's storage limit: an image too large to be *kept* should
    /// never be decoded, because decoding is where the memory is actually spent.
    limit: usize,
}

impl PngDecoder {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
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
        // Palettes gain an alpha channel, low-depth grayscale widens to eight
        // bits and sixteen-bit channels are reduced. Grayscale still arrives as
        // gray and alpha, two bytes a pixel, so it is widened to RGBA below.
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);

        let mut reader = decoder.read_info().ok()?;
        let (width, height) = reader.info().size();

        // libghostty stores four bytes a pixel whatever the PNG held, so that
        // is what the limit is checked against, and *before* allocating: the
        // declared size of a PNG is attacker-controlled, and a decoder that
        // allocates first and checks afterwards can be asked for a gigabyte.
        let stored = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        let needed = reader.output_buffer_size()?;
        if stored == 0 || stored > self.limit || needed > stored {
            return None;
        }

        // Sized, not merely reserved: `next_frame` writes into the slice the
        // vector's *length* describes, and a reserved-but-empty vector is a
        // zero-length slice. This is the upstream bug this decoder exists to
        // avoid repeating.
        let mut decoded = vec![0_u8; needed];
        let info = reader.next_frame(&mut decoded).ok()?;
        reader.finish().ok()?;

        let produced = info.buffer_size();
        if info.bit_depth != png::BitDepth::Eight || produced == 0 || produced > decoded.len() {
            return None;
        }

        // The buffer must come from libghostty's allocator: it takes ownership
        // and frees it with the same allocator. It holds the four bytes a
        // pixel already checked against the limit; a frame that does not fill
        // exactly that many pixels is refused by `widen_to_rgba`.
        let mut bytes = Bytes::new_with_alloc(alloc, stored).ok()?;
        if !widen_to_rgba(info.color_type, &decoded[..produced], &mut bytes) {
            return None;
        }

        Some(DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

/// Writes eight-bit `samples` of `color` into `rgba`, four bytes a pixel.
///
/// Returns `false`, writing nothing useful, when the two do not describe the
/// same number of pixels or the colour type is not one the transformations
/// above can produce.
fn widen_to_rgba(color: png::ColorType, samples: &[u8], rgba: &mut [u8]) -> bool {
    let channels = match color {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        // `ALPHA` expands every palette, so an index here is not a colour.
        png::ColorType::Indexed => return false,
    };
    if !samples.len().is_multiple_of(channels) || samples.len() / channels != rgba.len() / 4 {
        return false;
    }
    for (pixel, out) in samples.chunks_exact(channels).zip(rgba.chunks_exact_mut(4)) {
        let (rgb, alpha) = match *pixel {
            [red, green, blue, alpha] => ([red, green, blue], alpha),
            [red, green, blue] => ([red, green, blue], u8::MAX),
            [gray, alpha] => ([gray; 3], alpha),
            [gray] => ([gray; 3], u8::MAX),
            _ => return false,
        };
        out[..3].copy_from_slice(&rgb);
        out[3] = alpha;
    }
    true
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

    /// A PNG of one colour type and depth, every pixel `samples`, built by the
    /// same crate that reads it back. Indexed images get a two-entry palette
    /// whose entry 1 is `10 20 30`.
    fn encoded(
        color: png::ColorType,
        depth: png::BitDepth,
        width: u32,
        height: u32,
        samples: &[u16],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if color == png::ColorType::Indexed {
                encoder.set_palette(vec![0, 0, 0, 0x10, 0x20, 0x30]);
            }
            let mut writer = encoder.write_header().expect("write the header");
            writer
                .write_image_data(&uniform_rows(width, height, depth, samples))
                .expect("write the pixels");
        }
        out
    }

    /// `width x height` copies of one pixel's samples at `depth` bits, each
    /// row starting on a byte boundary, as PNG lays them out.
    fn uniform_rows(width: u32, height: u32, depth: png::BitDepth, samples: &[u16]) -> Vec<u8> {
        let bits = depth as usize;
        let mut data = Vec::new();
        for _ in 0..height {
            let mut packed = 0_u16;
            let mut filled = 0;
            for _ in 0..width {
                for &sample in samples {
                    match bits {
                        16 => data.extend_from_slice(&sample.to_be_bytes()),
                        8 => data.push(sample as u8),
                        _ => {
                            packed = (packed << bits) | sample;
                            filled += bits;
                            if filled == 8 {
                                data.push(packed as u8);
                                packed = 0;
                                filled = 0;
                            }
                        }
                    }
                }
            }
            if filled > 0 {
                data.push((packed << (8 - filled)) as u8);
            }
        }
        data
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
            "a larger image after a smaller one decodes whole"
        );
        assert_eq!(
            decode(&mut decoder, &png_bytes(1, 1)),
            Some((1, 1, 4)),
            "and reports the right length for a smaller one afterwards"
        );
    }

    #[test]
    fn an_image_larger_than_the_limit_is_refused_before_it_is_decoded() {
        // Room for far less than a 256x256 RGBA image.
        let mut decoder = PngDecoder::new(1024);
        let oversized = png_bytes(256, 256);
        let (refused, sample) =
            crate::test_allocations::measure(|| decode(&mut decoder, &oversized));
        assert!(refused.is_none());
        assert!(
            sample.largest < 256 * 256 * 4,
            "refused before a buffer was allocated for it: {sample:?}"
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

    /// libghostty accepts RGBA8 only, so every colour type at every depth PNG
    /// allows must arrive as four bytes a pixel with the right values.
    #[test]
    fn every_colour_type_and_bit_depth_decodes_to_rgba8() {
        use png::BitDepth::{Eight, Four, One, Sixteen, Two};
        use png::ColorType::{Grayscale, GrayscaleAlpha, Indexed, Rgb, Rgba};
        let (width, height) = (3_u32, 2_u32);
        let cases: &[(png::ColorType, png::BitDepth, &[u16], [u8; 4])] = &[
            (Grayscale, One, &[1], [0xFF, 0xFF, 0xFF, 0xFF]),
            (Grayscale, Two, &[2], [0xAA, 0xAA, 0xAA, 0xFF]),
            (Grayscale, Four, &[8], [0x88, 0x88, 0x88, 0xFF]),
            (Grayscale, Eight, &[0x80], [0x80, 0x80, 0x80, 0xFF]),
            (Grayscale, Sixteen, &[0x8000], [0x80, 0x80, 0x80, 0xFF]),
            (Rgb, Eight, &[0x10, 0x20, 0x30], [0x10, 0x20, 0x30, 0xFF]),
            (
                Rgb,
                Sixteen,
                &[0x1000, 0x2000, 0x3000],
                [0x10, 0x20, 0x30, 0xFF],
            ),
            (Indexed, One, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Two, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Four, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (Indexed, Eight, &[1], [0x10, 0x20, 0x30, 0xFF]),
            (
                GrayscaleAlpha,
                Eight,
                &[0x80, 0x40],
                [0x80, 0x80, 0x80, 0x40],
            ),
            (
                GrayscaleAlpha,
                Sixteen,
                &[0x8000, 0x4000],
                [0x80, 0x80, 0x80, 0x40],
            ),
            (
                Rgba,
                Eight,
                &[0x10, 0x20, 0x30, 0x40],
                [0x10, 0x20, 0x30, 0x40],
            ),
            (
                Rgba,
                Sixteen,
                &[0x1000, 0x2000, 0x3000, 0x4000],
                [0x10, 0x20, 0x30, 0x40],
            ),
        ];
        let mut decoder = PngDecoder::new(1024 * 1024);
        let allocator = Allocator::GLOBAL;
        for &(color, depth, samples, rgba) in cases {
            let png = encoded(color, depth, width, height, samples);
            let image = decoder
                .decode_png(&allocator, &png)
                .unwrap_or_else(|| panic!("{color:?} at {depth:?} must decode"));
            assert_eq!(
                (image.width, image.height),
                (width, height),
                "{color:?} at {depth:?}"
            );
            assert_eq!(
                image.data.len(),
                (width * height * 4) as usize,
                "{color:?} at {depth:?} must be four bytes a pixel"
            );
            for pixel in image.data.chunks_exact(4) {
                assert_eq!(pixel, &rgba[..], "{color:?} at {depth:?}");
            }
        }
    }

    /// Decoding a large image must not leave its size allocated afterwards.
    #[test]
    fn no_pixels_outlive_the_decode_that_produced_them() {
        let mut decoder = PngDecoder::new(64 * 1024 * 1024);
        let large = png_bytes(512, 512);
        let small = png_bytes(1, 1);
        let ((), sample) = crate::test_allocations::measure(|| {
            assert!(decode(&mut decoder, &large).is_some());
            assert!(decode(&mut decoder, &small).is_some());
        });
        let retained = sample.bytes.saturating_sub(sample.freed);
        assert!(
            retained < 64 * 1024,
            "{retained} bytes outlived the decodes: {sample:?}"
        );
    }
}
