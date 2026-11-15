use refineable::{IsEmpty, RangeRefinement, Refineable};
use std::ops::Range;

#[derive(Clone, Default, Refineable)]
struct Container {
    #[refineable]
    range: Option<Range<u16>>,
}

#[test]
fn range_refinement_preserves_the_std_range_type_and_other_endpoint() {
    let original: Range<u16> = 2..5;
    let mut result = original.clone();
    result.refine(&RangeRefinement {
        start: Some(3),
        end: None,
    });
    let _: Range<u16> = result.clone();
    assert_eq!(result, 3..5);
    assert_eq!(
        original.refined(RangeRefinement {
            start: Some(3),
            end: None
        }),
        result
    );
    assert!(result.is_superset_of(&RangeRefinement {
        start: Some(3),
        end: None
    }));
    assert_eq!(
        result.subtract(&RangeRefinement {
            start: Some(3),
            end: Some(7)
        }),
        RangeRefinement {
            start: None,
            end: Some(5)
        }
    );
}

#[test]
fn sparse_range_merge_and_subtract_use_endpoint_updates() {
    let patch = RangeRefinement {
        start: Some(2u16),
        end: Some(5),
    };
    let merged = patch.refined(RangeRefinement {
        start: Some(3),
        end: None,
    });
    assert_eq!(
        merged,
        RangeRefinement {
            start: Some(3),
            end: Some(5)
        }
    );
    assert_eq!(
        merged.subtract(&RangeRefinement {
            start: Some(3),
            end: Some(7)
        }),
        RangeRefinement {
            start: None,
            end: Some(5)
        }
    );
    assert!(merged.subtract(&merged).is_empty());
    assert!(!RangeRefinement::<u16>::default().is_some());
}

#[test]
fn derived_optional_ranges_create_defaults_and_merge_partial_updates() {
    let mut container: Container = ContainerRefinement {
        range: Some(RangeRefinement {
            start: Some(2),
            end: None,
        }),
    }
    .into();
    assert_eq!(container.range, Some(2..0));
    container.refine(&ContainerRefinement {
        range: Some(RangeRefinement {
            start: None,
            end: Some(5),
        }),
    });
    assert_eq!(container.range, Some(2..5));
    let complete: RangeRefinement<u16> = (3..7).into();
    assert_eq!(
        complete,
        RangeRefinement {
            start: Some(3),
            end: Some(7)
        }
    );
}
