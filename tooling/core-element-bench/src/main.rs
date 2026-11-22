//! Optimized, same-process core storage comparisons. Not a whole-GPUI/UI benchmark.
//! Import the actual implementation and the scoped System request counter, not a rewrite.

#[path = "../../../crates/gpui/src/elements/listener_list.rs"]
mod listener_list;
#[path = "../../../crates/gpui/src/elements/test_allocations.rs"]
mod test_allocations;

use listener_list::ListenerList;
use std::{hint::black_box, rc::Rc, time::Instant};

// Same 16-byte Rc fat pointer storage as the core ClickListener; callbacks are not invoked.
type Callback = Rc<dyn Fn()>;

fn run(batch: usize, count: usize, clone_capture: bool, legacy: bool, listener: &Callback) {
    for _ in 0..batch {
        if legacy {
            let mut callbacks = Vec::new();
            for _ in 0..count { callbacks.push(listener.clone()); }
            if clone_capture { black_box(callbacks.clone()); }
            black_box(callbacks);
        } else {
            let mut storage = ListenerList::default();
            for _ in 0..count { storage.push(listener.clone()); }
            let callbacks = storage.take();
            if clone_capture { black_box(callbacks.clone()); }
            black_box(callbacks);
        }
    }
}

fn main() {
    assert!(!cfg!(debug_assertions), "run this harness with --release");
    let listener: Callback = Rc::new(|| {});
    println!("optimized actual ListenerList, opt-level=3; same-process median7, alternating order");
    for (count, clone_capture) in [(0, false), (1, false), (1, true), (4, false), (16, true)] {
        let old_allocs = test_allocations::measure(|| run(1, count, clone_capture, true, &listener)).1;
        let new_allocs = test_allocations::measure(|| run(1, count, clone_capture, false, &listener)).1;
        run(2000, count, clone_capture, true, &listener);
        run(2000, count, clone_capture, false, &listener);
        let mut old = Vec::new();
        let mut new = Vec::new();
        for round in 0..7 {
            for legacy in if round % 2 == 0 { [true, false] } else { [false, true] } {
                let started = Instant::now();
                run(100_000, count, clone_capture, legacy, &listener);
                let nanoseconds = started.elapsed().as_secs_f64() * 1_000_000_000. / 100_000.;
                if legacy { old.push(nanoseconds); } else { new.push(nanoseconds); }
            }
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        println!("count={count} clone={clone_capture} requests/bytes legacy={}/{} compact={}/{} median7_ns/node legacy={:.2} compact={:.2}",
            old_allocs.count, old_allocs.requested_bytes, new_allocs.count, new_allocs.requested_bytes,
            old[3], new[3]);
    }
}
