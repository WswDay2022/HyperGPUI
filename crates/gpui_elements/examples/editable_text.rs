//! Editable text example.
//!
//! Shows the two editable text elements and the state behind them:
//!
//! - `text_input(..)` for a single line and `text_area(..)` for wrapped, scrollable
//!   multi-line text, both styled with the ordinary `Styled` API.
//! - Binding an element to a state the view owns, with `.state(state.downgrade())`, so the
//!   application can read the value, write to it, and focus it from outside.
//! - `EventEmitter<TextChanged>` as the way to re-render when the text changes (the state
//!   keeps the only copy — nothing snapshots the text on the render path).
//! - Caret styling: shape (`bar` / `block` / `underscore`, mirroring CSS `caret-shape`),
//!   thickness, corner radius, height ratio, color and blink interval.
//!
//! Type into either field: navigation, selection, IME, cut/copy/paste and undo/redo all
//! work through the key bindings installed in `main` (`default_bindings`).
//!
//! ```sh
//! cargo run -p hgpui_elements --example editable_text
//! ```

use hgpui::{
    App, AppContext as _, Bounds, Context, Entity, EntityInputHandler as _, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, rgb, size,
};
use hgpui_elements::editable_text::{
    CaretShape, EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_area, text_input,
};

struct Example {
    /// Single-line field, owned here rather than by the element so the buttons can drive it.
    name: Entity<EditableTextState>,
    /// Multi-line field.
    notes: Entity<EditableTextState>,
}

impl Example {
    fn new(cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let notes = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));

        // Re-render whenever either field changes, so the readout below stays current.
        cx.subscribe(&name, |_, _, _: &TextChanged, cx| cx.notify())
            .detach();
        cx.subscribe(&notes, |_, _, _: &TextChanged, cx| cx.notify())
            .detach();

        Self { name, notes }
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name_value = self.name.read(cx).as_str().to_string();
        let notes_len = self.notes.read(cx).as_str().chars().count();

        // Small labelled fields used to compare caret shapes. They keep their own state
        // (no `.state(..)`), since the app never reads their contents.
        let caret_field = |id: &'static str, placeholder: &'static str| {
            text_input(id)
                .placeholder(placeholder)
                .caret_blink_interval_500ms()
                .caret_color(hgpui::hsla(0.133, 0.845, 0.531, 1.0)) // #facc15
                .border_1()
                .border_color(rgb(0x3a3a40))
                .rounded_lg()
                .p_2()
                .w_24()
                .whitespace_nowrap()
        };

        div()
            .id("editable-text-example")
            .size_full()
            .background(rgb(0x1b1b1f))
            .text_color(rgb(0xe6e6e6))
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xl().child("Editable text"))
            .child(div().text_sm().text_color(rgb(0x9a9a9a)).child(
                "Type, select, undo — the field handles it. The buttons drive the same state \
                 the element edits.",
            ))
            .child(
                text_input("name-input")
                    .state(self.name.downgrade())
                    .placeholder("Name")
                    .caret_blink_interval_500ms()
                    // The colour hooks take `Hsla`; the hex values are in the comments.
                    .placeholder_color(hgpui::hsla(0.0, 0.0, 0.42, 1.0)) // #6b6b6b
                    .caret_color(hgpui::hsla(0.133, 0.845, 0.531, 1.0)) // #facc15
                    .selection_color(hgpui::hsla(0.13, 0.9, 0.6, 0.35))
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .rounded_lg()
                    .p_2()
                    .w_full()
                    .whitespace_nowrap(),
            )
            .child(
                text_area("notes-input")
                    .state(self.notes.downgrade())
                    .placeholder("Notes — multi-line, wraps, scrolls")
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .rounded_lg()
                    .p_2()
                    .w_full()
                    .min_h_24()
                    .max_h_32()
                    .overflow_y_scroll(),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    // Appending goes through the same input handler the platform uses, so it
                    // lands at the caret and is undoable.
                    .child(
                        div()
                            .id("append")
                            .px_3()
                            .py_1()
                            .rounded_lg()
                            .background(rgb(0x2f2f36))
                            .hover(|style| style.background(rgb(0x3d3d46)))
                            .child("Append to name")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.name.update(cx, |state, cx| {
                                    state.replace_text_in_range(None, "-postfix", window, cx);
                                });
                            })),
                    )
                    .child(
                        div()
                            .id("clear")
                            .px_3()
                            .py_1()
                            .rounded_lg()
                            .background(rgb(0x2f2f36))
                            .hover(|style| style.background(rgb(0x3d3d46)))
                            .child("Clear notes")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notes.update(cx, |state, cx| state.emplace("", cx));
                            })),
                    )
                    .child(
                        div()
                            .id("focus")
                            .px_3()
                            .py_1()
                            .rounded_lg()
                            .background(rgb(0x2f2f36))
                            .hover(|style| style.background(rgb(0x3d3d46)))
                            .child("Focus notes")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.notes.read(cx).focus_handle(cx).focus(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .text_color(rgb(0x9a9a9a))
                    .child(format!("name = {name_value:?}"))
                    .child(format!("notes = {notes_len} chars")),
            )
            // Caret styling: click into each field to compare the shapes.
            .child(
                div()
                    .mt_2()
                    .text_sm()
                    .text_color(rgb(0x9a9a9a))
                    .child("Caret styles:"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(caret_field("caret-bar", "bar"))
                    .child(
                        caret_field("caret-underscore", "underscore")
                            .caret_shape(CaretShape::Underline)
                            .caret_width(px(3.))
                            .caret_radius(px(1.5)),
                    )
                    .child(
                        caret_field("caret-short", "0.7 height")
                            .caret_width(px(4.))
                            .caret_radius(px(2.))
                            .caret_height_ratio(0.7),
                    ),
            )
            // A read-only field: the element lays it out, but edits are refused while
            // selection and copy still work.
            .child(
                text_input("readonly-input")
                    .placeholder("Read-only")
                    .accepts_input(false)
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .rounded_lg()
                    .p_2()
                    .w_full()
                    .whitespace_nowrap(),
            )
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        // Installs the keystroke bindings the fields listen for (navigation, delete,
        // cut/copy/paste, undo/redo) under the `EditableText` key context.
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));

        let bounds = Bounds::centered(None, size(px(520.), px(560.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(Example::new),
        )
        .unwrap();
        cx.activate(true);
    });
}
