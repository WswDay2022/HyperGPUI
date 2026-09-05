//! CSS Transform Example
//!
//! Demonstrates the CSS-style `transform` support added to `Styled`: `translate`,
//! `scale`, `rotate`, `skew` and `matrix`, composed left-to-right like CSS.
//! Layout and hit-testing keep the element's untransformed bounds (CSS semantics),
//! while the transform applies at paint time to the element's **whole subtree**:
//! its background and border, its text glyphs, emoji, underline/strikethrough
//! rules, and its child elements — whose own transforms compose with the
//! ancestor's (`rotate` a card containing a `translate_x` child and the child
//! shifts along the card's rotated axis).
//!
//! Every row pairs a **reference card** (untransformed, left) with a **transformed
//! card** (same size, right) so the effect is immediately visible. Hover the cards:
//! the hit boxes stay in the original (untransformed) position, exactly like CSS.
//!
//! Run it on either backend:
//!
//! ```sh
//! cargo run -p gpui-ce --example css-transform
//! cargo run -p gpui-ce --example css-transform --features wgpu
//! ```

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::f32::consts::FRAC_PI_8;

use gpui::{
    App, AppContext, Bounds, Context, CssTransform, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Window, WindowBounds, WindowOptions, div, px, radians, rgb,
    size,
};

struct CssTransformExample;

impl Render for CssTransformExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("transform-root")
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
                    .child("CSS transforms — reference (left) vs transformed (right)"),
            )
            .child(transform_row("translateX(40px)", rgb(0x3b82f6), CssTransform::identity().translate_x(px(40.0))))
            .child(transform_row("translateY(40px)", rgb(0xe2b714), CssTransform::identity().translate_y(px(40.0))))
            .child(transform_row("translate(-30, 20)", rgb(0xd14d42), CssTransform::identity().translate(px(-30.0), px(20.0))))
            .child(transform_row("scale(1.5)", rgb(0x2d8f5e), CssTransform::identity().scale(1.5, 1.5)))
            .child(transform_row("scaleX(0.7)", rgb(0x8b5cf6), CssTransform::identity().scale_x(0.7)))
            .child(transform_row("rotate(22.5°)", rgb(0xec6d3d), CssTransform::identity().rotate(radians(FRAC_PI_8))))
            .child(transform_row("skewX(15°)", rgb(0x0e9f9f), CssTransform::identity().skew_x(radians(FRAC_PI_8))))
            .child(transform_row("matrix(1,0,0.3,1,0,0)", rgb(0x9b5de5), CssTransform::identity().matrix(1.0, 0.0, 0.3, 1.0, px(0.0), px(0.0))))
            .child(subtree_row(
                "rotate(22.5°): text, emoji, underline & a nested child follow",
                rgb(0x5d3fd3),
                CssTransform::identity().rotate(radians(FRAC_PI_8)),
            ))
            .child(subtree_row(
                "skewX(15°): the same subtree under shear",
                rgb(0x0e7490),
                CssTransform::identity().skew_x(radians(FRAC_PI_8)),
            ))
    }
}

/// A row exercising "subtree follow": on the right, the *whole* painted subtree —
/// glyphs, emoji, underline/strikethrough rules and a child element with its own
/// transform — moves with the card's outer transform. In the transformed card the
/// nested `translate_x` child shifts along the card's rotated/sheared axis, proving
/// the inner transform composes with the outer one.
fn subtree_row(label: &'static str, color: gpui::Rgba, transform: CssTransform) -> impl IntoElement {
    div()
        .id(label)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(14.0))
        .child(content_card(color, None))
        .child(
            div()
                .w(px(150.0))
                .text_size(px(11.0))
                .text_color(rgb(0xffffff99))
                .child(label),
        )
        .child(content_card(color, Some(transform)))
}

/// A content card: colored background + border, a text line mixing CJK glyphs and an
/// emoji, an underlined / struck-through word line, and a nested child element that
/// carries its own transform. Pass `None` for the plain reference card.
fn content_card(color: gpui::Rgba, transform: Option<CssTransform>) -> impl IntoElement {
    let card = div()
        .w(px(190.0))
        .h(px(96.0))
        .rounded(px(10.0))
        .p(px(8.0))
        .background(color)
        .border_1()
        .border_color(rgb(0xffffff33))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(0xffffff))
                .child("Text 字形 follows 🚀"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(10.0))
                .text_size(px(13.0))
                .child(div().underline().text_color(rgb(0xffffff)).child("underlined"))
                .child(
                    div()
                        .line_through()
                        .text_color(rgb(0xffffffaa))
                        .child("struck"),
                ),
        )
        .child(
            div()
                .mt(px(2.0))
                .w(px(64.0))
                .h(px(16.0))
                .rounded(px(5.0))
                .background(rgb(0x00000033))
                .flex()
                .items_center()
                .justify_center()
                .transform(CssTransform::identity().translate_x(px(8.0)))
                .child(div().text_size(px(10.0)).text_color(rgb(0xffffff)).child("nested ↷")),
        );

    match transform {
        Some(transform) => card.transform(transform),
        None => card,
    }
}

/// A row: an untransformed reference card, an arrow label, and the transformed card.
fn transform_row(label: &'static str, color: gpui::Rgba, transform: CssTransform) -> impl IntoElement {
    div()
        .id(label)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(14.0))
        .child(reference_card(color))
        .child(
            div()
                .w(px(150.0))
                .text_size(px(11.0))
                .text_color(rgb(0xffffff99))
                .child(label),
        )
        .child(transformed_card(color, transform))
}

/// An untransformed card of the standard size and shape.
fn reference_card(color: gpui::Rgba) -> impl IntoElement {
    div()
        .w(px(140.0))
        .h(px(70.0))
        .rounded(px(10.0))
        .background(color)
        .border_1()
        .border_color(rgb(0xffffff33))
}

/// The same card with a CSS-style transform applied.
fn transformed_card(color: gpui::Rgba, transform: CssTransform) -> impl IntoElement {
    div()
        .w(px(140.0))
        .h(px(70.0))
        .rounded(px(10.0))
        .background(color)
        .border_1()
        .border_color(rgb(0xffffff33))
        .cursor(gpui::CursorStyle::PointingHand)
        .transform(transform)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(720.0), px(1020.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| CssTransformExample),
        );

        example_prelude::init_example(cx, "CSS Transform Example");
    });
}
