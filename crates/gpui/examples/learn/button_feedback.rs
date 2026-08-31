//! Button Feedback Test (borderless)
//!
//! The production press-hold issue was seen in a fully borderless (`WS_POPUP`) window, so this
//! example renders the same gradient button inside a frameless window with a custom-drawn
//! titlebar and shadow. Every mouse handler prints a timestamped log line:
//!
//! - **click** (quick press + release) logs normally.
//! - **press and hold** should log nothing between `MOUSE_DOWN` and `MOUSE_UP`.
//!   In production, a log line (and a cursor flicker) appeared ~1-2 s into the hold; this test
//!   identifies which handler fires late, should it ever reproduce.
//!
//! Run it and hold the button; watch the console for the first log after `MOUSE_DOWN`.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, CursorStyle, Hsla, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ResizeEdge, Rgba, SharedString,
    StatefulInteractiveElement, Styled, TitlebarOptions, Transition, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowDecorations, WindowOptions,
    div, hsla, linear_color_stop, linear_gradient, px, rgb, rgba, size, transparent_black,
};
use palette::IntoColor as _;

/// Size of the custom shadow around the window content.
const SHADOW_SIZE: f32 = 12.0;
/// Height of the custom title bar.
const TITLEBAR_HEIGHT: f32 = 36.0;
/// Thickness of the resize hit-boxes along the edges.
const EDGE_SIZE: f32 = 6.0;
/// Size of the resize hit-boxes in the corners.
const CORNER_SIZE: f32 = 12.0;

/// All eight resize edges and corners, paired with the cursor shown while hovering them.
const RESIZE_EDGES: [(ResizeEdge, CursorStyle); 8] = [
    (ResizeEdge::Top, CursorStyle::ResizeUpDown),
    (ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft),
    (ResizeEdge::Right, CursorStyle::ResizeLeftRight),
    (ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight),
    (ResizeEdge::Bottom, CursorStyle::ResizeUpDown),
    (ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft),
    (ResizeEdge::Left, CursorStyle::ResizeLeftRight),
    (ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight),
];

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn elapsed_ms() -> u128 {
    let start = *START.get_or_init(Instant::now);
    Instant::now().duration_since(start).as_millis()
}

fn log(label: &str) {
    println!("[{:>7} ms] {label}", elapsed_ms());
}

struct ButtonFeedback {
    hovered: bool,
    pressed: bool,
}

impl Render for ButtonFeedback {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = rgb(0x3b82f6);
        let hover = rgb(0x2f6fe0); // slightly darker on hover
        let pressed = rgb(0x1e4fae); // darker still while pressed

        // Smooth 150 ms color transition. The production issue reproduces with and without
        // this animation, so it is kept here only to mirror the production button.
        let color: Transition<Rgba> = window.use_keyed_transition(
            "button-color",
            cx,
            Duration::from_millis(150),
            |_, _| {
                if self.pressed {
                    pressed
                } else if self.hovered {
                    hover
                } else {
                    base
                }
            },
        );
        color.update(cx, |goal: &mut Rgba, _cx| {
            *goal = if self.pressed {
                pressed
            } else if self.hovered {
                hover
            } else {
                base
            };
        });
        let color = *color.evaluate(window, cx);
        // Derive a darker gradient stop by converting through HSL.
        let mut color_dark: Hsla = color.into_color();
        color_dark.color.lightness = (color_dark.color.lightness * 0.85).clamp(0.0, 1.0);
        let color_dark: Rgba = color_dark.into_color();

