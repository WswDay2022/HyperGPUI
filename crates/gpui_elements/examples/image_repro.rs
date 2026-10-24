use std::{borrow::Cow, rc::Rc, time::Duration};

use hgpui::{
    animated_div, div, ease_out_quint, img, px, size, AnimatedDivProps, App, AppContext as _,
    AssetSource, Bounds, Context, ImageSource, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Render, Resource, SharedString, StatefulInteractiveElement, Styled,
    StyledImage, Window, WindowBounds, WindowOptions,
};

struct Assets;

const FILES: [(&str, &[u8]); 4] = [
    ("images/Zcy.jpg", include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Zcy.jpg")),
    ("images/Longmao.jpg", include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Longmao.jpg")),
    ("images/Deepseek.png", include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Deepseek.png")),
    ("images/Rust.png", include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Rust.png")),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> hgpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(p, _)| *p == path).map(|(_, data)| Cow::Borrowed(*data)))
    }
    fn list(&self, _path: &str) -> hgpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

struct Example;

impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("image-repro-page")
            .smooth_scroll(true)
            .size_full()
            .overflow_y_scroll()
            .overflow_x_hidden()
            .flex()
            .flex_col()
            .p(px(16.))
            .gap(px(8.))
            .child(
                animated_div(
                    "image-card",
                    AnimatedDivProps {
                        animating_width: false,
                        animating_height: true,
                        animating_x: false,
                        animating_y: false,
                        easing: Rc::new(ease_out_quint()),
                        duration: Duration::from_millis(120),
                    },
                )
                .flex_shrink_0()
                .flex()
                .overflow_hidden()
                .flex_col()
                .p(px(16.))
                .gap(px(8.))
                .w_full()
                .rounded(px(5.))
                .shadow_md()
                .children(FILES.into_iter().map(|(path, _)| {
                div()
                    .w_full()
                    .h(px(40.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(20.))
                    .shadow_sm()
                    .child(
                        img(ImageSource::Resource(Resource::Embedded(path.into())))
                            .flex_shrink_0()
                            .max_h_full()
                            .h_full()
                            .aspect_square()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(div().child(path))
                })),
            )
    }
}

fn main() {
    hgpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(500.), px(400.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Example),
            )
            .unwrap();
            cx.activate(true);
        });
}
