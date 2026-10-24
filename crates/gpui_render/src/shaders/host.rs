use super::*;

impl From<bool> for common::ShaderBool {
    fn from(value: bool) -> Self {
        if value { Self::Enabled } else { Self::Disabled }
    }
}

impl From<hgpui::Bounds<hgpui::ScaledPixels>> for common::Bounds {
    fn from(bounds: hgpui::Bounds<hgpui::ScaledPixels>) -> Self {
        Self {
            origin: wgsl_rs::std::vec2f(bounds.origin.x.0, bounds.origin.y.0),
            size: wgsl_rs::std::vec2f(bounds.size.width.0, bounds.size.height.0),
        }
    }
}

impl From<hgpui::Edges<hgpui::ScaledPixels>> for common::Edges {
    fn from(edges: hgpui::Edges<hgpui::ScaledPixels>) -> Self {
        Self {
            top: edges.top.0,
            right: edges.right.0,
            bottom: edges.bottom.0,
            left: edges.left.0,
        }
    }
}

impl From<hgpui::ContentMask<hgpui::ScaledPixels>> for common::ContentMask {
    fn from(mask: hgpui::ContentMask<hgpui::ScaledPixels>) -> Self {
        Self {
            bounds: mask.bounds.into(),
            corner_radii: mask.corner_radii.into(),
            fade_out: mask.fade_out.into(),
        }
    }
}

impl From<hgpui::Corners<hgpui::ScaledPixels>> for common::Corners {
    fn from(corners: hgpui::Corners<hgpui::ScaledPixels>) -> Self {
        Self {
            top_left: corners.top_left.0,
            top_right: corners.top_right.0,
            bottom_right: corners.bottom_right.0,
            bottom_left: corners.bottom_left.0,
        }
    }
}
