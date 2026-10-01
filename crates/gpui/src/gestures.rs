//! Touch gesture recognition vocabulary.
//!
//! GPUI recognizes gestures from raw [`TouchEvent`](crate::TouchEvent)s in a
//! single, portable arena in hgpui core: recognizers compete for in-flight
//! touches, winners claim them, and losers are cancelled. Recognized gestures
//! are surfaced through *existing* semantic events wherever possible, a tap
//! becomes [`ClickEvent::Touch`](crate::ClickEvent), a pan becomes
//! [`ScrollWheelEvent`](crate::ScrollWheelEvent)s carrying a
//! [`TouchPhase`](crate::TouchPhase), and a pinch becomes
//! [`PinchEvent`](crate::PinchEvent)s — so components written against
//! `on_click` and scroll containers work untouched on mobile.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::{Axis, IsZero, Pixels, Point, TouchPhase, point, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const SCROLL_EVENT_SEPARATION: Duration = Duration::from_millis(28);

/// Tracks the dominant axis across the events in a scroll gesture.
#[derive(Clone, Copy, Debug, Default)]
pub struct OngoingScroll {
    last_event: Option<Instant>,
    axis: Option<Axis>,
}

impl OngoingScroll {
    /// Filters the given delta to the dominant axis of the current scroll gesture.
    ///
    /// Gestures are delimited by their touch phase when available, with a timeout
    /// fallback for platforms that only emit [`TouchPhase::Moved`].
    pub fn filter(&mut self, delta: &mut Point<Pixels>, touch_phase: TouchPhase) {
        self.filter_at(delta, touch_phase, Instant::now())
    }

    fn filter_at(&mut self, delta: &mut Point<Pixels>, touch_phase: TouchPhase, now: Instant) {
        const UNLOCK_PERCENT: f32 = 1.9;
        const UNLOCK_LOWER_BOUND: Pixels = px(6.);

        if matches!(touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.last_event = None;
            self.axis = None;
            return;
        }

        let x = delta.x.abs();
        let y = delta.y.abs();
        if x.is_zero() && y.is_zero() {
            if touch_phase == TouchPhase::Started {
                self.last_event = None;
                self.axis = None;
            }
            return;
        }

        let starts_new_gesture = touch_phase == TouchPhase::Started
            || self
                .last_event
                .is_none_or(|last_event| now.duration_since(last_event) >= SCROLL_EVENT_SEPARATION);
        let mut axis = self.axis;
        if starts_new_gesture {
            axis = if x <= y {
                Some(Axis::Vertical)
            } else {
                Some(Axis::Horizontal)
            };
        } else if x.max(y) >= UNLOCK_LOWER_BOUND {
            match axis {
                Some(Axis::Vertical) if x > y && x >= y * UNLOCK_PERCENT => {
                    axis = None;
                }
                Some(Axis::Horizontal) if y > x && y >= x * UNLOCK_PERCENT => {
                    axis = None;
                }
                _ => {}
            }
        }

        self.last_event = Some(now);
        self.axis = axis;
        match axis {
            Some(Axis::Vertical) => delta.x = Pixels::ZERO,
            Some(Axis::Horizontal) => delta.y = Pixels::ZERO,
            None => {}
        }
    }
}

/// Feel constants consumed by gesture recognizers. Provided on a best-effort
/// basis, depending on each platform's support, defaulting to GPUI's own
/// (iOS flavored) values
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureTuning {
    /// Distance a touch may travel before it stops being a potential tap and
    /// becomes a pan/drag.
    pub touch_slop: Pixels,
    /// Maximum interval between taps for them to accumulate a tap count.
    pub multi_tap_interval: Duration,
    /// Maximum distance between taps for them to accumulate a tap count.
    pub multi_tap_slop: Pixels,
    /// How long a touch must remain within [`Self::touch_slop`] to be
    /// recognized as a long press.
    pub long_press_duration: Duration,
    /// Per-millisecond decay factor applied to scroll momentum after a fling.
    /// (`UIScrollView` uses `0.998` per millisecond for its normal
    /// deceleration rate.)
    pub momentum_decay_per_ms: f32,
    /// Minimum release velocity, in pixels per second, required to start
    /// scroll momentum.
    pub min_fling_velocity: f32,
}

impl Default for GestureTuning {
    fn default() -> Self {
        Self {
            touch_slop: px(8.),
            multi_tap_interval: Duration::from_millis(400),
            multi_tap_slop: px(16.),
            long_press_duration: Duration::from_millis(500),
            momentum_decay_per_ms: 0.998,
            min_fling_velocity: 50.,
        }
    }
}

