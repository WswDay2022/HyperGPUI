//! CSS-style color filter diagnostic scene.
//!
//! Every non-spatial CSS filter function is exercised twice: once as a content filter
//! (`filter`, the element and its children) and once as a backdrop filter (`backdrop-filter`,
//! the content painted behind the element). The background strip is deliberately saturated so
//! a wrong matrix (swapped luma weights, missing offset, double-applied alpha) is visible.
//!
//! ```sh
//! cargo run -p hgpui --example color-filters              # DirectX/HLSL backend
//! cargo run -p hgpui --example color-filters --features wgpu   # wgpu/WGSL backend
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use hgpui::{
    App, Bounds, Context, Filter, Render, Window, WindowBounds, WindowOptions, div, hsla,
    prelude::*, px, rgb, rgba, size,
};

struct ColorFilters;

impl Render for ColorFilters {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let filters = [
            ("grayscale(1)", Filter::Grayscale(1.0)),
            ("sepia(1)", Filter::Sepia(1.0)),
            ("saturate(0.2)", Filter::Saturate(0.2)),
            ("brightness(1.6)", Filter::Brightness(1.6)),
            ("contrast(2)", Filter::Contrast(2.0)),
            ("invert(1)", Filter::Invert(1.0)),
            ("hue-rotate(90)", Filter::HueRotate(90.0)),
            ("opacity(0.5)", Filter::Opacity(0.5)),
        ];

        div()
            .id("color-filter-root")
            .relative()
            .size_full()
            .background(rgb(0x14141a))
            // Saturated background strip shared by the backdrop-filter row below.
            .child(
                div()
                    .id("color-strip")
                    .absolute()
                    .top(px(300.0))
                    .left(px(20.0))
                    .right(px(20.0))
                    .h(px(170.0))
                    .flex()
                    .flex_row()
                    .gap(px(4.0))
                    .children((0..42).map(|i| {
                        let hue = (i as f32 * 29.0) % 360.0;
                        div()
                            .flex_1()
                            .h_full()
                            .background(hsla(hue / 360.0, 0.9, 0.55, 1.0))
                    })),
            )
            // Content filters: each cell filters its own saturated child.
            .child(
                div()
                    .id("content-row")
                    .absolute()
                    .top(px(20.0))
                    .left(px(20.0))
                    .right(px(20.0))
                    .flex()
                    .flex_row()
                    .gap(px(4.0))
                    .children(filters.iter().map(|(label, filter)| {
                        div()
                            .flex_1()
                            .h(px(120.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .w_full()
                                    .h(px(50.0))
                                    .rounded(px(8.0))
                                    .filter(vec![*filter])
                                    .background(rgb(0x2d8f5e)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgba(0xffffffcc))
                                    .child(*label),
                            )
                    })),
            )
            // Backdrop filters over the background strip.
            .child(
                div()
                    .id("backdrop-row")
                    .absolute()
                    .top(px(330.0))
                    .left(px(40.0))
                    .right(px(40.0))
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .children(filters.iter().map(|(label, filter)| {
                        div()
                            .flex_1()
                            .h(px(110.0))
                            .rounded(px(10.0))
                            .backdrop_filter(vec![*filter])
                            .background(rgba(0x00000000))
                            .flex()
                            .items_end()
                            .justify_center()
                            .pb(px(4.0))
                            .text_size(px(10.0))
                            .text_color(rgb(0xffffff))
                            .child(*label)
                    })),
            )
            .child(
                div()
                    .id("hint")
                    .absolute()
                    .bottom(px(6.0))
                    .left(px(8.0))
                    .text_size(px(11.0))
                    .text_color(rgba(0xffffff66))
                    .child("Top: filter (element). Bottom: backdrop-filter (painted behind)."),
            )
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(980.0), px(560.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ColorFilters),
        );

        example_prelude::init_example(cx, "Color Filters");
    });
}
