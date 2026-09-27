//! Radial Gradient Example
//!
//! Demonstrates the CSS-style radial gradients added to `Background`:
//! `radial_gradient(center, from, to)` plus `.radial_shape(...)`, mirroring CSS
//! `radial-gradient(<shape> farthest-corner at <center>, from, to)`.
//!
//! - `center` is a **fraction of the element's size**: `point(0.5, 0.5)` is the
//!   middle, `point(0.5, 0.0)` the top edge. Values outside `0..=1` place the
//!   center outside the element, as in CSS.
//! - The gradient reaches the element's **farthest corner** at the last color
//!   stop, so the stops' percentages are the radius control: a glow that fades
//!   out halfway ends its last stop at `0.5`.
//! - The ending shape is an **ellipse** by default (CSS's default), which the
//!   element's aspect ratio stretches; `.radial_shape(RadialShape::Circle)`
//!   gives a round glow instead.
//! - `.radial_size(...)` picks CSS's four `<ending-shape-size>` keywords:
//!   `FarthestCorner` (the default, the largest), `ClosestSide`, `FarthestSide`
//!   and `ClosestCorner`. Explicit lengths are expressed by ending the last
//!   color stop closer to the center instead.
//! - Colors interpolate in Oklab by default, like the linear gradients
//!   (`.color_space(ColorSpace::Srgb)` interpolates in sRGB instead).
//!
//! Every row shows the same stops twice — **ellipse (left) vs circle (right)** —
//! so the difference in how the ramp reaches the corners is immediately visible.
//!
//! Run it on either backend:
//!
//! ```sh
//! cargo run -p gpui-ce --example radial-gradient
//! cargo run -p gpui-ce --example radial-gradient --features wgpu
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use gpui::{
    App, AppContext, Background, Bounds, Context, IntoElement, ParentElement, RadialShape,
    RadialSize, Render, Styled, Window, WindowBounds, WindowOptions, div, linear_color_stop,
    linear_gradient, point, px, radial_gradient, rgb, rgba, size,
};

struct RadialGradientExample;

impl Render for RadialGradientExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(28.0))
            .background(rgb(0x14141a))
            .child(
                div()
                    .text_size(px(18.0))
                    .text_color(rgb(0xffffff))
                    .child("Radial gradients — ellipse (left) vs circle (right)"),
            )
            .child(swatch_row(
                "ellipse at center",
                "circle at center",
                |shape| {
                    radial_gradient(
                        point(0.5, 0.5),
                        linear_color_stop(rgba(0x60a5facc), 0.0),
                        linear_color_stop(rgba(0x60a5fa00), 0.6),
                    )
                    .radial_shape(shape)
                },
            ))
            .child(swatch_row(
                "ellipse at 50% 0%",
                "circle at 50% 0%",
                |shape| {
                    radial_gradient(
                        point(0.5, 0.0),
                        linear_color_stop(rgba(0xfacc15cc), 0.0),
                        linear_color_stop(rgba(0xfacc1500), 0.7),
                    )
                    .radial_shape(shape)
                },
            ))
            .child(swatch_row(
                "ellipse at 20% 80%",
                "circle at 20% 80%",
                |shape| {
                    radial_gradient(
                        point(0.2, 0.8),
                        linear_color_stop(rgba(0x34d399cc), 0.0),
                        linear_color_stop(rgba(0x34d39900), 0.5),
                    )
                    .radial_shape(shape)
                },
            ))
            .child(
                // Text and children paint on top of the gradient, as with any other background.
                div()
                    .w(px(500.0))
                    .h(px(96.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(rgb(0xffffff22))
                    .flex()
                    .items_center()
                    .justify_center()
                    .background(
                        radial_gradient(
                            point(0.5, 0.5),
                            linear_color_stop(rgba(0xd14d42cc), 0.0),
                            linear_color_stop(rgba(0x8b5cf600), 1.0),
                        )
                        .radial_shape(RadialShape::Circle),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(0xffffffee))
                            .child("children & text paint over the gradient"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(14.0))
                    .items_start()
                    .child(value_ramp_card("radial gray25 -> white (Oklab)", true))
                    .child(value_ramp_card("radial gray25 -> white (linear ref)", false))
                    .child(hard_stop_card()),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(0xffffff))
                    .child("ending-shape size keywords (at 25% 50%, so the four differ):"),
            )
            .child(size_keyword_row())
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0xffffff99))
                    .child(
                        "stops are the radius control: 0% = center, 100% = farthest corner \
                         — same ramp as linear gradients, three backends share one implementation",
                    ),
            )
    }
}

