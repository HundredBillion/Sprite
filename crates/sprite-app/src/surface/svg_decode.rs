//! Turns SVG text into a picture: parses it, refuses a document whose
//! filters would cost too much, and rasterises the rest within the Surface
//! raster bounds. Element images and list assets both come through here.

use std::sync::{Arc, LazyLock};

use gpui::RenderImage;
use resvg::usvg;

/// Decoded pixels one Surface cache keeps at once.
pub(crate) const MAX_SURFACE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Pixels one decoded SVG may hold.
const MAX_SVG_RASTER_BYTES: usize = 16 * 1024 * 1024;
/// The longest side, in pixels, of one decoded SVG.
const MAX_SVG_RASTER_DIMENSION: f32 = 4096.0;

/// Filter primitives one document may apply, counted once for every group
/// that applies them.
///
/// Each primitive's result is a canvas-sized intermediate raster kept until
/// its filter finishes, so a few hundred encoded bytes can describe seconds
/// of work. The encoded-size limits cannot bound that; this does. Chosen as
/// the largest measured count whose document decodes in under 250 ms in a
/// release build (`measure_filter_cap`).
///
/// Measured: a 512x512 document of 32 merged flood results (33 uses) decodes
/// in 51 ms (median of 5, release build, macos/aarch64). Medians for 2, 3, 5,
/// 9, 17 and 33 uses were 6, 6, 9, 15, 27 and 51 ms (median of 3 runs).
pub(crate) const MAX_FILTER_PRIMITIVE_USES: usize = 33;

/// Why an SVG was not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefusalKind {
    /// usvg could not read the text.
    Unparsable,
    /// The document applies more than `MAX_FILTER_PRIMITIVE_USES` filter
    /// primitives.
    TooComplex,
    /// The raster would be empty, not finite, or larger than one image may
    /// be, however much of the budget is free.
    Unrasterisable,
    /// The raster fits one image but not what the cache's budget has left.
    OverBudget,
}

/// The picture `svg` draws, `target_width` pixels wide or at its own size,
/// holding no more than `available_bytes` of pixels.
pub(crate) fn decode(
    svg: &str,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<RenderImage>, RefusalKind> {
    let tree = parse(svg)?;
    if count_filter_primitive_uses(&tree) > MAX_FILTER_PRIMITIVE_USES {
        return Err(RefusalKind::TooComplex);
    }
    rasterise(&tree, target_width, available_bytes)
}

/// The parsed document. The font database is loaded the first time a
/// document with text is parsed, on whichever thread parses it.
fn parse(svg: &str) -> Result<usvg::Tree, RefusalKind> {
    static FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Regular.ttf").to_vec());
        db.load_font_data(include_bytes!("../../assets/fonts/AdwaitaSans-Bold.ttf").to_vec());
        db.set_sans_serif_family("Adwaita Sans");
        Arc::new(db)
    });
    static OPTIONS: LazyLock<usvg::Options<'static>> = LazyLock::new(|| {
        let select = usvg::FontResolver::default_font_selector();
        usvg::Options {
            font_resolver: usvg::FontResolver {
                select_font: Box::new(move |font, db| {
                    if db.is_empty() {
                        *db = FONT_DB.clone();
                    }
                    select(font, db)
                }),
                select_fallback: usvg::FontResolver::default_fallback_selector(),
            },
            ..Default::default()
        }
    });
    usvg::Tree::from_data(svg.as_bytes(), &OPTIONS).map_err(|_| RefusalKind::Unparsable)
}

/// How many filter primitives `tree` applies. Each group adds the primitives
/// of every filter it applies, so a filter two groups share counts twice.
/// Clip paths, masks, patterns, nested SVG images and text outlines are
/// walked too, because they are rendered too.
fn count_filter_primitive_uses(tree: &usvg::Tree) -> usize {
    fn walk(group: &usvg::Group, uses: &mut usize) {
        for filter in group.filters() {
            *uses = uses.saturating_add(filter.primitives().len());
        }
        for node in group.children() {
            if let usvg::Node::Group(child) = node {
                walk(child, uses);
            }
            node.subroots(|subroot| walk(subroot, uses));
        }
    }
    let mut uses = 0;
    walk(tree.root(), &mut uses);
    uses
}

