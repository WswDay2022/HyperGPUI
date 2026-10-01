use scheduler::Instant;
use std::{rc::Rc, time::Duration};

use crate::{
    AnyElement, App, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    ParentElement, Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// An animation that can be applied to an element.
#[derive(Clone)]
pub struct Animation {
    /// The amount of time for which this animation should run
    pub duration: Duration,
    /// Whether to repeat this animation when it finishes
    pub oneshot: bool,
    /// A function that takes a delta between 0 and 1 and returns a new delta
    /// between 0 and 1 based on the given easing function.
    pub easing: Rc<dyn Fn(f32) -> f32>,
}

impl Animation {
    /// Create a new animation with the given duration.
    /// By default the animation will only run once and will use a linear easing function.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            oneshot: true,
            easing: Rc::new(linear),
        }
    }

    /// Set the animation to loop when it finishes.
    pub fn repeat(mut self) -> Self {
        self.oneshot = false;
        self
    }

    /// Set the easing function to use for this animation.
    /// The easing function will take a time delta between 0 and 1 and return a new delta
    /// between 0 and 1
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }
}

/// An extension trait for adding the animation wrapper to both Elements and Components
///
/// Animations rendered through this trait automatically respect
/// [`App::reduce_motion`](crate::App::reduce_motion): when it is set,
/// the element is rendered in a static state (the end state for oneshot
/// animations, the start state for repeating ones) and no animation frames are
/// scheduled.
pub trait AnimationExt {
    /// Render this component or element with an animation
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
        }
    }

    /// Render this component or element with a chain of animations
    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(animator),
            animations: animations.into(),
        }
    }
}

impl<E: IntoElement + 'static> AnimationExt for E {}

/// A GPUI element that applies an animation to another element
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

