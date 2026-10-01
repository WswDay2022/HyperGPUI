//! An animated list for rows that the layout engine positions itself.
//!
//! This is the sibling of [`animated_uniform_list`](super::animated_uniform_list), and the two
//! are deliberately the same to use:
//!
//! | | [`animated_uniform_list`](super::animated_uniform_list) | this one |
//! |---|---|---|
//! | rows | virtualized: only what is on screen | every row, every frame |
//! | heights | uniform, and only known after a layout pass | anything, and they may change |
//! | positions come from | `index * row_height` | the layout engine |
//! | good for | thousands of rows | tens or hundreds of rows |
//! | scrolling | through a [`UniformListScrollHandle`](crate::UniformListScrollHandle) | its own style, e.g. `.overflow_y_scroll()` |
//!
//! Both take `count`, `key_of` and `rows`, both animate the same three things, and both have the
//! same knobs for them:
//!
//! - a row that *moves* — because a row above it appeared, disappeared or changed height —
//!   glides to its new place, paint only, with the layout and hit-testing already up to date;
//! - a new row animates in with [`.enter(..)`](AnimatedListBuilder::enter);
//! - a removed row animates out through a [`LeavingHandle`], painted where it was last seen, with
//!   [`.leave(..)`](AnimatedListBuilder::leave);
//! - resizing the element (or enabling [`App::reduce_motion`]) snaps everything instead of
//!   animating it.
//!
//! Because positions are measured rather than computed, this list needs no view anchoring: when
//! rows above the viewport change, the layout (and the scroll offset) already keep the content
//! where it belongs.
//!
//! A row that changes *height* does not stretch itself (its content is painted at its laid-out
//! size); it is its neighbours that glide out of the way.
//!
//! ```
//! # use std::rc::Rc;
//! # use hgpui::{App, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};
//! # use hgpui::animated_list;
//! # fn render(_window: &mut Window, _cx: &mut App, items: Rc<Vec<u64>>) -> impl IntoElement {
//! animated_list("rows")
//!     .count(items.len())
//!     .key_of({
//!         let items = items.clone();
//!         move |index| items[index]
//!     })
//!     .rows(move |range, _window, _cx| {
//!         range
//!             .map(|index| div().h(px(20.)).child(format!("row {}", items[index])))
//!             .collect()
//!     })
//! # }
//! ```

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Range,
    rc::Rc,
    time::Duration,
};

use collections::FxHasher;
use refineable::Refineable as _;
use scheduler::Instant;

use crate::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    InteractiveElement as _, LayoutId, Overflow, ParentElement as _, Pixels, Point,
    StyleRefinement, Styled, Window, div, px,
};

pub use super::animated_uniform_list::ease_out_cubic;
use super::animated_uniform_list::{EnterStyle, LeaveStyle, LeavingHandle};

/// How long a row takes to glide into its new place. Defaults to 150ms.
pub const DEFAULT_ROW_DURATION: Duration = Duration::from_millis(150);

/// Creates an animated list that lays out every row.
///
/// See the [module docs](self) for how it differs from
/// [`animated_uniform_list`](super::animated_uniform_list).
#[track_caller]
pub fn animated_list<K, R>(id: impl Into<ElementId>) -> AnimatedListBuilder<K, R> {
    AnimatedListBuilder {
        id: id.into(),
        count: 0,
        key_of: None,
        rows: None,
        duration: DEFAULT_ROW_DURATION,
        easing: Rc::new(ease_out_cubic),
        enter: None,
        leave: None,
        leaving: None,
        style: StyleRefinement::default(),
    }
}

/// An [`animated_list`] under construction. Same shape as
/// [`AnimatedUniformListBuilder`](super::AnimatedUniformListBuilder).
pub struct AnimatedListBuilder<K, R> {
    id: ElementId,
    count: usize,
    key_of: Option<Rc<dyn Fn(usize) -> K>>,
    rows: Option<Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>>>,
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    enter: Option<EnterStyle>,
    leave: Option<LeaveStyle>,
    leaving: Option<LeavingHandle<K>>,
    style: StyleRefinement,
}

