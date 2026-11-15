//! Recursive refinement of range endpoints without changing the resolved Range type.

use crate::{IsEmpty, Refineable};
use std::ops::Range;

/// Partial updates to a range. An absent endpoint is left unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RangeRefinement<T> {
    /// An explicit update of the start endpoint.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub start: Option<T>,
    /// An explicit update of the end endpoint.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub end: Option<T>,
}

impl<T> IsEmpty for RangeRefinement<T> {
    fn is_empty(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }
}

impl<T> RangeRefinement<T> {
    /// Whether at least one endpoint contains an update.
    pub fn is_some(&self) -> bool {
        !self.is_empty()
    }
}

impl<T> From<Range<T>> for RangeRefinement<T> {
    fn from(range: Range<T>) -> Self {
        Self {
            start: Some(range.start),
            end: Some(range.end),
        }
    }
}

impl<T: Default> From<RangeRefinement<T>> for Range<T> {
    fn from(refinement: RangeRefinement<T>) -> Self {
        refinement.start.unwrap_or_default()..refinement.end.unwrap_or_default()
    }
}

impl<T: Clone + Default + PartialEq> Refineable for Range<T> {
    type Refinement = RangeRefinement<T>;

    fn refine(&mut self, refinement: &Self::Refinement) {
        if let Some(start) = &refinement.start {
            self.start = start.clone();
        }
        if let Some(end) = &refinement.end {
            self.end = end.clone();
        }
    }

    fn refined(mut self, refinement: Self::Refinement) -> Self {
        self.refine(&refinement);
        self
    }

    fn is_superset_of(&self, refinement: &Self::Refinement) -> bool {
        refinement
            .start
            .as_ref()
            .is_none_or(|start| start == &self.start)
            && refinement.end.as_ref().is_none_or(|end| end == &self.end)
    }

    fn subtract(&self, refinement: &Self::Refinement) -> Self::Refinement {
        RangeRefinement {
            start: if refinement.start.as_ref() == Some(&self.start) {
                None
            } else {
                Some(self.start.clone())
            },
            end: if refinement.end.as_ref() == Some(&self.end) {
                None
            } else {
                Some(self.end.clone())
            },
        }
    }
}

impl<T: Clone + Default + PartialEq> Refineable for RangeRefinement<T> {
    type Refinement = Self;

    fn refine(&mut self, refinement: &Self) {
        if let Some(start) = &refinement.start {
            self.start = Some(start.clone());
        }
        if let Some(end) = &refinement.end {
            self.end = Some(end.clone());
        }
    }

    fn refined(mut self, refinement: Self) -> Self {
        self.refine(&refinement);
        self
    }

    fn is_superset_of(&self, refinement: &Self) -> bool {
        (refinement.start.is_none() || self.start == refinement.start)
            && (refinement.end.is_none() || self.end == refinement.end)
    }

    fn subtract(&self, refinement: &Self) -> Self {
        Self {
            start: if self.start == refinement.start {
                None
            } else {
                self.start.clone()
            },
            end: if self.end == refinement.end {
                None
            } else {
                self.end.clone()
            },
        }
    }
}
