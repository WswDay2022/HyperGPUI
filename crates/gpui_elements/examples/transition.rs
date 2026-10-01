//! Transition example: the three modes side by side.
//!
//! Click "Switch" in each column and compare:
//!
//! - `Simultaneous`: the old card fades out while the new one fades in.
//! - `OutIn`: the column empties first, and only then does the new card appear.
//! - `InOut`: the new card is in place before the old one starts leaving.
//!
//! The two cards deliberately differ in height: during a transition the outgoing card sits in
//! an absolutely positioned overlay, so the column is sized by the incoming card and never
//! reflows.
//!
//! ```sh
//! cargo run -p hgpui_elements --example transition
//! ```

use std::time::Duration;

use hgpui::{
    App, AppContext as _, Bounds, Context, Div, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    WindowBounds, WindowOptions, div, px, rgb, size,
};
use hgpui_elements::transition::{TransitionMode, transition};

/// The three modes, one per column.
const MODES: [(TransitionMode, &str); 3] = [
    (TransitionMode::Simultaneous, "Simultaneous"),
    (TransitionMode::OutIn, "OutIn"),
    (TransitionMode::InOut, "InOut"),
];

const DURATION: Duration = Duration::from_millis(250);

struct Example {
    /// One key per column; bumping it is what triggers the transition.
    keys: [usize; 3],
}

impl Example {
    fn column(&mut self, index: usize, cx: &mut Context<Self>) -> Div {
        let (mode, name) = MODES[index];
        let key = self.keys[index];

        div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .child(div().text_sm().text_color(rgb(0x9a9a9a)).child(name))
            .child(
                transition(("mode", index), key)
                    .duration(DURATION)
                    .mode(mode)
                    .easing(hgpui::ease_in_out)
                    // `t` goes 0 -> 1. Entering: slide down into place. Leaving: slide up away.
                    .enter(|child, t| {
                        div()
                            .opacity(t)
                            .translate_y(px(12.) * (1. - t))
                            .child(child)
                            .into_any_element()
                    })
                    .leave(|child, t| {
                        div()
                            .opacity(1. - t)
                            .translate_y(px(-12.) * t)
                            .child(child)
                            .into_any_element()
                    })
                    .child(move |_window, _cx| card(key).into_any_element()),
            )
            .child(
                div()
                    .id(("switch", index))
                    .px_3()
                    .py_1()
                    .rounded_lg()
                    .background(rgb(0x2f2f36))
                    .hover(|style| style.background(rgb(0x3d3d46)))
                    .child("Switch")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.keys[index] += 1;
                        cx.notify();
                    })),
            )
    }
}

/// Two cards of different heights, so the layout behaviour is visible.
fn card(key: usize) -> Div {
    let (background, title, extra_lines) = if key % 2 == 0 {
        (rgb(0x2b4c6f), "Card A", 3)
    } else {
        (rgb(0x6f2b4c), "Card B", 0)
    };

    div()
        .w_full()
        .p_4()
        .rounded_lg()
        .background(background)
        .flex()
        .flex_col()
        .gap_1()
        .child(div().child(title))
        .children((0..extra_lines).map(|line| {
            div()
                .text_sm()
                .text_color(rgb(0xc0c0c8))
                .child(format!("extra line {line}"))
        }))
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_row()
            .gap_4()
            .p_6()
            .background(rgb(0x1b1b1f))
            .text_color(rgb(0xe6e6e6))
            .children((0..MODES.len()).map(|index| self.column(index, cx)))
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.), px(360.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Example { keys: [0, 0, 0] }),
        )
        .unwrap();
        cx.activate(true);
    });
}
