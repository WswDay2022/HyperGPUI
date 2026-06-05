use gpui::Hsla;

#[derive(Clone, Copy, Debug)]
pub struct PaintColors {
    pub selection: Hsla,
    pub cursor: Hsla,
    pub placeholder: Hsla,
}

impl Default for PaintColors {
    fn default() -> Self {
        Self {
            selection: Hsla::blue().opacity(0.2),
            cursor: Hsla::white().opacity(0.8),
            placeholder: gpui::hsla(0.6, 0.6, 0.6, 1.0),
        }
    }
}
