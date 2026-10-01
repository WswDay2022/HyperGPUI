//! An animated, virtualized list.
//!
//! [`animated_list`] wraps [`uniform_list`](crate::uniform_list) and makes the visible items
//! *slide* when the list changes — the classic "everything makes way" effect when an item is
//! inserted or removed.
//!
//! # How it works
//!
//! In a virtualized list rows never move: an item's position is `index * row_height - scroll
//! offset`, computed rather than laid out. So "an item makes way" is really "**an item's index
//! changed**". Each frame the list records the index of every key near the viewport and, when
//! an index changes, paints that item at its *previous* index and eases the offset back to
//! zero (the FLIP technique). Only painting moves; layout and hit-testing stay where the list
//! says the item is.
//!
//! Because the animation is driven by index changes:
//!
//! - **Scrolling** never animates (indices don't change, the offset does).
//! - **Resizing the window** never animates for the same reason.
//! - Only the items near the viewport are tracked, so this costs a bounding map of a few
//!   dozen entries no matter how long the list is.
//!
//! # Anchoring
//!
//! When items are inserted or removed *above* the viewport while the list is scrolled down,
//! keeping the scroll offset would make the whole screen appear to slide by itself. Instead the
//! list notices that the item at the top of the viewport changed without the offset changing
//! and **adjusts the scroll offset** so that item stays put — the view is anchored on content,
//! exactly like an editor or a terminal. At the very top of the list (offset zero) nothing is
//! anchored, so a prepended item is visible immediately.
//!
//! A new item can also animate itself in with [`AnimatedListBuilder::enter`]; only genuinely
//! new items do, never ones that merely scrolled into view.
//!
//! An exit animation is painted by this element, above the list. If the element has rounded
//! corners, add `overflow_hidden`: rounded corners only clip with `overflow: hidden`, exactly
//! like CSS, and that mask covers the exit animations too.
//!
//! The element is [`Styled`], so sizing it works like any other element
//! (`.h(px(400.))`, `.flex_1()`, …) — the style is applied to the list itself.
//!
//! # Example
//!
//! ```
//! # use std::rc::Rc;
//! # use hgpui::{App, IntoElement, ParentElement as _, Styled as _, UniformListScrollHandle, Window, div, uniform_list};
//! # use hgpui::animated_list;
//! # fn render(window: &mut Window, cx: &mut App, items: Rc<Vec<u64>>, scroll: UniformListScrollHandle) -> impl IntoElement {
//! animated_list("items", &scroll)
//!     .count(items.len())
//!     .key_of({
//!         let items = items.clone();
//!         move |index| items[index]
//!     })
//!     .rows(move |range, window, cx| {
//!         range
//!             .map(|index| div().h_8().child(format!("item {}", items[index])))
//!             .collect()
//!     })
//! # }
//! ```

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::Hash,
    ops::Range,
    rc::Rc,
    time::Duration,
};

use refineable::Refineable as _;
use scheduler::Instant;

use crate::{
    AnyElement, App, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, ParentElement as _, Pixels, StyleRefinement, Styled, UniformListScrollHandle,
    Window, div, point, px, uniform_list,
};

/// Wraps a row for the enter animation: `t` goes `0.0 -> 1.0`.
pub type EnterStyle = Rc<dyn Fn(AnyElement, f32) -> AnyElement>;

/// Wraps a row for the leave animation: `t` goes `0.0 -> 1.0`.
pub type LeaveStyle = EnterStyle;

/// Builds one row for an item that has been removed and is animating out.
type LeavingRow = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A queue of removed items, so the list can animate them out on their way.
///
/// The list cannot render an item that is no longer in your data, and a rendered element
/// cannot be kept across frames, so an item that should animate out has to be handed over at
/// the moment it is removed — while the caller still has it:
///
/// ```
/// # use hgpui::{AnyElement, IntoElement, LeavingHandle, ParentElement as _};
/// # fn remove(leaving: &LeavingHandle<u64>, item: String) {
/// let key = 7;
/// leaving.push(key, move |_window, _cx| {
///     hgpui::div().child(item.clone()).into_any_element()
/// });
/// # }
/// ```
///
/// The captured data is released as soon as the animation finishes; nothing needs to call back
/// into your list.
#[derive(Clone)]
pub struct LeavingHandle<K> {
    entries: Rc<RefCell<Vec<(K, LeavingRow)>>>,
}

