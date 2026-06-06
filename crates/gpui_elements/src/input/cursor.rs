use gpui::Context;
use std::time::Duration;

/// Default interval for cursor blinking.
pub const DEFAULT_BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Configuration for cursor blinking, to be provided to InputState.
pub enum CursorBlinkType<'app> {
    /// The cursor will not blink.
    Disabled,
    /// The cursor will blink at some interval.
    Enabled {
        /// Provide the app so that the internal state to track cursor blinking can be created.
        app: &'app mut gpui::App,
        /// The interval to blink at. If none, the default value of 500ms is used (defined by `DEFAULT_BLINK_INTERVAL`).
        interval: Option<Duration>,
    },
}

/// The state of an input's cursor blinking. While active, the cursor's visibility changes at some interval.
/// This blinking can be temporarily paused (e.g. during typing).
pub(super) struct CursorBlink {
    interval: Duration,
    generation: usize,
    visible: bool,
    active: bool,
    paused: bool,
}

impl CursorBlink {
    /// Initializes the cursor blinking with the cursor already being visible.
    #[track_caller]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            generation: 0,
            visible: true,
            active: false,
            paused: false,
        }
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
}
