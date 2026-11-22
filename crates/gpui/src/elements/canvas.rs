use refineable::Refineable as _;

use crate::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, Pixels,
    Style, StyleRefinement, Styled, Window,
};

/// Construct a canvas element with the given paint callback.
/// Useful for adding short term custom drawing to a view.
pub fn canvas<T>(
    prepaint: impl 'static + FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T,
    paint: impl 'static + FnOnce(Bounds<Pixels>, T, &mut Window, &mut App),
) -> Canvas<T> {
    Canvas {
        prepaint: Some(Box::new(prepaint)),
        paint: Some(Box::new(paint)),
        style: StyleRefinement::default(),
    }
}

/// A canvas element, meant for accessing the low level paint API without defining a whole
/// custom element
pub struct Canvas<T> {
    prepaint: Option<Box<dyn FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T>>,
    paint: Option<Box<dyn FnOnce(Bounds<Pixels>, T, &mut Window, &mut App)>>,
    style: StyleRefinement,
}

impl<T: 'static> IntoElement for Canvas<T> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: 'static> Element for Canvas<T> {
    type RequestLayoutState = Style;
    type PrepaintState = Option<T>;

    fn id(&self) -> Option<ElementId> {
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
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<T> {
        Some(self.prepaint.take().unwrap()(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let prepaint = prepaint.take().unwrap();
        style.paint(bounds, window, cx, |window, cx| {
            (self.paint.take().unwrap())(bounds, prepaint, window, cx)
        });
    }
}

impl<T> Styled for Canvas<T> {
    fn style(&mut self) -> &mut crate::StyleRefinement {
        &mut self.style
    }
}

#[cfg(test)]
mod tests {
    use super::{canvas, Canvas};
    use crate::{AnyWindowHandle, AppContext as _, Context, IntoElement, ParentElement,
        Render, Styled, TestAppContext, Window, div, px};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn core_component_unstyled_canvas_has_compact_payload() {
        let (_, allocations) = crate::elements::test_allocations::measure(|| canvas(|_, _, _| (), |_, _, _, _| {}));
        eprintln!("unstyled canvas payload={} construction_requests={allocations:?}", std::mem::size_of::<Canvas<()>>());
        assert_eq!(allocations.count, 0, "unstyled canvas must not gain a style allocation");
        assert!(std::mem::size_of::<Canvas<()>>() <= 64,
            "optional style must not occupy the full 1352-byte refinement in every canvas");
    }

    #[hgpui::test]
    fn core_component_styled_canvas_keeps_layout_and_paint(cx: &mut TestAppContext) {
        struct Probe(Rc<Cell<(f32, f32)>>);
        impl Render for Probe {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let measured = self.0.clone();
                div().size_full().child(canvas(move |bounds, _, _| {
                    measured.set((bounds.size.width.0, bounds.size.height.0));
                }, |_, _, _, _| {}).w(px(24.)).h(px(16.)).background(crate::rgb(0x0000ff)))
            }
        }
        let measured = Rc::new(Cell::new((0., 0.)));
        let window: AnyWindowHandle = cx.add_window({ let measured = measured.clone(); move |_, _| Probe(measured) }).into();
        assert_eq!(measured.get(), (24., 16.));
        cx.update_window(window, |_, window, _| {
            let blue = crate::Background::from(crate::rgb(0x0000ff)).solid;
            assert!(window.rendered_frame.scene.quads.iter().any(|quad| quad.background.solid == blue),
                "style materialization must still paint the configured background");
        }).unwrap();
    }
}
