//! Borderless Resizable Window Example
//!
//! Demonstrates a frameless window with a custom-drawn shadow that can still be resized and moved
//! using the system window manager. This is the pattern to use when an application draws its own
//! shadow (or other custom chrome) and therefore cannot rely on the native window frame.
//!
//! The window is created with `WindowDecorations::Client` and a transparent background. Invisible
//! hit-box elements along every edge and corner call `Window::start_window_resize`, which hands the
//! press back to the platform so the standard resize loop (with live preview and snap layouts) runs
//! as usual. The title bar uses a drag region for moving the window.
//!
//! Try dragging any edge or corner of the window: the cursor changes and the window resizes with
//! the normal system behavior.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use gpui::{
    App, Bounds, Context, CursorStyle, MouseButton, ResizeEdge, SharedString, TitlebarOptions,
    Window, WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowDecorations,
    WindowOptions, div, hsla, prelude::*, px, rgb, size, transparent_black,
};

/// Size of the custom shadow around the window content.
const SHADOW_SIZE: f32 = 12.0;
/// Thickness of the resize hit-boxes along the edges.
const EDGE_SIZE: f32 = 6.0;
/// Size of the resize hit-boxes in the corners.
const CORNER_SIZE: f32 = 12.0;

struct BorderlessResizableWindow;

impl Render for BorderlessResizableWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Make room for the shadow around the window content.
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
                            .bg(rgb(0x1e1e2e))
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
                                    .h(px(36.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .flex_shrink_0()
                                    .window_control_area(WindowControlArea::Drag)
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(rgb(0xcdd6f4))
                                            .child("Drag the title bar to move, edges to resize"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(14.0))
                                    .text_color(rgb(0x89b4fa))
                                    .child("Borderless · Resizable"),
                            ),
                    ),
            )
            .children(RESIZE_EDGES.map(resize_handle))
    }
}

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
        let bounds = Bounds::centered(None, size(px(480.0), px(360.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Borderless Resizable Window")),
                    appears_transparent: false,
                    ..Default::default()
                }),
                window_background: WindowBackgroundAppearance::Transparent,
                is_resizable: true,
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BorderlessResizableWindow),
        );

        example_prelude::init_example(cx, "Borderless Resizable Window");
    });
}
