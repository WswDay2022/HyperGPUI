//! Bounded per-batch path attachments. Only the attachment and UVs are local;
//! paint, clipping and final composition retain the original scene coordinates.
use hgpui::{Bounds, Path, PathVertex, ScaledPixels};

use crate::abi::{PathRasterizationVertex, PathSprite};

const EDGES: [u32; 2] = [128, 512];

/// Two reusable compact attachments and the original full-window fallback.
pub const PATH_TARGET_SLOTS: usize = EDGES.len() + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathTarget {
    size: [u32; 2],
    mapping: [u32; 3],
    slot: usize,
}

impl PathTarget {
    /// Original path target, used for large, scattered or uncertain bounds.
    pub fn full(viewport: [u32; 2]) -> Self {
        Self {
            size: viewport.map(|value| value.max(1)),
            mapping: [0; 3],
            slot: 0,
        }
    }

    /// Select an attachment from the raster geometry visible inside the window.
    pub fn for_paths(viewport: [u32; 2], paths: &[Path<ScaledPixels>]) -> Self {
        // Do not add a second clip plane inside a path that already carries a clip mask.
        // Its unclipped vertex bounds guard the original triangle interpolation at that mask.
        Self::for_bounds(viewport, paths.iter().map(|path| path.bounds))
    }

    fn for_bounds(
        viewport: [u32; 2],
        bounds: impl IntoIterator<Item = Bounds<ScaledPixels>>,
    ) -> Self {
        let full = Self::full(viewport);
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        for bounds in bounds {
            let min = [bounds.left().0, bounds.top().0];
            let max = [bounds.right().0, bounds.bottom().0];
            if !min.into_iter().chain(max).all(f32::is_finite) {
                return full;
            }
            for i in 0..2 {
                lo[i] = lo[i].min(min[i]);
                hi[i] = hi[i].max(max[i]);
            }
        }
        // A batch may need one spanning composite rect, so include gaps between paths.
        // Outward rounding and a texel on each side keep the linear-sampling footprint valid.
        let mut origin = [0; 2];
        let mut span = [0; 2];
        for i in 0..2 {
            let min = lo[i].max(0.0);
            let max = hi[i].min(viewport[i] as f32);
            if min >= max {
                return full;
            }
            // Preserve the scene's 2×2 derivative quad phase for analytic curves.
            origin[i] = (min.floor() as u32).saturating_sub(1) & !1;
            let end = (max.ceil() as u32).saturating_add(1).min(viewport[i]);
            span[i] = end - origin[i];
        }
        let area = u64::from(viewport[0]) * u64::from(viewport[1]);
        for (index, edge) in EDGES.into_iter().enumerate() {
            if span[0] <= edge && span[1] <= edge && u64::from(edge).pow(2) <= area / 2 {
                return Self {
                    size: [edge; 2],
                    mapping: [origin[0], origin[1], edge],
                    slot: index + 1,
                };
            }
        }
        full
    }

    pub fn size(self) -> [u32; 2] {
        self.size
    }
    pub fn slot(self) -> usize {
        self.slot
    }

    pub fn vertex(
        self,
        path: &Path<ScaledPixels>,
        vertex: &PathVertex<ScaledPixels>,
    ) -> PathRasterizationVertex {
        PathRasterizationVertex {
            xy_position: vertex.xy_position,
            curve_position: vertex.st_position,
            color: path.color,
            bounds: path.bounds,
            content_mask: path.content_mask.bounds,
            mask_chain: path.mask_chain,
            tile_info: self.mapping,
        }
    }

    pub fn sprite(self, bounds: Bounds<ScaledPixels>) -> PathSprite {
        PathSprite {
            bounds,
            tile_info: self.mapping,
            padding: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PathTarget;
    use hgpui::{Bounds, ScaledPixels, bounds, point, size};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
        bounds(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(w), ScaledPixels(h)),
        )
    }

    #[test]
    fn fractional_and_clipped_bounds_select_a_guarded_tile() {
        let tile = PathTarget::for_bounds([3840, 2160], [rect(1530.75, 2100.25, 32.0, 60.75)]);
        assert_eq!(tile.size(), [128; 2]);
        assert_eq!(tile.mapping, [1528, 2098, 128]);
        assert_eq!(
            tile.sprite(rect(1530.75, 2100.25, 1.0, 1.0)).tile_info,
            tile.mapping
        );
    }

    #[test]
    fn uses_union_not_individual_size_and_avoids_marginal_tiles() {
        let viewport = [1920, 1080];
        assert_eq!(
            PathTarget::for_bounds(viewport, [rect(400.0, 500.0, 200.0, 280.0)]).size(),
            [512; 2]
        );
        assert_eq!(
            PathTarget::for_bounds(
                viewport,
                [rect(50.0, 50.0, 5.0, 5.0), rect(800.0, 520.0, 5.0, 5.0)]
            ),
            PathTarget::full(viewport)
        );
        assert_eq!(
            PathTarget::for_bounds([160, 160], [rect(10.0, 10.0, 10.0, 10.0)]).slot(),
            0
        );
        assert_eq!(
            PathTarget::for_bounds(viewport, [rect(f32::NAN, 1.0, 5.0, 5.0)]).slot(),
            0
        );
    }
}
