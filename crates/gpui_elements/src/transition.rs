//! A Vue-like `<Transition>` for hgpui.
//!
//! [`transition`] wraps one child and animates it **in** and **out** whenever the key you
//! give it changes. It is the immediate-mode answer to Vue's `<Transition>`: you supply the
//! child as a *factory* (a closure that builds an element), and the styles that turn a progress
//! value into whatever you like.
//!
//! ```
//! # use hgpui::{App, IntoElement, ParentElement as _, Styled as _, Window, div, px};
//! # use hgpui_elements::transition::{transition, TransitionMode};
//! # fn render(_window: &mut Window, _cx: &mut App, tab: usize) -> impl IntoElement {
//! transition("page", tab)
//!     .duration(std::time::Duration::from_millis(180))
//!     .mode(TransitionMode::OutIn)
//!     // `delta` is the eased progress, `time` the plain one, both going 0 -> 1. During a
//!     // leave they are the *leave* progress.
//!     .enter(|child, delta, time| {
//!         div()
//!             .opacity(delta)
//!             .translate_y(px(12.) * (1. - time))
//!             .child(child)
//!             .into_any_element()
//!     })
//!     .leave(|child, delta, time| {
//!         div()
//!             .opacity(1. - delta)
//!             .translate_y(px(-12.) * time)
//!             .child(child)
//!             .into_any_element()
//!     })
//!     .child(move |_window, _cx| match tab {
//!         0 => div().child("home").into_any_element(),
//!         _ => div().child("settings").into_any_element(),
//!     })
//! # }
//! ```
//!
//! ### Semantics
//!
//! - The key identifies the child. **A key change is what triggers a transition**; a child
//!   that merely re-renders with new data does not animate (same as Vue).
//! - The outgoing child is rendered from the factory captured on the *previous* frame, so it
//!   shows the state it had just before the switch — a snapshot, for free.
//! - Only painting is animated, never layout: the incoming child owns the space, and the
//!   outgoing one sits in an absolutely positioned overlay, so siblings never reflow. (The
//!   outgoing child is therefore laid out against the *new* child's size.) `OutIn` is the
//!   exception the ordering forces: the outgoing child leaves **in the flow**, keeping the
//!   space until it is gone, and only then does the incoming child take it over — so there the
//!   layout really does change, once the exit is over.
//! - [`TransitionMode`] picks whether the two animations overlap, are sequenced, or are
//!   ordered the other way around. If the key changes again mid-flight the newest key wins.
//! - While the animation runs, frames are requested through
//!   [`Transition::evaluate`](hgpui::Transition::evaluate); once it finishes the requests
//!   stop. [`App::reduce_motion`] is respected: transitions jump straight to the end.
//!
//! ### Limitations
//!
//! - The element's keyed state lives as long as the element keeps rendering: if the element
//!   itself unmounts mid-transition, the transition goes with it.
//! - The leaving child can still receive mouse events for the duration of its exit: in `OutIn`
//!   it is the only child on screen, and in the other modes it is painted underneath the
//!   incoming child, which usually covers it.

use hgpui::{div, AnyElement, App, AppContext as _, Div, Element, ElementId, EnterStyle, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, LeaveStyle, ParentElement as _, Stateful, Styled as _, Transition, TransitionState, Window};
use std::{rc::Rc, time::Duration};

/// Builds the child each frame. Stored as an `Rc` so the same factory can both render this
/// frame's child and be kept as the snapshot for a future exit.
pub type ChildFactory = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// One half of a transition's progress as a style closure sees it: `(delta, time)`, both running
/// `0.0 -> 1.0`. See [`EnterStyle`] — the transition hands its styles the same two numbers an
/// [`animated_list`](hgpui::animated_list) hands its rows.
type Progress = (f32, f32);

/// How the outgoing and incoming child share the timeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransitionMode {
    /// The outgoing child leaves while the incoming one enters. This is the default, and
    /// what Vue does without a `mode`.
    #[default]
    Simultaneous,
    /// The outgoing child finishes leaving before the incoming one enters (Vue's `out-in`).
    /// It keeps the space while it leaves, so the new child only shows up once it is gone.
    OutIn,
    /// The incoming child finishes entering before the outgoing one starts leaving
    /// (Vue's `in-out`).
    InOut,
}

/// The default transition duration: 150ms.
pub const DEFAULT_DURATION: Duration = Duration::from_millis(150);

/// Creates a transition for the child identified by `key`.
///
/// Call [`TransitionBuilder::child`] with the factory last; the returned value is an element.
#[track_caller]
pub fn transition(id: impl Into<ElementId>, key: impl Into<ElementId>) -> TransitionBuilder {
    TransitionBuilder {
        id: id.into(),
        key: key.into(),
        duration: DEFAULT_DURATION,
        easing: None,
        mode: TransitionMode::default(),
        enter: None,
        leave: None,
        style: Rc::new(|this, _, _| this.relative().size_full()),
        appear: false,
        factory: None,
        container: None,
    }
}

