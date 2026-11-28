// Previous imports: use crate::{Bounds, Half};
use crate::{Bounds, Half, Point};
use std::{
    cmp,
    fmt::Debug,
    ops::{Add, Sub},
    ptr::NonNull,
};

/// Maximum children per internal node (R-tree style branching factor).
/// Higher values = shorter tree = fewer cache misses, but more work per node.
const MAX_CHILDREN: usize = 12;

/// A spatial tree optimized for finding maximum ordering among intersecting bounds.
///
/// This is an R-tree variant specifically designed for the use case of assigning
/// z-order to overlapping UI elements. Key optimizations:
/// - Tracks the leaf with global max ordering for O(1) fast-path queries
// Previous description: Uses higher branching factor (4) for lower tree height.
/// - Keeps leaves at equal depth by splitting overflowing internal nodes
/// - Aggressive pruning during search based on max_order metadata
#[derive(Debug)]
pub(crate) struct BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    /// All nodes stored contiguously for cache efficiency.
    nodes: Vec<Node<U>>,
    /// Index of the root node, if any.
    root: Option<usize>,
    /// Index of the leaf with the highest ordering (for fast-path lookups).
    max_leaf: Option<usize>,
    /// Minimum ordering assigned to any subsequent insert. Raised before painting deferred
    /// draws so overlays always sort above the main scene (and their orders can't fall inside a
    /// content-filter order range from the main scene). 0 means no floor.
    order_floor: u32,
    /// Reusable stack for tree traversal during insertion.
    insert_path: Vec<usize>,
    /// Reusable stack for search operations.
    search_stack: Vec<NonNull<Node<U>>>,
}

/// A node in the bounds tree.
#[derive(Debug, Clone)]
struct Node<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    // Previous: Bounding box containing this node and all descendants.
    /// Exact minimum corner, plus a size used for insertion costs only. With
    /// floats, rebuilding origin + size may round inward; pruning uses max_corner.
    bounds: Bounds<U>,
    /// Exact maximum of descendant leaf endpoints, without subtract/add rounding.
    max_corner: Point<U>,
    /// Maximum ordering value in this subtree.
    max_order: u32,
    /// Node-specific data.
    kind: NodeKind,
}

#[derive(Debug, Clone)]
enum NodeKind {
    /// Leaf node containing actual bounds data.
    Leaf {
        /// The ordering assigned to this bounds.
        order: u32,
    },
    /// Internal node with children.
    Internal {
        /// Indices of child nodes (2 to MAX_CHILDREN).
        children: NodeChildren,
    },
}

/// Fixed-size array for child indices, avoiding heap allocation.
#[derive(Debug, Clone)]
struct NodeChildren {
    // Keeps an invariant where the max order child is always at the end
    indices: [usize; MAX_CHILDREN],
    len: u8,
}

impl NodeChildren {
    fn new() -> Self {
        Self {
            indices: [0; MAX_CHILDREN],
            len: 0,
        }
    }

    fn push(&mut self, index: usize) {
        debug_assert!((self.len as usize) < MAX_CHILDREN);
        self.indices[self.len as usize] = index;
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len as usize
    }

    fn as_slice(&self) -> &[usize] {
        &self.indices[..self.len as usize]
    }
}