impl<K, R> Styled for AnimatedListBuilder<K, R> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K, R> AnimatedListBuilder<K, R>
where
    K: Clone + Eq + Hash + 'static,
    R: IntoElement + 'static,
{
    /// How many rows the list has. Required.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    /// Maps a row index to a key that is **stable across frames** (an id from your own data).
    ///
    /// A row's animation is attached to its key, so a row that keeps its key keeps its state
    /// when it moves. Required.
    pub fn key_of(mut self, key_of: impl Fn(usize) -> K + 'static) -> Self {
        self.key_of = Some(Rc::new(key_of));
        self
    }

    /// Builds the row for an index. Every row is built, every frame. Required, and it terminates
    /// the builder.
    pub fn rows(
        mut self,
        rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<R> + 'static,
    ) -> AnimatedList<K, R> {
        self.rows = Some(Rc::new(rows));
        AnimatedList {
            id: self.id,
            count: self.count,
            key_of: self.key_of.expect("animated_list(..).key_of(..) is required"),
            rows: self.rows.clone().expect("animated_list(..).rows(..) is required"),
            duration: self.duration,
            easing: self.easing,
            enter: self.enter,
            leave: self.leave,
            leaving: self.leaving,
            style: self.style,
            container: None,
        }
    }

    /// How long a row takes to glide into place. Defaults to 150ms. The whole list shares one
    /// duration and one easing.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Eases the glide. Defaults to [`ease_out_cubic`].
    pub fn easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Styles a row as it enters, with `t` going `0.0 -> 1.0`. Only new rows animate in.
    pub fn enter(mut self, enter: impl Fn(AnyElement, f32) -> AnyElement + 'static) -> Self {
        self.enter = Some(Rc::new(enter));
        self
    }

    /// Styles a row as it leaves, with `t` going `0.0 -> 1.0`, painted where it was last seen.
    pub fn leave(mut self, leave: impl Fn(AnyElement, f32) -> AnyElement + 'static) -> Self {
        self.leave = Some(Rc::new(leave));
        self
    }

    /// Animates removed rows out, using the rows queued on the handle. See [`LeavingHandle`].
    pub fn leaving(mut self, handle: &LeavingHandle<K>) -> Self {
        self.leaving = Some(handle.clone());
        self
    }

    /// Makes the list scroll vertically.
    ///
    /// The same as the `overflow_y_scroll` helper on interactive elements: this list carries its
    /// own layout, so it does not need the interactivity state that helper implies.
    pub fn overflow_y_scroll(mut self) -> Self {
        self.style.overflow.y = Some(Overflow::Scroll);
        self
    }

    /// Makes the list scroll in both directions.
    pub fn overflow_scroll(mut self) -> Self {
        self.style.overflow.x = Some(Overflow::Scroll);
        self.style.overflow.y = Some(Overflow::Scroll);
        self
    }
}

/// The list element. See [`animated_list`].
pub struct AnimatedList<K, R> {
    id: ElementId,
    count: usize,
    key_of: Rc<dyn Fn(usize) -> K>,
    rows: Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>>,
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    enter: Option<EnterStyle>,
    leave: Option<LeaveStyle>,
    leaving: Option<LeavingHandle<K>>,
    style: StyleRefinement,
    /// Built during `request_layout`, painted again in `prepaint`/`paint`.
    container: Option<AnyElement>,
}

