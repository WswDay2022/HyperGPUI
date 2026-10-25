use std::{borrow::Cow, rc::Rc, time::Duration};

use hgpui::{
    animated_div, div, ease_out_cubic, ease_out_quint, img, px, rgb, size, AnimatedDivProps,
    AnyElement, App, AppContext as _, AssetSource, Bounds, Context, ImageSource, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, Render, Resource, SharedString, StatefulInteractiveElement,
    Styled, StyledImage, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use hgpui_elements::{
    borderless_window::borderless_window,
    transition::{transition, TransitionMode},
};

struct Assets;

const FILES: [(&str, &[u8]); 5] = [
    (
        "images/Background.png",
        include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Background.png"),
    ),
    (
        "images/Zcy.jpg",
        include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Zcy.jpg"),
    ),
    (
        "images/Longmao.jpg",
        include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Longmao.jpg"),
    ),
    (
        "images/Deepseek.png",
        include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Deepseek.png"),
    ),
    (
        "images/Rust.png",
        include_bytes!("D:/WswDay2022/projects/IshiTone/ishitone/assets/images/Rust.png"),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> hgpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, data)| Cow::Borrowed(*data)))
    }
    fn list(&self, _path: &str) -> hgpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

fn image_page(tick: u64) -> AnyElement {
    div()
        .id("image-page")
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
                .w_full()
                .flex_shrink_0()
                .overflow_hidden()
                .rounded(px(5.))
                .shadow_md()
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .h(px(24.))
                        .w_full()
                        .background(if tick % 2 == 0 {
                            rgb(0x3366FF)
                        } else {
                            rgb(0xFF6633)
                        })
                        .child(format!("image tick {tick}")),
                )
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
        .into_any_element()
}

fn text_page(tick: u64) -> AnyElement {
    div()
        .id("text-page")
        .size_full()
        .flex()
        .flex_col()
        .p(px(16.))
        .children((0..20).map(move |i| div().h(px(36.)).child(format!("text {tick} row {i}"))))
        .into_any_element()
}

struct Example {
    tick: u64,
    show_images: bool,
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tick = self.tick;
        let show_images = self.show_images;
        let key = SharedString::from(format!("page-{tick}"));
        borderless_window("window").inset(px(5.)).child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(99.))
                .background(rgb(0xFFFFFF))
                .child(
                    img(ImageSource::Resource(Resource::Embedded(
                        "images/Background.png".into(),
                    )))
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
                )
                .child(
                    transition("page", key)
                        .mode(TransitionMode::Simultaneous)
                        .easing(ease_out_cubic)
                        .duration(Duration::from_millis(250))
                        .enter(|raw, delta, time, _, _| {
                            div()
                                .size_full()
                                .child(raw)
                                .opacity(time)
                                .translate_x(px(30. * (1. - delta)))
                                .into_any_element()
                        })
                        .leave(|raw, delta, time, _, _| {
                            div()
                                .size_full()
                                .child(raw)
                                .opacity(1. - time)
                                .translate_x(px(30. * delta))
                                .into_any_element()
                        })
                        .child(move |_, _| {
                            if show_images {
                                image_page(tick)
                            } else {
                                text_page(tick)
                            }
                        }),
                ),
        )
    }
}

fn main() {
    hgpui_platform::application()
        .with_assets(Assets)
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(500.), px(400.)), cx);
            let handle = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_background: WindowBackgroundAppearance::Transparent,
                        window_decorations: None,
                        titlebar: None,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| Example {
                            tick: 0,
                            show_images: false,
                        })
                    },
                )
                .unwrap();
            handle
                .update(cx, |example, _, cx| {
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(Duration::from_secs(2)).await;
                            if this
                                .update(cx, |example, cx| {
                                    example.tick += 1;
                                    example.show_images = !example.show_images;
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                })
                .unwrap();
            cx.activate(true);
        });
}