/// A [`transition`] under construction. See the [module docs](self) for the semantics.
pub struct TransitionBuilder {
    id: ElementId,
    key: ElementId,
    duration: Duration,
    easing: Option<Rc<dyn Fn(f32) -> f32>>,
    mode: TransitionMode,
    enter: Option<EnterStyle>,
    leave: Option<LeaveStyle>,
    style: Rc<dyn Fn(Stateful<Div>, &mut Window, &mut App) -> Stateful<Div>>,
    appear: bool,
    factory: Option<ChildFactory>,
    /// Assembled during `request_layout` and painted again in `prepaint`/`paint`.
    container: Option<AnyElement>,
}

impl TransitionBuilder {
    /// How long each of the two animations takes. Defaults to 150ms.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Eases the progress passed to the style closures. Defaults to linear.
    pub fn easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Some(Rc::new(easing));
        self
    }

    /// How the two animations share the timeline. Defaults to [`TransitionMode::Simultaneous`].
    pub fn mode(mut self, mode: TransitionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Styles the incoming child while it enters: `|child, delta, time|`, both numbers going
    /// `0.0 -> 1.0` — see [`EnterStyle`] for what the two of them are.
    ///
    /// Without one, the child appears instantly (the progress is simply unused).
    pub fn enter(mut self, style: impl Fn(AnyElement, f32, f32) -> AnyElement + 'static) -> Self {
        self.enter = Some(Rc::new(style));
        self
    }

    /// Styles the outgoing child while it leaves: `|child, delta, time|`, both numbers going
    /// `0.0 -> 1.0` — see [`EnterStyle`].
    ///
    /// In [`TransitionMode::OutIn`] this wraps the child where it stands, in the flow; in the
    /// other modes it wraps it in an overlay painted underneath the incoming child.
    pub fn leave(mut self, style: impl Fn(AnyElement, f32, f32) -> AnyElement + 'static) -> Self {
        self.leave = Some(Rc::new(style));
        self
    }

    /// Also run the enter animation on the first render. Defaults to `false`.
    pub fn appear(mut self, appear: bool) -> Self {
        self.appear = appear;
        self
    }

    /// The style of the transition.
    pub fn style(mut self, style: impl Fn(Stateful<Div>, &mut Window, &mut App) -> Stateful<Div> + 'static) -> Self {
        self.style = Rc::new(style);
        self
    }

    /// The child, built fresh on each frame it is needed — including the frames where it is
    /// the outgoing child, where the factory captured before the switch is called instead.
    #[track_caller]
    pub fn child(
        mut self,
        factory: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.factory = Some(Rc::new(factory));
        self
    }
}

impl IntoElement for TransitionBuilder {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Which animation (if any) is in flight, and who is where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Nothing is animating.
    Idle,
    /// The incoming child is in the flow; an outgoing child may be in the overlay.
    Entering {
        /// Whether the leave animation runs alongside (or waits for the enter to finish).
        leave_armed: bool,
    },
    /// [`TransitionMode::OutIn`] with the outgoing child still in the flow, leaving, while
    /// the incoming child waits in `pending`.
    LeavingInFlow,
}

struct PendingChild {
    key: ElementId,
    factory: ChildFactory,
}

struct TransitionSlot {
    /// The key of the child the user last asked for. Compared each frame to detect a switch.
    key: Option<ElementId>,
    /// Factory of the child currently in the flow, as of the previous frame — so a switch
    /// leaves us holding the pre-switch snapshot.
    previous: Option<ChildFactory>,
    /// Factory of the child animating out in the overlay.
    leaving: Option<ChildFactory>,
    /// The child queued behind an [`TransitionMode::OutIn`] exit.
    pending: Option<PendingChild>,
    phase: Phase,
    enter_state: hgpui::Entity<TransitionState<f32>>,
    leave_state: hgpui::Entity<TransitionState<f32>>,
}

impl Element for TransitionBuilder {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut container = self.assemble(window, cx);
        let layout_id = container.request_layout(window, cx);
        // Fresh every frame, and used again by `prepaint`/`paint` in this same frame.
        self.container = Some(container);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: hgpui::Bounds<hgpui::Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(container) = self.container.as_mut() {
            container.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: hgpui::Bounds<hgpui::Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(container) = self.container.as_mut() {
            container.paint(window, cx);
        }
    }
}

