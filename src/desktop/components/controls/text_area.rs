//! Token-styled multi-line editor backed by the kit's IME-capable editing engine.
use super::button::{ring, Interaction};
use super::text_field::{input_id, FieldStyle};
use crate::desktop::components::tokens::{button_colors, radius, ButtonVariant, Radius, Tokens};
use crate::desktop::motion::{self, Driver};
use gpui_kit::component::input::{
    IndentInline, InputEvent, OutdentInline, Textarea, TextareaState,
};
use gpui_kit::{
    div, App, AppContext, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window,
};
use std::rc::Rc;

/// Events forwarded from the underlying editor after committed changes or submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAreaEvent {
    Change,
    Submit,
}

fn area_event(event: &InputEvent, submit_on_enter: bool) -> Option<TextAreaEvent> {
    match event {
        InputEvent::Change => Some(TextAreaEvent::Change),
        InputEvent::PressEnter { shift: false, .. } if submit_on_enter => {
            Some(TextAreaEvent::Submit)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusDirection {
    Next,
    Previous,
}

fn focus_direction(owned: bool, outdent: bool) -> Option<FocusDirection> {
    if !owned {
        None
    } else if outdent {
        Some(FocusDirection::Previous)
    } else {
        Some(FocusDirection::Next)
    }
}

fn move_focus(direction: FocusDirection, window: &mut Window, cx: &mut App) {
    match direction {
        FocusDirection::Next => window.focus_next(cx),
        FocusDirection::Previous => window.focus_prev(cx),
    }
}

pub struct TextAreaState {
    inner: Entity<TextareaState>,
    submit_on_enter: bool,
}
impl EventEmitter<TextAreaEvent> for TextAreaState {}
impl Focusable for TextAreaState {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.inner.read(cx).focus_handle(cx)
    }
}
impl TextAreaState {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let inner = cx.new(|cx| {
                let mut state = TextareaState::new(window, cx);
                state.set_submit_on_enter(true, cx);
                state
            });
            cx.subscribe(&inner, |this: &mut Self, _, event: &InputEvent, cx| {
                if let Some(event) = area_event(event, this.submit_on_enter) {
                    cx.emit(event);
                }
                cx.notify();
            })
            .detach();
            Self {
                inner,
                submit_on_enter: true,
            }
        })
    }
    pub fn value(&self, cx: &App) -> SharedString {
        self.inner.read(cx).value()
    }
    pub fn set_value(
        &mut self,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inner
            .update(cx, |state, cx| state.set_value(value.into(), window, cx));
    }
    pub fn set_placeholder(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inner
            .update(cx, |state, cx| state.set_placeholder(text, window, cx));
    }
    pub fn set_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        self.inner.update(cx, |state, cx| state.set_rows(rows, cx));
    }
    pub fn set_submit_on_enter(&mut self, submit: bool, cx: &mut Context<Self>) {
        self.submit_on_enter = submit;
        self.inner
            .update(cx, |state, cx| state.set_submit_on_enter(submit, cx));
    }
}

