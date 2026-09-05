//! A reusable chrome for borderless, resizable windows.

use crate::{
    AnyElement, App, Bounds, CursorStyle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, RenderOnce, ResizeEdge, SharedString, Styled, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, div, px,
    transparent_black,
};

/// Default inset reserved around the window content (room for a drop shadow).
const DEFAULT_INSET: f32 = 12.0;
/// Default thickness of the edge resize hit-boxes.
const DEFAULT_EDGE_SIZE: f32 = 6.0;
/// Default size of the corner resize hit-boxes.
const DEFAULT_CORNER_SIZE: f32 = 12.0;

/// Each resize edge paired with the cursor to show over its hit-box.
const RESIZE_EDGES: [(ResizeEdge, CursorStyle); 8] = [
    (ResizeEdge::Top, CursorStyle::ResizeUpDown),
    (ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft),
    (ResizeEdge::Right, CursorStyle::ResizeLeftRight),
    (ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight),
    (ResizeEdge::Bottom, CursorStyle::ResizeUpDown),
    (ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft),
    (ResizeEdge::Left, CursorStyle::ResizeLeftRight),
    (ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight),
];

/// A borderless window chrome that keeps native resize behavior.
///
/// Renders its children inset by [`Self::inset`] inside a full-size transparent
/// root, and overlays eight invisible resize hit-boxes along every edge and
/// corner. Each hit-box shows the appropriate resize cursor and starts a system
/// resize operation (on Windows, `WM_SYSCOMMAND SC_SIZE`) on left mouse down,
/// so the standard resize loop — live preview and snap layouts — runs as usual.
///
/// Pair with [`Self::options`] when opening the window (transparent background,
/// no title bar, client-side decorations), draw a drop shadow on the content so
/// it fills the inset margin, and add a drag region for moving the window with
/// `.window_control_area(WindowControlArea::Drag)`.
///
/// See `examples/learn/borderless_resizeable_window.rs` for the full pattern.
///
/// ```ignore
/// cx.open_window(
///     BorderlessWindow::options(Bounds::centered(None, size(px(480.), px(360.)), cx)),
///     |_, cx| cx.new(|cx| WindowView),
/// );
///
/// impl Render for WindowView {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         BorderlessWindow::new()
///             .child(div().size_full().bg(rgb(0xFFFFFF)).shadow_lg())
///     }
/// }
/// ```
#[derive(IntoElement)]
pub struct BorderlessWindow {
    id: SharedString,
    inset: Pixels,
    edge_size: Pixels,
    corner_size: Pixels,
    children: Vec<AnyElement>,
}

impl Default for BorderlessWindow {
    fn default() -> Self {
        Self {
            id: "borderless-window".into(),
            inset: px(DEFAULT_INSET),
            edge_size: px(DEFAULT_EDGE_SIZE),
            corner_size: px(DEFAULT_CORNER_SIZE),
            children: Vec::new(),
        }
    }
}

impl BorderlessWindow {
    /// Create a new borderless window chrome with default sizing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the element id prefix used for the root and resize hit-boxes.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the inset reserved around the content (room for a drop shadow).
    pub fn inset(mut self, inset: Pixels) -> Self {
        self.inset = inset;
        self
    }

    /// Set the thickness of the edge resize hit-boxes.
    pub fn edge_size(mut self, edge_size: Pixels) -> Self {
        self.edge_size = edge_size;
        self
    }

    /// Set the size of the corner resize hit-boxes.
    pub fn corner_size(mut self, corner_size: Pixels) -> Self {
        self.corner_size = corner_size;
        self
    }

    /// Add a child to the window content.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Add children to the window content.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }

    /// [`WindowOptions`] for a borderless window: no title bar, a transparent
    /// background, client-side decorations and native resizing.
    pub fn options(bounds: Bounds<Pixels>) -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            is_resizable: true,
            ..Default::default()
        }
    }
}

impl RenderOnce for BorderlessWindow {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        // Reserve room for the shadow at the platform level (a no-op on
        // Windows, where the content padding below does the same job).
        window.set_client_inset(self.inset);

        let BorderlessWindow {
            id,
            inset,
            edge_size,
            corner_size,
            children,
        } = self;

        div()
            .id(id.clone())
            .relative()
            .size_full()
            .background(transparent_black())
            .child(div().size_full().p(inset).children(children))
            .children(RESIZE_EDGES.map(move |(edge, cursor)| {
                resize_handle(id.clone(), edge, cursor, edge_size, corner_size)
            }))
    }
}

/// Build one invisible resize hit-box for the given edge or corner.
fn resize_handle(
    id: SharedString,
    edge: ResizeEdge,
    cursor: CursorStyle,
    edge_size: Pixels,
    corner_size: Pixels,
) -> impl IntoElement {
    let handle = div()
        .id(SharedString::from(format!("{id}-resize-{edge:?}")))
        .absolute()
        .cursor(cursor)
        .on_mouse_down(MouseButton::Left, move |_event, window, _cx| {
            window.start_window_resize(edge);
        });
    match edge {
        ResizeEdge::Top => {
            handle.top(px(0.)).left(corner_size).right(corner_size).h(edge_size)
        }
        ResizeEdge::TopRight => {
            handle.top(px(0.)).right(px(0.)).w(corner_size).h(corner_size)
        }
        ResizeEdge::Right => {
            handle.top(corner_size).bottom(corner_size).right(px(0.)).w(edge_size)
        }
        ResizeEdge::BottomRight => {
            handle.bottom(px(0.)).right(px(0.)).w(corner_size).h(corner_size)
        }
        ResizeEdge::Bottom => {
            handle.bottom(px(0.)).left(corner_size).right(corner_size).h(edge_size)
        }
        ResizeEdge::BottomLeft => {
            handle.bottom(px(0.)).left(px(0.)).w(corner_size).h(corner_size)
        }
        ResizeEdge::Left => {
            handle.top(corner_size).bottom(corner_size).left(px(0.)).w(edge_size)
        }
        ResizeEdge::TopLeft => {
            handle.top(px(0.)).left(px(0.)).w(corner_size).h(corner_size)
        }
    }
}
