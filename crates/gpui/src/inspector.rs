/// A unique identifier for an element that can be inspected.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct InspectorElementId {
    /// Stable part of the ID.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub path: std::rc::Rc<InspectorElementPath>,
    /// Disambiguates elements that have the same path.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub instance_id: usize,
}

impl Into<InspectorElementId> for &InspectorElementId {
    fn into(self) -> InspectorElementId {
        self.clone()
    }
}

#[cfg(any(feature = "inspector", debug_assertions))]
pub use conditional::*;

#[cfg(any(feature = "inspector", debug_assertions))]
mod conditional {
    use super::*;
    use crate::{
        AnyElement, App, Context, DivInspectorState, FontWeight, InteractiveElement, IntoElement,
        ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px, rgb,
    };
    use collections::{FxHashMap, TypeIdHashMap};
    use std::any::{Any, TypeId};

    /// `GlobalElementId` qualified by source location of element construction.
    #[derive(Debug, Eq, PartialEq, Hash)]
    pub struct InspectorElementPath {
        /// The path to the nearest ancestor element that has an `ElementId`.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub global_id: crate::GlobalElementId,
        /// Source location where this element was constructed.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub source_location: &'static std::panic::Location<'static>,
    }

    impl Clone for InspectorElementPath {
        fn clone(&self) -> Self {
            Self {
                global_id: self.global_id.clone(),
                source_location: self.source_location,
            }
        }
    }

    impl Into<InspectorElementPath> for &InspectorElementPath {
        fn into(self) -> InspectorElementPath {
            self.clone()
        }
    }

    /// Function set on `App` to render the inspector UI.
    pub type InspectorRenderer =
        Box<dyn Fn(&mut Inspector, &mut Window, &mut Context<Inspector>) -> AnyElement>;

    /// Manages inspector state - which element is currently selected and whether the inspector is
    /// in picking mode.
    pub struct Inspector {
        active_element: Option<InspectedElement>,
        pub(crate) pick_depth: Option<f32>,
    }

    struct InspectedElement {
        id: InspectorElementId,
        states: TypeIdHashMap<Box<dyn Any>>,
    }

    impl InspectedElement {
        fn new(id: InspectorElementId) -> Self {
            InspectedElement {
                id,
                states: Default::default(),
            }
        }
    }

    impl Inspector {
        pub(crate) fn new() -> Self {
            Self {
                active_element: None,
                pick_depth: Some(0.0),
            }
        }

        pub(crate) fn select(&mut self, id: InspectorElementId, window: &mut Window) {
            self.set_active_element_id(id, window);
            self.pick_depth = None;
        }

        pub(crate) fn hover(&mut self, id: InspectorElementId, window: &mut Window) {
            if self.is_picking() {
                let changed = self.set_active_element_id(id, window);
                if changed {
                    self.pick_depth = Some(0.0);
                }
            }
        }

        pub(crate) fn set_active_element_id(
            &mut self,
            id: InspectorElementId,
            window: &mut Window,
        ) -> bool {
            let changed = Some(&id) != self.active_element_id();
            if changed {
                self.active_element = Some(InspectedElement::new(id));
                window.refresh();
            }
            changed
        }

        /// ID of the currently hovered or selected element.
        pub fn active_element_id(&self) -> Option<&InspectorElementId> {
            self.active_element.as_ref().map(|e| &e.id)
        }

        pub(crate) fn with_active_element_state<T: 'static, R>(
            &mut self,
            window: &mut Window,
            f: impl FnOnce(&mut Option<T>, &mut Window) -> R,
        ) -> R {
            let Some(active_element) = &mut self.active_element else {
                return f(&mut None, window);
            };

            let type_id = TypeId::of::<T>();
            let mut inspector_state = active_element
                .states
                .remove(&type_id)
                .map(|state| *state.downcast().unwrap());

            let result = f(&mut inspector_state, window);

            if let Some(inspector_state) = inspector_state {
                active_element
                    .states
                    .insert(type_id, Box::new(inspector_state));
            }

            result
        }

        /// Starts element picking mode, allowing the user to select elements by clicking.
        pub fn start_picking(&mut self) {
            self.pick_depth = Some(0.0);
        }

        /// Returns whether the inspector is currently in picking mode.
        pub fn is_picking(&self) -> bool {
            self.pick_depth.is_some()
        }

        /// Renders elements for all registered inspector states of the active inspector element.
        pub fn render_inspector_states(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Vec<AnyElement> {
            let mut elements = Vec::new();
            if let Some(active_element) = self.active_element.take() {
                for (type_id, state) in &active_element.states {
                    if let Some(render_inspector) = cx
                        .inspector_element_registry
                        .renderers_by_type_id
                        .remove(type_id)
                    {
                        let mut element = (render_inspector)(
                            active_element.id.clone(),
                            state.as_ref(),
                            window,
                            cx,
                        );
                        elements.push(element);
                        cx.inspector_element_registry
                            .renderers_by_type_id
                            .insert(*type_id, render_inspector);
                    }
                }

                self.active_element = Some(active_element);
            }

            elements
        }
    }

