//! Host-side layout values that are independent of drawing and text shaping.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Border metrics passed to layout, independently of the painted border.
///
/// Taffy accepts percentage borders as well as absolute lengths. This opt-in
/// override exposes those metrics without changing Hgpui's painted stroke API.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum LayoutBorder {
    /// Use the existing painted, device-pixel-snapped border widths.
    #[default]
    Paint,
    /// Reserve these absolute/relative widths in layout only.
    Widths {
        /// Top layout border width.
        top: crate::DefiniteLength,
        /// Right layout border width.
        right: crate::DefiniteLength,
        /// Bottom layout border width.
        bottom: crate::DefiniteLength,
        /// Left layout border width.
        left: crate::DefiniteLength,
    },
}

impl From<crate::Edges<crate::DefiniteLength>> for LayoutBorder {
    fn from(widths: crate::Edges<crate::DefiniteLength>) -> Self {
        Self::Widths {
            top: widths.top,
            right: widths.right,
            bottom: widths.bottom,
            left: widths.left,
        }
    }
}

/// Direction of layout. Unlike motion's `Direction`, this controls RTL layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LayoutDirection {
    /// Left-to-right layout (the existing default).
    #[default]
    Ltr,
    /// Right-to-left layout, including Grid columns and positional alignment.
    Rtl,
}

impl From<LayoutDirection> for taffy::style::Direction {
    fn from(value: LayoutDirection) -> Self {
        match value {
            LayoutDirection::Ltr => Self::Ltr,
            LayoutDirection::Rtl => Self::Rtl,
        }
    }
}

/// Whether authored sizes include padding and border.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BoxSizing {
    /// Width/height include padding and border, preserving existing behavior.
    #[default]
    BorderBox,
    /// Width/height specify the content box.
    ContentBox,
}

impl From<BoxSizing> for taffy::style::BoxSizing {
    fn from(value: BoxSizing) -> Self {
        match value {
            BoxSizing::BorderBox => Self::BorderBox,
            BoxSizing::ContentBox => Self::ContentBox,
        }
    }
}

/// Floating of a child in Taffy's block layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FloatSide {
    /// Do not float.
    #[default]
    None,
    /// Float to the left.
    Left,
    /// Float to the right.
    Right,
}

impl From<FloatSide> for taffy::style::Float {
    fn from(value: FloatSide) -> Self {
        match value {
            FloatSide::None => Self::None,
            FloatSide::Left => Self::Left,
            FloatSide::Right => Self::Right,
        }
    }
}

/// Which preceding floats a block child clears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ClearFloats {
    /// Do not clear floats.
    #[default]
    None,
    /// Clear left floats.
    Left,
    /// Clear right floats.
    Right,
    /// Clear floats on both sides.
    Both,
}

impl From<ClearFloats> for taffy::style::Clear {
    fn from(value: ClearFloats) -> Self {
        match value {
            ClearFloats::None => Self::None,
            ClearFloats::Left => Self::Left,
            ClearFloats::Right => Self::Right,
            ClearFloats::Both => Self::Both,
        }
    }
}

/// Legacy alignment of block child boxes, not glyph/text alignment.
///
/// These are Taffy's existing `-webkit-*`/`-moz-*` legacy block behaviors.
/// Use `Styled::text_align` for Hgpui's text shaping/painting alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BlockTextAlign {
    /// Normal block layout.
    #[default]
    Auto,
    /// Legacy left alignment of block children.
    LegacyLeft,
    /// Legacy right alignment of block children.
    LegacyRight,
    /// Legacy centered alignment of block children.
    LegacyCenter,
}

impl From<BlockTextAlign> for taffy::style::TextAlign {
    fn from(value: BlockTextAlign) -> Self {
        match value {
            BlockTextAlign::Auto => Self::Auto,
            BlockTextAlign::LegacyLeft => Self::LegacyLeft,
            BlockTextAlign::LegacyRight => Self::LegacyRight,
            BlockTextAlign::LegacyCenter => Self::LegacyCenter,
        }
    }
}
