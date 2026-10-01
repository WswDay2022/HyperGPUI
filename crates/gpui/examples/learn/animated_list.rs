//! Animated List Example
//!
//! A virtualized list of 5,000 items whose visible rows *slide* when the list changes: insert
//! near the top and everything below makes way, remove and the gap closes.
//!
//! **"Live" is on by default**: items are inserted and removed near the top of the viewport
//! every 600ms, so there is always something animating. That is the setup to drag the window
//! around in — resize it while items come and go, and the division of labour is visible:
//!
//! - **Resizing never animates.** A row only moves when its *index* changes, and a resize
//!   doesn't change indices — so the content tracks the window edge exactly.
//! - **Content changes do animate**, even mid-drag: rows make way while you are still dragging.
//!
//! Things worth trying:
//!
//! - **Insert at the top while scrolled down.** The view stays anchored on the same items —
//!   nothing moves on screen, because the list adjusts the scroll offset to keep the item that
//!   was at the top of the viewport in place.
//! - **Insert at the top while you are at the very top.** There is nothing to anchor to, so the
//!   new row fades in where it belongs and the rows below slide down to make room.
//! - **Insert 50 / Remove 50.** Many rows make way at once, each sliding its own distance.
//! - **Remove #3.** The removed row (tinted red here, so you can see it) fades out where it was
//!   painted while the rows below slide up to close the gap.
//!
//! Turn "Live" off when you want to drive the list by hand without the background churn.
//!
//! ```sh
//! cargo run -p hgpui --example animated-list
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::rc::Rc;
use std::time::Duration;

use hgpui::{
    AnyElement, App, AppContext, Bounds, Context, InteractiveElement, IntoElement, LeavingHandle,
    ParentElement, Render, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window,
    WindowBounds, WindowOptions, animated_list, div, px, rgb, size,
};

/// The height of every row, in pixels.
const ROW_HEIGHT: f32 = 28.0;
/// How many items to start with.
const INITIAL_ITEMS: u64 = 5_000;
/// How often the live mode changes the list.
const LIVE_INTERVAL: Duration = Duration::from_millis(600);
/// How long the list takes to slide a row.
const SLIDE: Duration = Duration::from_millis(180);

#[derive(Clone)]
struct Item {
    id: u64,
}

struct AnimatedListExample {
    /// The data, shared with the row closures the list keeps.
    items: Rc<Vec<Item>>,
    scroll: UniformListScrollHandle,
    /// Rows of removed items, so they can fade out on their way.
    leaving: LeavingHandle<u64>,
    next_id: u64,
    /// A xorshift state: the example needs no random-number dependency, and every run is
    /// reproducible.
    rng: u64,
    live: bool,
    /// Bumped whenever the ticker is (re)started, so a stale ticker stops when it notices.
    live_generation: usize,
    /// The index at the top of the viewport, refreshed every frame so that live changes land
    /// where they can be seen.
    top_index: usize,
}

impl AnimatedListExample {
    fn new() -> Self {
        let items = (0..INITIAL_ITEMS).map(|id| Item { id }).collect();
        Self {
            items: Rc::new(items),
            scroll: UniformListScrollHandle::new(),
            leaving: LeavingHandle::new(),
            next_id: INITIAL_ITEMS,
            rng: 0x2545_F491_4F6C_DD1D,
            live: true,
            live_generation: 0,
            top_index: 0,
        }
    }

    /// A small xorshift, just so the example doesn't need a dependency.
    fn random(&mut self, bound: usize) -> usize {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        if bound == 0 {
            0
        } else {
            (x % bound as u64) as usize
        }
    }

    /// A position near the top of the viewport, so a change is actually visible.
    fn visible_index(&mut self) -> usize {
        let top = self.top_index;
        (top + self.random(10)).min(self.items.len())
    }

    fn insert(&mut self, index: usize, cx: &mut Context<Self>) {
        let item = Item { id: self.next_id };
        self.next_id += 1;
        let items = Rc::make_mut(&mut self.items);
        items.insert(index.min(items.len()), item);
        cx.notify();
    }

    /// Inserts `count` items at one position, as a batch.
    fn insert_many(&mut self, count: usize, cx: &mut Context<Self>) {
        let index = self.visible_index();
        let items = Rc::make_mut(&mut self.items);
        let index = index.min(items.len());
        for offset in 0..count {
            let item = Item { id: self.next_id };
            self.next_id += 1;
            items.insert(index + offset, item);
        }
        cx.notify();
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = Rc::make_mut(&mut self.items);
        if index >= items.len() {
            return;
        }

        // Hand the removed row to the list while we still have the item: the list cannot render
        // an item that is no longer in the data, and a rendered element cannot be kept across
        // frames. (A real app would reuse its row builder here for a seamless hand-off; this one
        // tints the ghost so the exit animation is easy to see.)
        let removed = items.remove(index);
        self.leaving
            .push(removed.id, move |_window, _cx| ghost_row(removed.id));
        cx.notify();
    }

    /// Removes `count` items from around the top of the viewport, as a batch.
    fn remove_many(&mut self, count: usize, cx: &mut Context<Self>) {
        for _ in 0..count {
            let index = self.visible_index();
            self.remove(index, cx);
        }
    }

