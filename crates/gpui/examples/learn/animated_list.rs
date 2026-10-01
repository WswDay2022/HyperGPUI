//! Animated List Example
//!
//! A small animated list whose rows are laid out by the layout engine — so every row can be a
//! different height, and rows that change height push their neighbours out of the way smoothly.
//!
//! This is the non-virtualized [`animated_list`]: it builds every row, every frame, which is
//! fine for tens or hundreds of rows. For thousands of uniform rows (virtualized, only what is
//! on screen) see `animated-uniform-list`, which has the same API and the same behaviour.
//!
//! Things worth trying:
//!
//! - **Insert at top / Remove row 3.** The rows below glide to their new places; the removed row
//!   fades out where it was (tinted red here so you can see it).
//! - **Make row 3 taller.** Its neighbours glide down to make room — this is the one thing the
//!   virtualized list cannot do, because it needs every row to be the same height.
//! - Scroll the list with the wheel: it eases, and keeps coasting for a moment.
//!
//! ```sh
//! cargo run -p hgpui --example animated-list
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::rc::Rc;
use std::time::Duration;

use hgpui::{
    App, AppContext, Bounds, Context, InteractiveElement, IntoElement, LeavingHandle, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions, animated_list,
    div, px, rgb, size,
};

/// How long a row takes to glide.
const SLIDE: Duration = Duration::from_millis(220);
/// How many rows to start with.
const INITIAL_ROWS: usize = 12;

#[derive(Clone)]
struct Row {
    id: u64,
    /// The height of this row. Rows differ, and they can change.
    height: f32,
}

struct AnimatedListExample {
    rows: Rc<Vec<Row>>,
    leaving: LeavingHandle<u64>,
    next_id: u64,
}

impl AnimatedListExample {
    fn new() -> Self {
        let rows = (0..INITIAL_ROWS as u64).map(new_row).collect();
        Self {
            rows: Rc::new(rows),
            leaving: LeavingHandle::new(),
            next_id: INITIAL_ROWS as u64,
        }
    }

    fn insert(&mut self, index: usize, cx: &mut Context<Self>) {
        let row = new_row(self.next_id);
        self.next_id += 1;
        let rows = Rc::make_mut(&mut self.rows);
        rows.insert(index.min(rows.len()), row);
        cx.notify();
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        let rows = Rc::make_mut(&mut self.rows);
        if index >= rows.len() {
            return;
        }

        // Hand the removed row over while we still have it: the list cannot render a row that is
        // no longer in the data, and a rendered element cannot be kept across frames.
        let removed = rows.remove(index);
        self.leaving.push(removed.id, move |_window, _cx| {
            let mut row = row_body(&removed);
            row.style().background = Some(rgb(0x5a2a2a).into());
            row.into_any_element()
        });
        cx.notify();
    }

    /// Grows or shrinks one row, so the rows below have to make room.
    fn toggle_height(&mut self, index: usize, cx: &mut Context<Self>) {
        let rows = Rc::make_mut(&mut self.rows);
        if let Some(row) = rows.get_mut(index) {
            row.height = if row.height > 40.0 { 28.0 } else { 64.0 };
        }
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.rows = Rc::new((0..INITIAL_ROWS as u64).map(new_row).collect());
        self.next_id = INITIAL_ROWS as u64;
        cx.notify();
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
            .text_sm()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
    }
}

fn new_row(id: u64) -> Row {
    Row {
        id,
        // Deliberately uneven: the layout engine decides where these go.
        height: 28.0 + (id * 13 % 36) as f32,
    }
}

fn row_body(row: &Row) -> hgpui::Div {
    div()
        .h(px(row.height))
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .background(if row.id % 2 == 0 {
            rgb(0x232329)
        } else {
            rgb(0x1f1f24)
        })
        .child(format!("Row #{} — {}px", row.id, row.height as u32))
}

impl Render for AnimatedListExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let count = rows.len();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .background(rgb(0x1b1b1f))
            .text_color(rgb(0xe6e6e6))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button("prepend", "Insert at top", |this, cx| this.insert(0, cx), cx))
                    .child(self.button("insert-middle", "Insert in the middle", |this, cx| {
                        let middle = this.rows.len() / 2;
                        this.insert(middle, cx)
                    }, cx))
                    .child(self.button("append", "Append", |this, cx| {
                        let end = this.rows.len();
                        this.insert(end, cx);
                    }, cx))
                    .child(self.button("remove", "Remove row 3", |this, cx| this.remove(3, cx), cx))
                    .child(self.button("grow", "Grow row 3", |this, cx| this.toggle_height(3, cx), cx))
                    .child(self.button("reset", "Reset", |this, cx| this.reset(cx), cx)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9a9a9a))
                    .child(format!(
                        "{count} rows of different heights · every row is built every frame · \
                         wheel to feel the eased scrolling"
                    )),
            )
            .child(
                animated_list("rows")
                    .flex_1()
                    .rounded_lg()
                    // Rounded corners only clip with `overflow: hidden`, exactly like CSS.
                    .overflow_hidden()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .count(count)
                    .duration(SLIDE)
                    .smooth_scroll(true)
                    .leaving(&self.leaving)
                    .leave(|row, t| {
                        div()
                            .opacity(1. - t)
                            .translate_y(px(-6.) * t)
                            .child(row)
                            .into_any_element()
                    })
                    .enter(|row, t| {
                        div()
                            .opacity(t)
                            .translate_y(px(6.) * (1. - t))
                            .child(row)
                            .into_any_element()
                    })
                    .key_of({
                        let rows = rows.clone();
                        move |index| rows[index].id
                    })
                    .rows({
                        let rows = rows.clone();
                        move |range, _window, _cx| {
                            range.map(|index| row_body(&rows[index])).collect()
                        }
                    }),
            )
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(660.), px(600.)), cx);
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
