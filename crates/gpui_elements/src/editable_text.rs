//! Implementation for editable-text elements (gpui equivalent of html `<input>` and `<textarea>`).
//! TODO: More documentation
//!
//! Backlog of not-yet implemented features:
//! - text sanitation & validation (see no-op implementation of [`EditableTextState::validate_incoming_text`])
//! - nav & select via PageUp/PageDown
//! - masking text (e.g. for passwords)

pub mod actions;
mod caret;
mod element;
mod history;
mod layout;
mod state;
mod storage;

pub use element::*;
pub use state::*;
pub use storage::*;
