//! Turns SVG text into a picture, off the UI thread.
//!
//! A cache asks for an SVG and gets `Pending` until a decode on GPUI's
//! background executor finishes. The decode parses the text, refuses a
//! document whose filters would cost too much, and rasterises the rest
//! within the Surface raster bounds. Each cache runs at most
//! `MAX_CONCURRENT_SVG_DECODES` at once, oldest request first. A finished
//! picture is charged to the cache's budget, and to the budget every cache in
//! the process shares, when it is installed, and the view that asked is told
//! to draw again.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{Context, RenderImage};

use crate::picture::Picture;
use resvg::usvg;

/// Decoded pixels one Surface cache keeps at once.
const MAX_SURFACE_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Decoded pixels every Surface cache in the process keeps at once: four
/// full Surface caches, or sixteen of the largest single rasters. A 32 px
/// icon is 4 KiB, so 64 Surfaces of 500 distinct icons each take 125 MiB.
const MAX_PROCESS_IMAGE_BYTES: usize = 256 * 1024 * 1024;
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
/// Measured: a 512x512 document of 160 merged flood results (161 uses)
/// decodes in 229 ms (median of 3 runs of 5, release build, macos/aarch64).
/// Medians for 33, 65, 97, 129, 161, 193 and 257 uses were 47, 92, 139, 178,
/// 229, 277 and 359 ms. The machine was under background load, so the cap is
/// conservative.
pub(crate) const MAX_FILTER_PRIMITIVE_USES: usize = 161;

/// Decodes one cache runs at the same time. The rest wait in the order they
/// were asked for, so the memory filter intermediates take is bounded per
/// cache by this many capped decodes, not by how many images a description
/// holds.
pub(crate) const MAX_CONCURRENT_SVG_DECODES: usize = 2;

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
    /// The raster fits one image but not what the cache's budget, or the
    /// process's, has left.
    OverBudget,
}

thread_local! {
    /// Pixels the ready pictures of every cache hold together, at most
    /// `MAX_PROCESS_IMAGE_BYTES`. A cache is shared through `Rc` and never
    /// leaves the UI thread, so this one ledger is the whole process's; in
    /// tests, each test thread gets its own.
    static PROCESS_RETAINED_BYTES: Cell<usize> = const { Cell::new(0) };
    /// What `PROCESS_RETAINED_BYTES` may reach. Only a test lowers it.
    static PROCESS_IMAGE_LIMIT: Cell<usize> = const { Cell::new(MAX_PROCESS_IMAGE_BYTES) };
}

/// What one SVG a cache asked for has come to.
#[derive(Clone, Debug)]
pub(crate) enum SvgState {
    /// Waiting for, or in, its decode. Draws nothing.
    Pending,
    Ready(Arc<Picture>),
    Refused(RefusalKind),
}

impl SvgState {
    /// The picture to draw, if the decode is ready.
    pub(crate) fn picture(&self) -> Option<Arc<Picture>> {
        match self {
            Self::Ready(picture) => Some(Arc::clone(picture)),
            Self::Pending | Self::Refused(_) => None,
        }
    }
}

/// The decodes of one cache: what each SVG it asked for has come to, and the
/// decodes still waiting for a turn.
///
/// Shared with the tasks running its decodes. A task holds it weakly, so a
/// cache that is dropped (its Surface closed) drops whatever finishes after.
pub(crate) struct SvgDecodes<K> {
    queue: Rc<RefCell<Queue<K>>>,
}

struct Queue<K> {
    entries: HashMap<K, Entry>,
    /// Asked for and not yet started, oldest first.
    waiting: VecDeque<Job<K>>,
    in_flight: usize,
    /// Pixels the ready pictures hold, at most `MAX_SURFACE_IMAGE_BYTES`.
    /// Charged to the process budget too: change it only through
    /// `set_retained`.
    retained_bytes: usize,
    /// How many entries have settled. A cache that remembers what it found
    /// compares this to know when to look again.
    settled: u64,
    /// Stamped on each entry, so a decode finishing for an entry that has
    /// been dropped, or dropped and asked for again, is told apart.
    next_ticket: u64,
    target_width: Option<f32>,
    #[cfg(test)]
    started: Vec<K>,
    #[cfg(test)]
    peak_in_flight: usize,
}

struct Entry {
    state: SvgState,
    ticket: u64,
}

struct Job<K> {
    key: K,
    ticket: u64,
    svg: Arc<str>,
}

impl<K> Default for SvgDecodes<K> {
    fn default() -> Self {
        Self::new(None)
    }
}

