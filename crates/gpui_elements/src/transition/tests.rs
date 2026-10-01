//! Behaviour tests for [`transition`](super::transition).
//!
//! The animations are driven by the real clock (`hgpui::Transition` uses wall time), so these
//! tests use a short duration and really sleep between frames.
//!
//! Everything is observed through the closures the caller supplies: each child factory logs
//! which key it built, and each style closure logs the progress it was handed. That is enough
//! to pin down who is on screen, in which layer, and when each half of the animation runs.

use std::{cell::RefCell, rc::Rc, time::Duration};

use hgpui::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, WindowHandle, div, px, size,
};

use super::{TransitionMode, transition};

const DURATION: Duration = Duration::from_millis(60);

/// What the closures saw.
///
/// Children are grouped per frame: `render` opens a new group, and the child factories (which
/// run later in the same frame, during the transition element's layout) fill it in. The order
/// inside a group is the paint order: the outgoing child in the overlay first, then the child
/// in the flow.
#[derive(Default)]
struct Log {
    children: RefCell<Vec<usize>>,
    frames: RefCell<Vec<Vec<usize>>>,
    /// Progress handed to the enter style.
    enters: RefCell<Vec<f32>>,
    /// Progress handed to the leave style.
    leaves: RefCell<Vec<f32>>,
}

impl Log {
    fn begin_frame(&self) {
        let finished = self.children.replace(Vec::new());
        if !finished.is_empty() {
            self.frames.borrow_mut().push(finished);
        }
    }

    /// The children built per frame, in order.
    fn frames(&self) -> Vec<Vec<usize>> {
        self.frames.borrow().clone()
    }

    /// The most recent frame, including the one still being built (a group is only pushed to
    /// `frames` when the *next* frame starts).
    fn last_frame(&self) -> Vec<usize> {
        let current = self.children.borrow().clone();
        if !current.is_empty() {
            current
        } else {
            self.frames.borrow().last().cloned().unwrap_or_default()
        }
    }

    /// Every child ever built, flattened.
    fn all_children(&self) -> Vec<usize> {
        self.frames.borrow().iter().flatten().copied().collect()
    }

    fn animating(&self) -> bool {
        !self.enters.borrow().is_empty() || !self.leaves.borrow().is_empty()
    }
}

struct Harness {
    key: usize,
    mode: TransitionMode,
    appear: bool,
    log: Rc<Log>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        log.begin_frame();
        let (enter_log, leave_log) = (log.clone(), log.clone());
        let key = self.key;

        transition("transition-id", self.key)
            .duration(DURATION)
            .mode(self.mode)
            .appear(self.appear)
            .enter(move |child, t| {
                enter_log.enters.borrow_mut().push(t);
                div().opacity(t).child(child).into_any_element()
            })
            .leave(move |child, t| {
                leave_log.leaves.borrow_mut().push(t);
                div().opacity(1.0 - t).child(child).into_any_element()
            })
            .child(move |_window, _cx| {
                log.children.borrow_mut().push(key);
                div().child(format!("child {key}")).into_any_element()
            })
    }
}

fn setup(
    cx: &mut TestAppContext,
    mode: TransitionMode,
    appear: bool,
) -> (WindowHandle<Harness>, Rc<Log>) {
    let log = Rc::new(Log::default());
    let log_for_view = log.clone();
    let window = cx.open_window(size(px(400.), px(300.)), move |_, _cx| Harness {
        key: 0,
        mode,
        appear,
        log: log_for_view,
    });
    cx.run_until_parked();
    (window, log)
}

/// Draws one frame.
fn frame(cx: &mut TestAppContext, window: &WindowHandle<Harness>) {
    cx.update_window(**window, |_, window, cx| {
        let token = window.draw(cx);
        token.clear(cx);
    })
    .expect("window update failed");
}

/// Switches the key and draws the frame that observes it.
fn switch_to(cx: &mut TestAppContext, window: &WindowHandle<Harness>, key: usize) {
    window
        .update(cx, |view, _window, cx| {
            view.key = key;
            cx.notify();
        })
        .expect("window update failed");
    frame(cx, window);
}

/// Lets the animations finish (they run on the wall clock).
fn settle() {
    std::thread::sleep(DURATION * 3);
}