    /// Starts or stops the background churn.
    fn set_live(&mut self, live: bool, cx: &mut Context<Self>) {
        self.live = live;
        self.live_generation += 1;
        if live {
            self.spawn_ticker(cx);
        }
        cx.notify();
    }

    fn spawn_ticker(&mut self, cx: &mut Context<Self>) {
        let generation = self.live_generation;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(LIVE_INTERVAL).await;

            let still_running = this
                .update(cx, |this, cx| {
                    // A newer ticker, or "Live: off", has taken over.
                    if !this.live || this.live_generation != generation {
                        return false;
                    }
                    let insert_at = this.visible_index();
                    this.insert(insert_at, cx);
                    let remove_at = this.visible_index();
                    this.remove(remove_at, cx);
                    true
                })
                .unwrap_or(false);

            if !still_running {
                break;
            }
        })
        .detach();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.items = Rc::new((0..INITIAL_ITEMS).map(|id| Item { id }).collect());
        self.next_id = INITIAL_ITEMS;
        cx.notify();
    }

    /// The id of the item currently at the top of the viewport, so the anchoring is visible.
    fn top_item(&self) -> Option<u64> {
        self.items.get(self.top_index).map(|item| item.id)
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_lg()
            .background(rgb(0x2f2f36))
            .hover(|style| style.background(rgb(0x3d3d46)))
            .text_sm()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
    }
}

/// The row of an item on its way out. Deliberately different from a normal row so the exit
/// animation is easy to see; a real application would render the item itself.
fn ghost_row(id: u64) -> AnyElement {
    div()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .background(rgb(0x5a2a2a))
        .child(format!("Item #{id} — leaving"))
        .into_any_element()
}

impl Render for AnimatedListExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items.clone();
        let row_count = items.len();

        // Where the viewport is, so that live changes land in view.
        let offset = f32::from(self.scroll.0.borrow().base_handle.offset().y);
        self.top_index = ((-offset / ROW_HEIGHT).floor().max(0.0)) as usize;
        let top_item = self.top_item();
        let live = self.live;

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .background(rgb(0x1b1b1f))
            .text_color(rgb(0xe6e6e6))
            // One at a time.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button("prepend", "Insert at top", |this, cx| this.insert(0, cx), cx))
                    .child(self.button("insert-3", "Insert #3", |this, cx| this.insert(3, cx), cx))
                    .child(self.button("insert-2500", "Insert #2500", |this, cx| {
                        this.insert(2500, cx)
                    }, cx))
                    .child(self.button("append", "Append", |this, cx| {
                        let end = this.items.len();
                        this.insert(end, cx);
                    }, cx))
                    .child(self.button("remove-3", "Remove #3", |this, cx| this.remove(3, cx), cx)),
            )
            // Batches, and the live mode.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button("insert-5", "Insert 5", |this, cx| this.insert_many(5, cx), cx))
                    .child(self.button("insert-50", "Insert 50", |this, cx| this.insert_many(50, cx), cx))
                    .child(self.button("remove-5", "Remove 5", |this, cx| this.remove_many(5, cx), cx))
                    .child(self.button("remove-50", "Remove 50", |this, cx| this.remove_many(50, cx), cx))
                    .child(self.button("reset", "Reset", |this, cx| this.reset(cx), cx))
                    .child(self.button(
                        "live",
                        if live { "Live: on" } else { "Live: off" },
                        |this, cx| {
                            let live = this.live;
                            this.set_live(!live, cx)
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9a9a9a))
                    .child(format!(
                        "{row_count} items · top of the viewport: #{} · scroll {}px · \
                         resize the window while live is on",
                        top_item.map_or("–".to_string(), |id| id.to_string()),
                        -offset as i32,
                    )),
            )
            .child(
                animated_list("items", &self.scroll)
                    .flex_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .count(row_count)
                    .duration(SLIDE)
                    .leaving(&self.leaving)
                    // Removed rows fade out where they were, over the rows closing up.
                    .leave(|row, t| {
                        div()
                            .opacity(1. - t)
                            .translate_y(px(-6.) * t)
                            .child(row)
                            .into_any_element()
                    })
                    // New rows fade and slide up into place.
                    .enter(|row, t| {
                        div()
                            .opacity(t)
                            .translate_y(px(6.) * (1. - t))
                            .child(row)
                            .into_any_element()
                    })
                    .key_of({
                        let items = items.clone();
                        move |index| items[index].id
                    })
                    .rows({
                        let items = items.clone();
                        move |range, _window, _cx| {
                            range
                                .map(|index| {
                                    let item = &items[index];
                                    div()
                                        .h(px(ROW_HEIGHT))
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .px_3()
                                        .background(if index % 2 == 0 {
                                            rgb(0x232329)
                                        } else {
                                            rgb(0x1f1f24)
                                        })
                                        .child(format!("Item #{}", item.id))
                                })
                                .collect()
                        }
                    }),
            )
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(700.), px(620.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let example = cx.new(|_| AnimatedListExample::new());
                // Kick off the live mode once the view exists.
                example.update(cx, |example, cx| example.set_live(true, cx));
                example
            },
        )
        .unwrap();
        example_prelude::init_example(cx, "Animated List");
    });
}
