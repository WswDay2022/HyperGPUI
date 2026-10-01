//! Animated List Example
//!
//! A virtualized list of 5,000 items whose visible rows *slide* when the list changes: insert
//! near the top and everything below makes way, remove and the gap closes.
//!
//! Two things worth trying:
//!
//! - **Insert at the top while scrolled down.** The view stays anchored on the same items —
//!   nothing moves on screen, because the list adjusts the scroll offset to keep the item that
//!   was at the top of the viewport in place.
//! - **Insert at the top while you are at the very top.** There is nothing to anchor to, so the
//!   new row fades in where it belongs and the rows below slide down to make room.
//! - **Remove #3.** The removed row (tinted red here, so you can see it) fades out where it was
//!   painted while the rows below slide up to close the gap.
//!
//! Scrolling and resizing the window never animate: an item's row position only moves when its
//! *index* changes.
//!
//! ```sh
//! cargo run -p hgpui --example animated-list
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::rc::Rc;

use hgpui::{
    App, AppContext, Bounds, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, animated_list, div, px, rgb, size,
};
use hgpui::LeavingHandle;

/// The height of every row, in pixels.
const ROW_HEIGHT: f32 = 28.0;
/// How many items to start with.
const INITIAL_ITEMS: u64 = 5_000;

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
}

impl AnimatedListExample {
    fn new() -> Self {
        let items = (0..INITIAL_ITEMS).map(|id| Item { id }).collect();
        Self {
            items: Rc::new(items),
            scroll: UniformListScrollHandle::new(),
            leaving: LeavingHandle::new(),
            next_id: INITIAL_ITEMS,
        }
    }

    fn insert(&mut self, index: usize, cx: &mut Context<Self>) {
        let item = Item { id: self.next_id };
        self.next_id += 1;
        let items = Rc::make_mut(&mut self.items);
        items.insert(index.min(items.len()), item);
        cx.notify();
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        let items = Rc::make_mut(&mut self.items);
        if index >= items.len() {
            return;
        }

        // Hand the removed row to the list while we still have the item: the list cannot render
        // an item that is no longer in the data, and a rendered element cannot be kept across
        // frames. (A real app would reuse its row builder here, for a seamless hand-off; this
        // one tints the ghost so the exit animation is easy to see.)
        let removed = items.remove(index);
        self.leaving.push(removed.id, move |_window, _cx| {
            div()
                .h(px(ROW_HEIGHT))
                .w_full()
                .flex()
                .items_center()
                .px_3()
                .background(rgb(0x5a2a2a))
                .child(format!("Item #{} — leaving", removed.id))
                .into_any_element()
        });
        cx.notify();
    }

    /// The id of the item currently at the top of the viewport, so the anchoring is visible.
    fn top_item(&self) -> Option<u64> {
        let offset = f32::from(self.scroll.0.borrow().base_handle.offset().y);
        let index = ((-offset / ROW_HEIGHT).floor().max(0.0)) as usize;
        self.items.get(index).map(|item| item.id)
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
            .px_3()
            .py_1()
            .rounded_lg()
            .background(rgb(0x2f2f36))
            .hover(|style| style.background(rgb(0x3d3d46)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
    }
}

impl Render for AnimatedListExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items.clone();
        let row_count = items.len();
        let top_item = self.top_item();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .background(rgb(0x1b1b1f))
            .text_color(rgb(0xe6e6e6))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(self.button(
                        "prepend",
                        "Insert at top",
                        |this, cx| this.insert(0, cx),
                        cx,
                    ))
                    .child(self.button(
                        "insert-near-top",
                        "Insert at #3",
                        |this, cx| this.insert(3, cx),
                        cx,
                    ))
                    .child(self.button(
                        "insert-middle",
                        "Insert at #2500",
                        |this, cx| this.insert(2500, cx),
                        cx,
                    ))
                    .child(self.button(
                        "remove",
                        "Remove #3",
                        |this, cx| this.remove(3, cx),
                        cx,
                    ))
                    .child(self.button(
                        "append",
                        "Append",
                        |this, cx| {
                            let end = this.items.len();
                            this.insert(end, cx);
                        },
                        cx,
                    )),
            )
            .child(div().text_sm().text_color(rgb(0x9a9a9a)).child(format!(
                "{row_count} items · top of the viewport: #{} — scroll down and prepend to see \
                 the view stay anchored",
                top_item.map_or("–".to_string(), |id| id.to_string()),
            )))
            .child(
                animated_list("items", &self.scroll)
                    .flex_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .count(row_count)
                    .duration(std::time::Duration::from_millis(180))
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
        let bounds = Bounds::centered(None, size(px(640.), px(560.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| AnimatedListExample::new()),
        )
        .unwrap();
        example_prelude::init_example(cx, "Animated List");
    });
}
