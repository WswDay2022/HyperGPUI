use gpui::Hsla;

/// Style colors applied to the Input element
#[derive(Clone, Copy, Debug)]
pub struct InputColors {
    /// This is the background color applied to the range of text that is currently selected by the user.
    pub selection: Hsla,
    /// This is the color of the user's text cursor.
    pub cursor: Hsla,
    /// This is the color of the placeholder string, when one is assigned and the text field is empty.
    pub placeholder: Hsla,
    pub marked: Hsla,
}

impl Default for InputColors {
    fn default() -> Self {
        Self {
            selection: gpui::hsla(0.583, 0.519, 0.31, 1.0),
            cursor: Hsla::white().opacity(0.8),
            marked: Hsla::white().opacity(0.6),
            placeholder: gpui::hsla(0., 0., 0.5, 1.0),
        }
    }
}
