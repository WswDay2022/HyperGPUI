use crate::filter_matrix::combined_blur_radius;
use hgpui::{MAX_FILTER_GROUP_DEPTH, ScaledFilter, Scene};
use smallvec::SmallVec;

/// Counts the isolation targets required by filter markers in paint order.
pub fn filter_group_depth(boundaries: impl IntoIterator<Item = bool>) -> usize {
    let mut depth = 0usize;
    let mut peak = 0;
    for start in boundaries {
        if start {
            depth += 1;
            if depth == MAX_FILTER_GROUP_DEPTH {
                return depth;
            }
            peak = peak.max(depth);
        } else {
            depth = depth.saturating_sub(1);
        }
    }
    peak
}

/// Whether this frame's backdrop filters or isolated content groups can run Gaussian passes.
/// Color-only filters and clip groups never need the half-resolution scratch.
pub fn needs_gaussian_targets(scene: &Scene) -> bool {
    let uses_blur = |filters: &[ScaledFilter]| {
        let sigma = combined_blur_radius(filters) * 0.5;
        sigma.is_finite() && sigma > 0.0
    };
    if scene
        .backdrop_filters
        .iter()
        .any(|filter| uses_blur(&filter.filters))
    {
        return true;
    }

    let mut depth = 0usize;
    for boundary in &scene.filter_boundaries {
        if boundary.is_start {
            depth += 1;
            if depth <= MAX_FILTER_GROUP_DEPTH
                && !boundary.clip_group
                && uses_blur(&boundary.filters)
            {
                return true;
            }
        } else {
            depth = depth.saturating_sub(1);
        }
    }
    false
}

/// Active render target and the parents visible to backdrop filters.
pub struct TargetStack<Target> {
    current: Target,
    parents: SmallVec<[(Target, bool); MAX_FILTER_GROUP_DEPTH]>,
}

impl<Target> TargetStack<Target> {
    /// Starts rendering into the root target.
    pub fn new(root: Target) -> Self {
        Self {
            current: root,
            parents: SmallVec::new(),
        }
    }

    /// Returns the active render target.
    pub fn current(&self) -> &Target {
        &self.current
    }

    /// Enters an isolated group, optionally including its parent in backdrop sampling.
    pub fn enter(&mut self, next: Target, inherits_backdrop: bool) {
        self.parents.push((
            std::mem::replace(&mut self.current, next),
            inherits_backdrop,
        ));
    }

    /// Returns whether the active group samples its parent's backdrop.
    pub fn inherits_backdrop(&self) -> bool {
        self.parents.last().is_some_and(|(_, inherits)| *inherits)
    }

    /// Returns backdrop layers in paint order, stopping at a content-filter group.
    pub fn backdrop_layers(&self) -> impl Iterator<Item = &Target> {
        let start = self
            .parents
            .iter()
            .rposition(|(_, inherits)| !inherits)
            .map_or(0, |idx| idx + 1);

        self.parents[start..]
            .iter()
            .map(|(target, _)| target)
            .chain(std::iter::once(&self.current))
    }

    /// Leaves the current group and returns its target and restored parent.
    pub fn exit(&mut self) -> (Target, &Target) {
        let (parent, _) = self
            .parents
            .pop()
            .expect("render plan ended an isolated filter without beginning one");
        let filtered = std::mem::replace(&mut self.current, parent);

        (filtered, &self.current)
    }

    /// Checks that every isolated group has ended.
    pub fn assert_balanced(&self) {
        assert!(
            self.parents.is_empty(),
            "render plan left an isolated filter group open"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_filter_groups_share_one_target_but_nested_groups_need_two() {
        // Same number of markers; only their actual lifetimes determine allocation.
        assert_eq!(filter_group_depth([true, false, true, false]), 1);
        assert_eq!(filter_group_depth([true, true, false, false]), 2);
        assert_eq!(
            filter_group_depth([true, true, true, false, false, false]),
            2
        );
        assert_eq!(filter_group_depth([]), 0);
    }

    #[test]
    fn gaussian_scratch_is_only_needed_by_rendered_blur_filters() {
        use hgpui::{BackdropFilter, FilterBoundary, ScaledPixels};

        let group = |clip_group, filters| FilterBoundary {
            order: 0,
            bounds: Default::default(),
            content_mask: Default::default(),
            corner_radii: Default::default(),
            filters,
            opacity: 1.0,
            is_start: true,
            mask_chain: 0,
            clip_group,
        };
        let mut scene = Scene::default();
        assert!(!needs_gaussian_targets(&scene));
        scene.backdrop_filters.push(BackdropFilter {
            filters: [ScaledFilter::Brightness(0.5)].into_iter().collect(),
            ..Default::default()
        });
        scene
            .filter_boundaries
            .push(group(true, Default::default()));
        scene.filter_boundaries.push(group(
            false,
            [ScaledFilter::Grayscale(0.5)].into_iter().collect(),
        ));
        assert!(!needs_gaussian_targets(&scene));

        let blur = ScaledFilter::Blur(ScaledPixels(12.0));
        scene
            .filter_boundaries
            .push(group(false, [blur].into_iter().collect()));
        assert!(
            !needs_gaussian_targets(&scene),
            "a third group renders inline"
        );
        scene.filter_boundaries.remove(1);
        assert!(
            needs_gaussian_targets(&scene),
            "the same blur at depth two is rendered"
        );

        scene.filter_boundaries.clear();
        scene.backdrop_filters[0].filters.push(blur);
        assert!(needs_gaussian_targets(&scene));
    }

    #[test]
    fn nested_groups_inherit_backdrops_until_a_content_filter_and_restore_parents() {
        let mut targets = TargetStack::new("root");
        targets.enter("opacity", true);
        assert!(targets.inherits_backdrop());
        assert_eq!(
            targets.backdrop_layers().copied().collect::<Vec<_>>(),
            ["root", "opacity"]
        );

        targets.enter("content", false);
        assert!(!targets.inherits_backdrop());
        assert_eq!(
            targets.backdrop_layers().copied().collect::<Vec<_>>(),
            ["content"]
        );

        targets.enter("nested", true);
        assert!(targets.inherits_backdrop());
        assert_eq!(
            targets.backdrop_layers().copied().collect::<Vec<_>>(),
            ["content", "nested"]
        );

        assert_eq!(targets.exit(), ("nested", &"content"));
        assert_eq!(targets.exit(), ("content", &"opacity"));
        assert!(targets.inherits_backdrop());
        assert_eq!(
            targets.backdrop_layers().copied().collect::<Vec<_>>(),
            ["root", "opacity"]
        );

        assert_eq!(targets.exit(), ("opacity", &"root"));
        assert!(!targets.inherits_backdrop());
        assert_eq!(
            targets.backdrop_layers().copied().collect::<Vec<_>>(),
            ["root"]
        );
        targets.assert_balanced();
    }
}
