use crate::editable_text::{
    InitStorage, StateBackedEditableText, TextAreaState,
    actions::{DEFAULT_INPUT_CONTEXT, EditableInputActionElement},
    shared_element::{self, EditableTextElement},
};
use gpui::{
    App, Bounds, Element, ElementId, InteractiveElement, Interactivity, IntoElement, Pixels,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, WeakEntity, Window,
};
use std::{cell::RefCell, rc::Rc};

pub fn text_area(id: impl Into<ElementId>) -> TextAreaElement {
    let mut this = TextAreaElement {
        interactivity: Interactivity::new(),
        state_entity: Rc::new(RefCell::new(WeakEntity::new_invalid())),
        init_storage: InitStorage::default(),
        placeholder: None,
    };
    this.interactivity.element_id = Some(id.into());

    this = this.key_context(DEFAULT_INPUT_CONTEXT);
    this.register_actions();

    this
}

// TODO: Disabled flag/state?
pub struct TextAreaElement {
    interactivity: Interactivity,
    // Populated on first render with an entity stored/attached to the view.
    // This reference is shared with the action handlers, which are processed between renders
    // and therefore cannot otherwise access state attached to the view.
    state_entity: Rc<RefCell<WeakEntity<TextAreaState>>>,
    init_storage: InitStorage,
    placeholder: Option<SharedString>,
}

impl TextAreaElement {
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn with_storage(mut self, fn_init: impl Into<InitStorage>) -> Self {
        self.init_storage = fn_init.into();
        self
    }
}

impl InteractiveElement for TextAreaElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for TextAreaElement {}

impl Styled for TextAreaElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for TextAreaElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl StateBackedEditableText for TextAreaElement {
    type State = TextAreaState;
}

impl EditableInputActionElement for TextAreaElement {
    fn state_entity_rc(&self) -> &Rc<RefCell<WeakEntity<Self::State>>> {
        &self.state_entity
    }
}

impl EditableTextElement for TextAreaElement {
    fn init_state(&self, cx: &mut gpui::prelude::Context<Self::State>) -> Self::State {
        Self::State::new(self.init_storage.exec(cx), cx)
    }

    fn placeholder(&self) -> &Option<SharedString> {
        &self.placeholder
    }
}

impl Element for TextAreaElement {
    type RequestLayoutState = shared_element::LayoutState<TextAreaState>;
    type PrepaintState = shared_element::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        self.shared_request_layout(global_id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.shared_prepaint(global_id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.shared_paint(
            global_id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}