/// The set of gesture kinds that participate in recognition.
///
/// Used by [`PlatformGestures::native_recognizers`] to declare which gestures
/// the platform recognizes natively rather than leaving to hgpui core's
/// portable recognizers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureKinds {
    /// Tap (and multi-tap), surfaced as [`ClickEvent::Touch`](crate::ClickEvent).
    pub tap: bool,
    /// Long press, surfaced as [`LongPressEvent`].
    pub long_press: bool,
    /// Pan/scroll (including fling momentum), surfaced as
    /// [`ScrollWheelEvent`](crate::ScrollWheelEvent)s.
    pub pan: bool,
    /// Pinch to zoom, surfaced as [`PinchEvent`](crate::PinchEvent)s.
    pub pinch: bool,
}

impl GestureKinds {
    /// No gestures; hgpui core's portable recognizers handle everything.
    pub const NONE: Self = Self {
        tap: false,
        long_press: false,
        pan: false,
        pinch: false,
    };

    /// All gesture kinds.
    pub const ALL: Self = Self {
        tap: true,
        long_press: true,
        pan: true,
        pinch: true,
    };
}

/// A long-press gesture, mobile's context-menu trigger.
///
/// A bare long press is surfaced as a [`ClickEvent`](crate::ClickEvent) with
/// `long_press: true`, delivered to aux-click listeners alongside right
/// clicks. This event is the raw hook for elements that need the gesture
/// itself (e.g. long-press to start a drag); the registration API ships
/// together with the gesture arena.
#[derive(Clone, Debug, Default)]
pub struct LongPressEvent {
    /// The position of the touch that was recognized as a long press.
    pub position: Point<Pixels>,
}

/// Platform gesture recognition services.
///
/// If your mobile platform supports native gesture recognition, use this
/// to share it with GPUI.
pub trait PlatformGestures {
    /// Feel constants for the portable recognizers on this platform.
    fn tuning(&self) -> GestureTuning {
        GestureTuning::default()
    }

    /// The gesture kinds this platform recognizes natively.
    fn native_recognizers(&self) -> GestureKinds {
        GestureKinds::NONE
    }
}

/// A no-op [`PlatformGestures`] implementation: no native recognizers and
/// default tuning. Suitable for desktop platforms and tests.
pub struct NullPlatformGestures;

impl PlatformGestures for NullPlatformGestures {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point;

    const FRAME: Duration = Duration::from_millis(16);

    /// The omega the state uses for a given length.
    fn omega(length: Duration) -> f32 {
        SPRING_OMEGA_PER_LENGTH / length.as_secs_f32()
    }

    #[test]
    fn the_spring_converges_without_overshooting() {
        let mut position = 100.0_f32;
        let mut velocity = 0.0_f32;
        let omega = omega(SmoothScroll::default().length);

        for _ in 0..60 {
            let previous = position;
            (position, velocity) = step_spring(position, velocity, omega, 0.016);
            assert!(
                position >= 0.0 && position <= previous,
                "critically damped means no overshoot: {position} after {previous}"
            );
        }

        assert!(position.abs() < 0.1, "should have arrived, got {position}");
    }

    #[test]
    fn the_spring_reaches_two_percent_in_the_configured_length() {
        let length = Duration::from_millis(90);
        let (position, _) = step_spring(
            100.0,
            0.0,
            omega(length),
            length.as_secs_f32(),
        );

        assert!(
            position.abs() <= 2.5,
            "a 90ms glide should be within 2% after 90ms, got {position}"
        );
    }

    #[test]
    fn smoothing_glides_to_the_target_and_stops() {
        let mut state = SmoothScrollState::default();
        let mut rendered = Point::<Pixels>::default();
        let start = Instant::now();

        state.set_config(Some(SmoothScroll::default()), &mut rendered);
        state.set_target(point(Pixels::ZERO, px(-100.)), &mut rendered);
        assert_eq!(rendered.y, Pixels::ZERO, "the target moves, not the offset");

        // One frame in: part of the way there, still moving.
        assert!(state.step(&mut rendered, start + FRAME));
        assert!(
            rendered.y < Pixels::ZERO && rendered.y > px(-100.),
            "in flight: {:?}",
            rendered.y
        );

        // Once it is over, it sits exactly on the target.
        for frame in 2..30 {
            state.step(&mut rendered, start + FRAME * frame);
        }
        assert_eq!(rendered.y, px(-100.));
        assert!(!state.is_animating());
    }

