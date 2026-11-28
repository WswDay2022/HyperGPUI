use crate::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Window,
};

/// Builds a `Deferred` element, which delays the layout and paint of its child.
pub fn deferred(child: impl IntoElement) -> Deferred {
    Deferred {
        child: Some(child.into_any_element()),
        priority: 0,
        ignore_mask: false,
    }
}

/// An element which delays the painting of its child until after all of
/// its ancestors, while keeping its layout as part of the current element tree.
pub struct Deferred {
    child: Option<AnyElement>,
    priority: usize,
    ignore_mask: bool,
}

impl Deferred {
    /// Sets the `priority` value of the `deferred` element, which
    /// determines the drawing order relative to other deferred elements,
    /// with higher values being drawn on top.
    pub fn with_priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }

    /// Paints this deferred element without the inherited `context_mask`.
    ///
    /// This allows deferred content such as popovers to escape clipping from
    /// ancestor elements. The window's viewport clipping still applies.
    pub fn ignore_mask(mut self) -> Self {
        self.ignore_mask = true;
        self
    }
}

impl Element for Deferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout_id = self.child.as_mut().unwrap().request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let child = self.child.take().unwrap();
        let element_offset = window.element_offset();
        if self.ignore_mask {
            window.defer_draw_ignoring_context_mask(child, element_offset, self.priority)
        } else {
            window.defer_draw(child, element_offset, self.priority, None)
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for Deferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Deferred {
    /// Sets a priority for the element. A higher priority conceptually means painting the element
    /// on top of deferred draws with a lower priority (i.e. closer to the viewer).
    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Context, Entity, StyleRefinement, TestAppContext, Window, anchored, deferred, div, point,
        prelude::*, px, size,
    };

    #[hgpui::test]
    fn deferred_can_opt_out_of_ancestor_clip_masks(cx: &mut TestAppContext) {
        struct ClipProbe {
            ignore_mask: bool,
        }

        impl Render for ClipProbe {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                let popup = deferred(
                    anchored()
                        .position(point(px(80.), px(0.)))
                        .child(div().size(px(60.)).background(crate::rgb(0x336699))),
                );
                let popup = if self.ignore_mask {
                    popup.ignore_mask()
                } else {
                    popup
                };

                div()
                    .size(px(100.))
                    .overflow_hidden()
                    .rounded(px(20.))
                    .child(popup)
            }
        }

        for (ignore_mask, expect_clipped) in [(false, true), (true, false)] {
            let window = cx.add_window(move |_, _| ClipProbe { ignore_mask });
            let window: crate::AnyWindowHandle = window.into();
            cx.update_window(window, |_, window, cx| {
                window.draw(cx).clear(cx);
                let quads = &window.rendered_frame.scene.quads;
                assert!(!quads.is_empty(), "the deferred popup should be painted");
                if expect_clipped {
                    assert!(
                        quads.iter().any(|quad| quad.mask_chain != 0),
                        "default deferred draws must retain the ancestor rounded clip"
                    );
                } else {
                    assert!(
                        quads.iter().all(|quad| quad.mask_chain == 0),
                        "ignore_mask must omit ancestor masks from deferred primitives"
                    );
                }
            })
            .unwrap();
        }
    }

    #[hgpui::test]
    fn deferred_mask_escape_keeps_viewport_and_local_clip_on_cached_replay(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};

        const POPUP: u32 = 0x336699;
        const CONTENT: u32 = 0x669933;

        struct ClipProbe(Rc<Cell<usize>>);
        impl Render for ClipProbe {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                self.0.set(self.0.get() + 1);
                div()
                    .relative()
                    .size(px(100.))
                    .rounded(px(12.))
                    .overflow_hidden()
                    .child(
                        deferred(
                            div()
                                .absolute()
                                .left(px(80.))
                                .top(px(5.))
                                .size(px(60.))
                                .background(crate::rgb(POPUP))
                                .child(div().size(px(20.)).rounded(px(5.)).overflow_hidden().child(
                                    div().flex_none().size(px(40.)).background(crate::rgb(CONTENT)),
                                )),
                        )
                        .ignore_mask(),
                    )
            }
        }

        struct CachedRoot(Entity<ClipProbe>);
        impl Render for CachedRoot {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.0.clone().cached(StyleRefinement::default().size_full()))
            }
        }

        let renders = Rc::new(Cell::new(0));
        let window = cx.open_window(size(px(120.), px(120.)), {
            let renders = renders.clone();
            move |_, cx| CachedRoot(cx.new(|_| ClipProbe(renders)))
        });
        cx.run_until_parked();

        let check_scene = |cx: &mut TestAppContext| {
            window.update(cx, |_, window, _| {
                let quad = |rgb| {
                    let solid = crate::Background::from(crate::rgb(rgb)).solid;
                    window
                        .rendered_frame
                        .scene
                        .quads
                        .iter()
                        .find(|quad| quad.background.solid == solid)
                        .expect("popup quad missing")
                };
                let popup = quad(POPUP);
                let viewport_right = window.viewport_size().width.scale(window.scale_factor());
                assert!(
                    popup.bounds.right() > viewport_right,
                    "probe must extend past the viewport"
                );
                assert_eq!(popup.content_mask.bounds.right(), viewport_right);
                assert_eq!(popup.mask_chain, 0, "no inherited clip on the escaped popup");

                let content = quad(CONTENT);
                assert_eq!(
                    content.content_mask.bounds.size.width,
                    px(20.).scale(window.scale_factor())
                );
                assert!(content.bounds.size.width > content.content_mask.bounds.size.width);
                assert!(
                    content.content_mask.corner_radii.top_left.0 > 0.,
                    "the popup's own clip must remain rounded"
                );
            }).unwrap();
        };
        check_scene(cx);
        let before = renders.get();
        assert!(before > 0);
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        assert_eq!(renders.get(), before, "the second frame must replay the cached subtree");
        check_scene(cx);
    }

    /// A stand-in for a dock panel hosting a popover (deferred draw) whose
    /// content opens another popover (a deferred draw created while
    /// prepainting the first one's content).
    struct PanelView;

    impl Render for PanelView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Panel").size_full().child(
                deferred(
                    anchored().position(point(px(10.), px(10.))).child(
                        div().key_context("Popover").w(px(200.)).h(px(200.)).child(
                            deferred(
                                anchored().position(point(px(30.), px(30.))).child(
                                    div()
                                        .key_context("NestedMenu")
                                        .debug_selector(|| "NESTED_MENU".into())
                                        .w(px(50.))
                                        .h(px(50.)),
                                ),
                            )
                            .with_priority(2),
                        ),
                    ),
                )
                .with_priority(1),
            )
        }
    }

    struct RootView {
        panel: Entity<PanelView>,
    }

    impl Render for RootView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Root").size_full().child(
                self.panel
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        }
    }

    /// Regression test for a crash with nested deferred draws (e.g. a popover
    /// menu inside a popover hosted by a cached dock panel). Prepaint indices
    /// recorded during the deferred draw rounds must index the same
    /// `deferred_draws` vector that `reuse_prepaint` slices on the next frame;
    /// previously they were measured against a transient per-round vector, so
    /// reusing the panel's subtree grafted the wrong deferred draws and
    /// panicked in the dispatch tree.
    #[hgpui::test]
    fn test_nested_deferred_draws_with_reused_views(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, cx| {
            let panel = cx.new(|_| PanelView);
            RootView { panel }
        });
        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window
                    .rendered_frame
                    .debug_bounds
                    .get("NESTED_MENU")
                    .copied()
            })
            .unwrap()
            .expect("NESTED_MENU debug bounds not found");
        assert_eq!(menu_bounds.size, size(px(50.), px(50.)));

        // Re-render only the root view; the panel is cached, so its subtree -
        // including both deferred draw records - is reused from the previous
        // frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Reuse the subtree a second time, exercising ranges that were
        // themselves recorded during a reused frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Re-render the panel itself again to prove the popovers still draw.
        window
            .update(cx, |root, _, cx| {
                root.panel.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        cx.run_until_parked();

        window
            .update(cx, |_, window, _| {
                assert_eq!(window.rendered_frame.deferred_draws.len(), 2);
                assert!(
                    window
                        .rendered_frame
                        .debug_bounds
                        .contains_key("NESTED_MENU")
                );
            })
            .unwrap();
    }
}
