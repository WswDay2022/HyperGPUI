//! Backdrop Blur Test
//!
//! A diagnostic scene for the backdrop/content blur filters. The window shows a set of frosted
//! panels over a high-frequency background (a strip of colored squares and a bright red marker
//! at the right edge) so that uneven blur is easy to spot:
//!
//! - **center / left-edge / corner / right-edge panels** exercise `backdrop_blur` at different
//!   distances from the window edges. Blur taps that cross a window edge must behave like CSS
//!   (clamp, no contribution from the far side) — a wrapping sampler would smear the red
//!   marker (or the squares) into panels near the opposite edge.
//! - **the large-radius panel** (r=48) exercises the spread-tap path (`tap_step > 1`), where a
//!   truncated kernel or an undersized blur region shows up as a sharp ring or garbage band
//!   around the panel instead of a uniform gaussian.
//! - **the green content-blur group** exercises `filter: blur` on a subtree: the blur must
//!   bleed uniformly past the group's box, without a contaminated band along its border.
//!
//! Run it on either backend:
//!
//! ```sh
//! cargo run -p gpui-ce --example blur-test              # native (DirectX/HLSL) backend
//! cargo run -p gpui-ce --example blur-test --features wgpu   # wgpu (WGSL) backend
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use gpui::{
    App, Bounds, Context, Window, WindowBounds, WindowOptions, div, hsla, prelude::*, px, rgb,
    rgba, size,
};

struct BlurTest;

impl Render for BlurTest {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("blur-test-root")
            .relative()
            .size_full()
            .background(rgb(0x14141a))
            // High-frequency background: a strip of small colored squares across the top.
            .child(
                div()
                    .id("bg-strip")
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w_full()
                    .h(px(60.0))
                    .flex()
                    .flex_row()
                    .gap(px(2.0))
                    .p(px(4.0))
                    .children((0..90).map(|i| {
                        let hue = (i as f32 * 37.0) % 360.0;
                        div()
                            .w(px(8.0))
                            .h(px(52.0))
                            .background(hsla(hue / 360.0, 0.8, 0.55, 1.0))
                    })),
            )
            // Bright marker at the right edge: with a wrapping sampler, blur taps leaving the
            // window on the left wrap around and smear this marker into left-edge panels.
            .child(
                div()
                    .id("edge-marker")
                    .absolute()
                    .right(px(0.0))
                    .top(px(110.0))
                    .w(px(60.0))
                    .h(px(200.0))
                    .background(rgb(0xff2222)),
            )
            .child(blur_panel("center r=16", 16.0, 330.0, 90.0, 260.0, 140.0, true))
            .child(blur_panel("left edge r=24", 24.0, 8.0, 80.0, 220.0, 110.0, true))
            .child(blur_panel("corner r=8", 8.0, 8.0, 330.0, 200.0, 100.0, true))
            .child(blur_panel("big r=48", 48.0, 260.0, 300.0, 300.0, 130.0, true))
            .child(blur_panel("right r=12", 12.0, 790.0, 290.0, 120.0, 120.0, false))
            // Content blur (`filter: blur`) over a subtree, exercising the content-filter path.
            .child(
                div()
                    .id("content-blur-group")
                    .absolute()
                    .left(px(60.0))
                    .top(px(500.0))
                    .w(px(300.0))
                    .h(px(150.0))
                    .blur(px(18.0))
                    .rounded(px(12.0))
                    .background(rgb(0x2d8f5e))
                    .flex()
                    .flex_row()
                    .gap(px(10.0))
                    .items_center()
                    .justify_center()
                    .child(div().w(px(60.0)).h(px(60.0)).background(rgb(0xe2b714)))
                    .child(div().w(px(60.0)).h(px(60.0)).rounded_full().background(rgb(0xd14d42)))
                    .child(
                        div()
                            .w(px(80.0))
                            .h(px(60.0))
                            .background(rgb(0x1e66f5))
                            .child("blur(18)"),
                    ),
            )
            // Hint line at the bottom.
            .child(
                div()
                    .id("hint")
                    .absolute()
                    .bottom(px(4.0))
                    .left(px(8.0))
                    .text_size(px(11.0))
                    .text_color(rgba(0xffffff66))
                    .child("Backdrop blur diagnostics — run with --features wgpu for the WGSL backend"),
            )
    }
}

/// A frosted panel: `backdrop_blur(radius)` over the scene painted beneath it, with a
/// translucent tint so the blur is visible, plus a label.
fn blur_panel(
    label: &'static str,
    radius: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    rounded: bool,
) -> impl IntoElement {
    let mut panel = div()
        .id(label)
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .h(px(height))
        .backdrop_blur(px(radius))
        .background(rgba(0xffffff22))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.0))
        .text_color(rgb(0xffffff));
    if rounded {
        panel = panel.rounded(px(14.0));
    }
    panel.child(label)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(920.0), px(720.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BlurTest),
        );

        example_prelude::init_example(cx, "Blur Test");
    });
}
