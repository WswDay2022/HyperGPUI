//! Exercises the public factory interface exactly as a consumer crate uses it.

use hgpui::{
    GridMaxTrackSizingFunction as Max, GridMinTrackSizingFunction as Min,
    GridRepetitionCount as Count, GridTemplateComponent as Component,
    GridTemplateDefinition as Definition, GridTemplateRepetition as Repetition,
    GridTrackSizingFunction as Sizing, StyleRefinement, Styled, div, px, relative, rems, repeat,
    track,
};

fn style(mut element: impl Styled) -> StyleRefinement {
    element.style().clone()
}

#[test]
fn track_factory_selectors_match_existing_typed_constructors() {
    assert_eq!(track(), Component::Single(Sizing::auto()));
    assert_eq!(
        track().size(px(40.)),
        Component::Single(Sizing::length(px(40.)))
    );
    assert_eq!(
        track().length(rems(2.)),
        Component::Single(Sizing::length(rems(2.)))
    );
    assert_eq!(
        track().size(relative(0.25)),
        Component::Single(Sizing::length(relative(0.25)))
    );
    assert_eq!(track().fr(1.), Component::Single(Sizing::fr(1.)));
    assert_eq!(track().flex(2.), Component::Single(Sizing::flex(2.)));
    assert_eq!(track().auto(), Component::Single(Sizing::auto()));
    assert_eq!(
        track().min_content(),
        Component::Single(Sizing::min_content())
    );
    assert_eq!(
        track().max_content(),
        Component::Single(Sizing::max_content())
    );
    assert_eq!(
        track().fit_content(px(80.)),
        Component::Single(Sizing::fit_content(px(80.)))
    );
}

#[test]
fn bare_repeat_can_be_passed_directly_to_template_rows() {
    let actual = style(
        div()
            .grid()
            .grid_template_rows(repeat(3, [track().size(px(40.))])),
    );
    let expected = style(
        div()
            .grid()
            .grid_template_rows([Component::Repeat(Repetition::new(
                3u16,
                [Sizing::length(px(40.))],
            ))]),
    );
    assert_eq!(actual, expected);
}

#[test]
fn tracks_and_repeats_mix_without_enum_wrappers_or_casts() {
    let template = [
        track().size(px(120.)),
        repeat(2, [track().fr(1.)]),
        track().auto(),
    ];
    let actual = style(div().grid_template_cols(template));
    let expected = style(div().grid_template_columns([
        Component::Single(Sizing::length(px(120.))),
        Component::Repeat(Repetition::new(2u16, [Sizing::fr(1.)])),
        Component::Single(Sizing::auto()),
    ]));
    assert_eq!(actual, expected);
}

#[test]
fn bare_track_can_be_used_as_a_singleton_template() {
    assert_eq!(
        style(div().grid_template_cols(track().fr(1.))),
        style(div().grid_template_columns([Sizing::fr(1.)]))
    );
}

#[test]
fn minmax_accepts_lengths_and_factory_bounds() {
    assert_eq!(
        track().minmax(px(120.), track().fr(1.)),
        Component::Single(Sizing::minmax(
            Min::Length(px(120.).into()),
            Max::Fraction(1.)
        ))
    );
    assert_eq!(
        track().minmax(track().min_content(), track().fit_content(relative(0.5))),
        Component::Single(Sizing::minmax(
            Min::MinContent,
            Max::FitContent(relative(0.5).into())
        ))
    );
    assert_eq!(
        track().fr(2.).min(px(80.)),
        Component::Single(Sizing::minmax(
            Min::Length(px(80.).into()),
            Max::Fraction(2.)
        ))
    );
    assert_eq!(
        track().size(px(40.)).max(track().fr(2.)),
        Component::Single(Sizing::minmax(
            Min::Length(px(40.).into()),
            Max::Fraction(2.)
        ))
    );
}

#[test]
fn repeat_accepts_integer_and_automatic_counts() {
    for count in [Count::AutoFill, Count::AutoFit] {
        assert_eq!(
            repeat(count, [track().minmax(px(130.), track().fr(1.))]),
            Component::Repeat(Repetition::new(
                count,
                [Sizing::minmax(
                    Min::Length(px(130.).into()),
                    Max::Fraction(1.),
                )]
            ))
        );
    }
    assert_eq!(repeat(3, [track().fr(1.)]), repeat(3u16, [Sizing::fr(1.)]));
    assert_eq!(
        repeat(3usize, [track().fr(1.)]),
        repeat(Count::Count(3), [track().fr(1.)])
    );
}

#[test]
fn implicit_tracks_accept_the_same_factory_values() {
    let actual = style(
        div()
            .grid_auto_rows([track().size(px(40.)), track().fr(1.)])
            .grid_auto_columns([track().min_content(), track().fit_content(px(80.))]),
    );
    let expected = style(
        div()
            .grid_auto_rows([Sizing::length(px(40.)), Sizing::fr(1.)])
            .grid_auto_columns([Sizing::min_content(), Sizing::fit_content(px(80.))]),
    );
    assert_eq!(actual, expected);
}

#[test]
fn template_cols_alias_preserves_legacy_override_order() {
    assert_eq!(
        style(div().grid_template_cols(track().size(px(40.))).grid_cols(3)),
        style(div().grid_cols(3))
    );
    assert_eq!(
        style(div().grid_cols(3).grid_template_cols(track().size(px(40.)))),
        style(div().grid_template_columns(track().size(px(40.))))
    );
}

#[test]
fn factory_values_preserve_the_existing_serializable_model() {
    let definition: Definition = repeat(3, [track().size(rems(2.)), track().fr(1.)]).into();
    let json = serde_json::to_string(&definition).unwrap();
    let decoded: Definition = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, definition);
}

#[test]
#[should_panic(expected = "expected a single Grid track")]
fn nested_repeat_is_rejected() {
    let _ = repeat(2, [repeat(3, [track().size(px(40.))])]);
}

#[test]
#[should_panic(expected = "expected a single Grid track")]
fn repeat_is_rejected_in_implicit_tracks() {
    let _ = div().grid_auto_rows([repeat(3, [track().size(px(40.))])]);
}

#[test]
#[should_panic(expected = "Grid repeat count must be between")]
fn negative_repeat_count_is_not_truncated() {
    let _ = repeat(-1, [track()]);
}

#[test]
#[should_panic(expected = "Grid repeat count must be between")]
fn oversized_repeat_count_is_not_truncated() {
    let _ = repeat(usize::MAX, [track()]);
}
