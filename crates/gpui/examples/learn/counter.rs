//! Counter Example
//!
//! A clickable counter with a click-feedback ripple animation. This example walks through all
//! three layers of GPUI:
//!
//! 1. **Entities**: the `Counter` view stores application state (`value`, `ripple_t`) and
//!    communicates through `cx.notify()`.
//! 2. **Views**: `Render` builds a styled element tree with `div()`, actions, key bindings,
//!    focus, and a custom titlebar with a drag region.
//! 3. **Elements**: `RippleElement` implements `Element` directly, painting an expanding,
//!    fading ring with `PathBuilder` and driving the animation with `window.on_next_frame`.
//!
//! Controls:
//! - `space` or click the button: increment the counter
//! - `q`: quit
//! - `ctrl-i`: toggle the built-in inspector (debug builds)

#[path = "../shared/prelude.rs"]
mod example_prelude;

use std::f32::consts::TAU;

use gpui::{
    App, Bounds, Context, Element, ElementId, FocusHandle, FontWeight, GlobalElementId,
    InteractiveElement, IntoElement, KeyBinding, LayoutId, ParentElement, PathBuilder, Point,
    Render, SharedString, Size, Style, Styled, TitlebarOptions, Window, WindowBounds,
    WindowControlArea, WindowDecorations, WindowOptions, actions, div, hsla, point, prelude::*,
    px, relative, rgb, rgba, size,
};

actions!(Counter, [Increment]);
actions!(app, [ToggleInspector]);

const RIPPLE_DURATION_FRAMES: f32 = 22.0;
const TITLEBAR_HEIGHT: f32 = 36.0;

// ── RippleElement ─────────────────────────────────────────────────────────────

/// A custom element that paints an expanding, fading ring. The `t` field holds the
/// animation progress in the range `0.0..=1.0`.
struct RippleElement {
    t: f32,
}

impl Element for RippleElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Fill the bounds of the parent element.
        let mut style = Style::default();
        style.size = Size {
            width: relative(1.0).into(),
            height: relative(1.0).into(),
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if self.t <= 0.0 {
            return;
        }

        let t = self.t;
        let scale = 1.0 - (1.0 - t).powi(3);
        let alpha = (1.0 - t).powf(1.8);

        let center: Point<gpui::Pixels> = bounds.center();
        let max_r = bounds.size.width.min(bounds.size.height) / 2.0;
        let radius = max_r * scale;
        let stroke_width = px(max_r.as_f32() * 0.28 * (1.0 - t * 0.7));

        let segments = 128usize;
        let mut builder = PathBuilder::stroke(stroke_width);
        builder.move_to(point(center.x + radius, center.y));
        for i in 1..=segments {
            let angle = (i as f32 / segments as f32) * TAU;
            builder.line_to(point(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }

        if let Ok(path) = builder.build() {
            window.paint_path(path, hsla(0.6, 0.9, 0.88, alpha));
        }
    }
}

impl IntoElement for RippleElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

// ── Counter ───────────────────────────────────────────────────────────────────

/// The counter view: an entity that renders the counter and drives the ripple animation.
struct Counter {
    value: i32,
    focus_handle: FocusHandle,
    ripple_t: f32,
    ripple_active: bool,
}

impl Counter {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            value: 0,
            focus_handle,
            ripple_t: 0.0,
            ripple_active: false,
        }
    }

    fn increment(&mut self, _: &Increment, _window: &mut Window, cx: &mut Context<Self>) {
        self.value += 1;
        self.ripple_t = 0.001;
        self.ripple_active = true;
        cx.notify();
    }

    fn advance_ripple(&mut self, cx: &mut Context<Self>) {
        if !self.ripple_active {
            return;
        }
        self.ripple_t += 1.0 / RIPPLE_DURATION_FRAMES;
        if self.ripple_t >= 1.0 {
            self.ripple_t = 0.0;
            self.ripple_active = false;
        }
        cx.notify();
    }
}

impl Render for Counter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.ripple_active {
            let entity = cx.entity();
            window.on_next_frame(move |_window, cx| {
                entity.update(cx, |this, cx| this.advance_ripple(cx));
            });
        }

        let ripple_t = self.ripple_t;
        let value = self.value;
        let button_size = px(160.0);

        let root = div()
            .id("counter-root")
            .key_context("Counter")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Counter::increment))
            .on_action(|_: &example_prelude::Quit, _window, cx| cx.quit());

        // The built-in inspector is only compiled into debug builds.
        #[cfg(any(feature = "inspector", debug_assertions))]
        let root = root.on_action(|_: &ToggleInspector, window, cx| window.toggle_inspector(cx));

        root
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0f0f0f))
            // Titlebar: a drag region for the window, with a close button.
            .child(
                div()
                    .id("titlebar")
                    .w_full()
                    .h(px(TITLEBAR_HEIGHT))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_center()
                            .text_color(rgba(0xffffff55))
                            .text_size(px(13.0))
                            .child("Counter"),
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
                            .text_color(rgba(0xffffff33))
                            .text_size(px(14.0))
                            .child("✕")
                            .window_control_area(WindowControlArea::Close)
                            .on_click(|_event, window, _cx| {
                                window.remove_window();
                            }),
                    ),
            )
            // Counter content: the ripple button and keyboard hints.
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(24.0))
                    .child(
                        div()
                            .id("ripple-button")
                            .w(button_size)
                            .h(button_size)
                            .rounded_full()
                            .border(px(2.0))
                            .border_color(rgb(0x3b82f6))
                            .bg(rgb(0x1d4ed8))
                            .cursor_pointer()
                            .overflow_hidden()
                            .relative()
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.increment(&Increment, window, cx);
                            }))
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .child(RippleElement { t: ripple_t }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(0xffffff))
                                    .text_size(px(48.0))
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!("{value}")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(kbd_hint("space", "increment"))
                            .child(
                                div()
                                    .text_color(rgba(0xffffff18))
                                    .text_size(px(11.0))
                                    .child("·"),
                            )
                            .child(kbd_hint("q", "quit")),
                    ),
            )
    }
}

/// A small key-cap hint, e.g. `space · increment`.
fn kbd_hint(key: &'static str, label: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .border(px(1.0))
                .border_color(rgba(0xffffff18))
                .bg(rgba(0xffffff0a))
                .text_color(rgba(0xffffff55))
                .text_size(px(11.0))
                .child(key),
        )
        .child(
            div()
                .text_color(rgba(0xffffff28))
                .text_size(px(11.0))
                .child(label),
        )
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("space", Increment, Some("Counter")),
            KeyBinding::new("q", example_prelude::Quit, Some("Counter")),
            KeyBinding::new("ctrl-i", ToggleInspector, None),
        ]);

        let bounds = Bounds::centered(None, size(px(360.0), px(420.0)), cx);

        let _ = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Counter")),
                    appears_transparent: false,
                    ..Default::default()
                }),
                window_background: gpui::WindowBackgroundAppearance::Opaque,
                is_resizable: false,
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Counter::new(window, cx)),
        );

        example_prelude::init_example(cx, "Counter");
    });
}