/// Two swatches with the same stops and center, one elliptical and one circular, built
/// by the same closure so only the ending shape differs.
fn swatch_row(
    ellipse_label: &'static str,
    circle_label: &'static str,
    make: impl Fn(RadialShape) -> Background,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(px(14.0))
        .items_start()
        .child(swatch(ellipse_label, make(RadialShape::Ellipse)))
        .child(swatch(circle_label, make(RadialShape::Circle)))
}

fn swatch(label: &'static str, background: Background) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .w(px(170.0))
                .h(px(84.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(rgb(0xffffff22))
                .background(background),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(rgb(0xffffff99))
                .child(label),
        )
}

/// The grey ramp the offscreen readback tests assert on, next to a linear gradient with the
/// same stops: the radial one is brightest where it meets the corners.
fn value_ramp_card(label: &'static str, radial: bool) -> impl IntoElement {
    let gray25 = linear_color_stop(rgba(0x404040ff), 0.0);
    let white = linear_color_stop(rgba(0xffffffff), 1.0);
    let background = if radial {
        radial_gradient(point(0.5, 0.5), gray25, white).radial_shape(RadialShape::Circle)
    } else {
        linear_gradient(90.0, gray25, white)
    };

    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .w(px(170.0))
                .h(px(84.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(rgb(0xffffff22))
                .background(background),
        )
        .child(
            div()
                .w(px(170.0))
                .text_size(px(11.0))
                .text_color(rgb(0xffffff99))
                .child(label),
        )
}

/// The four CSS `<ending-shape-size>` keywords on an off-center gradient, where they differ
/// visibly: `farthest-corner` is the largest, `closest-side` the smallest.
fn size_keyword_row() -> impl IntoElement {
    let from = linear_color_stop(rgba(0x38bdf8cc), 0.0);
    let to = linear_color_stop(rgba(0x38bdf800), 1.0);
    let card = |label: &'static str, size: RadialSize| {
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .w(px(150.0))
                    .h(px(84.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(rgb(0xffffff22))
                    .background(radial_gradient(point(0.25, 0.5), from, to).radial_size(size)),
            )
            .child(
                div()
                    .w(px(150.0))
                    .text_size(px(11.0))
                    .text_color(rgb(0xffffff99))
                    .child(label),
            )
    };

    div()
        .flex()
        .flex_row()
        .gap(px(14.0))
        .items_start()
        .child(card("farthest-corner (default)", RadialSize::FarthestCorner))
        .child(card("farthest-side", RadialSize::FarthestSide))
        .child(card("closest-corner", RadialSize::ClosestCorner))
        .child(card("closest-side", RadialSize::ClosestSide))
}

/// CSS `radial-gradient(farthest-side at 50% 50%, white 0%, white 50%, transparent)`.
///
/// The first two stops share a color, which is exactly what clamping to the first stop does, so
/// the two-stop form below renders the same thing: a solid disc out to half the farthest side,
/// then a linear fade to the edge.
fn hard_stop_card() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .w(px(170.0))
                .h(px(84.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(rgb(0xffffff22))
                .background(rgb(0x0b0b10))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(72.0))
                        .h(px(72.0))
                        .background(
                            radial_gradient(
                                point(0.5, 0.5),
                                linear_color_stop(rgba(0xffffffff), 0.5),
                                linear_color_stop(rgba(0xffffff00), 1.0),
                            )
                            .radial_size(RadialSize::FarthestSide),
                        ),
                ),
        )
        .child(
            div()
                .w(px(170.0))
                .text_size(px(11.0))
                .text_color(rgb(0xffffff99))
                .child("hard stop: white 0%, white 50%, transparent"),
        )
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.0), px(880.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RadialGradientExample),
        );

        example_prelude::init_example(cx, "Radial Gradient Example");
    });
}
