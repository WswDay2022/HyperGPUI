use refineable::{Cascade, IsEmpty, Refineable};

#[derive(Clone, Default, Debug, PartialEq, Refineable)]
#[refineable(Debug, PartialEq)]
struct Details {
    first: u32,
    second: u32,
}

#[derive(Clone, Default, Debug, PartialEq, Refineable)]
#[refineable(Debug, PartialEq)]
struct Container {
    #[refineable]
    nested: Option<Details>,
    // Ordinary optional fields must keep their existing atomic replacement.
    atomic: Option<Details>,
}

fn patch(first: Option<u32>, second: Option<u32>) -> ContainerRefinement {
    ContainerRefinement {
        nested: Some(DetailsRefinement { first, second }),
        atomic: None,
    }
}

#[test]
fn optional_nested_fields_merge_without_changing_atomic_fields() {
    let original = Container {
        nested: Some(Details {
            first: 1,
            second: 2,
        }),
        atomic: Some(Details {
            first: 1,
            second: 2,
        }),
    };
    let update = ContainerRefinement {
        nested: patch(Some(3), None).nested,
        atomic: Some(Details {
            first: 3,
            second: 0,
        }),
    };
    let mut merged = original.clone();
    merged.refine(&update);
    assert_eq!(
        merged.nested,
        Some(Details {
            first: 3,
            second: 2
        })
    );
    assert_eq!(
        merged.atomic,
        Some(Details {
            first: 3,
            second: 0
        })
    );
    assert_eq!(original.refined(update), merged);
}

#[test]
fn absent_and_empty_nested_updates_do_not_create_values() {
    let empty = patch(None, None);
    assert!(empty.is_empty());
    assert!(!empty.is_some());
    assert!(Container::default().is_superset_of(&empty));
    assert_eq!(
        Container::default().refined(empty.clone()),
        Container::default()
    );
    let from: Container = empty.into();
    assert_eq!(from, Container::default());
    let populated = Container {
        nested: Some(Details {
            first: 4,
            second: 5,
        }),
        atomic: None,
    };
    assert_eq!(populated.clone().refined(patch(None, None)), populated);
    assert_eq!(
        populated.clone().refined(ContainerRefinement::default()),
        populated
    );
}

#[test]
fn nested_updates_create_only_when_a_field_is_present() {
    let result = Container::default().refined(patch(Some(7), None));
    assert_eq!(
        result.nested,
        Some(Details {
            first: 7,
            second: 0
        })
    );
    let from: Container = patch(Some(7), None).into();
    assert_eq!(from, result);
    // A value equal to Default is still an explicit update, not an empty patch.
    let explicit_zero: Container = patch(Some(0), None).into();
    assert_eq!(explicit_zero.nested, Some(Details::default()));
}

#[test]
fn optional_nested_superset_and_subtract_follow_the_partial_patch() {
    let base = Container {
        nested: Some(Details {
            first: 1,
            second: 2,
        }),
        atomic: None,
    };
    assert!(base.is_superset_of(&patch(Some(1), None)));
    assert!(!base.is_superset_of(&patch(Some(1), Some(3))));
    let remaining = base.subtract(&patch(Some(1), Some(3)));
    // subtract keeps the left-hand values that the right-hand patch does not match.
    assert_eq!(remaining, patch(None, Some(2)));
    assert!(base.subtract(&patch(Some(1), Some(2))).is_empty());
    assert!(
        Container::default()
            .subtract(&patch(Some(1), None))
            .is_empty()
    );
    assert_eq!(
        base.subtract(&ContainerRefinement::default()),
        patch(Some(1), Some(2))
    );
    assert_eq!(base.subtract(&patch(None, None)), patch(Some(1), Some(2)));
}

#[test]
fn refinement_to_refinement_merge_preserves_unmentioned_fields() {
    let merged = patch(Some(1), Some(2)).refined(patch(Some(3), None));
    assert_eq!(merged, patch(Some(3), Some(2)));
    assert!(merged.is_superset_of(&patch(None, Some(2))));
    assert_eq!(
        merged.subtract(&patch(Some(3), Some(4))),
        patch(None, Some(2))
    );
}

#[test]
fn optional_nested_creation_uses_the_struct_default_not_field_defaults() {
    #[derive(Clone, Debug, PartialEq, Refineable)]
    #[refineable(Debug, PartialEq)]
    struct CustomDefault {
        first: u32,
        second: u32,
    }
    impl Default for CustomDefault {
        fn default() -> Self {
            Self {
                first: 0,
                second: 99,
            }
        }
    }
    #[derive(Clone, Default, Refineable)]
    struct Holder {
        #[refineable]
        nested: Option<CustomDefault>,
    }
    let from: Holder = HolderRefinement {
        nested: Some(CustomDefaultRefinement {
            first: Some(7),
            second: None,
        }),
    }
    .into();
    assert_eq!(
        from.nested,
        Some(CustomDefault {
            first: 7,
            second: 99
        })
    );
}

#[test]
fn cascade_merging_uses_recursive_optional_refinement() {
    let mut cascade = Cascade::<Container>::default();
    *cascade.base() = patch(Some(1), Some(2));
    let slot = cascade.reserve();
    cascade.set(slot, Some(patch(Some(3), None)));
    assert_eq!(
        Container::from_cascade(&cascade).nested,
        Some(Details {
            first: 3,
            second: 2
        })
    );
}