impl<K> Default for LeavingHandle<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> LeavingHandle<K> {
    /// Creates an empty handle.
    pub fn new() -> Self {
        Self {
            entries: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Queues a removed item to be animated out by the list it is handed to.
    ///
    /// Call this at the point of removal, with the item still in hand — the closure is called
    /// on every frame of the animation, so it must be able to rebuild the row by itself
    /// (a snapshot of the item is the usual way).
    pub fn push(&self, key: K, row: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) {
        self.entries.borrow_mut().push((key, Rc::new(row)));
    }

    /// Whether anything is waiting to animate out.
    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }
}

/// How long an item takes to slide into its new place. Defaults to 150ms.
pub const DEFAULT_SLIDE_DURATION: Duration = Duration::from_millis(150);

/// How many items beyond the viewport are tracked, so an item that is about to scroll in is
/// already following the list.
const TRACKING_MARGIN: usize = 8;

/// Creates an animated [`uniform_list`](crate::uniform_list).
///
/// See the [module docs](self) for what animates and what does not.
#[track_caller]
pub fn animated_list<K, R>(
    id: impl Into<ElementId>,
    scroll: &UniformListScrollHandle,
) -> AnimatedListBuilder<K, R> {
    AnimatedListBuilder {
        id: id.into(),
        scroll: scroll.clone(),
        count: 0,
        key_of: None,
        rows: None,
        duration: DEFAULT_SLIDE_DURATION,
        easing: Rc::new(ease_out_cubic),
        anchor: true,
        enter: None,
        leaving: None,
        leave: None,
        style: StyleRefinement::default(),
    }
}

/// A [`animated_list`] under construction.
pub struct AnimatedListBuilder<K, R> {
    id: ElementId,
    scroll: UniformListScrollHandle,
    count: usize,
    key_of: Option<Rc<dyn Fn(usize) -> K>>,
    rows: Option<Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>>>,
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    anchor: bool,
    enter: Option<EnterStyle>,
    leaving: Option<LeavingHandle<K>>,
    leave: Option<LeaveStyle>,
    style: StyleRefinement,
}

impl<K, R> Styled for AnimatedListBuilder<K, R> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K, R> AnimatedListBuilder<K, R>
where
    K: Hash + Eq + Clone + 'static,
    R: IntoElement + 'static,
{
    /// How many items the list has. Required.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    /// Maps an item index to a key that is **stable across frames** (an id from your own data,
    /// not the index and not a hash of the contents).
    ///
    /// The key is what ties "the thing I painted last frame" to "the thing at this index now",
    /// which is the whole basis of the animation. Required.
    pub fn key_of(mut self, key_of: impl Fn(usize) -> K + 'static) -> Self {
        self.key_of = Some(Rc::new(key_of));
        self
    }

    /// Builds the row for an index, exactly like [`uniform_list`](crate::uniform_list).
    ///
    /// The returned elements are wrapped in a painted offset by the list itself, so nothing
    /// needs to be applied here. Required, and it terminates the builder.
    pub fn rows(
        mut self,
        rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<R> + 'static,
    ) -> AnimatedList<K, R> {
        self.rows = Some(Rc::new(rows));
        AnimatedList {
            id: self.id,
            scroll: self.scroll,
            count: self.count,
            key_of: self.key_of.expect("animated_list(..).key_of(..) is required"),
            rows: self.rows.clone().expect("animated_list(..).rows(..) is required"),
            duration: self.duration,
            easing: self.easing,
            anchor: self.anchor,
            enter: self.enter,
            leaving: self.leaving,
            leave: self.leave,
            style: self.style,
            list: None,
        }
    }

    /// How long an item takes to slide into place. Defaults to 150ms.
    ///
    /// The whole list shares one duration and one easing.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Eases the slide. Defaults to [`ease_out_cubic`].
    pub fn easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Styles a row as it enters, with `t` going `0.0 -> 1.0`.
    ///
    /// Only genuinely new items animate in: a row that merely scrolled into view does not.
    /// The wrapped element is the row, so the layout is untouched (`opacity`, transforms and
    /// paint offsets only).
    pub fn enter(
        mut self,
        enter: impl Fn(AnyElement, f32) -> AnyElement + 'static,
    ) -> Self {
        self.enter = Some(Rc::new(enter));
        self
    }

    /// Animates removed items out, using the rows queued on the handle.
    ///
    /// Without this (and [`AnimatedListBuilder::leave`]) a removed item simply disappears while
    /// the rows below slide up over it. See [`LeavingHandle`].
    pub fn leaving(mut self, handle: &LeavingHandle<K>) -> Self {
        self.leaving = Some(handle.clone());
        self
    }

    /// Styles a row as it leaves, with `t` going `0.0 -> 1.0`.
    ///
    /// The row is painted in an overlay above the list, at the position it was last painted,
    /// so painting it back into being at `t = 0` looks continuous.
    pub fn leave(mut self, leave: impl Fn(AnyElement, f32) -> AnyElement + 'static) -> Self {
        self.leave = Some(Rc::new(leave));
        self
    }

    /// Whether to keep the item at the top of the viewport in place when items above it are
    /// inserted or removed. Defaults to `true`. See the [module docs](self).
    pub fn anchor(mut self, anchor: bool) -> Self {
        self.anchor = anchor;
        self
    }
}

/// The list element. See [`animated_list`].
pub struct AnimatedList<K, R> {
    id: ElementId,
    scroll: UniformListScrollHandle,
    count: usize,
    key_of: Rc<dyn Fn(usize) -> K>,
    rows: Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>>,
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    anchor: bool,
    enter: Option<EnterStyle>,
    leaving: Option<LeavingHandle<K>>,
    leave: Option<LeaveStyle>,
    style: StyleRefinement,
    /// Assembled during `request_layout`, painted again in `prepaint`/`paint`.
    list: Option<AnyElement>,
}

impl<K, R> Styled for AnimatedList<K, R> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K, R> Element for AnimatedList<K, R>
where
    K: Hash + Eq + Clone + 'static,
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
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            ListState::<K>::default()
        });

        let (row_height, offset) = read_scroll(&self.scroll, self.count);

        // Nothing to do until the list has been laid out once and told us how tall a row is.
        if let Some(row_height) = row_height.filter(|height| *height > Pixels::ZERO)
            && self.count > 0
        {
            let top_index = top_index_for(offset, row_height, self.count);
            let input = FrameInput {
                row_height,
                offset,
                top_index,
                count: self.count,
                viewport_height: window.viewport_size().height,
                key_of: self.key_of.as_ref(),
                duration: self.duration,
                easing: self.easing.as_ref(),
                anchor: self.anchor,
                // Without a leave style there is nothing to do with a queued item, but the
                // queue still has to be drained so it doesn't grow.
                leaving: match (&self.leaving, &self.leave) {
                    (Some(handle), Some(_)) => Some(handle),
                    (Some(handle), None) => {
                        handle.entries.borrow_mut().clear();
                        None
                    }
                    (None, _) => None,
                },
                reduce_motion: cx.reduce_motion(),
                now: Instant::now(),
            };

            let anchored_offset = state.update(cx, |state, _| state.advance(&input));
            if let Some(y) = anchored_offset {
                // A correction, not a scroll: it has to land this frame, even when the list
                // is scrolling smoothly.
                self.scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset_immediate(point(offset.x, y));
            }
            if state.read(cx).is_animating() {
                window.request_animation_frame();
            }
        }

        let state_for_rows = state.clone();
        let count = self.count;
        let key_of = self.key_of.clone();
        let rows = self.rows.clone();
        let scroll = self.scroll.clone();
        let duration = self.duration;
        let easing = self.easing.clone();
        let enter_style = self.enter.clone();

        let wrapped_rows = move |range: Range<usize>, window: &mut Window, cx: &mut App| {
            let (row_height, _) = read_scroll(&scroll, count);
            let now = Instant::now();
            let animating: Vec<(Pixels, Option<f32>)> = match row_height
                .filter(|height| *height > Pixels::ZERO)
            {
                Some(row_height) => {
                    let state = state_for_rows.read(cx);
                    (range.start..range.end)
                        .map(|index| {
                            let key = (key_of)(index);
                            (
                                state.offset_for(&key, row_height, duration, easing.as_ref(), now),
                                enter_style.as_ref().and_then(|_| {
                                    state.enter_progress(&key, duration, easing.as_ref(), now)
                                }),
                            )
                        })
                        .collect()
                }
                None => vec![(Pixels::ZERO, None); range.len()],
            };

            (rows)(range, window, cx)
                .into_iter()
                .zip(animating)
                .map(|(row, (offset, enter_t))| {
                    let mut row = row.into_any_element();
                    if let (Some(style), Some(t)) = (enter_style.as_ref(), enter_t) {
                        row = style(row, t);
                    }
                    if offset == Pixels::ZERO {
                        row
                    } else {
                        // Painting only: the row keeps its laid-out position and hitbox.
                        div().translate_y(offset).child(row).into_any_element()
                    }
                })
                .collect()
        };

        // The container carries the element's own layout (that is how `.h(..)`, `.flex_1()`
        // and friends size the element), the list fills it, and the exit animations are
        // painted over it.
        let mut container = div().relative();
        container.style().refine(&self.style);

        let mut list = uniform_list(self.id.clone(), self.count, wrapped_rows)
            .track_scroll(&self.scroll)
            .size_full();
        // The scroll-related part of the style belongs to the element that actually scrolls
        // (the container only carries layout).
        list.style().smooth_scroll = self.style.smooth_scroll;
        container = container.child(list);

        if let Some(leave_style) = self.leave.clone() {
            let now = Instant::now();
            let duration = self.duration;
            let easing = self.easing.clone();
            let rows: Vec<(LeavingRow, Pixels, f32)> = state.update(cx, |state, _| {
                state
                    .leaving
                    .iter()
                    .map(|leave| {
                        let t = easing(progress(leave.started, now, duration).min(1.0));
                        (leave.row.clone(), leave.painted_y, t)
                    })
                    .collect()
            });

            if !rows.is_empty() {
                let mut overlay = div().absolute().inset_0();
                for (row, painted_y, t) in rows {
                    let element = (row)(window, cx);
                    let element = leave_style(element, t);
                    overlay = overlay.child(
                        div()
                            .absolute()
                            .top(painted_y)
                            .left(px(0.))
                            .right(px(0.))
                            .child(element),
                    );
                }
                container = container.child(overlay);
            }
        }

        let mut list = container.into_any_element();
        let layout_id = list.request_layout(window, cx);
        self.list = Some(list);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(list) = self.list.as_mut() {
            list.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(list) = self.list.as_mut() {
            list.paint(window, cx);
        }
    }
}

