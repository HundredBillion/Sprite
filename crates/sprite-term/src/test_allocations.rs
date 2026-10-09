//! Counts allocations on one measured test thread, excluding setup and other tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sample {
    pub allocations: usize,
    pub bytes: usize,
    /// Bytes released on the measured thread, a reallocation's old block
    /// included. With `bytes` this says what the work left allocated.
    pub freed: usize,
    /// The largest single block requested.
    pub largest: usize,
}

thread_local! {
    static SAMPLE: Cell<Option<Sample>> = const { Cell::new(None) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record(bytes: usize) {
    let _ = SAMPLE.try_with(|sample| {
        if let Some(mut count) = sample.get() {
            count.allocations += 1;
            count.bytes += bytes;
            count.largest = count.largest.max(bytes);
            sample.set(Some(count));
        }
    });
}

fn record_free(bytes: usize) {
    let _ = SAMPLE.try_with(|sample| {
        if let Some(mut count) = sample.get() {
            count.freed += bytes;
            sample.set(Some(count));
        }
    });
}

// SAFETY: allocation and deallocation preserve System's pointer and layout contracts.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        record_free(layout.size());
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_free(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
}

pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, Sample) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            SAMPLE.set(None);
        }
    }
    assert!(SAMPLE.replace(Some(Sample::default())).is_none());
    let reset = Reset;
    let result = work();
    let sample = SAMPLE.get().expect("measurement active");
    drop(reset);
    (result, sample)
}

#[test]
fn counts_real_allocations_and_ignores_work_outside_measurement() {
    let outside = std::hint::black_box(vec![0_u8; 13]);
    let (_, sample) = measure(|| {
        std::hint::black_box(vec![0_u8; 29]);
    });
    assert_eq!(sample.allocations, 1);
    assert_eq!(sample.bytes, 29);
    assert_eq!(
        sample.freed, 29,
        "the vector was dropped inside the measurement"
    );
    assert_eq!(sample.largest, 29);
    assert_eq!(outside.len(), 13);
}