impl<E: ParentElement> ParentElement for AnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> AnimationElement<E> {
    /// Returns a new [`AnimationElement<E>`] after applying the given function
    /// to the element being animated.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> AnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for AnimationElement<E> {
    type Element = AnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct AnimationState {
    start: Instant,
    animation_ix: usize,
}

impl<E: IntoElement + 'static> Element for AnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: Instant::now(),
                animation_ix: 0,
            });
            let (animation_ix, delta, done) = if cx.reduce_motion() {
                let animation_ix = self.animations.len() - 1;
                let delta = if self.animations[animation_ix].oneshot {
                    1.0
                } else {
                    0.0
                };
                (animation_ix, delta, true)
            } else {
                let animation_ix = state.animation_ix;

                let mut delta = state.start.elapsed().as_secs_f32()
                    / self.animations[animation_ix].duration.as_secs_f32();

                let mut done = false;
                if delta > 1.0 {
                    if self.animations[animation_ix].oneshot {
                        if animation_ix >= self.animations.len() - 1 {
                            done = true;
                        } else {
                            state.start = Instant::now();
                            state.animation_ix += 1;
                        }
                        delta = 1.0;
                    } else {
                        delta %= 1.0;
                    }
                }
                (animation_ix, delta, done)
            };
            let delta = (self.animations[animation_ix].easing)(delta);

            debug_assert!(
                (0.0..=1.0).contains(&delta),
                "delta should always be between 0 and 1"
            );

            let element = self.element.take().expect("should only be called once");
            let mut element = (self.animator)(element, animation_ix, delta).into_any_element();

            if !done {
                window.request_animation_frame();
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

mod easing {
    use std::f32::consts::PI;

    /// The linear easing function, or delta itself
    pub fn linear(delta: f32) -> f32 {
        delta
    }

    /// The quadratic easing function, delta * delta
    pub fn quadratic(delta: f32) -> f32 {
        delta * delta
    }

    /// The quadratic ease-in-out function, which starts and ends slowly but speeds up in the middle
    pub fn ease_in_out(delta: f32) -> f32 {
        if delta < 0.5 {
            2.0 * delta * delta
        } else {
            let x = -2.0 * delta + 2.0;
            1.0 - x * x / 2.0
        }
    }

    /// The Quint ease-out function, which starts quickly and decelerates to a stop
    pub fn ease_out_quint() -> impl Fn(f32) -> f32 {
        move |delta| 1.0 - (1.0 - delta).powi(5)
    }

    /// The cubic ease-out function: fast at first, settling gently.
    ///
    /// This is the default easing of the animated lists. It is the same shape as CSS
    /// `ease-out`, just cheap to evaluate.
    pub fn ease_out_cubic(delta: f32) -> f32 {
        let n = delta - 1.0;
        n * n * n + 1.0
    }

    /// Apply the given easing function, first in the forward direction and then in the reverse direction
    pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
        move |delta| {
            if delta < 0.5 {
                easing(delta * 2.0)
            } else {
                easing((1.0 - delta) * 2.0)
            }
        }
    }

    /// A cubic Bézier easing curve, exactly like CSS `cubic-bezier(x1, y1, x2, y2)`.
    ///
    /// The curve runs from `(0, 0)` to `(1, 1)` through the two given control points. As in CSS,
    /// `x1` and `x2` are clamped to `0..=1` (the time axis has to keep moving forward), while
    /// `y1` and `y2` are free — going above `1` is how an overshooting "back" curve is written:
    ///
    /// ```
    /// # use hgpui::cubic_bezier;
    /// // `ease-out-back`: overshoots past the target, then settles back onto it.
    /// let ease_out_back = cubic_bezier(0.34, 1.56, 0.64, 1.0);
    /// assert!(ease_out_back(0.7) > 1.0);
    /// assert!((ease_out_back(1.0) - 1.0).abs() < 1e-6);
    ///
    /// // The diagonal is the linear curve.
    /// let linear = cubic_bezier(0.0, 0.0, 1.0, 1.0);
    /// assert!((linear(0.4) - 0.4).abs() < 1e-5);
    /// ```
    pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
        let x1 = x1.clamp(0.0, 1.0);
        let x2 = x2.clamp(0.0, 1.0);

        // Each axis as a polynomial, as in WebKit's `UnitBezier`.
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;

        let sample = |a: f32, b: f32, c: f32, t: f32| ((a * t + b) * t + c) * t;

        move |delta: f32| {
            let target = delta.clamp(0.0, 1.0);
            if target <= 0.0 {
                return 0.0;
            } else if target >= 1.0 {
                return 1.0;
            }

            // The curve is parameterized by `t`, not by time: find the `t` whose x is the time
            // we were asked about. Newton-Raphson first, since it converges in a few steps...
            let error_at = |t: f32| sample(ax, bx, cx, t) - target;
            let mut t = target;
            for _ in 0..8 {
                let error = error_at(t);
                if error.abs() < 1e-6 {
                    return sample(ay, by, cy, t);
                }
                let slope = (3.0 * ax * t + 2.0 * bx) * t + cx;
                if slope.abs() < 1e-6 {
                    break;
                }
                t -= error / slope;
            }

            // ...and bisection as the fallback, which cannot diverge.
            let (mut low, mut high) = (0.0f32, 1.0f32);
            let mut t = target;
            for _ in 0..40 {
                let x_at_t = sample(ax, bx, cx, t);
                if (x_at_t - target).abs() < 1e-6 {
                    break;
                }
                if x_at_t < target {
                    low = t;
                } else {
                    high = t;
                }
                t = (low + high) / 2.0;
            }

            sample(ay, by, cy, t)
        }
    }

    /// A custom easing function for pulsating alpha that slows down as it approaches 0.1
    pub fn pulsating_between(min: f32, max: f32) -> impl Fn(f32) -> f32 {
        let range = max - min;

        move |delta| {
            // Use a combination of sine and cubic functions for a more natural breathing rhythm
            let t = (delta * 2.0 * PI).sin();
            let breath = (t * t * t + t) / 2.0;

            // Map the breath to our desired alpha range
            let normalized_alpha = (breath + 1.0) / 2.0;

            min + (normalized_alpha * range)
        }
    }
}

#[cfg(test)]
mod easing_tests {
    use super::easing::*;

    #[test]
    fn cubic_bezier_matches_the_css_keywords() {
        // The diagonal is linear.
        let linear = cubic_bezier(0.0, 0.0, 1.0, 1.0);
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            assert!(
                (linear(t) - t).abs() < 1e-5,
                "linear at {t}: {}",
                linear(t)
            );
        }