impl<K> SvgDecodes<K> {
    /// The decodes of a cache that draws each SVG `target_width` pixels wide,
    /// or at its own size.
    pub(crate) fn new(target_width: Option<f32>) -> Self {
        Self {
            queue: Rc::new(RefCell::new(Queue {
                entries: HashMap::new(),
                waiting: VecDeque::new(),
                in_flight: 0,
                retained_bytes: 0,
                settled: 0,
                next_ticket: 0,
                target_width,
                #[cfg(test)]
                started: Vec::new(),
                #[cfg(test)]
                peak_in_flight: 0,
            })),
        }
    }

    /// Goes up each time a decode is installed or refused.
    pub(crate) fn settled(&self) -> u64 {
        self.queue.borrow().settled
    }
}

impl<K: Clone + Eq + Hash + 'static> SvgDecodes<K> {
    /// What `key` has come to. The first time it is asked for, `job` names it
    /// and its SVG text, its decode joins the end of the queue, and it is
    /// `Pending`. Nothing is decoded here: `start` does that.
    pub(crate) fn want<Q>(&self, key: &Q, job: impl FnOnce() -> (K, Arc<str>)) -> SvgState
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let mut queue = self.queue.borrow_mut();
        if let Some(entry) = queue.entries.get(key) {
            return entry.state.clone();
        }
        let (key, svg) = job();
        let ticket = queue.next_ticket;
        queue.next_ticket += 1;
        queue.entries.insert(
            key.clone(),
            Entry {
                state: SvgState::Pending,
                ticket,
            },
        );
        queue.waiting.push_back(Job { key, ticket, svg });
        SvgState::Pending
    }

    /// Starts waiting decodes, oldest first, until `MAX_CONCURRENT_SVG_DECODES`
    /// are running. `cx` is the view that draws this cache: as each decode
    /// finishes, the view is told to draw again.
    pub(crate) fn start<T: 'static>(&self, cx: &mut Context<T>) {
        start_waiting(&self.queue, cx);
    }

    /// Keeps the entries `keep` accepts and drops the rest. A decode still
    /// running for a dropped entry is discarded when it finishes. A refusal
    /// for lack of budget is dropped too, so the next frame asks again
    /// against whatever this freed. Every other refusal stays for as long as
    /// it is kept.
    pub(crate) fn retain(&self, keep: impl Fn(&K) -> bool) {
        let mut queue = self.queue.borrow_mut();
        let Queue {
            entries, waiting, ..
        } = &mut *queue;
        entries.retain(|key, entry| {
            keep(key) && !matches!(entry.state, SvgState::Refused(RefusalKind::OverBudget))
        });
        waiting.retain(|job| {
            entries
                .get(&job.key)
                .is_some_and(|entry| entry.ticket == job.ticket)
        });
        let kept = entries
            .values()
            .filter_map(|entry| entry.state.picture())
            .map(|picture| raster_bytes(&picture))
            .sum();
        queue.set_retained(kept);
    }

    /// Forgets every entry and waiting decode, to draw from now on at
    /// `target_width`. A decode still running is discarded when it finishes.
    pub(crate) fn reset(&self, target_width: Option<f32>) {
        let mut queue = self.queue.borrow_mut();
        queue.entries.clear();
        queue.waiting.clear();
        queue.set_retained(0);
        queue.target_width = target_width;
    }
}

impl<K> Queue<K> {
    /// Pixels the next picture may hold: what is left of this cache's budget
    /// or of the process's, whichever is less.
    fn available(&self) -> usize {
        let process = PROCESS_IMAGE_LIMIT
            .with(Cell::get)
            .saturating_sub(PROCESS_RETAINED_BYTES.with(Cell::get));
        MAX_SURFACE_IMAGE_BYTES
            .saturating_sub(self.retained_bytes)
            .min(process)
    }

    /// Makes `bytes` what this cache retains, charging or refunding the
    /// difference to the process budget.
    fn set_retained(&mut self, bytes: usize) {
        PROCESS_RETAINED_BYTES.with(|process| {
            process.set(process.get() - self.retained_bytes + bytes);
        });
        self.retained_bytes = bytes;
    }
}

/// A cache dropped with its Surface gives its pictures back to the process.
impl<K> Drop for Queue<K> {
    fn drop(&mut self) {
        self.set_retained(0);
    }
}

