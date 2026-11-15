//! Regression tests across the Styled -> Style -> Taffy seam.

use super::ToTaffy;
use crate::{
    AlignContent, AlignItems, BlockTextAlign, BoxSizing, ClearFloats, FloatSide, GridAutoFlow,
    GridPlacement, GridRepetitionCount, GridTemplateArea, GridTemplateComponent,
    GridTemplateDefinition, GridTemplateRepetition, GridTemplateTracks,
    GridTrackSizingFunction as Track, LayoutDirection, Refineable, Style, StyleRefinement, Styled,
    div, px, relative, rems,
};

#[crate::test]
fn complete_grid_reaches_window_layout_and_scene(cx: &mut crate::TestAppContext) {
    use crate::{AppContext, Context, IntoElement, ParentElement, Render, Window, rgb};

    struct GridView;
    impl Render for GridView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .grid()
                .w(px(600.))
                .h(px(100.))
                .grid_template_columns([Track::length(px(120.)), Track::flex(1.), Track::flex(2.)])
                .children((0..3).map(|_| div().background(rgb(0x336699))))
        }
    }

    let window: crate::AnyWindowHandle = cx
        .open_window(crate::size(px(640.), px(120.)), |_, _| GridView)
        .into();
    cx.update_window(window, |_, window, cx| {
        window.draw(cx).clear(cx);
        let scale = window.scale_factor();
        let mut bounds: Vec<_> = window
            .rendered_frame
            .scene
            .quads
            .iter()
            .map(|quad| (quad.bounds.origin.x.0, quad.bounds.size.width.0))
            .collect();
        bounds.sort_by(|left, right| left.0.total_cmp(&right.0));
        assert_eq!(
            bounds,
            vec![
                (0., 120. * scale),
                (120. * scale, 160. * scale),
                (280. * scale, 320. * scale)
            ]
        );
    })
    .unwrap();
}

fn style(mut value: impl Styled) -> Style {
    Style::default().refined(value.style().clone())
}

fn single(track: Track) -> GridTemplateComponent {
    track.into()
}

fn layout(root: Style, children: Vec<Style>) -> Vec<taffy::Layout> {
    let mut tree = taffy::TaffyTree::<()>::new();
    tree.disable_rounding();
    let children: Vec<_> = children
        .into_iter()
        .map(|child| tree.new_leaf(child.to_taffy(px(16.), 1.)).unwrap())
        .collect();
    let root = tree
        .new_with_children(root.to_taffy(px(16.), 1.), &children)
        .unwrap();
    tree.compute_layout(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::MaxContent,
            height: taffy::AvailableSpace::MaxContent,
        },
    )
    .unwrap();
    children
        .iter()
        .map(|id| tree.layout(*id).unwrap().clone())
        .collect()
}

#[test]
fn old_and_new_templates_share_call_and_cascade_order() {
    let custom = vec![single(Track::length(px(80.)))];
    let legacy = style(div().grid_cols(3));
    assert_eq!(
        style(div().grid_template_columns(custom.clone()).grid_cols(3)).grid_cols,
        legacy.grid_cols,
    );
    let custom_style = style(div().grid_template_columns(custom.clone()));
    assert_eq!(
        style(div().grid_cols(3).grid_template_columns(custom.clone())).grid_cols,
        custom_style.grid_cols,
    );
    let mut base = custom_style.clone();
    base.refine(&div().grid_cols(3).style().clone());
    assert_eq!(base.grid_cols, legacy.grid_cols);
    base.refine(&div().grid_template_columns(custom).style().clone());
    assert_eq!(base.grid_cols, custom_style.grid_cols);
}

#[test]
fn empty_template_is_an_explicit_cascade_reset() {
    let mut base = style(div().grid_cols(3).grid_rows(4));
    base.refine(
        &div()
            .grid_template_columns(Vec::<GridTemplateComponent>::new())
            .style()
            .clone(),
    );
    let converted = base.to_taffy(px(16.), 1.);
    assert!(converted.grid_template_columns.is_empty());
    assert!(!converted.grid_template_rows.is_empty());
}

