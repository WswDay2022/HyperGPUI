use hgpui::{App, Window};
use hgpui_macros::{AppContext, VisualContext};

#[derive(AppContext, VisualContext)]
struct CustomContext<'a, 'b> {
    #[app]
    app: &'a mut App,
    #[window]
    window: &'b mut Window,
}

#[test]
fn derives_context_traits() {
    let _ = std::marker::PhantomData::<CustomContext<'static, 'static>>;
}