/// Draws `tree`. Refuses a raster that is empty, not finite, past one
/// image's dimension or byte limit, or past `available_bytes`.
fn rasterise(
    tree: &usvg::Tree,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<RenderImage>, RefusalKind> {
    let size = tree.size();
    let scale = target_width.map_or(1.0, |target| target / size.width());
    let width = (size.width() * scale).ceil();
    let height = (size.height() * scale).ceil();
    if [width, height].iter().any(|dimension| {
        !dimension.is_finite() || *dimension <= 0.0 || *dimension > MAX_SVG_RASTER_DIMENSION
    }) {
        return Err(RefusalKind::Unrasterisable);
    }
    let width = width as u32;
    let height = height as u32;
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(RefusalKind::Unrasterisable)?;
    if bytes > MAX_SVG_RASTER_BYTES {
        return Err(RefusalKind::Unrasterisable);
    }
    if bytes > available_bytes {
        return Err(RefusalKind::OverBudget);
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or(RefusalKind::Unrasterisable)?;
    #[cfg(test)]
    RASTERISED.with(|count| count.set(count.get() + 1));
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let width = pixmap.width();
    let height = pixmap.height();
    let mut pixels = pixmap.take();
    // tiny-skia holds premultiplied RGBA; GPUI draws straight BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u16;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u16 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let buffer =
        image::RgbaImage::from_raw(width, height, pixels).ok_or(RefusalKind::Unrasterisable)?;
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// What a decoded picture holds against the image budget.
pub(crate) fn raster_bytes(picture: &RenderImage) -> usize {
    picture.as_bytes(0).map_or(0, <[u8]>::len)
}

/// `decode` with a whole image's budget, for tests that only care whether
/// and how an SVG draws.
#[cfg(test)]
pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<RenderImage>> {
    decode(svg, target_width, MAX_SVG_RASTER_BYTES).ok()
}

#[cfg(test)]
thread_local! {
    /// How many rasters this thread has drawn, counted just before resvg
    /// renders, so a test can see that a refused document cost no raster.
    static RASTERISED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many rasters this thread has drawn so far.
#[cfg(test)]
pub(crate) fn rasterised() -> usize {
    RASTERISED.with(std::cell::Cell::get)
}

/// A `side`-pixel square whose one filter floods `results` differently named
/// results and merges them all: `results + 1` primitive uses in a few hundred
/// bytes, the shape that once took seconds to decode.
#[cfg(test)]
pub(crate) fn merged_floods(results: usize, side: u32) -> String {
    let floods: String = (0..results)
        .map(|n| {
            format!(
                "<feFlood flood-color='#{:06x}' flood-opacity='0.5' result='r{n}'/>",
                n * 0x01_0101
            )
        })
        .collect();
    let merges: String = (0..results)
        .map(|n| format!("<feMergeNode in='r{n}'/>"))
        .collect();
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{side}' height='{side}'>\
         <filter id='f' x='0' y='0' width='1' height='1'>{floods}<feMerge>{merges}</feMerge></filter>\
         <rect width='{side}' height='{side}' filter='url(#f)'/></svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(svg: &str) -> usvg::Tree {
        usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap()
    }

    #[test]
    fn filter_primitives_are_counted_once_for_every_group_that_applies_them() {
        let filter = "<filter id='f'><feFlood result='a'/><feFlood result='b'/>\
                      <feMerge><feMergeNode in='a'/><feMergeNode in='b'/></feMerge></filter>";
        let svg = |body: &str| {
            format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'>{filter}{body}</svg>"
            )
        };
        let uses = |body: &str| count_filter_primitive_uses(&tree(&svg(body)));
        assert_eq!(uses(""), 0, "a filter nothing applies costs nothing");
        assert_eq!(uses("<rect width='8' height='8' filter='url(#f)'/>"), 3);
        assert_eq!(
            uses(
                "<rect width='8' height='8' filter='url(#f)'/><rect width='4' height='4' filter='url(#f)'/>"
            ),
            6,
            "a shared filter counts once per use"
        );
        assert_eq!(
            uses(
                "<g filter='url(#f)'><g filter='url(#f)'><rect width='8' height='8' filter='url(#f)'/></g></g>"
            ),
            9,
            "nested groups each count"
        );
        assert_eq!(
            uses(
                "<pattern id='p' width='8' height='8' patternUnits='userSpaceOnUse'>\
                  <rect width='8' height='8' filter='url(#f)'/></pattern>\
                  <rect width='8' height='8' fill='url(#p)'/>"
            ),
            3,
            "a filter inside a pattern is rendered, so it counts"
        );
        assert_eq!(count_filter_primitive_uses(&tree(&merged_floods(4, 64))), 5);
    }

    #[test]
    fn a_document_over_the_filter_cap_is_refused_before_it_is_rasterised() {
        let before = rasterised();
        assert_eq!(
            decode(&merged_floods(128, 64), None, MAX_SVG_RASTER_BYTES).map(|_| ()),
            Err(RefusalKind::TooComplex)
        );
        assert_eq!(
            rasterised(),
            before,
            "a refused document is never rasterised"
        );
        let at_cap = merged_floods(MAX_FILTER_PRIMITIVE_USES - 1, 64);
        assert!(
            decode(&at_cap, None, MAX_SVG_RASTER_BYTES).is_ok(),
            "a document at the cap is drawn"
        );
        assert_eq!(rasterised(), before + 1);
    }

    #[test]
    fn a_refusal_says_why_the_svg_was_not_drawn() {
        let refused = |svg: &str, available| decode(svg, None, available).map(|_| ()).err();
        assert_eq!(
            refused("<svg invalid", MAX_SVG_RASTER_BYTES),
            Some(RefusalKind::Unparsable)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='4097' height='1'/>",
                MAX_SVG_RASTER_BYTES
            ),
            Some(RefusalKind::Unrasterisable)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2048' height='2048'/>",
                1024
            ),
            Some(RefusalKind::OverBudget)
        );
        assert_eq!(
            refused(
                "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'/>",
                MAX_SVG_RASTER_BYTES
            ),
            None
        );
    }

    /// Prints release-build decode times for 512×512 documents of merged
    /// flood results, to choose `MAX_FILTER_PRIMITIVE_USES`. Asserts nothing
    /// about time.
    #[test]
    #[ignore = "a measurement: run by hand in a release build"]
    fn measure_filter_cap() {
        const BUDGET_MS: f64 = 250.0;
        assert!(
            decode(&merged_floods(1, 512), None, MAX_SVG_RASTER_BYTES).is_ok(),
            "the warm-up decode builds the parser options"
        );
        let mut chosen = None;
        for results in [1usize, 2, 4, 8, 16, 32] {
            let svg = merged_floods(results, 512);
            let uses = count_filter_primitive_uses(&parse(&svg).unwrap());
            let mut times: Vec<f64> = (0..5)
                .map(|_| {
                    let started = std::time::Instant::now();
                    let tree = parse(&svg).unwrap();
                    assert!(rasterise(&tree, None, MAX_SVG_RASTER_BYTES).is_ok());
                    started.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            let median = times[times.len() / 2];
            println!("{results:>3} results, {uses:>3} primitive uses: median {median:.0} ms of 5");
            if median < BUDGET_MS {
                chosen = Some((results, uses, median));
            }
        }
        match chosen {
            Some((results, uses, median)) => println!(
                "cap: MAX_FILTER_PRIMITIVE_USES = {uses}; a 512x512 document of {results} merged \
                 flood results ({uses} uses) decodes in {median:.0} ms (median of 5, release \
                 build, {}/{})",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            None => println!("no measured document decodes in under {BUDGET_MS} ms"),
        }
    }
}