#[test]
fn mixed_tracks_allocate_fixed_and_weighted_remaining_space() {
    let root = style(div().grid().w(px(600.)).grid_template_columns([
        single(Track::length(px(120.))),
        single(Track::flex(1.)),
        single(Track::flex(2.)),
    ]));
    let children = layout(root, vec![Style::default(); 3]);
    assert_eq!(children[0].size.width, 120.);
    assert_eq!(children[1].size.width, 160.);
    assert_eq!(children[2].size.width, 320.);
    assert_eq!(children[2].location.x, 280.);
}

#[test]
fn logical_units_scale_once_and_percentages_remain_relative() {
    let root = style(div().grid().grid_template_columns([
        single(Track::length(px(10.))),
        single(Track::length(rems(2.))),
        single(Track::length(relative(0.25))),
        single(Track::fr(2.)),
    ]));
    let converted = root.to_taffy(px(16.), 1.5);
    assert_eq!(
        converted.grid_template_columns,
        vec![
            taffy::GridTemplateComponent::Single(taffy::style_helpers::length(15_f32)),
            taffy::GridTemplateComponent::Single(taffy::style_helpers::length(48_f32)),
            taffy::GridTemplateComponent::Single(taffy::style_helpers::percent(0.25_f32)),
            taffy::GridTemplateComponent::Single(taffy::style_helpers::fr(2_f32)),
        ]
    );
}

#[test]
fn auto_fit_collapses_empty_tracks_but_auto_fill_keeps_them() {
    let make = |count| {
        style(
            div()
                .grid()
                .w(px(500.))
                .grid_template_columns([GridTemplateComponent::Repeat(
                    GridTemplateRepetition::new(
                        count,
                        [Track::minmax(
                            crate::GridMinTrackSizingFunction::Length(px(100.).into()),
                            crate::GridMaxTrackSizingFunction::Fraction(1.),
                        )],
                    ),
                )]),
        )
    };
    let fill = layout(
        make(GridRepetitionCount::AutoFill),
        vec![Style::default(); 2],
    );
    let fit = layout(
        make(GridRepetitionCount::AutoFit),
        vec![Style::default(); 2],
    );
    assert_eq!(fill[0].size.width, 100.);
    assert_eq!(fill[1].location.x, 100.);
    assert_eq!(fit[0].size.width, 250.);
    assert_eq!(fit[1].location.x, 250.);
}

#[test]
fn dense_placement_fills_holes_without_reordering_children() {
    let make = |flow| {
        style(
            div()
                .grid()
                .grid_cols(3)
                .w(px(300.))
                .grid_auto_rows([Track::length(px(10.))])
                .grid_auto_flow(flow),
        )
    };
    let children = || {
        vec![
            style(div().col_span(2)),
            style(div().col_span(2)),
            Style::default(),
        ]
    };
    let sparse = layout(make(GridAutoFlow::Row), children());
    let dense = layout(make(GridAutoFlow::RowDense), children());
    assert_eq!((sparse[2].location.x, sparse[2].location.y), (200., 10.));
    assert_eq!((dense[2].location.x, dense[2].location.y), (200., 0.));
    assert_eq!(dense[1].location.y, 10.);
}

#[test]
fn column_auto_flow_fills_down_before_across() {
    let root = style(
        div()
            .grid()
            .grid_rows(2)
            .h(px(40.))
            .grid_auto_columns([Track::length(px(30.))])
            .grid_auto_flow(GridAutoFlow::Column),
    );
    let children = layout(root, vec![Style::default(); 3]);
    assert_eq!((children[1].location.x, children[1].location.y), (0., 20.));
    assert_eq!((children[2].location.x, children[2].location.y), (30., 0.));
}

#[test]
fn implicit_tracks_cycle_and_numeric_negative_lines_still_work() {
    let root = style(
        div()
            .grid()
            .grid_template_columns([single(Track::length(px(100.)))])
            .grid_auto_columns([Track::length(px(30.)), Track::length(px(50.))]),
    );
    let children = layout(
        root,
        vec![style(
            div().grid_column(GridPlacement::Line(3)..GridPlacement::Line(4)),
        )],
    );
    assert_eq!(children[0].location.x, 130.);
    assert_eq!(children[0].size.width, 50.);
    let root = style(div().grid().grid_cols(3).w(px(300.)));
    let children = layout(root, vec![style(div().col_span_full())]);
    assert_eq!(children[0].size.width, 300.);
}

