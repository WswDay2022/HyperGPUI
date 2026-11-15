//! Run with `cargo run -p hgpui --example grid_complete`.
//! Add `--features wgpu` to exercise the alternative Windows renderer.

#[path = "shared/prelude.rs"]
mod example_prelude;

use hgpui::{
    AlignItems, App, AppContext, Bounds, Context, GridAutoFlow, GridMaxTrackSizingFunction as Max,
    GridMinTrackSizingFunction as Min, GridRepetitionCount, GridTemplateArea,
    GridTemplateComponent, GridTemplateRepetition, GridTrackSizingFunction as Track,
    InteractiveElement, IntoElement, LayoutDirection, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions, div, px, rgb, size,
};

struct GridExample;

fn tile(label: impl Into<hgpui::SharedString>) -> hgpui::Div {
    div()
        .p(px(10.))
        .text_size(px(14.))
        .rounded(px(6.))
        .background(rgb(0x26364c))
        .child(label.into())
}

impl Render for GridExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let auto = |count| {
            div()
                .grid()
                .flex_none()
                .gap(px(8.))
                .grid_template_columns([GridTemplateComponent::Repeat(
                    GridTemplateRepetition::new(
                        count,
                        [Track::minmax(
                            Min::Length(px(130.).into()),
                            Max::Fraction(1.),
                        )],
                    ),
                )])
                .children((1..=3).map(|index| tile(format!("Item {index}"))))
        };
        div()
            .id("grid-complete-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(20.))
            .background(rgb(0x101720))
            .text_color(rgb(0xe4edf7))
            .child(
                div()
                    .text_xl()
                    .child("Taffy Grid: full track and placement interface"),
            )
            .child("Mixed tracks: 120px / minmax(0, 1fr) / minmax(0, 2fr)")
            .child(
                div()
                    .grid()
                    .flex_none()
                    .gap(px(8.))
                    .grid_template_columns([
                        Track::length(px(120.)),
                        Track::flex(1.),
                        Track::flex(2.),
                    ])
                    .child(tile("120px"))
                    .child(tile("1fr"))
                    .child(tile("2fr")),
            )
            .child("Auto-fill keeps empty tracks")
            .child(auto(GridRepetitionCount::AutoFill))
            .child("Auto-fit collapses empty tracks; resize the window")
            .child(auto(GridRepetitionCount::AutoFit))
            .child("Dense placement: small items backfill holes")
            .child(
                div()
                    .grid()
                    .flex_none()
                    .grid_cols(3)
                    .gap(px(8.))
                    .grid_auto_flow(GridAutoFlow::RowDense)
                    .grid_auto_rows([Track::length(px(44.))])
                    .child(tile("Wide A").col_span(2))
                    .child(tile("Wide B").col_span(2))
                    .child(tile("Hole filler")),
            )
            .child("Named areas: header / side / main")
            .child(
                div()
                    .grid()
                    .flex_none()
                    .gap(px(8.))
                    .h(px(150.))
                    .grid_template_columns([Track::length(px(120.)), Track::flex(1.)])
                    .grid_template_rows([Track::length(px(44.)), Track::flex(1.)])
                    .grid_template_areas([
                        GridTemplateArea {
                            name: "header".into(),
                            row_start: 1,
                            row_end: 2,
                            column_start: 1,
                            column_end: 3,
                        },
                        GridTemplateArea {
                            name: "side".into(),
                            row_start: 2,
                            row_end: 3,
                            column_start: 1,
                            column_end: 2,
                        },
                        GridTemplateArea {
                            name: "main".into(),
                            row_start: 2,
                            row_end: 3,
                            column_start: 2,
                            column_end: 3,
                        },
                    ])
                    .child(tile("Header").grid_area("header"))
                    .child(tile("Side").grid_area("side"))
                    .child(tile("Main").grid_area("main")),
            )
            .child("RTL: child source order stays 1 / 2 / 3; columns mirror")
            .child(
                div()
                    .grid()
                    .flex_none()
                    .grid_cols(3)
                    .gap(px(8.))
                    .layout_direction(LayoutDirection::Rtl)
                    .justify_items(AlignItems::Center)
                    .child(tile("1"))
                    .child(tile("2"))
                    .child(tile("3")),
            )
    }
}

fn main() {
    hgpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| GridExample),
        )
        .unwrap();
        example_prelude::init_example(cx, "Complete Grid");
    });
}