impl TransitionBuilder {
    /// Runs the state machine for this frame and assembles the container element.
    fn assemble(&mut self, window: &mut Window, cx: &mut App) -> AnyElement {
        let factory = self
            .factory
            .clone()
            .expect("transition(..).child(..) must be given a child factory");

        let slot_entity = window.use_keyed_state(self.id.clone(), cx, |_window, cx| {
            TransitionSlot {
                key: None,
                previous: None,
                leaving: None,
                pending: None,
                phase: Phase::Idle,
                enter_state: cx.new(|_| TransitionState::new(0.0)),
                leave_state: cx.new(|_| TransitionState::new(0.0)),
            }
        });

        let enter_state = slot_entity.read(cx).enter_state.clone();
        let leave_state = slot_entity.read(cx).leave_state.clone();
        let enter = Transition::new(enter_state, self.duration);
        let leave = Transition::new(leave_state, self.duration);
        let (enter, leave) = match &self.easing {
            Some(easing) => (
                enter.with_raw_easing(easing.clone()),
                leave.with_raw_easing(easing.clone()),
            ),
            None => (enter, leave),
        };

        let reduce_motion = cx.reduce_motion();
        let key_changed = slot_entity.read(cx).key.as_ref() != Some(&self.key);

        if key_changed {
            let outgoing = slot_entity.read(cx).previous.clone();
            // A first render has nothing to animate out of; `appear` decides whether it
            // animates in.
            let animate_out = outgoing.is_some() && !reduce_motion;
            let animate_in = (outgoing.is_some() || self.appear) && !reduce_motion;

            slot_entity.update(cx, |slot, _| {
                slot.key = Some(self.key.clone());
                match self.mode {
                    TransitionMode::Simultaneous => {
                        slot.pending = None;
                        slot.leaving = if animate_out { outgoing } else { None };
                        slot.previous = Some(factory.clone());
                        slot.phase = if animate_in || animate_out {
                            Phase::Entering {
                                leave_armed: animate_out,
                            }
                        } else {
                            Phase::Idle
                        };
                    }
                    TransitionMode::OutIn => {
                        if animate_out {
                            // The outgoing child keeps the flow while it leaves; the new one
                            // waits in `pending`. (A newer key simply replaces it.)
                            slot.pending = Some(PendingChild {
                                key: self.key.clone(),
                                factory: factory.clone(),
                            });
                            slot.phase = Phase::LeavingInFlow;
                        } else {
                            slot.pending = None;
                            slot.leaving = None;
                            slot.previous = Some(factory.clone());
                            slot.phase = if animate_in {
                                Phase::Entering { leave_armed: false }
                            } else {
                                Phase::Idle
                            };
                        }
                    }
                    TransitionMode::InOut => {
                        slot.pending = None;
                        slot.leaving = if animate_out { outgoing } else { None };
                        slot.previous = Some(factory.clone());
                        slot.phase = if animate_in || animate_out {
                            Phase::Entering {
                                // The exit waits for the entrance to finish.
                                leave_armed: false,
                            }
                        } else {
                            Phase::Idle
                        };
                    }
                }
            });

            if animate_in {
                start(&enter, cx);
            }
            if animate_out && self.mode != TransitionMode::InOut {
                start(&leave, cx);
            }
        } else {
            // Keep the snapshot of the child in the flow current, without disturbing a child
            // that is currently on its way out.
            slot_entity.update(cx, |slot, _| match slot.phase {
                Phase::LeavingInFlow => {
                    if let Some(pending) = slot.pending.as_mut() {
                        pending.factory = factory.clone();
                    }
                }
                Phase::Idle | Phase::Entering { .. } => {
                    slot.previous = Some(factory.clone());
                }
            });
        }

        // A user preference for less motion also wins over an animation already in flight:
        // jumping both halves to the end lets the completion logic below settle this frame.
        if reduce_motion {
            enter.jump_to(1.0, cx);
            leave.jump_to(1.0, cx);
        }

        // Advance the animations and read this frame's progress.
        let mut phase = slot_entity.read(cx).phase;
        let (mut enter_t, mut leave_t) = progress(phase, &enter, &leave, window, cx);

        // Phase transitions caused by finished animations.
        match phase {
            Phase::Idle => {}
            Phase::Entering { .. } => {
                let enter_done = enter_t.is_none_or(|(delta, _)| delta >= 1.0);
                match leave_t {
                    // The exit is running alongside the entrance.
                    Some((leave_delta, _)) => {
                        let leave_done = leave_delta >= 1.0;
                        slot_entity.update(cx, |slot, _| {
                            if leave_done {
                                slot.leaving = None;
                            }
                            if enter_done && leave_done {
                                slot.phase = Phase::Idle;
                            }
                        });
                    }
                    // `InOut`: the exit starts once the entrance has finished.
                    None => {
                        let has_leaving = slot_entity.read(cx).leaving.is_some();
                        if enter_done {
                            if has_leaving {
                                start(&leave, cx);
                                slot_entity.update(cx, |slot, _| {
                                    slot.phase = Phase::Entering { leave_armed: true };
                                });
                            } else {
                                slot_entity.update(cx, |slot, _| slot.phase = Phase::Idle);
                            }
                        }
                    }
                }
            }
            Phase::LeavingInFlow => {
                let leave_done = leave_t.is_none_or(|(delta, _)| delta >= 1.0);
                if leave_done {
                    let pending = slot_entity.update(cx, |slot, _| slot.pending.take());
                    slot_entity.update(cx, |slot, _| {
                        if let Some(pending) = pending {
                            slot.key = Some(pending.key);
                            slot.previous = Some(pending.factory);
                        }
                        slot.phase = if self.enter.is_some() {
                            Phase::Entering { leave_armed: false }
                        } else {
                            Phase::Idle
                        };
                    });
                    start(&enter, cx);
                }
            }
        }

        // A phase transition above may have started an animation in *this* frame (the swap out
        // of `OutIn`, or `InOut`'s exit once the entrance ends). Re-read the progress so the
        // frame that starts an animation already renders its first step instead of flashing the
        // finished state for one frame.
        let phase_after = slot_entity.read(cx).phase;
        if phase_after != phase {
            phase = phase_after;
            (enter_t, leave_t) = progress(phase, &enter, &leave, window, cx);
        }

        // Assemble: the outgoing child goes in the overlay (painted first, underneath), the
        // child in the flow owns the space. The factories are taken out of the slot first so
        // that calling them (which needs `&mut App`) doesn't overlap with reading it.
        let (leaving, in_flow, phase) = {
            let slot = slot_entity.read(cx);
            let in_flow = match slot.phase {
                Phase::LeavingInFlow => slot.previous.clone(),
                Phase::Idle | Phase::Entering { .. } => Some(factory.clone()),
            };
            (slot.leaving.clone(), in_flow, slot.phase)
        };

        let mut container = (self.style)(div().id(self.id.clone()), window, cx);

        if let Some(leaving) = &leaving {
            let child = leaving(window, cx);
            let child = styled(self.leave.as_ref(), leave_t, child);
            container = container.child(div().absolute().inset_0().child(child));
        }

        let child = match &in_flow {
            Some(in_flow) => in_flow(window, cx),
            None => div().into_any_element(),
        };
        // Which style owns the child in the flow, and the progress to hand it. The incoming
        // child is the one in the flow — except in `OutIn`, where the outgoing child keeps the
        // flow while it leaves, and is styled there rather than in an overlay.
        let (style, progress) = match phase {
            Phase::LeavingInFlow => (self.leave.as_ref(), leave_t),
            Phase::Entering { .. } => (self.enter.as_ref(), enter_t),
            Phase::Idle => (None, None),
        };
        container
            .child(styled(style, progress, child))
            .into_any_element()
    }
}