type Submit = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
fn clamp_rows(rows: usize, min: usize, max: usize) -> usize {
    let min = min.max(1);
    rows.clamp(min, max.max(min))
}
fn should_submit(enter: bool, primary: bool) -> bool {
    enter && primary
}
struct AreaStyle {
    id: ElementId,
    tokens: Tokens,
    field: FieldStyle,
}
fn area_style(id: impl Into<ElementId>, tokens: &Tokens) -> AreaStyle {
    let id = id.into();
    AreaStyle {
        field: FieldStyle::new(id.clone(), tokens),
        id,
        tokens: *tokens,
    }
}
impl AreaStyle {
    fn id(&self) -> ElementId {
        self.id.clone()
    }
    fn tab_stop(&self) -> bool {
        true
    }
    fn focus_ring(&self, focused: bool) -> Vec<gpui_kit::BoxShadow> {
        ring(focused, &self.tokens)
    }
}
#[derive(IntoElement)]
pub struct TextArea {
    style: AreaStyle,
    state: Entity<TextareaState>,
    min_rows: usize,
    max_rows: usize,
    placeholder: Option<SharedString>,
    on_submit: Option<Submit>,
    owned: bool,
    configure: bool,
    rows_configured: bool,
}
impl TextArea {
    pub fn new(id: impl Into<ElementId>, state: &Entity<TextareaState>, tokens: &Tokens) -> Self {
        Self {
            style: area_style(id, tokens),
            state: state.clone(),
            min_rows: 1,
            max_rows: 1,
            placeholder: None,
            on_submit: None,
            owned: false,
            configure: true,
            rows_configured: false,
        }
    }
    pub fn from_state(
        id: impl Into<ElementId>,
        state: &Entity<TextAreaState>,
        tokens: &Tokens,
        cx: &App,
    ) -> Self {
        let mut area = Self::new(id, &state.read(cx).inner, tokens);
        area.owned = true;
        area.configure = false;
        area
    }
    pub fn rows(mut self, min: usize, max: usize) -> Self {
        self.min_rows = clamp_rows(min, 1, usize::MAX);
        self.max_rows = max.max(self.min_rows);
        self.configure = true;
        self.rows_configured = true;
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self.configure = true;
        self
    }
    pub fn on_submit(
        mut self,
        callback: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_submit = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for TextArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.style.id();
        let config = window.use_keyed_state((id.clone(), "config"), cx, |_, _| {
            None::<(gpui_kit::EntityId, usize, usize, Option<SharedString>)>
        });
        let desired = (
            self.state.entity_id(),
            self.min_rows,
            self.max_rows,
            self.placeholder.clone(),
        );
        if self.configure && config.read(cx).as_ref() != Some(&desired) {
            self.state.update(cx, |state, cx| {
                if !self.owned || self.rows_configured {
                    state.set_auto_grow(desired.1, desired.2, cx);
                }
                if !self.owned || desired.3.is_some() {
                    state.set_placeholder(desired.3.clone().unwrap_or_default(), window, cx);
                }
            });
            config.update(cx, |value, _| *value = Some(desired));
        }
        let interaction =
            window.use_keyed_state(id.clone(), cx, |_, cx| Interaction::new(cx.focus_handle()));
        let focus = self
            .state
            .read(cx)
            .focus_handle(cx)
            .tab_stop(self.style.tab_stop());
        let mut driver = Driver::new(cx.reduce_motion(), cx);
        let style = self.style.field.style(
            interaction.read(cx),
            driver.now(),
            driver.reduce(),
            focus.is_focused(window),
        );
        driver.keep(style.moving);
        driver.finish(window);
        let colors = button_colors(ButtonVariant::Secondary, &self.style.tokens);
        let background = motion::mix(
            motion::mix(colors.background, colors.hover, style.hover),
            colors.pressed,
            style.press,
        );
        let hover = interaction.clone();
        let down = interaction.clone();
        let up = interaction.clone();
        let out = interaction;
        let input = self.state.clone();
        let submit = self.on_submit;
        let owned = self.owned;
        div()
            .id(id.clone())
            .rounded(radius(Radius::Md))
            .bg(background)
            .text_color(colors.foreground)
            .border_1()
            .border_color(self.style.tokens.border)
            .shadow(style.ring)
            .capture_action(move |_: &IndentInline, window, cx| {
                if let Some(direction) = focus_direction(owned, false) {
                    cx.stop_propagation();
                    move_focus(direction, window, cx);
                }
            })
            .capture_action(move |_: &OutdentInline, window, cx| {
                if let Some(direction) = focus_direction(owned, true) {
                    cx.stop_propagation();
                    move_focus(direction, window, cx);
                }
            })
            .child(
                Textarea::new(&self.state)
                    .appearance(false)
                    .bordered(false)
                    .accessibility_id(format!("{:?}", input_id(&id))),
            )
            .capture_key_down(move |event, window, cx| {
                let m = event.keystroke.modifiers;
                let primary = if cfg!(target_os = "macos") {
                    m.platform
                } else {
                    m.control
                };
                if should_submit(event.keystroke.key == "enter", primary) {
                    if let Some(callback) = &submit {
                        callback(input.read(cx).value(), window, cx);
                        cx.stop_propagation();
                    }
                }
            })
            .on_hover(move |on, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                hover.update(cx, |s, cx| {
                    s.hover(*on, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                down.update(cx, |s, cx| {
                    s.press(true, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                up.update(cx, |s, cx| {
                    s.press(false, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                out.update(cx, |s, cx| {
                    s.press(false, now, reduce);
                    cx.notify();
                });
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_state_exposes_editor_and_events() {
        let _: fn(&mut Window, &mut App) -> Entity<TextAreaState> = TextAreaState::new;
        let _: fn(&TextAreaState, &App) -> SharedString = TextAreaState::value;
        fn emits<T: gpui_kit::EventEmitter<TextAreaEvent>>() {}
        emits::<TextAreaState>();
        assert_eq!(
            area_event(&InputEvent::Change, true),
            Some(TextAreaEvent::Change)
        );
        let enter = InputEvent::PressEnter {
            secondary: false,
            shift: false,
        };
        let shift_enter = InputEvent::PressEnter {
            secondary: false,
            shift: true,
        };
        assert_eq!(area_event(&enter, true), Some(TextAreaEvent::Submit));
        assert_eq!(area_event(&shift_enter, true), None);
        assert_eq!(area_event(&enter, false), None);
    }
    #[test]
    fn owned_tab_actions_choose_focus_direction() {
        assert_eq!(focus_direction(true, false), Some(FocusDirection::Next));
        assert_eq!(focus_direction(true, true), Some(FocusDirection::Previous));
        assert_eq!(focus_direction(false, false), None);
        assert_eq!(focus_direction(false, true), None);
    }
    #[test]
    fn input_events_respect_enter_modifiers_and_submit_setting() {
        for submit in [false, true] {
            assert_eq!(
                area_event(&InputEvent::Change, submit),
                Some(TextAreaEvent::Change)
            );
            assert_eq!(area_event(&InputEvent::Focus, submit), None);
            assert_eq!(area_event(&InputEvent::Blur, submit), None);
            for secondary in [false, true] {
                for shift in [false, true] {
                    let event = InputEvent::PressEnter { secondary, shift };
                    let expected = if submit && !shift {
                        Some(TextAreaEvent::Submit)
                    } else {
                        None
                    };
                    assert_eq!(area_event(&event, submit), expected);
                }
            }
        }
    }
    #[test]
    fn rows_submit_and_field_style() {
        for t in crate::desktop::theme::both() {
            assert_eq!(clamp_rows(0, 0, 0), 1);
            assert_eq!(clamp_rows(12, 2, 5), 5);
            assert_eq!(clamp_rows(1, 2, 5), 2);
            assert_eq!(clamp_rows(usize::MAX, 5, 2), 5);
            assert!(should_submit(true, true));
            assert!(!should_submit(false, true));
            assert!(!should_submit(true, false));
            let style = area_style("area", &t);
            assert_eq!(style.id(), gpui_kit::ElementId::from("area"));
            assert!(style.tab_stop());
            assert_eq!(
                style.focus_ring(true),
                crate::desktop::theme::focus_ring(&t)
            );
            assert!(style.focus_ring(false).is_empty());
        }
    }
}
