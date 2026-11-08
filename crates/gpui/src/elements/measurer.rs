//! An element measurer.
//! Wraps an arbitrary element so that the bounds it is assigned during layout
//! can be observed, without painting it or registering any hitboxes.

use crate::{App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId, LayoutId, Pixels, Position, Size, Style, Window};
use hgpui::{AnyElement, IntoElement};

/// Construct a measurer that wraps the given element and reports its laid-out
/// bounds via `state`.
///
/// The wrapped element is a real node in the layout tree: it participates in
/// layout as usual and **occupies space**, so it affects the position and size
/// of its siblings exactly as it would if unwrapped. The reported bounds are
/// its final size and position for the current frame.
///
/// The measurer itself only observes: it does not paint the wrapped element
/// and registers no hitboxes. Use it when the element is already part of the
/// layout and you additionally need its resolved bounds; do not use it to
/// measure an element that should not take up space.
///
/// Bounds are written to `state` on every layout pass.
///
/// # Example
///
/// ```
/// # use hgpui::{div, px, App, Bounds, Entity, Pixels};
/// # fn example(cx: &mut App) {
/// let bounds: Entity<Bounds<Pixels>> = cx.new(|_| Bounds::default());
/// // The div still occupies 120x48 in the layout; the measurer just reports it.
/// measurer(div().w(px(120.)).h(px(48.)), bounds.clone());
/// // Read `bounds` elsewhere to observe the measured size.
/// # }
/// ```
pub fn measurer(
    element: impl IntoElement,
    state: Entity<Size<Pixels>>,
) -> Measurer {
    Measurer { element: element.into_any_element(), state }
}

/// An element measurer element, created with [`measurer`].
pub struct Measurer {
    element: AnyElement,
    state: Entity<Size<Pixels>>
}

impl Element for Measurer {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.element.request_layout(window, cx);
        (window.request_layout(Style {
            position: Position::Absolute,
            ..Default::default()
        }, [layout_id], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let size = bounds.size;
        self.state.update(cx, |old, cx|
            if *old != size {
                *old = size;
                cx.notify();
            });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        _: &mut Window,
        _: &mut App,
    ) {
        // paint nothing.
    }
}

impl IntoElement for Measurer {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}