impl<K: Clone + Eq + Hash> Queue<K> {
    /// The next decode to start, with the width to draw it at and the budget
    /// it may use, if a slot is free and one is waiting.
    fn next_job(&mut self) -> Option<(Job<K>, Option<f32>, usize)> {
        while self.in_flight < MAX_CONCURRENT_SVG_DECODES {
            let job = self.waiting.pop_front()?;
            if !self
                .entries
                .get(&job.key)
                .is_some_and(|entry| entry.ticket == job.ticket)
            {
                continue;
            }
            self.in_flight += 1;
            #[cfg(test)]
            {
                self.started.push(job.key.clone());
                self.peak_in_flight = self.peak_in_flight.max(self.in_flight);
            }
            return Some((job, self.target_width, self.available()));
        }
        None
    }

    /// Installs a finished decode, charging its pixels to the budget, unless
    /// its entry was dropped since it started. A picture the budget can no
    /// longer hold is refused for lack of budget. A decode refused for lack
    /// of the `given_bytes` it started with goes back to the front of the
    /// queue if an update, here or on another Surface, has since left more
    /// than that free, since the refusal describes a budget that no longer
    /// applies.
    fn finish(
        &mut self,
        job: Job<K>,
        given_bytes: usize,
        outcome: Result<Arc<Picture>, RefusalKind>,
    ) {
        self.in_flight = self.in_flight.saturating_sub(1);
        let available = self.available();
        let Some(entry) = self.entries.get_mut(&job.key) else {
            return;
        };
        if entry.ticket != job.ticket {
            return;
        }
        let mut charge = 0;
        entry.state = match outcome {
            Err(RefusalKind::OverBudget) if available > given_bytes => {
                let ticket = self.next_ticket;
                self.next_ticket += 1;
                entry.ticket = ticket;
                self.waiting.push_front(Job { ticket, ..job });
                return;
            }
            Ok(picture) => {
                let bytes = raster_bytes(&picture);
                if bytes > available {
                    SvgState::Refused(RefusalKind::OverBudget)
                } else {
                    charge = bytes;
                    SvgState::Ready(picture)
                }
            }
            Err(refusal) => SvgState::Refused(refusal),
        };
        self.set_retained(self.retained_bytes + charge);
        self.settled += 1;
    }
}