impl<K, R> IntoElement for AnimatedList<K, R>
where
    K: Hash + Eq + Clone + 'static,
    R: IntoElement + 'static,
{
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// What the state machine needs to know about the frame it is advancing.
///
/// `Copy` is implemented by hand: deriving it would require `K: Copy`, which the struct does
/// not need since `K` only appears behind a reference.
struct FrameInput<'a, K> {
    row_height: Pixels,
    offset: crate::Point<Pixels>,
    top_index: usize,
    count: usize,
    viewport_height: Pixels,
    key_of: &'a dyn Fn(usize) -> K,
    duration: Duration,
    easing: &'a dyn Fn(f32) -> f32,
    anchor: bool,
    /// Newly removed items queued by the application.
    leaving: Option<&'a LeavingHandle<K>>,
    /// [`App::reduce_motion`]: settle everything immediately instead of animating.
    reduce_motion: bool,
    now: Instant,
}

impl<K> Clone for FrameInput<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for FrameInput<'_, K> {}

/// Where a tracked item was last painted.
struct Tracked {
    /// The index it had.
    index: usize,
    /// Where it was painted, in the list's viewport coordinates.
    painted_y: Pixels,
}

/// A removed item on its way out: its row factory, and where to paint it.
struct ActiveLeave {
    row: LeavingRow,
    painted_y: Pixels,
    started: Instant,
}

/// An item that is currently sliding, in rows of offset from where the list puts it now.
struct Slide {
    /// How far from its new place the item is painted at the *start* of the slide, in rows.
    /// Negative means it is painted above where it now belongs.
    rows: f32,
    started: Instant,
}

/// Per-list animation state, keyed by the list's element id.
struct ListState<K> {
    /// Where each tracked key was painted on the previous frame.
    tracked: HashMap<K, Tracked>,
    /// The keys currently sliding.
    sliding: HashMap<K, Slide>,
    /// The keys that just appeared, and when they did.
    entering: HashMap<K, Instant>,
    /// Removed items animating out over the list.
    leaving: Vec<ActiveLeave>,
    last_offset: Option<Pixels>,
    last_row_height: Option<Pixels>,
    last_top_key: Option<K>,
    /// When the previous frame was advanced. A slide that is interrupted mid-flight has to
    /// carry on from what was *painted*, which is one frame behind the current time.
    last_now: Option<Instant>,
}

