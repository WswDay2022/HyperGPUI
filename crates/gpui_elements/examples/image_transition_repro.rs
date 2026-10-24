use std::{borrow::Cow, rc::Rc, time::Duration};

use hgpui::{
    animated_div, div, ease_out_cubic, ease_out_quint, img, px, size, AnimatedDivProps,
    AnyElement, App, AppContext as _, AssetSource, Bounds, Context, ImageSource,
    InteractiveElement, IntoElement, ObjectFit, ParentElement, Render, Resource, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, Window, WindowBounds, WindowOptions,
};
use hgpui_elements::transition::{TransitionMode, transition};

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

fn image_page() -> AnyElement {
    div()
        .id("about")
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
        .into_any_element()
}

fn text_page() -> AnyElement {
    div()
        .id("setting")
        .smooth_scroll(true)
        .size_full()
        .overflow_y_scroll()
        .overflow_x_hidden()
        .flex()
        .flex_col()
        .p(px(16.))
        .gap(px(8.))
        .children((0..20).map(|i| div().h(px(48.)).w_full().child(format!("row {i}"))))
        .into_any_element()
}

struct Example {
    tick: usize,
    generation: usize,
    show_images: bool,
}

impl Example {
    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        this.tick += 1;
                        this.show_images = !this.show_images;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }
}

impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let show_images = self.show_images;
        let tick = self.tick;
        div()
            .size_full()
            .child(
                transition("nav", tick)
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
                            image_page()
                        } else {
                            text_page()
                        }
                    }),
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
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| Example {
                            tick: 0,
                            generation: 0,
                            show_images: false,
                        })
                    },
                )
                .unwrap();
            handle
                .update(cx, |example, _, cx| example.start_ticker(cx))
                .unwrap();
            cx.activate(true);
        });
}
