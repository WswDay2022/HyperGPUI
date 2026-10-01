use crate::smallvec::SmallVec;
use crate::{accesskit, point, AnyElement, App, Bounds, Display, DivFrameState, Element, ElementId, GlobalElementId, Hitbox, ImageCacheProvider, InspectorElementId, InteractiveElement, Interactivity, IntoElement, LayoutId, ParentElement, Pixels, Point, Stateful, StyleRefinement, Styled, Window};
use hgpui::linear;
use stacksafe::{StackSafe, stacksafe};
use std::rc::Rc;
use std::time::Duration;

/// How an [`AnimatedDiv`] animates, and which parts of its bounds take part.
pub struct AnimatedDivProps {
    /// Animate changes to the width.
    pub animating_width: bool,
    /// Animate changes to the height.
    pub animating_height: bool,
    /// Animate changes to the horizontal position.
    pub animating_x: bool,
    /// Animate changes to the vertical position.
    pub animating_y: bool,
    /// Easing applied to the animation's progress.
    pub easing: Rc<dyn Fn(f32) -> f32 + 'static>,
    /// How long each animation takes.
    pub duration: Duration,
}

impl Default for AnimatedDivProps {
    fn default() -> Self {
        Self {
            animating_width: true,
            animating_height: true,
            animating_x: true,
            duration: Duration::from_millis(80),
            easing: Rc::new(linear),
            animating_y: true,
        }
    }
}

impl<F: Fn(f32) -> f32 + 'static> From<F> for AnimatedDivProps {
    fn from(easing: F) -> Self {
        Self {
            easing: Rc::new(easing),
            ..Self::default()
        }
    }
}

impl From<Duration> for AnimatedDivProps {
    fn from(duration: Duration) -> Self {
        Self { duration, ..Self::default() }
    }
}

/// Creates an [`AnimatedDiv`], keyed by `id`.
///
/// The animation state lives in a keyed transition, so the element must keep the same `id`
/// across frames (and unique among its siblings) for the animation to be continuous.
#[track_caller]
pub fn animated_div(
    id: impl Into<ElementId>,
    props: impl Into<AnimatedDivProps>,
) -> Stateful<AnimatedDiv> {
    let id = id.into();
    Stateful {
        element: AnimatedDiv {
            interactivity: Interactivity {
                element_id: Some(id.clone()),
                ..Interactivity::new()
            },
            element_id: id,
            children: SmallVec::default(),
            props: props.into(),
            children_prepaint_listener: None,
            prepaint_listener: None,
            image_cache: None,
            prepaint_order_fn: None,
        }
    }
}

/// A container that animates its bounds when its layout changes.
///
/// See [`animated_div`] and [`AnimatedDivProps`].
pub struct AnimatedDiv {
    interactivity: Interactivity,
    children: SmallVec<[StackSafe<AnyElement>; 2]>,
    props: AnimatedDivProps,
    element_id: ElementId,
    children_prepaint_listener: Option<Box<dyn Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static>>,
    prepaint_listener: Option<Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>>,
    image_cache: Option<Box<dyn ImageCacheProvider>>,
    prepaint_order_fn: Option<Box<dyn Fn(&mut Window, &mut App) -> SmallVec<[usize; 8]>>>,
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

    fn id(mut self, id: impl Into<ElementId>) -> Stateful<Self> {
        let id = id.into();
        self.element_id = id.clone();
        self.interactivity.element_id = Some(id);
        Stateful { element: self }
    }
}

impl ParentElement for AnimatedDiv {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements.into_iter().map(StackSafe::new))
    }
}

impl Element for AnimatedDiv {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = (Option<Hitbox>, Bounds<Pixels>);

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id.clone())
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

        (layout_id.clone(), DivFrameState { child_layout_ids })
    }

    #[stacksafe]
    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        raw_bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> (Option<Hitbox>, Bounds<Pixels>) {
        let transition = window.use_keyed_transition(
            self.element_id.clone(), cx, self.props.duration.clone(), |_, _| raw_bounds)
            .with_raw_easing(self.props.easing.clone());
        if *transition.read_goal(cx) != raw_bounds {
            transition.update(cx, |transition, _| *transition = raw_bounds);
        }
        let mut bounds = *transition.evaluate(window, cx);
        if !self.props.animating_width { bounds.size.width = raw_bounds.size.width };
        if !self.props.animating_height { bounds.size.height = raw_bounds.size.height };
        if !self.props.animating_x { bounds.origin.x = raw_bounds.origin.x };
        if !self.props.animating_y { bounds.origin.y = raw_bounds.origin.y };

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

        (self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, cx| {
                if style.display == Display::None { return hitbox; }
                window.with_image_cache(image_cache, |window| {
                    window.with_element_offset(scroll_offset + bounds.origin - raw_bounds.origin, |window| {
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
        ), bounds)
    }

    #[stacksafe]
    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        (hitbox, bounds): &mut (Option<Hitbox>, Bounds<Pixels>),
        window: &mut Window,
        cx: &mut App,
    ) {
        let image_cache = self
            .image_cache
            .as_mut()
            .map(|provider| provider.provide(window, cx));

        window.with_image_cache(image_cache, |window| {
            self.interactivity.paint(
                global_id, inspector_id, *bounds,
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