impl<K> Default for ListState<K> {
    fn default() -> Self {
        Self {
            tracked: HashMap::new(),
            sliding: HashMap::new(),
            entering: HashMap::new(),
            leaving: Vec::new(),
            last_offset: None,
            last_row_height: None,
            last_top_key: None,
            last_now: None,
        }
    }
}

impl<K: Hash + Eq + Clone> ListState<K> {
    /// Advances the animation one frame, returning a scroll offset to apply (only when the
    /// view was anchored), or `None`.
    fn advance(&mut self, input: &FrameInput<'_, K>) -> Option<Pixels> {
        let FrameInput {
            row_height,
            offset,
            top_index,
            count,
            viewport_height,
            key_of,
            duration,
            easing,
            anchor,
            leaving,
            reduce_motion,
            now,
        } = *input;

        // A user preference for less motion wins: drop everything in flight, including items
        // queued to animate out.
        if reduce_motion {
            if let Some(handle) = leaving {
                handle.entries.borrow_mut().clear();
            }
            self.sliding.clear();
            self.entering.clear();
            self.leaving.clear();
        }

        // Items the application just removed, queued while it still had them. They are painted
        // where they were last seen, so the hand-off is invisible.
        if let Some(handle) = leaving.filter(|_| !reduce_motion) {
            let queued = std::mem::take(&mut *handle.entries.borrow_mut());
            for (key, row) in queued {
                let painted_y = self
                    .tracked
                    .get(&key)
                    .map(|tracked| tracked.painted_y)
                    .unwrap_or(offset.y);
                self.leaving.push(ActiveLeave {
                    row,
                    painted_y,
                    started: now,
                });
            }
        }
        self.leaving
            .retain(|leave| progress(leave.started, now, duration) < 1.0);

        let top_key = (count > 0).then(|| key_of(top_index));

        // Scrolling — or the first frame, or the row height changing — is external: nothing
        // about the list's contents changed, so nothing should animate. The tracked indices
        // are still refreshed, they just don't produce any movement this frame.
        // The row height comes from `contents.height / count`, and on the frame where the list
        // grows or shrinks those two are read from different moments (last frame's content
        // extent, this frame's count), so it wobbles by a fraction of a pixel. Only a *real*
        // change of the row height should reset the animation, hence the tolerance.
        const ROW_HEIGHT_TOLERANCE: Pixels = px(0.5);
        let row_height_changed = match self.last_row_height {
            Some(last) => (last - row_height).abs() > ROW_HEIGHT_TOLERANCE,
            None => true,
        };
        let snapped = self.last_offset != Some(offset.y) || row_height_changed;
        let mut animate = !snapped && !reduce_motion;
        let mut anchored_offset = None;

        if snapped {
            self.sliding.clear();
            self.entering.clear();
        } else if anchor
            && offset.y < Pixels::ZERO
            && self.last_top_key.as_ref() != top_key.as_ref()
            && let Some(top_key) = top_key.as_ref()
            && let Some(old_index) = self.tracked.get(top_key).map(|tracked| tracked.index)
            && old_index != top_index
        {
            // Items were inserted or removed above the viewport: put the item that was at the
            // top back where it was, so the view stays anchored on content.
            let rows = old_index as f32 - top_index as f32;
            let anchored = offset.y + row_height * rows;
            self.sliding.clear();
            self.entering.clear();
            animate = false;
            anchored_offset = Some(anchored);
        }

        // Track the indices of the items near the viewport. When the view was anchored, the
        // offset we just applied is what decides which items those are.
        let effective_offset = anchored_offset.unwrap_or(offset.y);
        let effective_top =
            top_index_for(point(offset.x, effective_offset), row_height, count);
        let span = (viewport_height / row_height).ceil() as usize + 2 * TRACKING_MARGIN;
        let first = effective_top.saturating_sub(TRACKING_MARGIN);
        let last = (effective_top + span.max(TRACKING_MARGIN)).min(count);

        let previous = std::mem::take(&mut self.tracked);
        // A key that was not tracked on the previous frame is a genuinely new item — items that
        // merely scroll into view arrive on a frame that snapped, so they never get here.
        let known_keys = !previous.is_empty();
        for index in first..last {
            let key = key_of(index);
            if previous.get(&key).is_none() && animate && known_keys {
                self.entering.insert(key.clone(), now);
            }
            if let Some(old_index) = previous.get(&key).map(|tracked| tracked.index)
                && old_index != index
                && animate
            {
                let delta = old_index as f32 - index as f32;
                match self.sliding.get_mut(&key) {
                    Some(slide) => {
                        // Still sliding: carry on from where it is painted right now, so a
                        // second change mid-flight doesn't jump.
                        let painted_at = self.last_now.unwrap_or(now);
                        let remaining = slide.rows
                            * (1.0 - easing(progress(slide.started, painted_at, duration)));
                        slide.rows = remaining + delta;
                        slide.started = now;
                    }
                    None => {
                        self.sliding.insert(
                            key.clone(),
                            Slide {
                                rows: delta,
                                started: now,
                            },
                        );
                    }
                }
            }

            // Remember where this row was painted, so an exit animation can take over from
            // exactly there the moment its item disappears.
            let slide_offset = match self.sliding.get(&key) {
                Some(slide) => {
                    let t = progress(slide.started, now, duration).min(1.0);
                    row_height * (slide.rows * (1.0 - easing(t)))
                }
                None => Pixels::ZERO,
            };
            self.tracked.insert(
                key,
                Tracked {
                    index,
                    painted_y: row_height * index as f32 + offset.y + slide_offset,
                },
            );
        }

        if animate {
            let tracked = &self.tracked;
            self.sliding.retain(|key, slide| {
                tracked.contains_key(key) && progress(slide.started, now, duration) < 1.0
            });
            self.entering.retain(|key, started| {
                tracked.contains_key(key) && progress(*started, now, duration) < 1.0
            });
        }

        self.last_offset = Some(effective_offset);
        self.last_row_height = Some(row_height);
        // Note: computed from the *effective* offset, which is what decides whose the top of
        // the viewport is. After anchoring that is the item we just kept in place.
        self.last_top_key = (count > 0).then(|| key_of(effective_top));
        self.last_now = Some(now);
        anchored_offset
    }

