use std::{cell::Cell, hint::black_box, rc::Rc, time::Instant};

use crate::{
    App, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _, Render,
    StyleRefinement, Styled as _, TestAppContext, WeakEntity, Window, WindowHandle, div,
    elements::test_allocations::{Allocations, measure},
};

struct StateBatch {
    rows: usize,
    prefix_depth: usize,
    mode: BatchMode,
    allocations: Allocations,
    elapsed_ns: u128,
}

#[derive(Clone, Copy)]
enum BatchMode {
    Warmup,
    Allocations,
    Time,
}

fn draw_rows(window: &mut Window, cx: &mut App, rows: usize, prefix_depth: usize) {
    if prefix_depth > 0 {
        window.with_element_namespace("ancestor", |window| {
            draw_rows(window, cx, rows, prefix_depth - 1);
        });
        return;
    }
    for row in 0..rows {
        window.with_element_namespace(row, |window| {
            black_box(window.use_state(cx, |_, _| row));
            black_box(window.use_keyed_state("control", cx, |_, _| row));
        });
    }
}

impl Render for StateBatch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut draw = || draw_rows(window, cx, self.rows, self.prefix_depth);
        match self.mode {
            BatchMode::Warmup => draw(),
            BatchMode::Allocations => self.allocations = measure(draw).1,
            BatchMode::Time => {
                let start = Instant::now();
                draw();
                self.elapsed_ns = start.elapsed().as_nanos();
            }
        }
        div()
    }
}

fn warm_batch(
    cx: &mut TestAppContext,
    rows: usize,
    prefix_depth: usize,
) -> WindowHandle<StateBatch> {
    let handle = cx.add_window(|_, _| StateBatch {
        rows,
        prefix_depth,
        mode: BatchMode::Warmup,
        allocations: Allocations::default(),
        elapsed_ns: 0,
    });
    // Prime both frame tables and access logs to measure steady hits, not growth.
    for _ in 0..4 {
        draw_batch(cx, handle, BatchMode::Warmup);
    }
    handle
}

fn draw_batch(
    cx: &mut TestAppContext,
    handle: WindowHandle<StateBatch>,
    mode: BatchMode,
) -> (Allocations, u128) {
    handle.update(cx, |batch, _, _| batch.mode = mode).unwrap();
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    handle
        .update(cx, |batch, _, _| (batch.allocations, batch.elapsed_ns))
        .unwrap()
}

#[test]
fn state_hooks_avoid_allocating_keys_on_steady_hits() {
    let mut cx = TestAppContext::single();
    for prefix_depth in [0, 8] {
        let handle = warm_batch(&mut cx, 128, prefix_depth);
        assert_eq!(
            draw_batch(&mut cx, handle, BatchMode::Allocations).0,
            Allocations::default(),
            "prefix depth {prefix_depth}",
        );
    }
}

#[test]
#[ignore = "manual Release benchmark for the actual window state hooks"]
fn state_hooks_release_benchmark() {
    let mut cx = TestAppContext::single();
    for (rows, prefix_depth) in [(32, 0), (512, 0), (512, 8)] {
        let handle = warm_batch(&mut cx, rows, prefix_depth);
        let (allocations, _) = draw_batch(&mut cx, handle, BatchMode::Allocations);
        let mut samples = [0u128; 121];
        for sample in &mut samples {
            *sample = draw_batch(&mut cx, handle, BatchMode::Time).1;
        }
        samples.sort_unstable();
        let ns_per_hook = samples[samples.len() / 2] as f64 / (rows * 2) as f64;
        println!(
            "rows={rows} prefix_depth={prefix_depth} {ns_per_hook:.2} ns/hook, \
             {} allocation requests / {} B per frame",
            allocations.count, allocations.requested_bytes,
        );
    }
}

struct StateIdentity {
    initializations: usize,
    render_count: usize,
}

impl Render for StateIdentity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;
        let states = [1usize, 2].map(|row| {
            window.with_element_namespace(row, |window| {
                let implicit = window.use_state(cx, |_, _| {
                    self.initializations += 1;
                    row
                });
                let explicit = window.use_keyed_state("row", cx, |window, _| {
                    self.initializations += 1;
                    // Initializers see the keyed state's namespace just as a caller of
                    // with_global_id does, including when they create another state.
                    window.with_global_id("nested".into(), |id, window| {
                        assert!(id.ends_with(&[row.into(), "row".into(), "nested".into()]));
                        window.with_element_state::<usize, _>(id, |saved, _| {
                            assert!(saved.is_none());
                            ((), row)
                        });
                    });
                    row
                });
                let other_type = window.use_keyed_state("row", cx, |_, _| {
                    self.initializations += 1;
                    row as u16
                });
                window.with_global_id(ElementId::from("row"), |id, window| {
                    let recovered =
                        window.with_element_state(id, |stored: Option<Entity<usize>>, _| {
                            let stored =
                                stored.expect("same key through the public GlobalElementId API");
                            (stored.entity_id(), stored)
                        });
                    assert_eq!(recovered, explicit.entity_id());
                });
                let explicit_again = window.use_keyed_state("row", cx, |_, _| -> usize {
                    panic!("repeat lookup in the same frame must hit")
                });
                assert_eq!(explicit_again, explicit);
                assert_ne!(implicit, explicit);
                assert_eq!(*implicit.read(cx), row);
                assert_eq!(*other_type.read(cx), row as u16);
                explicit
            })
        });
        assert_ne!(states[0], states[1]);
        div()
    }
}

#[test]
fn state_hooks_preserve_identity_types_and_initializer_namespace() {
    let mut cx = TestAppContext::single();
    let handle = cx.add_window(|_, _| StateIdentity {
        initializations: 0,
        render_count: 0,
    });
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    handle
        .update(&mut cx, |view, _, _| {
            assert!(view.render_count >= 2);
            assert_eq!(view.initializations, 6);
        })
        .unwrap();
}

struct CachedState {
    renders: Rc<Cell<usize>>,
    hook: Option<WeakEntity<usize>>,
}

impl Render for CachedState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        self.hook = Some(window.use_state(cx, |_, _| 0usize).downgrade());
        div().size_full()
    }
}

struct CacheRoot {
    panel: Entity<CachedState>,
    visible: bool,
}

impl Render for CacheRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.visible.then(|| {
            self.panel
                .clone()
                .cached(StyleRefinement::default().size_full())
        }))
    }
}

#[test]
fn state_hooks_survive_cached_replay_notify_the_view_and_drop_when_hidden() {
    let mut cx = TestAppContext::single();
    let renders = Rc::new(Cell::new(0));
    let handle = cx.add_window(|_, cx| CacheRoot {
        panel: cx.new(|_| CachedState {
            renders: renders.clone(),
            hook: None,
        }),
        visible: true,
    });
    cx.run_until_parked();
    let hook = handle
        .update(&mut cx, |root, _, cx| {
            root.panel.read(cx).hook.as_ref().unwrap().clone()
        })
        .unwrap();
    let initial = renders.get();
    for _ in 0..2 {
        handle.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        assert_eq!(renders.get(), initial, "expected cached replay, not render");
        assert!(hook.upgrade().is_some());
    }
    handle
        .update(&mut cx, |_, _, cx| {
            // No Entity::read of the hook during render: the observer itself must
            // invalidate the view that called use_state.
            hook.upgrade().unwrap().update(cx, |value, cx| {
                *value += 1;
                cx.notify();
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert!(renders.get() > initial);
    handle
        .update(&mut cx, |root, _, cx| {
            root.visible = false;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(hook.upgrade().is_none());
}