        let mut button = div()
            .id("feedback-button")
            .w(px(240.0))
            .h(px(88.0))
            .rounded(px(16.0))
            .cursor(CursorStyle::PointingHand)
            .bg(linear_gradient(
                180.0,
                linear_color_stop(color, 0.0),
                linear_color_stop(color_dark, 1.0),
            ))
            // Event stream logs (timestamped).
            .on_mouse_down(MouseButton::Left, |_event, _window, _cx| {
                log("MOUSE_DOWN");
            })
            .on_mouse_up(MouseButton::Left, |_event, _window, _cx| {
                log("MOUSE_UP");
            })
            .on_click(|_event, _window, _cx| {
                log("CLICK");
            })
            .on_hover(|hovered, _window, _cx| {
                log(if *hovered { "HOVER_IN" } else { "HOVER_OUT" });
            })
            // State driving the visual feedback.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    log("STATE_PRESSED");
                    this.pressed = true;
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    log("STATE_RELEASED");
                    this.pressed = false;
                    cx.notify();
                }),
            )
            .on_hover(cx.listener(|this, hovered, _window, cx| {
                this.hovered = *hovered;
                // Dragging away while pressed cancels the press (browser-like), so the button
                // stops responding to the held press as soon as the pointer leaves it.
                if !*hovered && this.pressed {
                    this.pressed = false;
                }
                cx.notify();
            }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(16.0))
            .text_color(rgb(0xffffff))
            .child("HOLD ME");
        // `on_any_mouse_up` is only available through the imperative `Interactivity` API.
        button.interactivity().on_any_mouse_up(|_event, _window, _cx| {
            log("ANY_MOUSE_UP");
        });

        // Make room for the custom shadow around the window content.
        window.set_client_inset(px(SHADOW_SIZE));

        div()
            .id("borderless-root")
            .relative()
            .size_full()
            .bg(transparent_black())
            .child(
                div()
                    .id("borderless-content")
                    .size_full()
                    .p(px(SHADOW_SIZE))
                    .child(
                        div()
                            .id("borderless-window")
                            .size_full()
                            .rounded(px(10.0))
                            .bg(rgb(0x0f0f14))
                            .border_1()
                            .border_color(rgb(0x3b82f6))
                            .shadow(vec![
                                gpui::BoxShadow::new(px(0.0), px(0.0), hsla(0.0, 0.0, 0.0, 0.5))
                                    .blur_radius(px(SHADOW_SIZE)),
                            ])
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id("borderless-titlebar")
                                    .w_full()
                                    .h(px(TITLEBAR_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .px(px(12.0))
                                    .flex_shrink_0()
                                    .window_control_area(WindowControlArea::Drag)
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(13.0))
                                            .text_color(rgb(0xcdd6f4))
                                            .child("Button Feedback Test"),
                                    )
                                    .child(
                                        div()
                                            .id("close-btn")
                                            .w(px(TITLEBAR_HEIGHT))
                                            .h(px(TITLEBAR_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .text_color(rgb(0xffffff44))
                                            .text_size(px(14.0))
                                            .child("✕")
                                            .window_control_area(WindowControlArea::Close)
                                            .on_click(|_event, window, _cx| {
                                                window.remove_window();
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .justify_center()
                                    .gap(px(24.0))
                                    .child(button)
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgba(0xffffff66))
                                            .child(
                                                "Click (press+release) logs normally; press-and-hold \
                                                 should log nothing between MOUSE_DOWN and MOUSE_UP",
                                            ),
                                    ),
                            ),
                    ),
            )
            // Invisible resize hit-boxes along every edge and corner, so the frameless window
            // can be resized with the system sizing loop.
            .children(RESIZE_EDGES.map(resize_handle))
    }
}

/// An invisible hit-box that starts the system resize loop for the given edge when pressed.
fn resize_handle((edge, cursor): (ResizeEdge, CursorStyle)) -> impl IntoElement {
    let base = div()
        .absolute()
        .cursor(cursor)
        .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
            window.start_window_resize(edge);
        });

    match edge {
        ResizeEdge::Top => base
            .id("resize-top")
            .top(px(0.0))
            .left(px(CORNER_SIZE))
            .right(px(CORNER_SIZE))
            .h(px(EDGE_SIZE)),
        ResizeEdge::TopRight => base
            .id("resize-top-right")
            .top(px(0.0))
            .right(px(0.0))
            .w(px(CORNER_SIZE))
            .h(px(CORNER_SIZE)),
        ResizeEdge::Right => base
            .id("resize-right")
            .right(px(0.0))
            .top(px(CORNER_SIZE))
            .bottom(px(CORNER_SIZE))
            .w(px(EDGE_SIZE)),
        ResizeEdge::BottomRight => base
            .id("resize-bottom-right")
            .bottom(px(0.0))
            .right(px(0.0))
            .w(px(CORNER_SIZE))
            .h(px(CORNER_SIZE)),
        ResizeEdge::Bottom => base
            .id("resize-bottom")
            .bottom(px(0.0))
            .left(px(CORNER_SIZE))
            .right(px(CORNER_SIZE))
            .h(px(EDGE_SIZE)),
        ResizeEdge::BottomLeft => base
            .id("resize-bottom-left")
            .bottom(px(0.0))
            .left(px(0.0))
            .w(px(CORNER_SIZE))
            .h(px(CORNER_SIZE)),
        ResizeEdge::Left => base
            .id("resize-left")
            .left(px(0.0))
            .top(px(CORNER_SIZE))
            .bottom(px(CORNER_SIZE))
            .w(px(EDGE_SIZE)),
        ResizeEdge::TopLeft => base
            .id("resize-top-left")
            .top(px(0.0))
            .left(px(0.0))
            .w(px(CORNER_SIZE))
            .h(px(CORNER_SIZE)),
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.0), px(400.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Button Feedback Test")),
                    appears_transparent: false,
                    ..Default::default()
                }),
                window_background: WindowBackgroundAppearance::Transparent,
                is_resizable: true,
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| ButtonFeedback {
                    hovered: false,
                    pressed: false,
                })
            },
        );

        example_prelude::init_example(cx, "Button Feedback Test");
    });
}