    /// How far to shift an item's painting this frame.
    fn offset_for(
        &self,
        key: &K,
        row_height: Pixels,
        duration: Duration,
        easing: &dyn Fn(f32) -> f32,
        now: Instant,
    ) -> Pixels {
        match self.sliding.get(key) {
            Some(slide) => {
                let t = progress(slide.started, now, duration).min(1.0);
                row_height * (slide.rows * (1.0 - easing(t)))
            }
            None => Pixels::ZERO,
        }
    }

    /// The enter progress for an item, if it is currently animating in.
    fn enter_progress(
        &self,
        key: &K,
        duration: Duration,
        easing: &dyn Fn(f32) -> f32,
        now: Instant,
    ) -> Option<f32> {
        self.entering
            .get(key)
            .map(|started| easing(progress(*started, now, duration).min(1.0)))
    }

    fn is_animating(&self) -> bool {
        !self.sliding.is_empty() || !self.entering.is_empty() || !self.leaving.is_empty()
    }
}

/// Elapsed fraction of the animation, before easing.
fn progress(started: Instant, now: Instant, duration: Duration) -> f32 {
    let duration = duration.as_secs_f32();
    if duration <= 0.0 {
        return 1.0;
    }
    ((now - started).as_secs_f32() / duration).clamp(0.0, 1.0)
}

/// The scroll offset and the row height the list measured last time it laid out.
///
/// Note which field holds what: `ItemSize::item` is the *list's* own padded size, while
/// `ItemSize::contents` is the content extent — `row_height * item_count`. The row height is
/// therefore `contents.height / count`.
fn read_scroll(
    scroll: &UniformListScrollHandle,
    count: usize,
) -> (Option<Pixels>, crate::Point<Pixels>) {
    let state = scroll.0.borrow();
    let row_height = match (count, state.last_item_size) {
        (0, _) | (_, None) => None,
        (count, Some(size)) => Some(size.contents.height / count as f32),
    }
    .filter(|height| *height > Pixels::ZERO);
    (row_height, state.base_handle.offset())
}

/// The index of the first visible item, mirroring how `uniform_list` computes its range.
fn top_index_for(offset: crate::Point<Pixels>, row_height: Pixels, count: usize) -> usize {
    let rows = (-offset.y / row_height).floor();
    let index = if rows <= 0.0 { 0 } else { rows as usize };
    index.min(count.saturating_sub(1))
}