    #[test]
    fn without_smoothing_the_offset_jumps_to_the_target() {
        let mut state = SmoothScrollState::default();
        let mut rendered = Point::<Pixels>::default();

        state.set_target(point(Pixels::ZERO, px(-100.)), &mut rendered);

        assert_eq!(rendered.y, px(-100.), "no glide unless it was asked for");
        assert!(!state.step(&mut rendered, Instant::now()));
        assert_eq!(rendered.y, px(-100.));
    }

    #[test]
    fn a_second_target_mid_flight_keeps_the_movement_smooth() {
        let mut state = SmoothScrollState::default();
        let mut rendered = Point::<Pixels>::default();
        let start = Instant::now();

        state.set_config(Some(SmoothScroll::default()), &mut rendered);
        state.set_target(point(Pixels::ZERO, px(-100.)), &mut rendered);
        state.step(&mut rendered, start + FRAME);
        let first_step = rendered.y;

        // The wheel ticks again while the first glide is still running: the offset must carry
        // on from where it is, with the velocity it already had.
        state.set_target(point(Pixels::ZERO, px(-200.)), &mut rendered);
        assert_eq!(rendered.y, first_step, "re-targeting does not move the offset");

        state.step(&mut rendered, start + FRAME * 2);
        let second_step = rendered.y - first_step;
        assert!(
            second_step < Pixels::ZERO && second_step.abs() > first_step.abs(),
            "carrying momentum, not starting over: {first_step:?} then {second_step:?}"
        );
    }

    #[test]
    fn snapping_ignores_the_glide() {
        let mut state = SmoothScrollState::default();
        let mut rendered = Point::<Pixels>::default();

        state.set_config(Some(SmoothScroll::default()), &mut rendered);
        state.set_target(point(Pixels::ZERO, px(-100.)), &mut rendered);
        state.snap_to(point(Pixels::ZERO, px(-40.)), &mut rendered);

        assert_eq!(rendered.y, px(-40.));
        assert_eq!(state.target().y, px(-40.));
        assert!(!state.is_animating());
    }

    #[test]
    fn turning_smoothing_off_lands_on_the_target() {
        let mut state = SmoothScrollState::default();
        let mut rendered = Point::<Pixels>::default();

        state.set_config(Some(SmoothScroll::default()), &mut rendered);
        state.set_target(point(Pixels::ZERO, px(-100.)), &mut rendered);
        state.step(&mut rendered, Instant::now() + FRAME);
        assert!(rendered.y > px(-100.));

        state.set_config(None, &mut rendered);
        assert_eq!(rendered.y, px(-100.), "no half-way state once it is off");
    }

    #[test]
    fn ongoing_scroll_locks_to_dominant_axis() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Started, now);
        assert_eq!(ongoing_scroll.axis, Some(Axis::Horizontal));
        assert_eq!(horizontal_delta, point(px(10.), px(0.)));

        let mut continued_delta = point(px(3.), px(2.));
        ongoing_scroll.filter_at(
            &mut continued_delta,
            TouchPhase::Moved,
            now + Duration::from_millis(1),
        );
        assert_eq!(ongoing_scroll.axis, Some(Axis::Horizontal));
        assert_eq!(continued_delta, point(px(3.), px(0.)));
    }

    #[test]
    fn ongoing_scroll_unlocks_when_direction_changes() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Started, now);

        let mut vertical_delta = point(px(2.), px(10.));
        ongoing_scroll.filter_at(
            &mut vertical_delta,
            TouchPhase::Moved,
            now + Duration::from_millis(1),
        );
        assert_eq!(ongoing_scroll.axis, None);
        assert_eq!(vertical_delta, point(px(2.), px(10.)));
    }

    #[test]
    fn ongoing_scroll_starts_new_gesture_at_timeout_boundary() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Moved, now);

        let mut vertical_delta = point(px(2.), px(10.));
        ongoing_scroll.filter_at(
            &mut vertical_delta,
            TouchPhase::Moved,
            now + SCROLL_EVENT_SEPARATION,
        );
        assert_eq!(ongoing_scroll.axis, Some(Axis::Vertical));
        assert_eq!(vertical_delta, point(px(0.), px(10.)));
    }

    #[test]
    fn ongoing_scroll_ignores_zero_delta_and_resets_when_ended() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Started, now);

        let mut zero_delta = Point::default();
        ongoing_scroll.filter_at(
            &mut zero_delta,
            TouchPhase::Ended,
            now + Duration::from_millis(1),
        );
        assert_eq!(ongoing_scroll.axis, None);

        let mut vertical_delta = point(px(2.), px(3.));
        ongoing_scroll.filter_at(
            &mut vertical_delta,
            TouchPhase::Moved,
            now + Duration::from_millis(2),
        );
        assert_eq!(ongoing_scroll.axis, Some(Axis::Vertical));
        assert_eq!(vertical_delta, point(px(0.), px(3.)));
    }

    #[test]
    fn ongoing_scroll_ignores_zero_delta_movement() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Started, now);

        let mut zero_delta = Point::default();
        ongoing_scroll.filter_at(
            &mut zero_delta,
            TouchPhase::Moved,
            now + SCROLL_EVENT_SEPARATION,
        );

        let mut vertical_delta = point(px(2.), px(10.));
        ongoing_scroll.filter_at(
            &mut vertical_delta,
            TouchPhase::Moved,
            now + SCROLL_EVENT_SEPARATION,
        );
        assert_eq!(ongoing_scroll.axis, Some(Axis::Vertical));
        assert_eq!(vertical_delta, point(px(0.), px(10.)));
    }

    #[test]
    fn ongoing_scroll_supports_moved_only_platforms() {
        let now = Instant::now();
        let mut ongoing_scroll = OngoingScroll::default();
        let mut horizontal_delta = point(px(10.), px(2.));
        ongoing_scroll.filter_at(&mut horizontal_delta, TouchPhase::Moved, now);
        assert_eq!(ongoing_scroll.axis, Some(Axis::Horizontal));
        assert_eq!(horizontal_delta, point(px(10.), px(0.)));
    }
}

