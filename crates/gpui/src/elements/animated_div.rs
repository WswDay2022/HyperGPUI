use stacksafe::{stacksafe, StackSafe};
use crate::{accesskit, AnyElement, App, Bounds, Element, ElementId, Hitbox, ImageCacheProvider, InteractiveElement, Interactivity, LayoutId, ParentElement, Pixels, Size, StyleRefinement, Styled, Window, GlobalElementId, InspectorElementId, point, Point, Display, IntoElement, DivFrameState};
use crate::smallvec::SmallVec;

#[track_caller]
pub fn animated_div() -> AnimatedDiv {
    AnimatedDiv {
        interactivity: Interactivity::new(),
        children: SmallVec::default(),
        children_prepaint_listener: None,
        prepaint_listener: None,
        image_cache: None,
        prepaint_order_fn: None,
    }
}

pub struct AnimatedDiv {
    interactivity: Interactivity,
    children: SmallVec<[StackSafe<AnyElement>; 2]>,
    children_prepaint_listener: Option<Box<dyn Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static>>,
    prepaint_listener: Option<Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>>,
    image_cache: Option<Box<dyn ImageCacheProvider>>,
    prepaint_order_fn: Option<Box<dyn Fn(&mut Window, &mut App) -> SmallVec<[usize; 8]>>>,
}

impl AnimatedDiv {
    pub fn on_children_prepainted(
        mut self,
        listener: impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.children_prepaint_listener = Some(Box::new(listener));
        self
    }

    pub fn on_prepainted(
        mut self,
        listener: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.prepaint_listener = Some(Box::new(listener));
        self
    }

    pub fn image_cache(mut self, cache: impl ImageCacheProvider) -> Self {
        self.image_cache = Some(Box::new(cache));
        self
    }

    pub fn with_dynamic_prepaint_order(
        mut self,
        order_fn: impl Fn(&mut Window, &mut App) -> SmallVec<[usize; 8]> + 'static,
    ) -> Self {
        self.prepaint_order_fn = Some(Box::new(order_fn));
        self
    }
}

impl Styled for AnimatedDiv {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for AnimatedDiv {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl ParentElement for AnimatedDiv {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements.into_iter().map(StackSafe::new))
    }
}

impl Element for AnimatedDiv {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    #[stacksafe]
    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut child_layout_ids = SmallVec::new();
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        let layout_id = window.with_image_cache(image_cache, |window| {
            self.interactivity.request_layout(
                global_id,
                inspector_id,
                window,
                cx,
                |style, window, cx| {
                    window.with_text_style(style.text_style().cloned(), |window| {
                        child_layout_ids = self
                            .children
                            .iter_mut()
                            .map(|child| child.request_layout(window, cx))
                            .collect::<SmallVec<_>>();
                        window.request_layout(style, child_layout_ids.iter().copied(), cx)
                    })
                },
            )
        });

        (layout_id, DivFrameState { child_layout_ids })
    }

    #[stacksafe]
    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        let has_prepaint_listener = self.children_prepaint_listener.is_some();
        let mut children_bounds = Vec::with_capacity(if has_prepaint_listener {
            request_layout.child_layout_ids.len()
        } else {
            0
        });

        let mut child_min = point(Pixels::MAX, Pixels::MAX);
        let mut child_max = Point::default();
        if let Some(handle) = self.interactivity.scroll_anchor.as_ref() {
            *handle.last_origin.borrow_mut() = bounds.origin - window.element_offset();
        }
        let content_size = if request_layout.child_layout_ids.is_empty() {
            bounds.size
        } else if let Some(scroll_handle) = self.interactivity.tracked_scroll_handle.as_ref() {
            let mut state = scroll_handle.0.borrow_mut();
            state.child_bounds = Vec::with_capacity(request_layout.child_layout_ids.len());
            for child_layout_id in &request_layout.child_layout_ids {
                let child_bounds = window.layout_bounds(*child_layout_id);
                child_min = child_min.min(&child_bounds.origin);
                child_max = child_max.max(&child_bounds.bottom_right());
                state.child_bounds.push(child_bounds);
            }
            (child_max - child_min).into()
        } else {
            for child_layout_id in &request_layout.child_layout_ids {
                let child_bounds = window.layout_bounds(*child_layout_id);
                child_min = child_min.min(&child_bounds.origin);
                child_max = child_max.max(&child_bounds.bottom_right());

                if has_prepaint_listener {
                    children_bounds.push(child_bounds);
                }
            }
            (child_max - child_min).into()
        };

        if let Some(scroll_handle) = self.interactivity.tracked_scroll_handle.as_ref() {
            scroll_handle.scroll_to_active_item();
        }

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, cx| {
                if style.display == Display::None { return hitbox; }
                window.with_image_cache(image_cache, |window| {
                    window.with_element_offset(scroll_offset, |window| {
                        if let Some(order_fn) = &self.prepaint_order_fn {
                            let order = order_fn(window, cx);
                            for idx in order {
                                if let Some(child) = self.children.get_mut(idx) {
                                    child.prepaint(window, cx);
                                }
                            }
                        } else {
                            for child in &mut self.children {
                                child.prepaint(window, cx);
                            }
                        }
                    });

                    if let Some(listener) = self.children_prepaint_listener.as_ref() {
                        listener(children_bounds, window, cx);
                    }

                    if let Some(listener) = self.prepaint_listener.as_ref() {
                        listener(bounds, window, cx)
                    }
                });

                hitbox
            },
        )
    }

    #[stacksafe]
    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        window.with_image_cache(image_cache, |window| {
            self.interactivity.paint(
                global_id, inspector_id, bounds,
                hitbox.as_ref(), window, cx,
                |style, window, cx| {
                    if style.display == Display::None { return; }
                    for child in &mut self.children {
                        child.paint(window, cx);
                    }
                },
            )
        });
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.interactivity.override_role
            .filter(|role| *role != accesskit::Role::GenericContainer)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.interactivity.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Self::PrepaintState,
        builder: &mut crate::A11ySubtreeBuilder,
    ) {
        if let Some(f) = self.interactivity.a11y_synthetic_children.take() {
            f(builder);
        }
    }
}

impl IntoElement for AnimatedDiv {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}