impl<U> BoundsTree<U>
where
    U: Clone
        + Debug
        + PartialEq
        + PartialOrd
        + Add<U, Output = U>
        + Sub<Output = U>
        + Half
        + Default,
{
    /// Clears all nodes from the tree.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
        self.max_leaf = None;
        self.order_floor = 0;
        self.insert_path.clear();
        self.search_stack.clear();
    }

    /// Raise the minimum ordering for subsequent inserts to `floor`. Relative ordering above the
    /// floor is preserved (overlapping inserts still step above one another).
    pub fn set_order_floor(&mut self, floor: u32) {
        self.order_floor = self.order_floor.max(floor);
    }

    pub(crate) fn order_floor(&self) -> u32 {
        self.order_floor
    }

    /// The highest ordering assigned to any bounds so far (0 if empty).
    pub fn max_order(&self) -> u32 {
        self.max_leaf.map_or(0, |idx| self.nodes[idx].max_order)
    }

    /// Inserts bounds with an ordering strictly greater than *every* existing bounds (not just
    /// intersecting ones), and returns that ordering. Used for content-filter group boundaries
    /// (which must sort after all previously-painted content so their order range can't collide
    /// with unrelated non-overlapping content that reuses low orderings) and to raise the order
    /// floor before painting deferred draws (so overlays always sort above the main scene).
    pub fn insert_above_all(&mut self, new_bounds: Bounds<U>) -> u32 {
        let ordering = self.max_order() + 1;
        let new_leaf_idx = self.insert_leaf(new_bounds, ordering);
        self.max_leaf = Some(new_leaf_idx);
        ordering
    }

    /// Inserts bounds into the tree and returns its assigned ordering.
    ///
    /// The ordering is one greater than the maximum ordering of any
    /// existing bounds that intersect with the new bounds.
    pub fn insert(&mut self, new_bounds: Bounds<U>) -> u32 {
        // Find maximum ordering among intersecting bounds
        let max_intersecting = self.find_max_ordering(&new_bounds);
        let ordering = (max_intersecting + 1).max(self.order_floor);

        // Insert the new leaf
        let new_leaf_idx = self.insert_leaf(new_bounds, ordering);

        // Update max_leaf tracking
        self.max_leaf = match self.max_leaf {
            None => Some(new_leaf_idx),
            Some(old_idx) if self.nodes[old_idx].max_order < ordering => Some(new_leaf_idx),
            some => some,
        };

        ordering
    }

    /// Finds the maximum ordering among all bounds that intersect with the query.
    fn find_max_ordering(&mut self, query: &Bounds<U>) -> u32 {
        let Some(root_idx) = self.root else {
            return 0;
        };

        // Fast path: check if the max-ordering leaf intersects
        if let Some(max_idx) = self.max_leaf {
            let max_node = &self.nodes[max_idx];
            if query.intersects(&max_node.bounds) {
                return max_node.max_order;
            }
        }

        // Slow path: search the tree
        self.search_stack.clear();
        self.search_stack.push(NonNull::from(&self.nodes[root_idx]));

        let mut max_found = 0u32;
        let query_max = query.bottom_right();

        while let Some(node) = self.search_stack.pop() {
            // SAFETY: `node` is guaranteed to be valid as the `nodes` stack is unmodified in this function
            // and the `search_stack` only contains pointers from this function call.
            let node = unsafe { node.as_ref() };

            // Pruning: skip if this subtree can't improve our result
            if node.max_order <= max_found {
                continue;
            }

            // Previous spatial pruning: if !query.intersects(&node.bounds) { continue; }
            // Use exact endpoint metadata: origin + rounded size could otherwise
            // clip a descendant. The strict comparisons preserve touching semantics.
            if !(query.origin.x < node.max_corner.x
                && query_max.x > node.bounds.origin.x
                && query.origin.y < node.max_corner.y
                && query_max.y > node.bounds.origin.y)
            {
                continue;
            }

            match &node.kind {
                NodeKind::Leaf { order } => {
                    max_found = cmp::max(max_found, *order);
                }
                NodeKind::Internal { children } => {
                    // Children are maintained with highest max_order at the end.
                    // Push in forward order to highest (last) is popped first.
                    self.search_stack.extend(
                        children
                            .as_slice()
                            .iter()
                            .map(|&child_idx| &self.nodes[child_idx])
                            .filter(|node| node.max_order > max_found)
                            .map(NonNull::from),
                    );
                }
            }
        }

        max_found
    }

    /// Inserts a leaf node with the given bounds and ordering.
    /// Returns the index of the new leaf.
    fn insert_leaf(&mut self, bounds: Bounds<U>, order: u32) -> usize {
        self.insert_path.clear();
        let new_leaf_idx = self.nodes.len();
        self.nodes.push(Node {
            bounds: bounds.clone(),
            max_corner: bounds.bottom_right(),
            max_order: order,
            kind: NodeKind::Leaf { order },
        });

        let Some(root_idx) = self.root else {
            // Tree is empty, new leaf becomes root
            self.root = Some(new_leaf_idx);
            return new_leaf_idx;
        };

        // If root is a leaf, create internal node with both
        if matches!(self.nodes[root_idx].kind, NodeKind::Leaf { .. }) {
            let root_bounds = self.nodes[root_idx].bounds.clone();
            let root_order = self.nodes[root_idx].max_order;

            let mut children = NodeChildren::new();
            // Max end invariant
            if order > root_order {
                children.push(root_idx);
                children.push(new_leaf_idx);
            } else {
                children.push(new_leaf_idx);
                children.push(root_idx);
            }

            let new_root_idx = self.nodes.len();
            let max_corner = self.nodes[root_idx]
                .max_corner
                .max(&self.nodes[new_leaf_idx].max_corner);
            self.nodes.push(Node {
                bounds: root_bounds.union(&bounds),
                max_corner,
                max_order: cmp::max(root_order, order),
                kind: NodeKind::Internal { children },
            });
            self.root = Some(new_root_idx);
            return new_leaf_idx;
        }

        // All leaves have the same depth. Descend to their parent, then split
        // overflowing nodes upwards instead of growing a chain below a full node.
        // Only indices survive nodes.push(): reallocations must not invalidate a
        // borrowed node or a search-stack pointer used by this insertion.
        let mut current_idx = root_idx;
        loop {
            self.insert_path.push(current_idx);
            let NodeKind::Internal { children } = &self.nodes[current_idx].kind else {
                unreachable!("Insertion path contains only internal nodes");
            };
            let mut best_child = children.as_slice()[0];
            if matches!(self.nodes[best_child].kind, NodeKind::Leaf { .. }) {
                break;
            }
            let mut best_cost = bounds
                .union(&self.nodes[best_child].bounds)
                .half_perimeter();
            for &child_idx in children.as_slice().iter().skip(1) {
                let cost = bounds.union(&self.nodes[child_idx].bounds).half_perimeter();
                if cost < best_cost {
                    best_child = child_idx;
                    best_cost = cost;
                }
            }
            current_idx = best_child;
        }

        let mut pending_child = Some(new_leaf_idx);
        for path_pos in (0..self.insert_path.len()).rev() {
            let node_idx = self.insert_path[path_pos];
            let NodeKind::Internal { children } = &self.nodes[node_idx].kind else {
                unreachable!("Insertion path contains only internal nodes");
            };
            let mut children = children.clone();
            if let Some(child_idx) = pending_child.take() {
                if children.len() == MAX_CHILDREN {
                    let mut overflowing = [0; MAX_CHILDREN + 1];
                    overflowing[..MAX_CHILDREN].copy_from_slice(children.as_slice());
                    overflowing[MAX_CHILDREN] = child_idx;
                    pending_child = Some(self.split_node(node_idx, &overflowing));
                    continue;
                }
                children.push(child_idx);
            }
            // Splitting may lower the old child's maximum, so recompute metadata
            // and restore the max-at-end invariant, rather than only raising it.
            self.nodes[node_idx] = self.internal_node(children);
        }
        if let Some(sibling_idx) = pending_child {
            let mut children = NodeChildren::new();
            children.push(root_idx);
            children.push(sibling_idx);
            let root = self.internal_node(children);
            self.root = Some(self.nodes.len());
            self.nodes.push(root);
        }

        /* Previous insertion algorithm retained for restoration. It replaced a
         * leaf below a full node with a new internal node, producing linear depth
         * and quadratic cumulative insertion work on monotonic disjoint bounds.
        // Descend to find the best internal node to insert into
        self.insert_path.clear();
        let mut current_idx = root_idx;

        loop {
            let current = &self.nodes[current_idx];
            let NodeKind::Internal { children } = &current.kind else {
                unreachable!("Should only traverse internal nodes");
            };

            self.insert_path.push(current_idx);

            // Find the best child to descend into
            let mut best_child_idx = children.as_slice()[0];
            let mut best_child_pos = 0;
            let mut best_cost = bounds
                .union(&self.nodes[best_child_idx].bounds)
                .half_perimeter();

            for (pos, &child_idx) in children.as_slice().iter().enumerate().skip(1) {
                let cost = bounds.union(&self.nodes[child_idx].bounds).half_perimeter();
                if cost < best_cost {
                    best_cost = cost;
                    best_child_idx = child_idx;
                    best_child_pos = pos;
                }
            }

            // Check if best child is a leaf or internal
            if matches!(self.nodes[best_child_idx].kind, NodeKind::Leaf { .. }) {
                // Best child is a leaf. Check if current node has room for another child.
                if children.len() < MAX_CHILDREN {
                    // Add new leaf directly to this node
                    let node = &mut self.nodes[current_idx];

                    if let NodeKind::Internal { children } = &mut node.kind {
                        children.push(new_leaf_idx);
                        // Swap new leaf only if it has the highest max_order
                        if order <= node.max_order {
                            let last = children.len() - 1;
                            children.indices.swap(last - 1, last);
                        }
                    }

                    node.bounds = node.bounds.union(&bounds);
                    node.max_order = cmp::max(node.max_order, order);
                    break;
                } else {
                    // Node is full, create new internal with [best_leaf, new_leaf]
                    let sibling_bounds = self.nodes[best_child_idx].bounds.clone();
                    let sibling_order = self.nodes[best_child_idx].max_order;

                    let mut new_children = NodeChildren::new();
                    // Max end invariant
                    if order > sibling_order {
                        new_children.push(best_child_idx);
                        new_children.push(new_leaf_idx);
                    } else {
                        new_children.push(new_leaf_idx);
                        new_children.push(best_child_idx);
                    }

                    let new_internal_idx = self.nodes.len();
                    let new_internal_max = cmp::max(sibling_order, order);
                    self.nodes.push(Node {
                        bounds: sibling_bounds.union(&bounds),
                        max_order: new_internal_max,
                        kind: NodeKind::Internal {
                            children: new_children,
                        },
                    });

                    // Replace the leaf with the new internal in parent
                    let parent = &mut self.nodes[current_idx];
                    if let NodeKind::Internal { children } = &mut parent.kind {
                        let children_len = children.len();

                        children.indices[best_child_pos] = new_internal_idx;

                        // If new internal has highest max_order, swap it to the end
                        // to maintain sorting invariant
                        if new_internal_max > parent.max_order {
                            children.indices.swap(best_child_pos, children_len - 1);
                        }
                    }
                    break;
                }
            } else {
                // Best child is internal, continue descent
                current_idx = best_child_idx;
            }
        }

        // Propagate bounds and max_order updates up the tree
        let mut updated_child_idx = None;
        for &node_idx in self.insert_path.iter().rev() {
            let node = &mut self.nodes[node_idx];
            node.bounds = node.bounds.union(&bounds);

            if node.max_order < order {
                node.max_order = order;

                // Swap updated child to end (skip first iteration since the invariant is already handled by previous cases)
                if let Some(child_idx) = updated_child_idx {
                    if let NodeKind::Internal { children } = &mut node.kind {
                        if let Some(pos) = children.as_slice().iter().position(|&c| c == child_idx)
                        {
                            let last = children.len() - 1;
                            if pos != last {
                                children.indices.swap(pos, last);
                            }
                        }
                    }
                }
            }

            updated_child_idx = Some(node_idx);
        }
        */

        new_leaf_idx
    }

    /// Builds exact subtree metadata and puts a maximum-order child at the end.
    fn internal_node(&self, mut children: NodeChildren) -> Node<U> {
        let first = &self.nodes[children.as_slice()[0]];
        // Previous fold: let mut bounds = first.bounds.clone();
        let mut min_corner = first.bounds.origin.clone();
        let mut max_corner = first.max_corner.clone();
        let mut max_order = first.max_order;
        let mut max_pos = 0;
        for (pos, &child_idx) in children.as_slice().iter().enumerate().skip(1) {
            let child = &self.nodes[child_idx];
            // Previous fold: bounds = bounds.union(&child.bounds);
            min_corner = min_corner.min(&child.bounds.origin);
            max_corner = max_corner.max(&child.max_corner);
            if child.max_order > max_order {
                max_order = child.max_order;
                max_pos = pos;
            }
        }
        let last = children.len() - 1;
        children.indices.swap(max_pos, last);
        Node {
            bounds: Bounds::from_corners(min_corner, max_corner.clone()),
            max_corner,
            max_order,
            kind: NodeKind::Internal { children },
        }
    }

    /// Splits 13 children into groups of 6 and 7 at the same level. With at
    /// least 6 children per non-root internal node, leaf depth is O(log_6 n).
    /// Seed selection is O(MAX_CHILDREN^2); the branching factor stays fixed.
    fn split_node(&mut self, node_idx: usize, indices: &[usize; MAX_CHILDREN + 1]) -> usize {
        const MIN_CHILDREN: usize = MAX_CHILDREN / 2;
        let mut seeds = (0, 1);
        let mut seed_cost = self.nodes[indices[0]]
            .bounds
            .union(&self.nodes[indices[1]].bounds)
            .half_perimeter();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let cost = self.nodes[indices[i]]
                    .bounds
                    .union(&self.nodes[indices[j]].bounds)
                    .half_perimeter();
                if cost > seed_cost {
                    seeds = (i, j);
                    seed_cost = cost;
                }
            }
        }

        let mut left = NodeChildren::new();
        let mut right = NodeChildren::new();
        left.push(indices[seeds.0]);
        right.push(indices[seeds.1]);
        let mut left_bounds = self.nodes[indices[seeds.0]].bounds.clone();
        let mut right_bounds = self.nodes[indices[seeds.1]].bounds.clone();
        let mut remaining = indices.len() - 2;
        for (pos, &child_idx) in indices.iter().enumerate() {
            if pos == seeds.0 || pos == seeds.1 {
                continue;
            }
            let bounds = &self.nodes[child_idx].bounds;
            let left_union = left_bounds.union(bounds);
            let right_union = right_bounds.union(bounds);
            let left_cost = left_union.half_perimeter();
            let right_cost = right_union.half_perimeter();
            let choose_left = if left.len() + remaining == MIN_CHILDREN {
                true
            } else if right.len() + remaining == MIN_CHILDREN {
                false
            } else {
                left_cost < right_cost || (left_cost == right_cost && left.len() <= right.len())
            };
            if choose_left {
                left.push(child_idx);
                left_bounds = left_union;
            } else {
                right.push(child_idx);
                right_bounds = right_union;
            }
            remaining -= 1;
        }
        debug_assert!(left.len() >= MIN_CHILDREN && right.len() >= MIN_CHILDREN);
        self.nodes[node_idx] = self.internal_node(left);
        let sibling = self.internal_node(right);
        let sibling_idx = self.nodes.len();
        self.nodes.push(sibling);
        sibling_idx
    }
}