#[hgpui::test]
fn a_stable_key_animates_nothing(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);

    frame(cx, &window);
    settle();
    frame(cx, &window);

    assert_eq!(
        log.frames(),
        vec![vec![0], vec![0]],
        "the child is built once per frame"
    );
    assert!(
        !log.animating(),
        "re-rendering with the same key must not animate"
    );
}

#[hgpui::test]
fn simultaneous_renders_both_children_while_swapping(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);
    assert_eq!(
        log.last_frame(),
        vec![0, 1],
        "the outgoing child goes in the overlay (painted first), the incoming one owns the flow"
    );

    settle();
    frame(cx, &window);
    assert_eq!(
        log.last_frame(),
        vec![1],
        "once the exit finishes, the outgoing child is dropped"
    );
    assert!(
        !log.enters.borrow().is_empty() && !log.leaves.borrow().is_empty(),
        "both halves animated"
    );
}

#[hgpui::test]
fn out_in_finishes_leaving_before_entering(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::OutIn, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);
    assert_eq!(
        log.last_frame(),
        vec![0],
        "the outgoing child keeps the space while it leaves, and the new one waits"
    );
    assert!(
        log.enters.borrow().is_empty(),
        "the entrance has not started yet"
    );

    settle();
    frame(cx, &window);
    assert_eq!(
        log.last_frame(),
        vec![1],
        "once the exit finishes the new child takes over"
    );
    assert!(
        !log.enters.borrow().is_empty(),
        "and only then does the entrance run"
    );

    let before = log.all_children().iter().filter(|key| **key == 0).count();
    settle();
    frame(cx, &window);
    assert_eq!(
        log.all_children().iter().filter(|key| **key == 0).count(),
        before,
        "once it has left, the outgoing child is never built again"
    );
}

#[hgpui::test]
fn in_out_finishes_entering_before_leaving(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::InOut, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);
    assert_eq!(
        log.last_frame(),
        vec![0, 1],
        "the incoming child enters while the outgoing one waits in the overlay"
    );
    assert!(
        log.leaves.borrow().is_empty(),
        "the exit waits for the entrance to finish"
    );

    settle();
    frame(cx, &window);
    assert!(
        !log.leaves.borrow().is_empty(),
        "the exit runs once the entrance is done"
    );

    settle();
    frame(cx, &window);
    assert_eq!(
        log.last_frame(),
        vec![1],
        "and the outgoing child goes away when it is over"
    );
}

#[hgpui::test]
fn the_newest_key_wins_mid_flight(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);
    // Switch again while the first swap is still running.
    switch_to(cx, &window, 2);

    assert_eq!(
        log.last_frame(),
        vec![1, 2],
        "the child that was coming in becomes the one going out"
    );

    let before = log.all_children().iter().filter(|key| **key == 0).count();
    settle();
    frame(cx, &window);
    assert_eq!(
        log.all_children().iter().filter(|key| **key == 0).count(),
        before,
        "the first child is never built again"
    );
}

#[hgpui::test]
fn reduce_motion_swaps_without_animating(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);

    assert_eq!(log.last_frame(), vec![1], "only the new child is built");
    assert!(
        !log.animating(),
        "no styles are applied when motion is reduced"
    );
}

#[hgpui::test]
fn appear_animates_the_first_mount(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, true);
    frame(cx, &window);

    assert!(
        !log.enters.borrow().is_empty(),
        "appear(true) runs the entrance on the first render"
    );
    assert!(
        log.leaves.borrow().is_empty(),
        "there is nothing to leave on the first render"
    );
}

#[hgpui::test]
fn without_appear_the_first_mount_is_instant(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);
    frame(cx, &window);

    assert!(
        log.enters.borrow().is_empty(),
        "the first render is not animated unless appear(true)"
    );
}

#[hgpui::test]
fn turning_on_reduce_motion_mid_flight_settles_the_transition(cx: &mut TestAppContext) {
    let (window, log) = setup(cx, TransitionMode::Simultaneous, false);
    frame(cx, &window);

    switch_to(cx, &window, 1);
    cx.update(|cx| cx.set_reduce_motion(true));
    frame(cx, &window);

    assert_eq!(
        log.last_frame(),
        vec![1],
        "an animation already in flight is dropped when motion is reduced"
    );
}
