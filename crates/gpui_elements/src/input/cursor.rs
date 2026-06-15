use gpui::{
    Bounds, Context, Element, Entity, EventEmitter, Hsla, IntoElement, Pixels, Point, Render,
    Subscription,
};
use smallvec::SmallVec;
use std::time::Duration;

use crate::input::CursorTrigger;

/// Default interval for cursor blinking.
pub const DEFAULT_BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// The state of an input's cursor blinking. While active, the cursor's visibility changes at some interval.
/// This blinking can be temporarily paused (e.g. during typing).
pub struct Cursor {
    interval: Duration,
    generation: usize,
    visible: bool,
    active: bool,
    paused: bool,
    color: Hsla,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
    point: Point<Pixels>,
    height: Pixels,
    #[allow(dead_code)]
    subscriptions: SmallVec<[Subscription; 2]>,
}

impl Cursor {
    /// Initializes the cursor blinking with the cursor already being visible.
    #[track_caller]
    pub fn new(interval: Option<Duration>) -> Self {
        Self {
            interval: interval.unwrap_or_default(),
            generation: 0,
            visible: true,
            active: false,
            paused: false,
            color: Hsla::white(),
            was_focused: false,
            point: Point::default(),
            height: Pixels::ZERO,
            subscriptions: SmallVec::new(),
        }
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = color;
        self
    }

    pub fn subscribe_to<E>(&mut self, emitter: &Entity<E>, cx: &mut Context<Self>)
    where
        E: EventEmitter<CursorTrigger>,
    {
        let handle = cx.subscribe(emitter, |cursor, _emitter, event, cx| match event {
            CursorTrigger::PauseBlinkingForUserAction => {
                cursor.pause_blinking(cx);
                cx.notify();
            }
        });
        self.subscriptions.push(handle);
    }

    /// Returns whether the cursor should currently be rendered.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Activates cursor blinking.
    ///
    /// While active, the cursor will alternate between visible and hidden states at the configured interval. Has no effect if already active.
    pub fn enable(&mut self, cx: &mut Context<Self>) {
        if self.active {
            return;
        }

        self.active = true;
        self.visible = false;
        self.paused = false;
        self.spawn_ticker(cx);
    }

    /// Deactivates cursor blinking.
    ///
    /// Marks the cursor as invisible and pauses blinking indefinitely. `enable` must be called explicitly to resume visibility and blinking.
    /// Call `pause_blinking` instead if you want to temporarily stop blinking while keeping the cursor visible.
    pub fn disable(&mut self, cx: &mut Context<Self>) {
        self.active = false;
        self.visible = false;
        self.paused = false;
        cx.notify();
    }

    /// Temporarily pauses blinking and leaves the cursor visible. Blinking will resume after the pre-established interval elapses from the time this is called.
    pub fn pause_blinking(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            self.visible = true;
            cx.notify();
        }

        self.paused = true;
        self.generation = self.generation.wrapping_add(1);

        let generation = self.generation;
        let interval = self.interval;

        cx.spawn(async move |this, cx| {
            async_io::Timer::after(interval).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.paused = false;
                    this.spawn_ticker(cx);
                }
            })
        })
        .detach();
    }

    fn spawn_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.active || self.paused {
            return;
        }

        self.visible = !self.visible;
        cx.notify();

        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let interval = self.interval;

        cx.spawn(async move |this, cx| {
            async_io::Timer::after(interval).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.spawn_ticker(cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn update_input(
        &mut self,
        is_focused: bool,
        pos: Point<Pixels>,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let was_focused = self.was_focused;
        self.was_focused = is_focused;

        self.point = pos;
        self.height = line_height;

        match (self.interval.is_zero(), is_focused, was_focused) {
            (true, _, _) => true,
            (false, true, false) => {
                self.enable(cx);
                true
            }
            (false, false, true) => {
                self.disable(cx);
                false
            }
            (false, _, _) => self.visible,
        }
    }
}

impl IntoElement for Cursor {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Cursor {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let layout_id = window.request_layout(gpui::Style::default(), None, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        ()
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) {
        const CURSOR_WIDTH: f32 = 2.0;
        window.paint_quad(gpui::fill(
            Bounds::new(
                gpui::point(bounds.left(), bounds.top()) + self.point,
                gpui::size(gpui::px(CURSOR_WIDTH), self.height),
            ),
            self.color,
        ));
    }
}