impl<U> Default for BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    fn default() -> Self {
        BoundsTree {
            nodes: Vec::new(),
            root: None,
            max_leaf: None,
            order_floor: 0,
            insert_path: Vec::new(),
            search_stack: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bounds, Point, Size};
    use rand::{Rng, SeedableRng};

    #[test]
    fn test_insert() {
        let mut tree = BoundsTree::<f32>::default();
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 5.0, y: 5.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };

        // Insert the bounds into the tree and verify the order is correct
        assert_eq!(tree.insert(bounds1), 1);
        assert_eq!(tree.insert(bounds2), 2);
        assert_eq!(tree.insert(bounds3), 3);

        // Insert non-overlapping bounds and verify they can reuse orders
        let bounds4 = Bounds {
            origin: Point { x: 20.0, y: 20.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds5 = Bounds {
            origin: Point { x: 40.0, y: 40.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds6 = Bounds {
            origin: Point { x: 25.0, y: 25.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(bounds4), 1); // bounds4 does not overlap with bounds1, bounds2, or bounds3
        assert_eq!(tree.insert(bounds5), 1); // bounds5 does not overlap with any other bounds
        assert_eq!(tree.insert(bounds6), 2); // bounds6 overlaps with bounds4, so it should have a different order
    }

    fn tree_height(tree: &BoundsTree<f32>) -> usize {
        let mut height = 0;
        let mut stack = Vec::new();
        if let Some(root) = tree.root {
            stack.push((root, 1));
        }
        while let Some((index, depth)) = stack.pop() {
            height = height.max(depth);
            if let NodeKind::Internal { children } = &tree.nodes[index].kind {
                stack.extend(children.as_slice().iter().map(|&child| (child, depth + 1)));
            }
        }
        height
    }

    #[test]
    fn test_monotonic_disjoint_insertion_stays_logarithmic() {
        const NUM_BOUNDS: usize = 2048;
        let mut tree = BoundsTree::<f32>::default();
        let mut total_insert_path = 0;
        for i in 0..NUM_BOUNDS {
            let bounds = Bounds {
                origin: Point {
                    x: (2 * i) as f32,
                    y: 0.0,
                },
                size: Size {
                    width: 1.0,
                    height: 1.0,
                },
            };
            assert_eq!(tree.insert(bounds), 1);
            total_insert_path += tree.insert_path.len();
        }

        // Structural work, not elapsed time: a height-bounded tree must avoid both
        // a linear chain and quadratic cumulative descent on sorted input.
        let height = tree_height(&tree);
        let log2_n = NUM_BOUNDS.ilog2() as usize;
        let height_limit = 2 * log2_n + 1;
        let path_limit = NUM_BOUNDS * height_limit;
        eprintln!("bounds-tree n={NUM_BOUNDS} height={height} total_insert_path={total_insert_path}");
        assert!(
            height <= height_limit && total_insert_path <= path_limit,
            "monotonic disjoint insertion: height={height} (limit {height_limit}), \
             total_insert_path={total_insert_path} (limit {path_limit})"
        );
        assert_tree_invariants(&tree, NUM_BOUNDS);
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
        Bounds {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    fn assert_tree_invariants(tree: &BoundsTree<f32>, num_leaves: usize) {
        let Some(root) = tree.root else {
            assert_eq!(num_leaves, 0);
            assert!(tree.nodes.is_empty());
            assert!(tree.max_leaf.is_none());
            return;
        };
        let mut seen = vec![false; tree.nodes.len()];
        let mut stack = vec![(root, 1)];
        let mut leaf_depth = None;
        let mut leaves = 0;
        while let Some((index, depth)) = stack.pop() {
            assert!(!seen[index], "node {index} reached more than once");
            seen[index] = true;
            let node = &tree.nodes[index];
            match &node.kind {
                NodeKind::Leaf { order } => {
                    leaves += 1;
                    assert_eq!(node.max_order, *order);
                    assert_eq!(node.max_corner, node.bounds.bottom_right());
                    assert_eq!(*leaf_depth.get_or_insert(depth), depth);
                }
                NodeKind::Internal { children } => {
                    let min_children = if index == root { 2 } else { MAX_CHILDREN / 2 };
                    assert!((min_children..=MAX_CHILDREN).contains(&children.len()));
                    /* Previous invariant folded origin/size unions in child order:
                    let first = &tree.nodes[children.as_slice()[0]];
                    let mut bounds = first.bounds;
                    let mut max_order = first.max_order;
                    for &child in children.as_slice().iter().skip(1) {
                        bounds = bounds.union(&tree.nodes[child].bounds);
                        max_order = max_order.max(tree.nodes[child].max_order);
                    }
                    assert_eq!(node.bounds, bounds);
                    */
                    // Exact endpoint extrema are order-independent. No epsilon:
                    // each effective parent envelope must conservatively contain
                    // every child, even when origin + size would round inward.
                    let mut min_x = f32::INFINITY;
                    let mut min_y = f32::INFINITY;
                    let mut max_x = f32::NEG_INFINITY;
                    let mut max_y = f32::NEG_INFINITY;
                    let mut max_order = 0;
                    for &child in children.as_slice() {
                        let child = &tree.nodes[child];
                        min_x = min_x.min(child.bounds.origin.x);
                        min_y = min_y.min(child.bounds.origin.y);
                        max_x = max_x.max(child.max_corner.x);
                        max_y = max_y.max(child.max_corner.y);
                        max_order = max_order.max(child.max_order);
                        assert!(node.bounds.origin.x <= child.bounds.origin.x);
                        assert!(node.bounds.origin.y <= child.bounds.origin.y);
                        assert!(node.max_corner.x >= child.max_corner.x);
                        assert!(node.max_corner.y >= child.max_corner.y);
                    }
                    assert_eq!(
                        node.bounds,
                        rect(min_x, min_y, max_x - min_x, max_y - min_y)
                    );
                    assert_eq!(node.max_corner, Point { x: max_x, y: max_y });
                    assert_eq!(node.max_order, max_order);
                    let last = *children.as_slice().last().unwrap();
                    assert_eq!(tree.nodes[last].max_order, max_order);
                    stack.extend(children.as_slice().iter().map(|&child| (child, depth + 1)));
                }
            }
        }
        assert_eq!(leaves, num_leaves);
        assert!(seen.into_iter().all(|visited| visited));
        let max_leaf = &tree.nodes[tree.max_leaf.unwrap()];
        assert!(matches!(max_leaf.kind, NodeKind::Leaf { .. }));
        assert_eq!(max_leaf.max_order, tree.nodes[root].max_order);
    }

    fn assert_reference_insert(
        tree: &mut BoundsTree<f32>,
        reference: &mut Vec<(Bounds<f32>, u32)>,
        bounds: Bounds<f32>,
        floor: u32,
        above_all: bool,
    ) {
        let previous_max = reference.iter().map(|(_, order)| *order).max().unwrap_or(0);
        let expected = if above_all {
            previous_max + 1
        } else {
            let intersecting_max = reference
                .iter()
                .filter_map(|(old_bounds, order)| old_bounds.intersects(&bounds).then_some(*order))
                .max()
                .unwrap_or(0);
            (intersecting_max + 1).max(floor)
        };
        let actual = if above_all {
            tree.insert_above_all(bounds)
        } else {
            tree.insert(bounds)
        };
        assert_eq!(actual, expected, "insert {}: {bounds:?}", reference.len());
        assert_eq!(tree.max_order(), previous_max.max(expected));
        reference.push((bounds, expected));
    }

    fn assert_reference_query(
        tree: &mut BoundsTree<f32>,
        reference: &[(Bounds<f32>, u32)],
        query: Bounds<f32>,
    ) {
        let expected = reference
            .iter()
            .filter_map(|(bounds, order)| bounds.intersects(&query).then_some(*order))
            .max()
            .unwrap_or(0);
        assert_eq!(tree.find_max_ordering(&query), expected, "query {query:?}");
        // No pointer from a completed search can be dereferenced after nodes grows.
        assert!(tree.search_stack.is_empty());
    }

    #[test]
    fn test_split_tree_matches_linear_ordering_and_clear_reuse() {
        let mut tree = BoundsTree::<f32>::default();
        for seed in [1, 0x5eed, 0xdead_beef] {
            tree.clear();
            assert_tree_invariants(&tree, 0);
            assert_eq!(tree.max_order(), 0);
            assert_eq!(tree.order_floor, 0);
            assert!(tree.insert_path.is_empty());
            assert!(tree.search_stack.is_empty());
            let mut reference: Vec<(Bounds<f32>, u32)> = Vec::new();
            let mut floor = 0;
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            for i in 0..2048 {
                if i == 384 || i == 1280 {
                    floor = reference.iter().map(|(_, order)| *order).max().unwrap_or(0) + 32;
                    tree.set_order_floor(floor);
                    tree.set_order_floor(floor - 1);
                }
                let bounds = match i % 8 {
                    0 => rect((2 * i) as f32, 1000.0, 1.0, 1.0),
                    1 => rect(-((2 * i) as f32), -1000.0, 1.0, 1.0),
                    2 => rect(0.0, 0.0, 10.0, 10.0),
                    3 => rect(
                        rng.random_range(-20.0..20.0),
                        rng.random_range(-20.0..20.0),
                        100.0,
                        100.0,
                    ),
                    4 => rect(
                        rng.random_range(-10000.0..10000.0),
                        rng.random_range(-10000.0..10000.0),
                        rng.random_range(0.0..8.0),
                        rng.random_range(0.0..8.0),
                    ),
                    5 => rect(
                        rng.random_range(-128.0..128.0),
                        rng.random_range(-128.0..128.0),
                        rng.random_range(0.0..64.0),
                        rng.random_range(0.0..64.0),
                    ),
                    6 => rect(rng.random_range(-20.0..20.0), 5.0, 0.0, 10.0),
                    _ => reference[(i * 17) % reference.len()].0,
                };
                assert_reference_insert(&mut tree, &mut reference, bounds, floor, i % 61 == 0);
                if i % 64 == 63 {
                    assert_tree_invariants(&tree, reference.len());
                    let query = rect(
                        rng.random_range(-128.0..128.0),
                        rng.random_range(-128.0..128.0),
                        rng.random_range(0.0..128.0),
                        rng.random_range(0.0..128.0),
                    );
                    assert_reference_query(&mut tree, &reference, query);
                    let query = reference[(i * 19) % reference.len()].0;
                    assert_reference_query(&mut tree, &reference, query);
                }
            }
        }
    }

    #[test]
    fn test_split_tree_duplicate_and_touching_bounds() {
        let mut tree = BoundsTree::<f32>::default();
        for i in 0..96 {
            assert_eq!(tree.insert(rect(i as f32, 0.0, 1.0, 1.0)), 1);
        }
        for i in (0..96).rev() {
            assert_eq!(tree.insert(rect(i as f32, 0.0, 1.0, 1.0)), 2);
        }
        assert_eq!(tree.insert(rect(0.5, 0.0, 1.0, 1.0)), 3);
        assert_eq!(tree.insert(rect(96.0, 1.0, 1.0, 1.0)), 1);
        assert_eq!(tree.find_max_ordering(&rect(0.0, 1.0, 96.0, 1.0)), 0);
        assert_eq!(tree.find_max_ordering(&rect(0.0, 0.0, 1.0, 1.0)), 3);
        assert_tree_invariants(&tree, 194);
    }

    #[test]
    fn test_union_rounding_does_not_clip_descendant_bounds() {
        let mut tree = BoundsTree::<f32>::default();
        let mut reference = Vec::new();
        let right_edge = 1.0 / 4096.0;
        let left = rect(-4096.0, 0.0, 1.0, 1.0);
        let right = rect(0.0, 0.0, right_edge, 1.0);
        // 4096 + 2^-12 is halfway between f32 values and rounds to 4096.
        // Rebuilding the root's right endpoint from its size therefore yields 0,
        // which used to prune right even though its actual endpoint is positive.
        assert_reference_insert(&mut tree, &mut reference, left, 0, false);
        assert_reference_insert(&mut tree, &mut reference, right, 0, false);
        let root = &tree.nodes[tree.root.unwrap()];
        assert_eq!(root.bounds.bottom_right().x, 0.0);
        assert_eq!(root.max_corner.x, right_edge);
        assert_reference_query(&mut tree, &reference, right);
        assert_reference_insert(&mut tree, &mut reference, right, 0, false);
        assert_reference_query(
            &mut tree,
            &reference,
            rect(right_edge / 2.0, 0.0, right_edge / 2.0, 1.0),
        );
        assert_reference_query(
            &mut tree,
            &reference,
            rect(right_edge, 0.0, right_edge, 1.0),
        );
        assert_tree_invariants(&tree, reference.len());
    }

    #[test]
    fn test_order_floor_above_all_and_clear() {
        let mut tree = BoundsTree::<f32>::default();
        assert_eq!(tree.insert(rect(0.0, 0.0, 1.0, 1.0)), 1);
        tree.set_order_floor(100);
        // Preserve insert_above_all's global-max semantics, independent of floor.
        assert_eq!(tree.insert_above_all(rect(2.0, 0.0, 1.0, 1.0)), 2);
        assert_eq!(tree.insert(rect(4.0, 0.0, 1.0, 1.0)), 100);
        tree.set_order_floor(20);
        assert_eq!(tree.insert(rect(6.0, 0.0, 1.0, 1.0)), 100);
        assert_eq!(tree.insert(rect(4.0, 0.0, 1.0, 1.0)), 101);
        assert_eq!(tree.max_order(), 101);
        assert_tree_invariants(&tree, 5);
        tree.clear();
        assert_tree_invariants(&tree, 0);
        assert_eq!(tree.max_order(), 0);
        assert_eq!(tree.order_floor, 0);
        assert!(tree.insert_path.is_empty());
        assert!(tree.search_stack.is_empty());
        assert_eq!(tree.insert(rect(4.0, 0.0, 1.0, 1.0)), 1);
        assert_tree_invariants(&tree, 1);
    }

    #[test]
    fn test_random_iterations() {
        let max_bounds = 100;
        for seed in 1..=1000 {
            // let seed = 44;
            let mut tree = BoundsTree::default();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed as u64);
            let mut expected_quads: Vec<(Bounds<f32>, u32)> = Vec::new();

            // Insert a random number of random AABBs into the tree.
            let num_bounds = rng.random_range(1..=max_bounds);
            for _ in 0..num_bounds {
                let min_x: f32 = rng.random_range(-100.0..100.0);
                let min_y: f32 = rng.random_range(-100.0..100.0);
                let width: f32 = rng.random_range(0.0..50.0);
                let height: f32 = rng.random_range(0.0..50.0);
                let bounds = Bounds {
                    origin: Point { x: min_x, y: min_y },
                    size: Size { width, height },
                };

                let expected_ordering = expected_quads
                    .iter()
                    .filter_map(|quad| quad.0.intersects(&bounds).then_some(quad.1))
                    .max()
                    .unwrap_or(0)
                    + 1;
                expected_quads.push((bounds, expected_ordering));

                // Insert the AABB into the tree and collect intersections.
                let actual_ordering = tree.insert(bounds);
                assert_eq!(actual_ordering, expected_ordering);
            }
        }
    }
}