#[test]
fn named_area_resolves_both_axes() {
    let root = style(
        div()
            .grid()
            .grid_cols(2)
            .grid_rows(2)
            .w(px(200.))
            .h(px(40.))
            .grid_template_areas([GridTemplateArea {
                name: "main".into(),
                row_start: 1,
                row_end: 3,
                column_start: 1,
                column_end: 3,
            }]),
    );
    let children = layout(root, vec![style(div().grid_area("main"))]);
    assert_eq!(children[0].size.width, 200.);
    assert_eq!(children[0].size.height, 40.);
    assert_eq!(children[0].location, taffy::Point::ZERO);
}

#[test]
fn named_lines_and_named_spans_resolve_repeated_occurrences() {
    let tracks = GridTemplateTracks::new([
        single(Track::length(px(100.))),
        single(Track::length(px(100.))),
    ])
    .with_line_names(vec![
        vec!["edge".into()],
        vec!["edge".into()],
        vec!["edge".into()],
    ]);
    let root = style(div().grid().grid_template_columns(tracks));
    let children = layout(
        root,
        vec![style(div().grid_column(
            GridPlacement::NamedLine("edge".into(), 2)..GridPlacement::NamedSpan("edge".into(), 1),
        ))],
    );
    assert_eq!(children[0].location.x, 100.);
    assert_eq!(children[0].size.width, 100.);
}

#[test]
fn grid_cell_alignment_is_distinct_from_track_distribution() {
    let root = style(
        div()
            .grid()
            .grid_cols(1)
            .grid_rows(1)
            .w(px(100.))
            .h(px(100.))
            .justify_items(AlignItems::Center)
            .align_items(AlignItems::End),
    );
    let children = layout(root, vec![style(div().w(px(20.)).h(px(10.)))]);
    assert_eq!(children[0].location.x, 40.);
    assert_eq!(children[0].location.y, 90.);
}

#[test]
fn safe_center_preserves_the_start_edge_on_overflow() {
    let root = style(
        div()
            .grid()
            .grid_template_columns([single(Track::length(px(100.)))]),
    );
    let make = |alignment| vec![style(div().w(px(150.)).justify_self(alignment))];
    let unsafe_layout = layout(root.clone(), make(AlignItems::Center));
    let safe_layout = layout(root, make(AlignItems::SafeCenter));
    assert_eq!(unsafe_layout[0].location.x, -25.);
    assert_eq!(safe_layout[0].location.x, 0.);
}

#[test]
fn rtl_grid_mirrors_columns() {
    let root = style(
        div()
            .grid()
            .grid_cols(2)
            .w(px(200.))
            .layout_direction(LayoutDirection::Rtl),
    );
    let children = layout(root, vec![Style::default(); 2]);
    assert_eq!(children[0].location.x, 100.);
    assert_eq!(children[1].location.x, 0.);
}

#[test]
fn shared_layout_fields_are_forwarded_without_changing_defaults() {
    let default = Style::default().to_taffy(px(16.), 1.);
    assert_eq!(default.box_sizing, taffy::BoxSizing::BorderBox);
    assert_eq!(default.direction, taffy::Direction::Ltr);
    assert!(!default.item_is_replaced && !default.item_is_table);
    let configured = style(
        div()
            .box_sizing(BoxSizing::ContentBox)
            .layout_direction(LayoutDirection::Rtl)
            .item_is_replaced(true)
            .item_is_table(true)
            .float(FloatSide::Right)
            .clear(ClearFloats::Both)
            .block_text_align(BlockTextAlign::LegacyCenter)
            .align_content(AlignContent::SafeCenter),
    );
    let converted = configured.to_taffy(px(16.), 1.);
    assert_eq!(converted.box_sizing, taffy::BoxSizing::ContentBox);
    assert_eq!(converted.direction, taffy::Direction::Rtl);
    assert!(converted.item_is_replaced && converted.item_is_table);
    assert_eq!(converted.float, taffy::Float::Right);
    assert_eq!(converted.clear, taffy::Clear::Both);
    assert_eq!(converted.text_align, taffy::TextAlign::LegacyCenter);
    assert_eq!(
        converted.align_content,
        Some(taffy::AlignContent::SAFE_CENTER)
    );
}

