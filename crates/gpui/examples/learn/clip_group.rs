//! Side-by-side comparison of per-primitive rounded masks vs group-level clipping.
//!
//! The left panel uses the normal `overflow_hidden` mask (each child is clipped
//! independently). The right panel enables `clip_group()`, which renders its
//! children into an offscreen group and applies the rounded mask once.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use hgpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window,
    WindowBounds, WindowOptions, div, px, rgb, size,
};

struct ClipGroupExample;

fn panel(grouped: bool) -> impl IntoElement {
    let panel = div()
        .relative()
        .w(px(260.))
        .h(px(200.))
        .overflow_hidden()
        .rounded(px(48.))
        .background(rgb(0xFFFFFFFF));

    let panel = if grouped { panel.clip_group() } else { panel };

    panel
        .child(div().absolute().size_full().background(rgb(0xFFD23F)))
        .child(div().absolute().left_0().top_0().w_full().h(px(72.)).background(rgb(0x2D6CDF)))
        .child(
            div()
                .absolute()
                .left(px(36.))
                .top(px(36.))
                .w(px(188.))
                .h(px(128.))
                .rounded(px(28.))
                .background(rgb(0xFF6B6B)),
        )
}

impl Render for ClipGroupExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .gap(px(24.))
            .p(px(24.))
            .items_center()
            .justify_center()
            .background(rgb(0x101018))
            .child(panel(false))
            .child(panel(true))
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(600.), px(260.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ClipGroupExample),
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Clip group (left: per-primitive, right: group)");
    });
}