        // `ease` is cubic-bezier(0.25, 0.1, 0.25, 1), and its midpoint is a well-known value.
        let ease = cubic_bezier(0.25, 0.1, 0.25, 1.0);
        assert!((ease(0.5) - 0.8024).abs() < 1e-3, "ease(0.5) = {}", ease(0.5));
    }

    #[test]
    fn cubic_bezier_hits_both_ends() {
        let curve = cubic_bezier(0.42, 0.0, 0.58, 1.0);

        assert_eq!(curve(0.0), 0.0);
        assert_eq!(curve(1.0), 1.0);
        // Out-of-range times are clamped, like CSS.
        assert_eq!(curve(-1.0), 0.0);
        assert_eq!(curve(2.0), 1.0);
    }

    #[test]
    fn cubic_bezier_can_overshoot() {
        // `ease-out-back`, which is the reason `y` is not clamped.
        let ease_out_back = cubic_bezier(0.34, 1.56, 0.64, 1.0);

        assert!(ease_out_back(0.7) > 1.0, "overshoots: {}", ease_out_back(0.7));
        // It is a "fast out" curve: well ahead of linear before it overshoots. (0.703 here,
        // which matches the standard `easeOutBack` polynomial to within a thousandth.)
        assert!(ease_out_back(0.2) > 0.7, "starts fast: {}", ease_out_back(0.2));
    }

    #[test]
    fn cubic_bezier_is_monotonic_when_it_should_be() {
        let ease_in_out = cubic_bezier(0.42, 0.0, 0.58, 1.0);
        let mut previous = 0.0;

        for step in 0..=100 {
            let value = ease_in_out(step as f32 / 100.0);
            assert!(value >= previous, "not monotonic at {step}: {value} < {previous}");
            previous = value;
        }

        assert!((previous - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_bezier_clamps_the_control_points_on_the_time_axis() {
        // CSS requires x1 and x2 to be in 0..=1; out-of-range ones are clamped rather than
        // rejected, and a curve with them is still a usable (if odd) easing.
        let curve = cubic_bezier(-2.0, 0.0, 3.0, 1.0);

        assert_eq!(curve(0.0), 0.0);
        assert_eq!(curve(1.0), 1.0);
        let mid = curve(0.5);
        assert!(mid.is_finite() && (0.0..=1.0).contains(&mid), "{mid}");
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use crate::{
        Animation, Context, InteractiveElement, Render, TestAppContext, WindowHandle, div,
        prelude::*, px, size,
    };

    use super::*;

    struct AnimationTestView {
        rendered_deltas: Rc<RefCell<Vec<f32>>>,
    }

    impl Render for AnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_deltas = self.rendered_deltas.clone();
            div().size_full().child(div().with_animation(
                "repeating-animation",
                Animation::new(Duration::from_secs(1)).repeat(),
                move |this, delta| {
                    rendered_deltas.borrow_mut().push(delta);
                    this
                },
            ))
        }
    }

    fn open_test_window(
        cx: &mut TestAppContext,
    ) -> (Rc<RefCell<Vec<f32>>>, WindowHandle<AnimationTestView>) {
        let rendered_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let rendered_deltas = rendered_deltas.clone();
            move |_, _| AnimationTestView { rendered_deltas }
        });
        cx.run_until_parked();
        (rendered_deltas, window)
    }

    fn simulate_next_frame(
        window: &WindowHandle<AnimationTestView>,
        cx: &mut TestAppContext,
    ) -> usize {
        let callback_count = window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        callback_count
    }
    // Before parent-animation-element, using .with_animation
    // would not allow chaining .parent after. This is just a
    // build check that we can call div().id().with_animation().child()
    #[test]
    fn test_animation_parent() {
        div()
            .id("id")
            //
            .with_animation(
                "animation",
                Animation::new(Duration::from_secs(1)),
                |el, _t| {
                    //
                    el
                },
            )
            .child(
                //
                div(),
            );
    }

    #[hgpui::test]
    fn test_repeating_animation_schedules_animation_frames(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(rendered_deltas.borrow().len(), 1);

        for expected_frames in 2..=3 {
            assert_eq!(simulate_next_frame(&window, cx), 1);
            assert_eq!(rendered_deltas.borrow().len(), expected_frames);
        }
    }

    #[hgpui::test]
    fn test_reduce_motion_renders_single_static_frame(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(*rendered_deltas.borrow(), vec![0.0]);

        assert_eq!(simulate_next_frame(&window, cx), 0);
        assert_eq!(*rendered_deltas.borrow(), vec![0.0]);
    }
}