/// Makes a scroll container ease towards its target offset instead of jumping to it.
///
/// Off by default: opt in per element with
/// [`.smooth_scroll(true)`](crate::Styled::smooth_scroll).
///
/// Once enabled, *every* change to the offset glides — the wheel, a trackpad, `scroll_to_item`,
/// anything that sets the offset — because they all move the same target and the rendered
/// offset chases it with a critically damped spring. That is what reads as "a little damping,
/// and it keeps coasting for a moment after you stop".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SmoothScroll {
    /// How long the offset takes to converge to within 2% of its target.
    ///
    /// Bigger is slower and floatier — this is "how much damping" the glide has.
    /// Defaults to 200ms.
    pub length: Duration,
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self {
            length: Duration::from_millis(200),
        }
    }
}

impl SmoothScroll {
    /// Sets how long the glide takes to converge.
    pub fn length(mut self, length: Duration) -> Self {
        self.length = length;
        self
    }
}

/// Elapsed fraction of an animation, before easing: monotone, and clamped to `0.0 -> 1.0`.
pub(crate) fn progress(started: Instant, now: Instant, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32();
    if duration <= 0.0 {
        return 1.0;
    }
    ((now - started).as_secs_f32() / duration).clamp(0.0, 1.0)
}

/// The offset a scroll container is heading for: the same as the rendered offset when
/// smoothing is off, and the glide's destination when it is on.
pub(crate) fn scroll_target(
    smooth: Option<&Rc<RefCell<SmoothScrollState>>>,
    rendered: Point<Pixels>,
) -> Point<Pixels> {
    match smooth {
        Some(smooth) => smooth.borrow().target(),
        None => rendered,
    }
}

/// Moves a scroll offset to `target`: immediately when smoothing is off, and as a glide (the
/// rendered offset catches up over the following frames) when it is on.
pub(crate) fn set_scroll_target(
    smooth: Option<&Rc<RefCell<SmoothScrollState>>>,
    target: Point<Pixels>,
    rendered: &mut Point<Pixels>,
) {
    match smooth {
        Some(smooth) => smooth.borrow_mut().set_target(target, rendered),
        None => *rendered = target,
    }
}

/// The damping coefficient that puts a critically damped spring within 2% of its target after
/// exactly `length`: the solution of `(1 + x) * exp(-x) = 0.02`.
///
/// (Neovide's cursor uses `4.0 / length`, which is a little loose — that reaches ~9% at
/// `length`. Same shape, just an honest unit for the parameter.)
const SPRING_OMEGA_PER_LENGTH: f32 = 5.834;

/// Distances this small are indistinguishable from arrived.
const SMOOTH_SCROLL_EPSILON: f32 = 0.1;
/// Speeds this low, at that distance, are indistinguishable from stopped.
const SMOOTH_SCROLL_VELOCITY_EPSILON: f32 = 10.0;