/// Eases the slide out: fast at first, settling gently. The default easing.
pub fn ease_out_cubic(t: f32) -> f32 {
    let n = t - 1.0;
    n * n * n + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::px;
    use std::cell::RefCell;
    use std::time::Duration;

    /// A frame's worth of time, so the tests can advance animations without sleeping.
    const FRAME: Duration = Duration::from_millis(16);

    const ROW: f32 = 20.0;
    const DURATION: Duration = Duration::from_millis(100);
    /// Ten rows fit on screen.
    const VIEWPORT_ROWS: f32 = 10.0;

    /// Drives a `ListState` directly, with a list of item keys that the test can mutate.
    struct Harness {
        state: ListState<u64>,
        keys: RefCell<Vec<u64>>,
        leaving: LeavingHandle<u64>,
        reduce_motion: bool,
        offset: Pixels,
        row_height: Pixels,
        now: Instant,
    }

    impl Harness {
        fn new(items: usize) -> Self {
            Self {
                state: ListState::default(),
                keys: RefCell::new((0..items as u64).collect()),
                leaving: LeavingHandle::new(),
                reduce_motion: false,
                offset: Pixels::ZERO,
                row_height: px(ROW),
                now: Instant::now(),
            }
        }

        /// Replaces the list contents, leaving the ids of the items it keeps in place.
        fn set_keys(&self, keys: Vec<u64>) {
            *self.keys.borrow_mut() = keys;
        }

        fn frame(&mut self) -> Option<Pixels> {
            self.now += FRAME;
            self.run_frame()
        }

        fn run_frame(&mut self) -> Option<Pixels> {
            let keys = self.keys.borrow();
            let key_of = |index: usize| keys[index];
            let input = FrameInput {
                row_height: self.row_height,
                offset: point(Pixels::ZERO, self.offset),
                top_index: top_index_for(
                    point(Pixels::ZERO, self.offset),
                    self.row_height,
                    keys.len(),
                ),
                count: keys.len(),
                viewport_height: px(ROW * VIEWPORT_ROWS),
                key_of: &key_of,
                duration: DURATION,
                easing: &ease_out_cubic,
                anchor: true,
                leaving: Some(&self.leaving),
                reduce_motion: self.reduce_motion,
                now: self.now,
            };
            self.state.advance(&input)
        }

        fn offset_of(&self, key: u64) -> Pixels {
            self.state
                .offset_for(&key, px(ROW), DURATION, &ease_out_cubic, self.now)
        }
    }

    #[test]
    fn top_index_follows_the_scroll_offset() {
        assert_eq!(top_index_for(point(Pixels::ZERO, Pixels::ZERO), px(ROW), 100), 0);
        assert_eq!(
            top_index_for(point(Pixels::ZERO, px(-ROW * 2.5)), px(ROW), 100),
            2
        );
        // Never past the end of the list.
        assert_eq!(
            top_index_for(point(Pixels::ZERO, px(-ROW * 500.)), px(ROW), 10),
            9
        );
    }

    #[test]
    fn a_stable_list_never_animates() {
        let mut harness = Harness::new(50);
        harness.frame();
        assert!(!harness.state.is_animating());
        harness.frame();
        assert!(!harness.state.is_animating());
        assert!(harness.offset_of(3) == Pixels::ZERO);
    }

    #[test]
    fn inserting_above_makes_the_items_below_slide_down() {
        let mut harness = Harness::new(50);
        harness.frame();

        // Insert a new item at the top: everything else moves down one index.
        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();

        assert!(harness.state.is_animating());
        // Item 3 now lives at index 4, so it is painted one row higher to start with.
        assert_eq!(harness.offset_of(3), px(-ROW));
        assert_eq!(harness.offset_of(999), Pixels::ZERO, "the new item does not slide");
    }

    #[test]
    fn removing_above_makes_the_items_below_slide_up() {
        let mut harness = Harness::new(50);
        harness.frame();

        // Remove the third item: everything after it moves up one index.
        harness.set_keys((0..50).filter(|key| *key != 2).collect());
        harness.frame();

        // Item 5 moved from index 5 to index 4, so it is painted one row lower to start with.
        assert_eq!(harness.offset_of(5), px(ROW));
    }

    #[test]
    fn changes_below_the_viewport_do_not_move_anything() {
        let mut harness = Harness::new(500);
        harness.frame();

        // Delete something far below what is on screen.
        harness.set_keys((0..500).filter(|key| *key != 300).collect());
        harness.frame();

        assert!(!harness.state.is_animating());
        assert_eq!(harness.offset_of(3), Pixels::ZERO);
    }

    #[test]
    fn scrolling_snaps_instead_of_animating() {
        let mut harness = Harness::new(500);
        harness.frame();

        // The user scrolls: nothing about the contents changed.
        harness.offset = px(-ROW * 20.0);
        harness.frame();

        assert!(!harness.state.is_animating(), "scrolling must not slide anything");
        assert_eq!(
            harness.state.last_top_key,
            Some(20),
            "the tracked window follows the scroll"
        );
    }

    #[test]
    fn inserting_above_an_anchored_view_keeps_it_still() {
        let mut harness = Harness::new(500);
        harness.offset = px(-ROW * 20.0);
        harness.frame();
        let top_key = 20;
        assert_eq!(harness.state.last_top_key, Some(top_key));

        // Insert above: the item that was at the top is now one index further down.
        harness.set_keys(std::iter::once(999).chain(0..500).collect());
        let anchored = harness.frame().expect("the view should have been anchored");

        assert_eq!(anchored, px(-ROW * 21.0), "the offset follows the inserted row");
        assert!(
            !harness.state.is_animating(),
            "an anchored view does not need to slide: nothing moved on screen"
        );
        assert_eq!(
            harness.state.last_top_key,
            Some(top_key),
            "the same item is still at the top"
        );
    }

    #[test]
    fn prepending_at_the_top_scroll_position_is_not_anchored() {
        let mut harness = Harness::new(500);
        harness.frame();

        // At offset zero the new item should be seen, so the view is not anchored.
        harness.set_keys(std::iter::once(999).chain(0..500).collect());
        assert!(harness.frame().is_none());
        assert_eq!(harness.offset_of(0), px(-ROW), "the old first item slides down");
    }

    #[test]
    fn a_wobbling_row_height_does_not_reset_the_animation() {
        // `row_height` is derived from `contents.height / count`, and on the frame the list
        // grows those two come from different moments, so it wobbles by a fraction of a pixel.
        // That must not look like "the list was re-laid out externally".
        let mut harness = Harness::new(50);
        harness.frame();

        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.row_height = px(19.8);
        harness.frame();

        assert!(
            harness.state.is_animating(),
            "a sub-pixel row height change must not clear the slides"
        );
    }

    #[test]
    fn a_new_item_enters_and_does_not_slide() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();
        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();

        let t = harness
            .state
            .enter_progress(&999, DURATION, &ease_out_cubic, harness.now);
        assert!(t.is_some(), "the new item should be animating in");
        assert_eq!(
            harness.offset_of(999),
            Pixels::ZERO,
            "the new item appears where it belongs; only the others make way"
        );
    }

    #[test]
    fn entering_settles_and_is_forgotten() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();
        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();
        assert!(harness.state.is_animating());

        for _ in 0..10 {
            harness.frame();
        }

        assert!(!harness.state.is_animating());
        assert!(
            harness
                .state
                .enter_progress(&999, DURATION, &ease_out_cubic, harness.now)
                .is_none()
        );
    }

    #[test]
    fn items_scrolling_into_view_do_not_enter() {
        let mut harness = Harness::new(500);
        harness.frame();
        harness.frame();

        // The user scrolls a long way: everything on screen is "new" to the tracking table,
        // but nothing about the list changed.
        harness.offset = px(-ROW * 40.0);
        harness.frame();
        harness.frame();

        assert!(
            harness.state.entering.is_empty(),
            "scrolling must not look like items appearing"
        );
    }

    #[test]
    fn a_removed_item_is_kept_where_it_was_painted() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();

        // The application removes item 3 and hands its row over while it still has it.
        harness.leaving.push(3, |_window, _cx| unreachable!("not rendered in this test"));
        harness.set_keys((0..50).filter(|key| *key != 3).collect());
        harness.frame();

        assert_eq!(harness.state.leaving.len(), 1);
        assert_eq!(
            harness.state.leaving[0].painted_y,
            px(3.0 * ROW),
            "the exit animation starts exactly where the row was last painted"
        );
        assert!(harness.state.is_animating());
    }

    #[test]
    fn exit_animations_take_the_last_painted_position_with_them() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();

        // Insert at the top so that item 5 is *mid-slide* when it is removed: the ghost has to
        // start from where it was painted, not from where the list would put its index.
        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();
        harness.frame();
        harness.leaving.push(5, |_window, _cx| unreachable!());
        harness.set_keys(
            std::iter::once(999)
                .chain((0..50).filter(|key| *key != 5))
                .collect(),
        );
        harness.frame();

        let painted = harness.state.leaving[0].painted_y;
        assert!(
            painted > px(5.0 * ROW) && painted < px(6.0 * ROW),
            "item 5 was sliding from row 5 to row 6 and should be caught in between, got {painted:?}"
        );
    }

    #[test]
    fn exit_animations_settle_and_are_forgotten() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();
        harness.leaving.push(3, |_window, _cx| unreachable!());
        harness.frame();
        assert!(harness.state.is_animating());

        for _ in 0..10 {
            harness.frame();
        }

        assert!(harness.state.leaving.is_empty());
    }

    #[test]
    fn reduce_motion_settles_everything_immediately() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.frame();

        harness.reduce_motion = true;
        harness.leaving.push(3, |_window, _cx| unreachable!());
        harness.set_keys(
            std::iter::once(999)
                .chain((0..50).filter(|key| *key != 3))
                .collect(),
        );
        harness.frame();

        assert!(!harness.state.is_animating(), "nothing may animate");
        assert!(harness.state.leaving.is_empty(), "queued exits are dropped");
        assert_eq!(harness.offset_of(5), Pixels::ZERO);
        assert!(
            harness.leaving.is_empty(),
            "the handle is drained so it cannot grow"
        );
    }

    #[test]
    fn slides_settle_and_are_forgotten() {
        let mut harness = Harness::new(50);
        harness.frame();
        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();
        assert!(harness.state.is_animating());

        // Run past the duration (plus one frame to notice).
        for _ in 0..10 {
            harness.frame();
        }

        assert!(!harness.state.is_animating());
        assert_eq!(harness.offset_of(3), Pixels::ZERO);
    }

    #[test]
    fn a_second_change_mid_slide_does_not_jump() {
        let mut harness = Harness::new(50);
        harness.frame();

        harness.set_keys(std::iter::once(999).chain(0..50).collect());
        harness.frame();
        // Let it get part of the way through.
        harness.frame();
        harness.frame();
        let before = harness.offset_of(3);
        assert!(before < Pixels::ZERO && before > px(-ROW), "mid-slide: {before:?}");

        // `offset_of` is measured from where the list *now* puts the item, so compare the
        // painted position in content space: the item must not jump on screen.
        let before_painted = px(4.0 * ROW) + before;

        // Insert another item at the top while the first slide is still running.
        harness.set_keys(
            std::iter::once(998)
                .chain(std::iter::once(999))
                .chain(0..50)
                .collect(),
        );
        harness.frame();

        let after = harness.offset_of(3);
        let after_painted = px(5.0 * ROW) + after;
        assert!(
            (after_painted - before_painted).abs() < px(0.5),
            "the item must carry on from where it was painted              ({before_painted:?} -> {after_painted:?})"
        );

        // ... and still land exactly where the list puts it.
        for _ in 0..10 {
            harness.frame();
        }
        assert_eq!(harness.offset_of(3), Pixels::ZERO);
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    use crate::{AppContext as _, Context, ParentElement as _, Render, TestAppContext, px, size};
    use std::cell::Cell;
    use std::rc::Rc;

    struct View {
        scroll: UniformListScrollHandle,
        calls: Rc<Cell<usize>>,
        rendered: Rc<Cell<usize>>,
    }

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let calls = self.calls.clone();
            let rendered = self.rendered.clone();
            div().size_full().child(
                animated_list("items", &self.scroll)
                    .h(px(400.))
                    .count(100)
                    .key_of(move |index| index as u64)
                    .rows(move |range, _window, _cx| {
                        calls.set(calls.get() + 1);
                        rendered.set(rendered.get() + range.len());
                        range
                            .map(|index| div().h(px(20.)).child(format!("row {index}")))
                            .collect()
                    }),
            )
        }
    }

    #[hgpui::test]
    fn a_mounted_list_renders_its_rows(cx: &mut TestAppContext) {
        let scroll = UniformListScrollHandle::new();
        let scroll_for_view = scroll.clone();
        let calls = Rc::new(Cell::new(0));
        let rendered = Rc::new(Cell::new(0));
        let (calls_for_view, rendered_for_view) = (calls.clone(), rendered.clone());

        let window = cx.open_window(size(px(300.), px(400.)), move |_, _cx| View {
            scroll: scroll_for_view,
            calls: calls_for_view,
            rendered: rendered_for_view,
        });
        cx.run_until_parked();

        cx.update_window(*window, |_, window, cx| {
            let token = window.draw(cx);
            token.clear(cx);
        })
        .expect("window update failed");

        // A 400px viewport with 20px rows: most of the list is on screen.
        assert!(
            rendered.get() >= 15,
            "expected the visible rows to be rendered, got {} (rows closure ran {} times)",
            rendered.get(),
            calls.get()
        );
        assert_eq!(
            scroll.0.borrow().last_item_size.map(|s| s.contents.height / 100.0),
            Some(px(20.)),
            "the row height is read back as 20px"
        );
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::{
        AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, Window, div, px, size,
    };
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct LiveView {
        items: Rc<RefCell<Vec<u64>>>,
        scroll: UniformListScrollHandle,
        leaving: LeavingHandle<u64>,
        exit_rows: Rc<Cell<usize>>,
    }

    impl Render for LiveView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let items = Rc::new(self.items.borrow().clone());
            let count = items.len();
            div().size_full().child(
                animated_list("items", &self.scroll)
                    .h(px(400.))
                    .leaving(&self.leaving)
                    .leave(|row, t| {
                        div().opacity(1. - t).child(row).into_any_element()
                    })
                    .count(count)
                    .key_of({
                        let items = items.clone();
                        move |index| items[index]
                    })
                    .rows({
                        let items = items.clone();
                        move |range, _window, _cx| {
                            range
                                .map(|index| div().h(px(20.)).child(format!("row {}", items[index])))
                                .collect()
                        }
                    }),
            )
        }
    }

    fn draw<V: Render + 'static>(cx: &mut TestAppContext, window: &crate::WindowHandle<V>) {
        cx.update_window(**window, |_, window, cx| {
            let token = window.draw(cx);
            token.clear(cx);
        })
        .expect("window update failed");
    }

    #[hgpui::test]
    fn a_removed_item_is_painted_by_its_exit_animation(cx: &mut TestAppContext) {
        let items = Rc::new(RefCell::new((0..100u64).collect::<Vec<_>>()));
        let scroll = UniformListScrollHandle::new();
        let leaving = LeavingHandle::new();
        let exit_rows = Rc::new(Cell::new(0));
        let (items_for_view, scroll_for_view) = (items.clone(), scroll.clone());
        let (leaving_for_view, exit_for_view) = (leaving.clone(), exit_rows.clone());
        let window = cx.open_window(size(px(300.), px(400.)), move |_, _cx| LiveView {
            items: items_for_view,
            scroll: scroll_for_view,
            leaving: leaving_for_view,
            exit_rows: exit_for_view,
        });
        cx.run_until_parked();
        draw(cx, &window);
        draw(cx, &window);

        // Remove item 3, handing its row to the list.
        let removed = items.borrow_mut().remove(3);
        let exit_rows_for_row = exit_rows.clone();
        leaving.push(removed, move |_window, _cx| {
            exit_rows_for_row.set(exit_rows_for_row.get() + 1);
            div().h(px(20.)).child("ghost").into_any_element()
        });
        window
            .update(cx, |_, _, cx| cx.notify())
            .expect("update failed");
        draw(cx, &window);
        assert!(
            exit_rows.get() > 0,
            "the exit animation should be painting the removed row"
        );

        // ... and it lets go once the animation is over.
        std::thread::sleep(std::time::Duration::from_millis(250));
        draw(cx, &window);
        let painted = exit_rows.get();
        draw(cx, &window);
        assert_eq!(
            exit_rows.get(),
            painted,
            "the removed item is released when its animation ends"
        );
    }

    struct SmoothView {
        scroll: UniformListScrollHandle,
    }

    impl Render for SmoothView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                animated_list("items", &self.scroll)
                    .h(px(400.))
                    .smooth_scroll(true)
                    .count(200)
                    .key_of(move |index| index as u64)
                    .rows(move |range, _window, _cx| {
                        range
                            .map(|index| div().h(px(20.)).child(format!("row {index}")))
                            .collect()
                    }),
            )
        }
    }

    #[hgpui::test]
    fn smooth_scrolling_eases_towards_the_target(cx: &mut TestAppContext) {
        let scroll = UniformListScrollHandle::new();
        let scroll_for_view = scroll.clone();
        let window = cx.open_window(size(px(300.), px(400.)), move |_, _cx| SmoothView {
            scroll: scroll_for_view,
        });
        cx.run_until_parked();
        draw(cx, &window);

        // Ask to scroll 400px (20 rows): the offset should glide there, not jump.
        scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(-400.)));
        draw(cx, &window);

        let after_one_frame = -scroll.0.borrow().base_handle.offset().y;
        assert!(
            after_one_frame > px(0.) && after_one_frame < px(400.),
            "the offset should be part way there, got {after_one_frame:?}"
        );

        // Run it out the way a real frame loop would. (A single long sleep would not do: a
        // frame's delta time is clamped, so a stalled frame cannot teleport the offset.)
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            draw(cx, &window);
        }
        assert_eq!(
            scroll.0.borrow().base_handle.offset().y,
            px(-400.),
            "and it settles exactly on the target"
        );
    }

    #[hgpui::test]
    fn a_list_keeps_rendering_across_an_insert(cx: &mut TestAppContext) {
        let items = Rc::new(RefCell::new((0..100u64).collect::<Vec<_>>()));
        let scroll = UniformListScrollHandle::new();
        let (items_for_view, scroll_for_view) = (items.clone(), scroll.clone());
        let leaving = LeavingHandle::new();
        let window = cx.open_window(size(px(300.), px(400.)), move |_, _cx| LiveView {
            items: items_for_view,
            scroll: scroll_for_view,
            leaving,
            exit_rows: Rc::new(Cell::new(0)),
        });
        cx.run_until_parked();

        draw(cx, &window);
        draw(cx, &window);
        draw(cx, &window);

        // Insert at the top: the rows below have to keep rendering, with a slide applied.
        items.borrow_mut().insert(0, 999);
        window
            .update(cx, |_, _, cx| cx.notify())
            .expect("update failed");
        draw(cx, &window);
        draw(cx, &window);

        // And the list is still whole after the animation would have settled.
        std::thread::sleep(std::time::Duration::from_millis(200));
        draw(cx, &window);
        assert_eq!(
            scroll.0.borrow().last_item_size.map(|size| size.contents.height / 101.0),
            Some(px(20.)),
            "the list still measures one row per item"
        );
    }
}