impl<K, R> Styled for AnimatedList<K, R> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K, R> Element for AnimatedList<K, R>
where
    K: Clone + Eq + Hash + 'static,
    R: IntoElement + 'static,
{
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| ListState::<K>::default());
        let now = Instant::now();
        let duration = self.duration;
        let easing = self.easing.clone();

        // Rows that were removed since the last frame, painted where they were last seen. This
        // reads the previous frame's positions, before this frame's rows overwrite them.
        let leaving = state.update(cx, |state, _| {
            if let (Some(handle), true) = (&self.leaving, self.leave.is_some()) {
                for (key, row) in handle.take_queued() {
                    let painted = state
                        .painted
                        .borrow()
                        .get(&key)
                        .copied()
                        .unwrap_or_default();
                    state.leaving.push(ActiveLeave {
                        row,
                        painted,
                        started: now,
                    });
                }
            } else if let Some(handle) = &self.leaving {
                // Nothing to do with them, but the queue must not grow.
                handle.take_queued();
            }

            state
                .leaving
                .retain(|leave| progress(leave.started, now, duration) < 1.0);
            state
                .leaving
                .iter()
                .map(|leave| {
                    (
                        leave.row.clone(),
                        leave.painted,
                        easing(progress(leave.started, now, duration).min(1.0)),
                    )
                })
                .collect::<Vec<_>>()
        });

        // This frame's rows write their positions there during prepaint.
        state.update(cx, |state, _| state.painted.borrow_mut().clear());

        let shared = SharedAnimation {
            painted: state.read(cx).painted.clone(),
            snap: state.read(cx).snap.clone(),
        };

        // A real `div` carries the layout: that is where the style lands, and what scrolls.
        let mut container = div().id(self.id.clone());
        container.style().refine(&self.style);

        let (key_of, rows) = (self.key_of.clone(), self.rows.clone());
        let rendered = (rows)(0..self.count, window, cx);

        for (index, child) in rendered.into_iter().enumerate() {
            let key = (key_of)(index);
            let item = AnimatedItem {
                id: row_id(&self.id, &key),
                key,
                child: child.into_any_element(),
                duration: self.duration,
                easing: self.easing.clone(),
                enter: self.enter.clone(),
                shared: shared.clone(),
            };
            container = container.child(item);
        }

        // The exit animations go last, so they paint above the rows, and absolutely positioned,
        // so they take no part in the layout.
        if let Some(leave_style) = self.leave.clone() {
            for (row, painted, t) in leaving {
                let element = (row)(window, cx);
                let element = leave_style(element, t);
                container = container.child(
                    div()
                        .absolute()
                        .top(painted.y)
                        .left(px(0.))
                        .right(px(0.))
                        .child(element),
                );
            }
        }

        let mut container = container.into_any_element();
        let layout_id = container.request_layout(window, cx);
        self.container = Some(container);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| ListState::<K>::default());
        // A resize (or `reduce_motion`) is not a change of the rows: land everything instead.
        let snap = cx.reduce_motion() || {
            let previous = state.update(cx, |state, _| state.bounds.replace(bounds));
            previous.is_none_or(|previous| previous != bounds)
        };
        state.update(cx, |state, _| state.snap.set(snap));

        if let Some(container) = self.container.as_mut() {
            container.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
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

impl<K, R> IntoElement for AnimatedList<K, R>
where
    K: Clone + Eq + Hash + 'static,
    R: IntoElement + 'static,
{
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The id of one row's element: the list's id plus a stable hash of the row's key.
///
/// The key itself cannot become an [`ElementId`] — it is whatever the caller's data uses — so it
/// is hashed. A collision would be two rows sharing one row's animation state.
fn row_id<K: Hash>(list_id: &ElementId, key: &K) -> ElementId {
    let mut hasher = FxHasher::default();
    key.hash(&mut hasher);
    ElementId::NamedInteger(format!("{list_id}").into(), hasher.finish())
}

/// State shared between the list and its rows for this frame.
#[derive(Clone)]
struct SharedAnimation<K> {
    /// Where each row was painted as of the previous frame.
    painted: Rc<RefCell<HashMap<K, Point<Pixels>>>>,
    /// Set when the element was resized (or this is the first frame): everything lands at once.
    snap: Rc<Cell<bool>>,
}

/// A row: it paints where it was last painted and glides to where the layout puts it.
struct AnimatedItem<K> {
    id: ElementId,
    key: K,
    child: AnyElement,
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    enter: Option<EnterStyle>,
    shared: SharedAnimation<K>,
}

impl<K> Element for AnimatedItem<K>
where
    K: Clone + Eq + Hash + 'static,
{
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let now = Instant::now();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| RowState::default());
        let snap = self.shared.snap.get();
        let duration = self.duration;
        let easing = self.easing.clone();

        // The entrance is styled while it runs, which means the row is wrapped here, before it
        // is laid out. A row that appears on a frame that snaps (the first frame, a resize) does
        // not animate in, exactly like the virtualized list.
        let enter_t =
            state.update(cx, |state, _| state.enter_progress(snap, now, duration, easing.as_ref()));
        if let (Some(style), Some(t)) = (self.enter.clone(), enter_t) {
            let child = std::mem::replace(&mut self.child, div().into_any_element());
            self.child = style(child, t);
            window.request_animation_frame();
        }

        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let now = Instant::now();
        let snap = self.shared.snap.get();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| RowState::default());

        let offset = state.update(cx, |state, _| {
            state
                .advance(bounds.origin, snap, now, self.duration, self.easing.as_ref())
        });

        // Where this row is painted, so an exit animation can take over from exactly there.
        self.shared
            .painted
            .borrow_mut()
            .insert(self.key.clone(), bounds.origin + offset);

        if state.read(cx).gliding(now, self.duration) {
            window.request_animation_frame();
        }

        // Painting only: the layout, and so hit-testing, stays where the list put the row.
        window.with_element_offset(offset, |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

impl<K> IntoElement for AnimatedItem<K>
where
    K: Clone + Eq + Hash + 'static,
{
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Where one row is painted, what it is gliding from, and whether it has entered yet.
#[derive(Default)]
struct RowState {
    /// The origin the layout engine gave the row last frame.
    target: Option<Point<Pixels>>,
    /// Where the row was painted when the current glide started.
    from: Option<Point<Pixels>>,
    /// When the current glide started.
    started: Option<Instant>,
    /// When the row first appeared.
    appeared: Option<Instant>,
    /// Once the entrance is over (or skipped), it never runs again.
    entered: bool,
}

impl RowState {
    /// The enter animation's progress, if the row is currently entering.
    fn enter_progress(
        &mut self,
        snap: bool,
        now: Instant,
        duration: Duration,
        easing: &dyn Fn(f32) -> f32,
    ) -> Option<f32> {
        if snap || self.entered {
            self.entered = true;
            return None;
        }

        let appeared = *self.appeared.get_or_insert(now);
        let t = progress(appeared, now, duration);
        if t >= 1.0 {
            self.entered = true;
            return None;
        }
        Some(easing(t))
    }

    /// Moves the row towards the origin the layout gave it, and returns the paint offset to use.
    fn advance(
        &mut self,
        target: Point<Pixels>,
        snap: bool,
        now: Instant,
        duration: Duration,
        easing: &dyn Fn(f32) -> f32,
    ) -> Point<Pixels> {
        if snap {
            self.target = Some(target);
            self.from = None;
            self.started = None;
            return Point::default();
        }

        if self.target.is_none_or(|previous| previous != target) {
            // Carry on from where the row is painted right now, so a second move mid-glide does
            // not jump.
            let painted = self.painted(now, duration, easing);
            self.from = Some(painted);
            self.started = Some(now);
            self.target = Some(target);
        }

        self.painted(now, duration, easing) - target
    }

    /// Where the row is painted right now.
    fn painted(
        &self,
        now: Instant,
        duration: Duration,
        easing: &dyn Fn(f32) -> f32,
    ) -> Point<Pixels> {
        let Some(target) = self.target else {
            return Point::default();
        };
        let (Some(from), Some(started)) = (self.from, self.started) else {
            return target;
        };

        let t = easing(progress(started, now, duration));
        from + (target - from).map(|delta| delta * t)
    }

    fn gliding(&self, now: Instant, duration: Duration) -> bool {
        self.started
            .is_some_and(|started| progress(started, now, duration) < 1.0)
    }
}

/// A removed row on its way out: its row factory, and where it was last painted.
struct ActiveLeave {
    row: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
    painted: Point<Pixels>,
    started: Instant,
}

/// Per-list state, keyed by the list's element id.
struct ListState<K> {
    /// Rows animating out.
    leaving: Vec<ActiveLeave>,
    /// Where each row was painted on the previous frame.
    painted: Rc<RefCell<HashMap<K, Point<Pixels>>>>,
    /// Whether this frame should land everything immediately.
    snap: Rc<Cell<bool>>,
    /// The element's own bounds last frame, to notice a resize.
    bounds: Option<Bounds<Pixels>>,
}

impl<K> Default for ListState<K> {
    fn default() -> Self {
        Self {
            leaving: Vec::new(),
            painted: Rc::new(RefCell::new(HashMap::new())),
            // The first frame has nothing to animate from.
            snap: Rc::new(Cell::new(true)),
            bounds: None,
        }
    }
}

/// Elapsed fraction of an animation, before easing.
fn progress(started: Instant, now: Instant, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32();
    if duration <= 0.0 {
        return 1.0;
    }
    ((now - started).as_secs_f32() / duration).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point;
    use std::time::Duration;

    const FRAME: Duration = Duration::from_millis(16);
    const DURATION: Duration = Duration::from_millis(100);

    fn step(
        state: &mut RowState,
        target: Point<Pixels>,
        snap: bool,
        now: Instant,
    ) -> Point<Pixels> {
        state.advance(target, snap, now, DURATION, &ease_out_cubic)
    }

    /// Where the row is painted, in the list's coordinates.
    fn painted(state: &RowState, now: Instant) -> Point<Pixels> {
        state.painted(now, DURATION, &ease_out_cubic)
    }

    #[test]
    fn a_row_that_moves_glides_from_where_it_was() {
        let start = Instant::now();
        let mut state = RowState::default();

        // The first frame lands the row where the layout put it.
        assert_eq!(step(&mut state, Point::default(), true, start), Point::default());

        // A row above it disappeared, so the layout moved this one 100px down.
        let now = start + FRAME;
        let target = point(Pixels::ZERO, px(100.));
        let offset = step(&mut state, target, false, now);

        assert_eq!(
            offset,
            point(Pixels::ZERO, px(-100.)),
            "the glide starts where the row was painted: one row above its new place"
        );
        assert!(state.gliding(now, DURATION));

        // The frame after, it is on its way.
        let later = now + FRAME;
        let offset = step(&mut state, target, false, later);
        assert!(
            offset.y < Pixels::ZERO && offset.y > px(-100.),
            "part way there: {offset:?}"
        );

        // ... and it settles exactly there.
        let settled = start + DURATION + FRAME;
        assert_eq!(step(&mut state, target, false, settled), Point::default());
        assert_eq!(painted(&state, settled), target);
        assert!(!state.gliding(settled, DURATION));
    }

    #[test]
    fn resizing_lands_rows_instead_of_gliding() {
        let start = Instant::now();
        let mut state = RowState::default();
        step(&mut state, Point::default(), true, start);

        let target = point(Pixels::ZERO, px(100.));
        let offset = step(&mut state, target, true, start + FRAME);

        assert_eq!(offset, Point::default(), "a resize is not a row moving");
        assert!(!state.gliding(start + FRAME, DURATION));
    }

    #[test]
    fn a_second_move_mid_glide_does_not_jump() {
        let start = Instant::now();
        let mut state = RowState::default();
        step(&mut state, Point::default(), true, start);

        let first = point(Pixels::ZERO, px(100.));
        step(&mut state, first, false, start + FRAME);
        step(&mut state, first, false, start + FRAME * 2);

        // Another row appears: this one is pushed another 100px down mid-glide. Compare at the
        // *same instant*, before and after the re-target: a frame's worth of movement is
        // expected, a jump is not.
        let second = point(Pixels::ZERO, px(200.));
        let now = start + FRAME * 3;
        let before = painted(&state, now);
        step(&mut state, second, false, now);
        let after = painted(&state, now);

        assert!(
            (after - before).y.abs() < px(0.5),
            "the row must carry on from where it was painted ({before:?} -> {after:?})"
        );

        // And it still lands exactly where the layout puts it.
        let settled = start + FRAME * 3 + DURATION + FRAME;
        step(&mut state, second, false, settled);
        assert_eq!(painted(&state, settled), second);
    }

    #[test]
    fn rows_enter_once() {
        let start = Instant::now();
        let mut state = RowState::default();
        let progress = |state: &mut RowState, now| {
            state.enter_progress(false, now, DURATION, &ease_out_cubic)
        };

        assert_eq!(progress(&mut state, start), Some(0.), "the entrance starts at 0");
        let mid = progress(&mut state, start + FRAME * 3).expect("still entering");
        assert!(mid > 0. && mid < 1., "in flight: {mid}");
        assert_eq!(progress(&mut state, start + DURATION), None, "and it ends");
        assert_eq!(
            progress(&mut state, start + DURATION * 2),
            None,
            "a row does not enter twice"
        );
    }

    #[test]
    fn a_snapping_frame_suppresses_entrances() {
        let start = Instant::now();
        let mut state = RowState::default();

        assert_eq!(
            state.enter_progress(true, start, DURATION, &ease_out_cubic),
            None,
            "the first frame (or a resize) does not animate rows in"
        );
        assert_eq!(
            state.enter_progress(false, start + FRAME, DURATION, &ease_out_cubic),
            None,
            "and the rows that were already there do not start entering afterwards"
        );
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    use crate::{
        AppContext as _, Context, IntoElement, Render, TestAppContext, Window, px, size,
    };
    use std::cell::Cell;

    struct FlowView {
        rows: Rc<RefCell<Vec<u64>>>,
        leaving: LeavingHandle<u64>,
        enters: Rc<Cell<usize>>,
        leaves: Rc<Cell<usize>>,
    }

    impl Render for FlowView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let rows = Rc::new(self.rows.borrow().clone());
            let count = rows.len();
            let (enters, leaves) = (self.enters.clone(), self.leaves.clone());

            div().size_full().child(
                animated_list("rows")
                    .count(count)
                    .enter(move |element, _t| {
                        enters.set(enters.get() + 1);
                        element
                    })
                    .leave(move |element, _t| {
                        leaves.set(leaves.get() + 1);
                        element
                    })
                    .leaving(&self.leaving)
                    .key_of({
                        let rows = rows.clone();
                        move |index| rows[index]
                    })
                    .rows(move |range, _window, _cx| {
                        range
                            .map(|index| div().h(px(20.)).child(format!("row {}", rows[index])))
                            .collect()
                    }),
            )
        }
    }

    fn draw(cx: &mut TestAppContext, window: &crate::WindowHandle<FlowView>) {
        cx.update_window(**window, |_, window, cx| {
            let token = window.draw(cx);
            token.clear(cx);
        })
        .expect("window update failed");
    }

    /// A frame or two of a real frame loop, so animations progress.
    fn run_frames(cx: &mut TestAppContext, window: &crate::WindowHandle<FlowView>) {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(10));
            draw(cx, window);
        }
    }

    #[hgpui::test]
    fn rows_enter_and_leave_through_their_styles(cx: &mut TestAppContext) {
        let rows = Rc::new(RefCell::new((0..5u64).collect::<Vec<_>>()));
        let leaving = LeavingHandle::new();
        let enters = Rc::new(Cell::new(0));
        let leaves = Rc::new(Cell::new(0));
        let (rows_for_view, leaving_for_view) = (rows.clone(), leaving.clone());
        let (enters_for_view, leaves_for_view) = (enters.clone(), leaves.clone());

        let window = cx.open_window(size(px(300.), px(400.)), move |_, _cx| FlowView {
            rows: rows_for_view,
            leaving: leaving_for_view,
            enters: enters_for_view,
            leaves: leaves_for_view,
        });
        cx.run_until_parked();
        run_frames(cx, &window);

        assert_eq!(
            enters.get(),
            0,
            "the rows that were there from the start did not animate in"
        );

        // A new row shows up.
        rows.borrow_mut().insert(0, 99);
        window
            .update(cx, |_, _, cx| cx.notify())
            .expect("update failed");
        run_frames(cx, &window);
        assert!(enters.get() > 0, "the new row should animate in");

        // ... and one goes away.
        let removed = rows.borrow_mut().remove(3);
        let leaves_seen = leaves.get();
        leaving.push(removed, |_window, _cx| {
            div().h(px(20.)).child("leaving").into_any_element()
        });
        window
            .update(cx, |_, _, cx| cx.notify())
            .expect("update failed");
        run_frames(cx, &window);
        assert!(
            leaves.get() > leaves_seen,
            "the removed row should animate out"
        );

        // It lets go once the exit animation is over, so nothing is left hanging.
        std::thread::sleep(Duration::from_millis(300));
        let leaves_seen = leaves.get();
        run_frames(cx, &window);
        assert_eq!(
            leaves.get(),
            leaves_seen,
            "the removed row stopped being painted"
        );
    }
}