/// One axis of a critically damped spring, in the formulation Neovide uses for its cursor:
/// `position` is the distance still to travel and `velocity` its rate of change. Both decay
/// together, reaching a 2% tolerance after `5.834 / omega` seconds.
fn step_spring(position: f32, velocity: f32, omega: f32, dt: f32) -> (f32, f32) {
    if position == 0.0 && velocity == 0.0 {
        return (0.0, 0.0);
    }
    // Analytic solution of a critically damped oscillator (zeta = 1), with a and b the initial
    // conditions solved from the position and velocity at dt = 0.
    let a = position;
    let b = position * omega + velocity;
    let c = (-omega * dt).exp();
    let position = (a + b * dt) * c;
    let velocity = c * (-a * omega - b * dt * omega + b);
    (position, velocity)
}

/// Per-container state for [`SmoothScroll`], kept across frames by the scroll handle (or the
/// element state, for a scrolling `div`).
#[derive(Default, Debug)]
pub(crate) struct SmoothScrollState {
    /// Where the offset should end up. Every writer of the scroll offset sets this.
    target: Point<Pixels>,
    /// How fast the rendered offset is moving, per axis, in pixels per second.
    velocity: Point<Pixels>,
    /// When the offset was last stepped, so the spring can use a real delta time.
    last_tick: Option<Instant>,
    /// `None` while smoothing is off: the rendered offset simply is the target.
    config: Option<SmoothScroll>,
    /// Whether the rendered offset is still moving towards the target.
    animating: bool,
}

impl SmoothScrollState {
    /// Where the offset should end up.
    pub(crate) fn target(&self) -> Point<Pixels> {
        self.target
    }

    /// Moves the target, letting the rendered offset glide there. When smoothing is off the
    /// rendered offset is set immediately, so callers behave exactly as they did before.
    pub(crate) fn set_target(&mut self, target: Point<Pixels>, rendered: &mut Point<Pixels>) {
        self.target = target;
        if self.config.is_none() {
            *rendered = target;
        }
    }

    /// Moves the target by a delta (what the wheel does).
    pub(crate) fn add_target(&mut self, delta: Point<Pixels>, rendered: &mut Point<Pixels>) {
        self.set_target(self.target + delta, rendered);
    }

    /// Puts the offset where it is asked to be, with no glide at all. For corrections that
    /// must be invisible, like keeping a view anchored on its content while the list changes.
    pub(crate) fn snap_to(&mut self, target: Point<Pixels>, rendered: &mut Point<Pixels>) {
        self.target = target;
        *rendered = target;
        self.velocity = Point::default();
        self.animating = false;
    }

    /// Turns smoothing on or off, as configured on the element this frame.
    pub(crate) fn set_config(&mut self, config: Option<SmoothScroll>, rendered: &mut Point<Pixels>) {
        if self.config != config {
            self.config = config;
            if config.is_none() {
                self.snap_to(self.target, rendered);
            }
        }
    }

    /// Whether the rendered offset is still moving. (The lists drive their frames from the
    /// value [`step`](Self::step) returns; this is here for tests.)
    #[cfg(test)]
    pub(crate) fn is_animating(&self) -> bool {
        self.animating
    }

    /// Advances the rendered offset towards the target. Returns whether it is still moving.
    pub(crate) fn step(&mut self, rendered: &mut Point<Pixels>, now: Instant) -> bool {
        let Some(config) = self.config else {
            *rendered = self.target;
            self.animating = false;
            return false;
        };

        let dt = match self.last_tick {
            // A real delta time, clamped so a dropped frame doesn't teleport the offset.
            Some(last) => (now - last).as_secs_f32().clamp(0.0, 0.1),
            None => 1.0 / 60.0,
        };
        self.last_tick = Some(now);

        let omega = SPRING_OMEGA_PER_LENGTH / config.length.as_secs_f32().max(1e-4);
        let (remaining_x, velocity_x) =
            step_spring(self.target.x.0 - rendered.x.0, self.velocity.x.0, omega, dt);
        let (remaining_y, velocity_y) =
            step_spring(self.target.y.0 - rendered.y.0, self.velocity.y.0, omega, dt);

        rendered.x.0 = self.target.x.0 - remaining_x;
        rendered.y.0 = self.target.y.0 - remaining_y;
        self.velocity = point(px(velocity_x), px(velocity_y));

        self.animating = remaining_x.abs() > SMOOTH_SCROLL_EPSILON
            || remaining_y.abs() > SMOOTH_SCROLL_EPSILON
            || velocity_x.abs() > SMOOTH_SCROLL_VELOCITY_EPSILON
            || velocity_y.abs() > SMOOTH_SCROLL_VELOCITY_EPSILON;

        if !self.animating {
            *rendered = self.target;
            self.velocity = Point::default();
        }

        self.animating
    }
}