/// Applies one half's style to `child`, unless that half is missing or already over.
fn styled(style: Option<&EnterStyle>, progress: Option<Progress>, child: AnyElement) -> AnyElement {
    match (style, progress) {
        // `(delta, time)`, both `0.0 -> 1.0`.
        (Some(style), Some((delta, time))) if delta < 1.0 => style(child, delta, time),
        _ => child,
    }
}

/// This frame's progress for each half of the transition, given the phase.
fn progress(
    phase: Phase,
    enter: &Transition<f32>,
    leave: &Transition<f32>,
    window: &mut Window,
    cx: &mut App,
) -> (Option<Progress>, Option<Progress>) {
    match phase {
        Phase::Idle => (None, None),
        Phase::Entering { leave_armed } => {
            let enter_t = evaluate(enter, window, cx);
            let leave_t = leave_armed.then(|| evaluate(leave, window, cx));
            (Some(enter_t), leave_t)
        }
        Phase::LeavingInFlow => (None, Some(evaluate(leave, window, cx))),
    }
}

/// One running transition, as its style closure will see it: the eased `delta` it interpolates,
/// and the plain `time` it took to get there.
fn evaluate(transition: &Transition<f32>, window: &mut Window, cx: &mut App) -> Progress {
    let delta = *transition.evaluate(window, cx);
    (delta, transition.evaluate_time(cx))
}

/// Restarts a transition from `0.0` and runs it to `1.0`.
///
/// `update` on its own would keep the old `start_goal`, so `reset` is what makes a second
/// exit animate from the beginning again.
fn start(transition: &Transition<f32>, cx: &mut App) {
    transition.reset(cx);
    transition.update(cx, |goal, _| *goal = 1.0);
}

#[cfg(test)]
mod tests;