#[test]
fn grid_style_json_preserves_legacy_shape_and_round_trips_new_fields() {
    let old: StyleRefinement =
        serde_json::from_str(r#"{"grid_cols":{"repeat":3,"min_size":"Zero"}}"#).unwrap();
    assert_eq!(old.grid_cols, style(div().grid_cols(3)).grid_cols);
    let value = serde_json::to_value(&old).unwrap();
    assert_eq!(
        value["grid_cols"],
        serde_json::json!({"repeat":3,"min_size":"Zero"})
    );
    let new = div()
        .grid_template_columns([single(Track::fit_content(px(80.)))])
        .grid_auto_flow(GridAutoFlow::ColumnDense)
        .grid_area("main")
        .justify_self(AlignItems::SafeCenter)
        .style()
        .clone();
    let encoded = serde_json::to_string(&new).unwrap();
    let decoded: StyleRefinement = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, new);
    let _: GridTemplateDefinition = new.grid_cols.unwrap();
}

#[test]
fn named_boundary_after_an_unnamed_repeat_resolves_correctly() {
    let tracks = GridTemplateTracks::new([
        GridTemplateComponent::Repeat(GridTemplateRepetition::new(2u16, [Track::length(px(100.))])),
        single(Track::length(px(80.))),
    ])
    .with_line_names(vec![vec![], vec!["after-repeat".into()]]);
    let root = style(div().grid().grid_template_columns(tracks));
    let children = layout(
        root,
        vec![style(div().grid_column(
            GridPlacement::NamedLine("after-repeat".into(), 1)..GridPlacement::Auto,
        ))],
    );
    assert_eq!(children[0].location.x, 200.);
    assert_eq!(children[0].size.width, 80.);
}

#[test]
fn intrinsic_and_fit_content_tracks_use_the_host_measure_function() {
    let mut tree = taffy::TaffyTree::<(f32, f32)>::new();
    tree.disable_rounding();
    let mut children = Vec::new();
    for _ in 0..3 {
        children.push(
            tree.new_leaf_with_context(Style::default().to_taffy(px(16.), 1.), (30., 100.))
                .unwrap(),
        );
    }
    let root_style = style(div().grid().grid_template_columns([
        Track::min_content(),
        Track::max_content(),
        Track::fit_content(px(80.)),
    ]));
    let root = tree
        .new_with_children(root_style.to_taffy(px(16.), 1.), &children)
        .unwrap();
    tree.compute_layout_with_measure(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::MaxContent,
            height: taffy::AvailableSpace::MaxContent,
        },
        |known, available, _, context, _| {
            let (min, max) = context.copied().unwrap_or_default();
            let width = known.width.unwrap_or(match available.width {
                taffy::AvailableSpace::MinContent => min,
                taffy::AvailableSpace::MaxContent => max,
                taffy::AvailableSpace::Definite(width) => width.max(min).min(max),
            });
            taffy::Size {
                width,
                height: known.height.unwrap_or(10.),
            }
        },
    )
    .unwrap();
    let widths: Vec<_> = children
        .iter()
        .map(|id| tree.layout(*id).unwrap().size.width)
        .collect();
    assert_eq!(widths, vec![30., 100., 80.]);
}

#[test]
fn percentage_layout_borders_can_be_overridden_and_reset() {
    let widths = crate::Edges {
        top: relative(0.1).into(),
        right: px(4.).into(),
        bottom: rems(1.).into(),
        left: relative(0.2).into(),
    };
    let mut configured = style(div().border_1().layout_border(widths));
    let converted = configured.to_taffy(px(16.), 1.5);
    assert_eq!(converted.border.top, taffy::LengthPercentage::percent(0.1));
    assert_eq!(converted.border.right, taffy::LengthPercentage::length(6.));
    assert_eq!(
        converted.border.bottom,
        taffy::LengthPercentage::length(24.)
    );
    let encoded = serde_json::to_string(
        &div()
            .layout_border(configured.layout_border.clone())
            .style()
            .clone(),
    )
    .unwrap();
    let decoded: StyleRefinement = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        decoded.layout_border,
        Some(configured.layout_border.clone())
    );
    configured.refine(
        &div()
            .layout_border(crate::LayoutBorder::Paint)
            .style()
            .clone(),
    );
    assert_eq!(
        configured.to_taffy(px(16.), 1.5).border.top,
        taffy::LengthPercentage::length(1.)
    );
}
