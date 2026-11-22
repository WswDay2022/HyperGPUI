//! Scoped Rust allocation-request measurements for core element regression tests.
//!
//! This module is enabled only in hgpui's unit tests or explicitly imported by an independent
//! benchmark, never in the production library. The adapter delegates to the same System
//! allocator; a const, dropless thread-local counter isolates one measured caller
//! from parallel tests/background work. Counts are allocation requests, not live heap or RSS.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Allocations {
    pub(crate) count: usize,
    pub(crate) requested_bytes: usize,
}

thread_local! {
    static TRACKER: Cell<(bool, Allocations)> = const {
        Cell::new((false, Allocations { count: 0, requested_bytes: 0 }))
    };
}

struct MeasuringSystem;

#[global_allocator]
static ALLOCATOR: MeasuringSystem = MeasuringSystem;

fn record(bytes: usize) {
    // Never allocate or unwind inside GlobalAlloc. TLS can be unavailable during teardown.
    let _ = TRACKER.try_with(|tracker| {
        let (active, mut counts) = tracker.get();
        if active {
            counts.count = counts.count.saturating_add(1);
            counts.requested_bytes = counts.requested_bytes.saturating_add(bytes);
            tracker.set((true, counts));
        }
    });
}

unsafe impl GlobalAlloc for MeasuringSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() { record(layout.size()); }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() { record(layout.size()); }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() { record(size); }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout); }
    }
}

struct Scope;

impl Drop for Scope {
    fn drop(&mut self) {
        let _ = TRACKER.try_with(|tracker| {
            let (_, counts) = tracker.get();
            tracker.set((false, counts));
        });
    }
}

pub(crate) fn measure<T>(run: impl FnOnce() -> T) -> (T, Allocations) {
    TRACKER.with(|tracker| {
        assert!(!tracker.get().0, "allocation measurements cannot nest");
        tracker.set((true, Allocations::default()));
    });
    let scope = Scope;
    let result = run();
    drop(scope);
    let counts = TRACKER.with(|tracker| tracker.get().1);
    (result, counts)
}

#[cfg(test)]
mod tests {
    use super::measure;

    #[test]
    fn counts_one_request_and_resets_between_scopes() {
        let (value, counts) = measure(|| Box::new(std::hint::black_box(7u64)));
        assert_eq!(counts.count, 1);
        assert_eq!(counts.requested_bytes, std::mem::size_of::<u64>());
        assert_eq!(*value, 7);
        let (_, counts) = measure(|| std::hint::black_box(7u64));
        assert_eq!(counts.count, 0);
        drop(value);
    }

    #[test]
    fn restores_tracking_after_panic() {
        let result = std::panic::catch_unwind(|| measure(|| panic!("expected probe panic")));
        assert!(result.is_err());
        assert_eq!(measure(|| ()).1.count, 0);
    }
}