/// Starts waiting decodes until the queue is full or empty. Each one parses
/// and rasterises on the background executor. Then, back on the view's
/// thread, its result is installed, the next decode starts, and the view is
/// told to draw again.
fn start_waiting<K, T>(queue: &Rc<RefCell<Queue<K>>>, cx: &mut Context<T>)
where
    K: Clone + Eq + Hash + 'static,
    T: 'static,
{
    loop {
        let next = queue.borrow_mut().next_job();
        let Some((job, target_width, available_bytes)) = next else {
            return;
        };
        let svg = Arc::clone(&job.svg);
        let decoding = cx
            .background_executor()
            .spawn(async move { decode(&svg, target_width, available_bytes) });
        let weak = Rc::downgrade(queue);
        cx.spawn(async move |view, cx| {
            let outcome = decoding.await;
            let Some(queue) = weak.upgrade() else {
                return;
            };
            queue.borrow_mut().finish(job, available_bytes, outcome);
            view.update(cx, |_, cx| {
                start_waiting(&queue, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
impl<K: Clone + Eq + Hash> SvgDecodes<K> {
    /// What `key` has come to, without asking for it.
    pub(crate) fn state<Q>(&self, key: &Q) -> Option<SvgState>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.queue
            .borrow()
            .entries
            .get(key)
            .map(|entry| entry.state.clone())
    }

    pub(crate) fn keys(&self) -> Vec<K> {
        self.queue.borrow().entries.keys().cloned().collect()
    }

    /// Every key whose decode has started, in the order they started.
    pub(crate) fn started(&self) -> Vec<K> {
        self.queue.borrow().started.clone()
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.queue.borrow().in_flight
    }

    pub(crate) fn peak_in_flight(&self) -> usize {
        self.queue.borrow().peak_in_flight
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.queue.borrow().retained_bytes
    }
}

/// Pixels the ready pictures of every cache hold together.
#[cfg(test)]
pub(crate) fn process_retained_bytes() -> usize {
    PROCESS_RETAINED_BYTES.with(Cell::get)
}

/// Lowers the process budget to `limit` until the guard is dropped, so a test
/// can fill it with a few small rasters.
#[cfg(test)]
pub(crate) fn limit_process_images(limit: usize) -> impl Drop {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROCESS_IMAGE_LIMIT.with(|cell| cell.set(self.0));
        }
    }
    Restore(PROCESS_IMAGE_LIMIT.with(|cell| cell.replace(limit)))
}

/// The picture `svg` draws, `target_width` pixels wide or at its own size,
/// holding no more than `available_bytes` of pixels. Only a decode task
/// calls this, never a frame.
fn decode(
    svg: &str,
    target_width: Option<f32>,
    available_bytes: usize,
) -> Result<Arc<Picture>, RefusalKind> {
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
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_string: Box::new(|_, _| None),
                ..Default::default()
            },
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
) -> Result<Arc<Picture>, RefusalKind> {
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
    for pixel in pixels.as_chunks_mut::<4>().0 {
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
    Ok(Picture::new(buffer))
}

/// What a decoded picture holds against the image budget.
fn raster_bytes(picture: &RenderImage) -> usize {
    picture.as_bytes(0).map_or(0, <[u8]>::len)
}

/// `decode` with a whole image's budget, for tests that only care whether
/// and how an SVG draws.
#[cfg(test)]
pub(crate) fn render_svg(svg: &str, target_width: Option<f32>) -> Option<Arc<Picture>> {
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
    use gpui::AppContext as _;

    #[test]
    fn svg_data_does_not_load_external_files() {
        let path =
            std::env::temp_dir().join(format!("sprite-svg-resource-{}.svg", std::process::id()));
        std::fs::write(&path, "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><rect width='16' height='16' fill='red'/></svg>").unwrap();
        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='16' height='16'><image width='16' height='16' xlink:href='{}'/></svg>",
            path.display()
        );
        let parsed = parse(&svg).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            parsed.root().children().is_empty(),
            "external image was resolved"
        );
    }

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
            decode(
                &merged_floods(MAX_FILTER_PRIMITIVE_USES * 2, 64),
                None,
                MAX_SVG_RASTER_BYTES
            )
            .map(|_| ()),
            Err(RefusalKind::TooComplex)
        );
        assert_eq!(
            rasterised(),
            before,
            "a refused document is never rasterised"
        );
        let at_cap = merged_floods(MAX_FILTER_PRIMITIVE_USES - 1, 8);
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

    /// Stands in for the view that draws a Surface.
    struct Owner;

    /// A distinct `side`-pixel square: `side * side * 4` bytes once drawn.
    fn square(side: u32, n: usize) -> Arc<str> {
        Arc::from(format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{side}' height='{side}'><desc>{n}</desc></svg>"
        ))
    }

    /// Asks `cache` for `svgs`, as a frame does, and starts their decodes.
    fn want_all(
        cx: &mut gpui::TestAppContext,
        owner: &gpui::Entity<Owner>,
        cache: &SvgDecodes<Arc<str>>,
        svgs: &[Arc<str>],
    ) {
        for svg in svgs {
            cache.want(&**svg, || (Arc::clone(svg), Arc::clone(svg)));
        }
        owner.update(cx, |_, cx| cache.start(cx));
    }

    const RASTER: usize = 1024 * 1024 * 4;

    #[gpui::test]
    fn surfaces_together_retain_no_more_than_the_process_budget(cx: &mut gpui::TestAppContext) {
        let _limit = limit_process_images(2 * RASTER);
        let owner = cx.new(|_| Owner);
        let surfaces: Vec<SvgDecodes<Arc<str>>> = (0..3).map(|_| SvgDecodes::new(None)).collect();
        let svgs: Vec<Vec<Arc<str>>> = (0..3)
            .map(|s| (0..2).map(|n| square(1024, s * 2 + n)).collect())
            .collect();
        for (cache, svgs) in surfaces.iter().zip(&svgs) {
            want_all(cx, &owner, cache, svgs);
        }
        cx.run_until_parked();

        let states: Vec<SvgState> = surfaces
            .iter()
            .zip(&svgs)
            .flat_map(|(cache, svgs)| svgs.iter().map(|svg| cache.state(&**svg).unwrap()))
            .collect();
        let ready = states
            .iter()
            .filter(|state| state.picture().is_some())
            .count();
        let refused = states
            .iter()
            .filter(|state| matches!(state, SvgState::Refused(RefusalKind::OverBudget)))
            .count();
        assert_eq!(
            (ready, refused),
            (2, 4),
            "each Surface fits its own budget, but only two rasters fit the process's"
        );
        let per_surface: usize = surfaces.iter().map(SvgDecodes::retained_bytes).sum();
        assert_eq!(per_surface, 2 * RASTER);
        assert_eq!(process_retained_bytes(), 2 * RASTER);
    }

    #[gpui::test]
    fn every_way_a_cache_lets_go_of_pictures_returns_them_to_the_process_budget(
        cx: &mut gpui::TestAppContext,
    ) {
        let owner = cx.new(|_| Owner);
        let before = process_retained_bytes();
        let fill = |cx: &mut gpui::TestAppContext, cache: &SvgDecodes<Arc<str>>, first: usize| {
            want_all(
                cx,
                &owner,
                cache,
                &[square(1024, first), square(1024, first + 1)],
            );
            cx.run_until_parked();
            assert_eq!(cache.retained_bytes(), 2 * RASTER);
        };
        let dropped = SvgDecodes::new(None);
        let retained = SvgDecodes::new(None);
        let reset = SvgDecodes::new(None);
        fill(cx, &dropped, 0);
        fill(cx, &retained, 2);
        fill(cx, &reset, 4);
        assert_eq!(process_retained_bytes(), before + 6 * RASTER);

        drop(dropped);
        assert_eq!(
            process_retained_bytes(),
            before + 4 * RASTER,
            "a closed Surface"
        );
        let keep = square(1024, 2);
        retained.retain(|svg| *svg == keep);
        assert_eq!(
            process_retained_bytes(),
            before + 3 * RASTER,
            "an image no longer drawn"
        );
        reset.reset(Some(16.0));
        assert_eq!(
            process_retained_bytes(),
            before + RASTER,
            "a new raster width"
        );
        retained.retain(|_| false);
        assert_eq!(process_retained_bytes(), before);
    }

    #[gpui::test]
    fn every_way_a_cache_lets_go_of_a_picture_releases_its_gpu_copy(cx: &mut gpui::TestAppContext) {
        use crate::picture::released_by;
        let owner = cx.new(|_| Owner);
        let cache = SvgDecodes::new(None);
        let svgs = [square(4, 0), square(4, 1), square(4, 2)];
        want_all(cx, &owner, &cache, &svgs);
        cx.run_until_parked();
        let kept = Arc::clone(&svgs[0]);
        assert_eq!(
            released_by(|| cache.retain(|svg| *svg == kept)),
            2,
            "retain"
        );
        assert_eq!(released_by(|| cache.reset(None)), 1, "reset");
        want_all(cx, &owner, &cache, &svgs[..1]);
        cx.run_until_parked();
        assert_eq!(released_by(|| drop(cache)), 1, "dropped with its Surface");
    }

    #[gpui::test]
    fn a_decode_refused_for_the_process_budget_is_tried_again_once_it_frees(
        cx: &mut gpui::TestAppContext,
    ) {
        let _limit = limit_process_images(RASTER);
        let owner = cx.new(|_| Owner);
        let full = SvgDecodes::new(None);
        want_all(cx, &owner, &full, &[square(1024, 0)]);
        cx.run_until_parked();
        let waiting = SvgDecodes::new(None);
        let refused = square(1024, 1);
        want_all(cx, &owner, &waiting, std::slice::from_ref(&refused));
        cx.run_until_parked();
        assert!(matches!(
            waiting.state(&*refused),
            Some(SvgState::Refused(RefusalKind::OverBudget))
        ));

        // As with a Surface's own budget, the next description forgets the
        // refusal and asks again against what was freed meanwhile.
        drop(full);
        waiting.retain(|_| true);
        want_all(cx, &owner, &waiting, std::slice::from_ref(&refused));
        cx.run_until_parked();
        assert!(waiting.state(&*refused).unwrap().picture().is_some());

        // A decode that started while the budget was full goes back in the
        // queue if another Surface frees room before it finishes.
        let late = SvgDecodes::new(None);
        let started_full = square(1024, 2);
        want_all(cx, &owner, &late, std::slice::from_ref(&started_full));
        drop(waiting);
        cx.run_until_parked();
        assert!(late.state(&*started_full).unwrap().picture().is_some());
        assert_eq!(late.started().len(), 2, "refused once, then tried again");
    }

    #[gpui::test]
    fn a_window_of_icon_surfaces_fits_the_process_budget(cx: &mut gpui::TestAppContext) {
        let owner = cx.new(|_| Owner);
        // The most Surfaces one window hosts, each drawing eight distinct
        // icons at 64 px, a 32 pt icon on a Retina display.
        let icons = |surface: usize| -> Vec<Arc<str>> {
            (0..8).map(|n| square(32, surface * 8 + n)).collect()
        };
        let surfaces: Vec<SvgDecodes<Arc<str>>> =
            (0..64).map(|_| SvgDecodes::new(Some(64.0))).collect();
        for (surface, cache) in surfaces.iter().enumerate() {
            want_all(cx, &owner, cache, &icons(surface));
        }
        cx.run_until_parked();
        for (surface, cache) in surfaces.iter().enumerate() {
            for icon in icons(surface) {
                assert!(cache.state(&*icon).unwrap().picture().is_some());
            }
        }
        assert_eq!(process_retained_bytes(), 64 * 8 * 64 * 64 * 4);
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
        for results in [32usize, 64, 96, 128, 160, 192, 256] {
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