    impl Render for Inspector {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Some(inspector_renderer) = cx.inspector_renderer.take() {
                let result = inspector_renderer(self, window, cx);
                cx.inspector_renderer = Some(inspector_renderer);
                result
            } else {
                default_inspector_panel(self, window, cx)
            }
        }
    }

    /// The default inspector panel, shown when no custom [`InspectorRenderer`] has been installed.
    /// It provides a picking toggle and renders the registered inspector states of the currently
    /// selected element.
    fn default_inspector_panel(
        inspector: &mut Inspector,
        window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let is_picking = inspector.is_picking();
        let active_element = inspector.active_element_id().cloned();
        let state_elements = inspector.render_inspector_states(window, cx);

        div()
            .id("inspector")
            .size_full()
            .flex()
            .flex_col()
            .background(rgb(0x161616))
            .child(
                div()
                    .id("inspector-header")
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(8.0))
                    .py(px(4.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xffffff))
                            .child("Inspector"),
                    )
                    .child(
                        div()
                            .id("inspector-pick-toggle")
                            .px(px(4.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .background(if is_picking { rgb(0x3b82f6) } else { rgb(0x2a2a2a) })
                            .text_size(px(11.0))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _event, window, _cx| {
                                if !this.is_picking() {
                                    this.start_picking();
                                    window.refresh();
                                }
                            }))
                            .child(if is_picking { "Picking…" } else { "Pick element" }),
                    ),
            )
            .child(
                div()
                    .id("inspector-body")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .id("inspector-element-info")
                            .p(px(8.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .text_size(px(11.0))
                            .text_color(rgb(0xcccccc))
                            .child(match active_element {
                                Some(id) => div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(format!("element: {}", id.path.global_id))
                                    .child(format!("at {}", id.path.source_location)),
                                None => div().child("No element selected"),
                            }),
                    )
                    .children(state_elements),
            )
            .into_any_element()
    }

    /// Default renderer for [`DivInspectorState`]: displays the inspected element's computed
    /// bounds and content size.
    fn default_div_inspector_state_renderer(
        _id: InspectorElementId,
        state: &DivInspectorState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> impl IntoElement {
        div()
            .id("inspector-div-state")
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .text_size(px(11.0))
            .text_color(rgb(0xcccccc))
            .child(format!("bounds: {:?}", state.bounds))
            .child(format!("content size: {:?}", state.content_size))
    }

    #[derive(Default)]
    pub(crate) struct InspectorElementRegistry {
        renderers_by_type_id: FxHashMap<
            TypeId,
            Box<dyn Fn(InspectorElementId, &dyn Any, &mut Window, &mut App) -> AnyElement>,
        >,
    }

    impl InspectorElementRegistry {
        pub fn register<T: 'static, R: IntoElement>(
            &mut self,
            f: impl 'static + Fn(InspectorElementId, &T, &mut Window, &mut App) -> R,
        ) {
            self.renderers_by_type_id.insert(
                TypeId::of::<T>(),
                Box::new(move |id, value, window, cx| {
                    let value = value.downcast_ref().unwrap();
                    f(id, value, window, cx).into_any_element()
                }),
            );
        }

        /// Registers the default [`DivInspectorState`] renderer, unless a custom renderer for that
        /// state type has already been registered.
        pub(crate) fn register_default_div_state_renderer(&mut self) {
            self.renderers_by_type_id
                .entry(TypeId::of::<DivInspectorState>())
                .or_insert_with(|| {
                    Box::new(|id, value, window, cx| {
                        let state = value.downcast_ref::<DivInspectorState>().unwrap();
                        default_div_inspector_state_renderer(id, state, window, cx)
                            .into_any_element()
                    })
                });
        }
    }

    #[cfg(test)]
    mod tests {
        use crate::{AppContext, Empty, TestAppContext};

        #[test]
        fn default_inspector_panel_renders() {
            let mut test_app = TestAppContext::single();
            let window = test_app.add_window(|_, _| Empty);
            let any_window = window.into();

            test_app
                .update_window(any_window, |_, window, cx| {
                    window.toggle_inspector(cx);
                    // Opening the inspector starts element picking immediately.
                    assert!(window.is_inspector_picking(cx));
                    window.draw(cx).clear(cx);
                })
                .unwrap();
            test_app.run_until_parked();

            // The inspector keeps its default panel across frames.
            test_app
                .update_window(any_window, |_, window, cx| {
                    window.draw(cx).clear(cx);
                })
                .unwrap();
            test_app.run_until_parked();
        }
    }
}

/// Provides definitions used by `#[derive_inspector_reflection]`.
#[cfg(any(feature = "inspector", debug_assertions))]
pub mod inspector_reflection {
    use std::any::Any;

    /// Reification of a function that has the signature `fn some_fn(T) -> T`. Provides the name,
    /// documentation, and ability to invoke the function.
    #[derive(Clone, Copy)]
    pub struct FunctionReflection<T> {
        /// The name of the function
        pub name: &'static str,
        /// The method
        pub function: fn(Box<dyn Any>) -> Box<dyn Any>,
        /// Documentation for the function
        pub documentation: Option<&'static str>,
        /// `PhantomData` for the type of the argument and result
        pub _type: std::marker::PhantomData<T>,
    }

    impl<T: 'static> FunctionReflection<T> {
        /// Invoke this method on a value and return the result.
        pub fn invoke(&self, value: T) -> T {
            let boxed = Box::new(value) as Box<dyn Any>;
            let result = (self.function)(boxed);
            *result
                .downcast::<T>()
                .expect("Type mismatch in reflection invoke")
        }
    }
}
