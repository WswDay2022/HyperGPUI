//! Implementation for editable-text elements (gpui equivalent of html
//! [`<input>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/input) and
//! [`<textarea>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/textarea)).
//!
//! Both [`text_input`] and [`text_area`] create an [`EditableTextElement`]. This element supports:
//! - navigating via keyboard & mouse (by character, word, line, and document)
//! - highlight selection via keyboard & mouse (holding shift, double/triple click mouse, mouse drag)
//! - typing using an InputMethodEditor (IME) for writing Chinese, Japanese, and Korean utf-16
//! - inserting newlines (`\n`) and tabs (`\t`)
//! - cut/copy/paste
//! - caret / text cursor that can blink
//! - simple undo/redo within a single field
//!
//! For all input actions, see documentation in the [`actions`] module.
//!
//! Editable text elements will default to using [`String`] as the storage medium (see [`StringStorage`]).
//! Standard library strings are not ideal though for large text documents. For such uses,
//! it is encouraged that implementers consider rolling their own [`UnicodeTextStorage`] medium.
//!
//! ### Usage Samples
//!
//! A single-line text input with a fixed width and text that does not wrap
//! (overflow text is clipped and does not scroll).
//! ```
//! # use gpui::prelude::*;
//! # fn test() -> gpui_elements::editable_text::EditableTextElement {
//! use gpui_elements::editable_text::text_input;
//! text_input("my_input")
//!     .placeholder("empty text")
//!     .w_5()
//!     .min_h_auto()
//!     .whitespace_nowrap()
//! # }
//! ```
//!
//! A single-line text input with a flexible width and text that does not wrap, but will scroll if overflowing.
//! ```
//! # use gpui::{prelude::*, Hsla};
//! # fn test() -> gpui_elements::editable_text::EditableTextElement {
//! use gpui_elements::editable_text::text_input;
//! text_input("my_input")
//!     .placeholder("empty text")
//!     .border_1().rounded_lg().border_color(Hsla::white()) // has a border
//!     .p_2() // padding between the text and border
//!     .min_w_10().max_w_128()
//!     .min_h_auto()
//!     .whitespace_nowrap()
//!     .overflow_x_scroll()
//! # }
//! ```
//!
//! A multi-line text area with flexible height, wrapping text, and scrolling overflow on both axes.
//! ```
//! # use gpui::{prelude::*, Hsla};
//! # fn test() -> gpui_elements::editable_text::EditableTextElement {
//! use gpui_elements::editable_text::text_area;
//! text_area("message")
//!     .placeholder("empty text")
//!     .border_1().rounded_lg().border_color(Hsla::white()) // has a border
//!     .p_2() // padding between the text and border
//!     .min_w_10().max_w_128()
//!     .min_h_24().max_h_128()
//!     .whitespace_normal() // default
//!     .overflow_x_scroll().overflow_y_scroll()
//! # }
//! ```
//!
//! The user-inputted text can be accessed via event callbacks on the element.
//! There is no callback representing the concept of "user is done editing". Its recommended that
//! users write a [debounce](https://developer.mozilla.org/en-US/docs/Glossary/Debounce)
//! or some way to detect "focus lost" to signify the user leaving the field.
//! ```
//! # use gpui::{prelude::*, App, Entity};
//! # fn test() -> gpui_elements::editable_text::EditableTextElement {
//! use gpui_elements::editable_text::{text_input, EditableTextState};
//! text_input("my_input")
//!     .whitespace_nowrap()
//!     .overflow_x_scroll()
//!     // this will trigger on every character input or other mutation to the underlying string
//!     .on_text_changed(|state: &Entity<EditableTextState>, cx: &mut App| {
//!         println!("text changed to: {:?}", state.read(cx).as_str());
//!     })
//! # }
//! ```
//!
//! You can view more complex examples in the gpui crate examples.
//! TODO: there is no example with editable text yet, and we should link it here when there is.
//!
//! ### Backlog of not-yet implemented features:
//! - detecting focus being lost on an EditableText field
//! - text sanitation & validation (see no-op implementation of [`EditableTextState::validate_incoming_text`])
//! - nav & select via PageUp/PageDown
//! - screen reader support via a11y
//! - masking text (e.g. for passwords)
//! - disabling `insert_tab` if favor of tab being used to change focus between elements (i.e. escaping the field)
//!

pub mod actions;
mod caret;
mod element;
mod history;
mod layout;
mod state;
mod storage;

pub use caret::*;
pub use element::*;
pub use state::*;
pub use storage::*;
