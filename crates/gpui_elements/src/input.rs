pub mod actions;
mod colors;
mod cursor;
mod element;
mod history;
mod layout;
mod paint;
mod state;
mod state_input_handler;
pub(self) mod unicode;

pub use colors::*;
pub(self) use cursor::*;
pub use element::*;
pub(self) use history::*;
pub use layout::*;
pub use state::*;

pub(self) fn replace_range(
    string: &mut gpui::SharedString,
    range: std::ops::Range<usize>,
    replace_with: &str,
) {
    // NOTE: reallocates the SharedString bc SharedString is immutable
    let mut content = string.to_string();
    content.replace_range(range, replace_with);
    *string = content.into();
}
