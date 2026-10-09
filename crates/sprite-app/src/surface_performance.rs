use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AllocationSample {
    allocations: u64,
    bytes: u64,
}

thread_local! {
    // Thread-local counting excludes unrelated threads and parallel test cases.
    static ACTIVE_SAMPLE: Cell<Option<AllocationSample>> = const { Cell::new(None) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count(bytes: usize) {
    let _ = ACTIVE_SAMPLE.try_with(|active| {
        if let Some(mut sample) = active.get() {
            sample.allocations += 1;
            sample.bytes += bytes as u64;
            active.set(Some(sample));
        }
    });
}

// Safety: every pointer and layout is forwarded unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

struct MeasurementGuard;

impl MeasurementGuard {
    fn start() -> Self {
        ACTIVE_SAMPLE.with(|active| {
            assert!(active.get().is_none(), "measurement scopes must not nest");
            active.set(Some(AllocationSample::default()));
        });
        Self
    }

    fn finish(self) -> AllocationSample {
        ACTIVE_SAMPLE.with(|active| active.take().expect("an active measurement"))
    }
}

impl Drop for MeasurementGuard {
    fn drop(&mut self) {
        ACTIVE_SAMPLE.with(|active| active.set(None));
    }
}

pub(crate) fn measure<T>(f: impl FnOnce() -> T) -> (T, u64, u64) {
    let guard = MeasurementGuard::start();
    let result = f();
    let sample = guard.finish();
    (result, sample.allocations, sample.bytes)
}

#[test]
fn surface_allocation_probe() {
    use crate::config::Highlights;
    use crate::surface::{grid::*, list, render};
    use serde_json::json;
    let metrics = render::GridMetrics {
        cells: crate::terminal_view::CellMetrics::fixture(8.0, 16.0),
        defaults: (
            sprite_term::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            sprite_term::Rgb { r: 0, g: 0, b: 0 },
        ),
        blink_on: true,
        focused: true,
    };
    let (mut grid, a, b) = measure(|| GridSurface::new(200, 60));
    println!(
        "grid cell_bytes={} create allocations={a} bytes={b}",
        std::mem::size_of::<Cell>()
    );
    let (_, a, b) = measure(|| render::render_grid(&mut grid, &Highlights::default(), &metrics));
    println!("grid first render allocations={a} bytes={b}");
    let (_, a, b) = measure(|| render::render_grid(&mut grid, &Highlights::default(), &metrics));
    println!("grid idle render allocations={a} bytes={b}");
    assert!(a <= 1 && b <= 1_024);
    let op = parse_ops(&json!({"type":"rows","rows":[{"row":30,"cells":[["👩‍💻",2]]}]})).unwrap();
    let (_, a, b) = measure(|| {
        grid.apply_all(op).unwrap();
        render::render_grid(&mut grid, &Highlights::default(), &metrics)
    });
    println!("grid one-row update/render allocations={a} bytes={b}");
    assert!(a <= 8 && b <= 12_000);
    let rows = (0..100_000)
        .map(|i| json!({"id":format!("r{i}"),"text":"shared row text","indent":0,"guides":[]}))
        .collect::<Vec<_>>();
    let message = json!({"type":"list_rows","revision":1,"rows":rows,"selected":null});
    let mut model = list::ListModel::default();
    let (_, a, b) = measure(|| model.apply(list::parse_op(&message).unwrap()).unwrap());
    println!("list parse/apply 100k allocations={a} bytes={b}");
    assert!(a <= 300_025 && b <= 41_000_000);
    model.scroll = Some(list::ScrollAnchor {
        id: "r50000".into(),
        offset: 3.0,
    });
    let mut message2 = message.clone();
    message2["revision"] = 2.into();
    let (_, a, b) = measure(|| model.apply(list::parse_op(&message2).unwrap()).unwrap());
    println!("list replace with anchor 100k allocations={a} bytes={b}");
    assert!(a <= 300_025 && b <= 41_000_000);
    let state = json!({"type":"list_state","revision":2,"selected":"r99999","scroll":{"id":"r50000","offset":3.0}});
    let (_, a, b) = measure(|| model.apply(list::parse_op(&state).unwrap()).unwrap());
    println!("list state 100k allocations={a} bytes={b}");
    assert!(a <= 16 && b <= 1_000);
}
