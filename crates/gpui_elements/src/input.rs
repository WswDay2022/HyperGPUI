pub mod actions;
mod colors;
mod cursor;
mod element;
mod history;
mod layout;
mod paint;
mod state;
mod state_input_handler;
mod storage;
pub(self) mod unicode;

pub use colors::*;
pub use cursor::*;
pub use element::*;
pub(self) use history::*;
pub use layout::*;
pub use state::*;
pub use storage::*;

#[allow(dead_code)]
fn make_element(app: &mut gpui::App) -> impl gpui::IntoElement {
    use gpui::AppContext;
    let state = app.new(|cx| InputState::new(cx));
    input(&state, app).text_cursor(default_cursor(&state, app))
}
