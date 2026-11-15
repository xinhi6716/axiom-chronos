#[derive(Clone, Debug, PartialEq)]
pub enum PanelState { Loading, Ready, Empty, Stale { since_ms: i64 }, Offline { reason: String }, Corrupt { reason: String }, Failed { reason: String } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::i18n::Lang;
    use std::prelude::v1::test;

    #[test]
    fn all_states_have_localized_copy_and_only_ready_has_no_retry() {
        let states = [PanelState::Loading, PanelState::Ready, PanelState::Empty,
            PanelState::Stale { since_ms: 1 }, PanelState::Offline { reason: "offline".into() },
            PanelState::Corrupt { reason: "bad".into() }, PanelState::Failed { reason: "error".into() }];
        for lang in [Lang::En, Lang::ZhTw] {
            for state in &states {
                if *state == PanelState::Ready {
                    assert_eq!(copy(lang, state), None);
                    assert_eq!(retry_for_state("example", state), None);
                } else {
                    let (title, reason) = copy(lang, state).unwrap();
                    assert!(!title.trim().is_empty() && !reason.trim().is_empty());
                    assert!(!reason.contains(['\n', '\r']));
                    assert_eq!(retry_for_state("example", state).as_deref(), Some("retry:example"));
                }
            }
        }
        assert_eq!(retry_id("a:b"), "retry:a:b");
    }

    #[test]
    fn content_catches_panics_and_success_is_unchanged() {
        assert_eq!(run_content("test-success", || 42), Ok(42));
        assert_eq!(run_content::<()>("test-panic", || panic!("failure")), Err("failure".into()));
        assert_eq!(run_content::<()>("test-panic", || panic!("failure")), Err("failure".into()));
        assert!(!first_panic("test-panic"));
        assert!(first_panic("test-other-panel"));
        assert_eq!(run_content::<()>("test-non-string", || std::panic::panic_any(7)), Err("unknown panic".into()));
    }

    #[test]
    fn render_guard_nests_and_restores_after_unwind() {
        assert_eq!(rendering_panel(), None);
        let outer = RenderGuard::new("outer");
        assert_eq!(rendering_panel(), Some("outer"));
        let _ = std::panic::catch_unwind(|| {
            let _inner = RenderGuard::new("inner");
            assert_eq!(rendering_panel(), Some("inner"));
            panic!("test");
        });
        assert_eq!(rendering_panel(), Some("outer"));
        drop(outer);
        assert_eq!(rendering_panel(), None);
    }

    #[test]
    fn panic_hook_is_replaceable() {
        fn one(_: &str, _: &gpui_kit::App) -> bool { true }
        fn two(_: &str, _: &gpui_kit::App) -> bool { false }
        install_panic_hook(one);
        assert!(std::ptr::fn_addr_eq(panic_hook(), one as PanicHook));
        install_panic_hook(two);
        assert!(std::ptr::fn_addr_eq(panic_hook(), two as PanicHook));
        install_panic_hook(no_panic);
    }
}

use crate::desktop::components::{Button, ButtonVariant, Tokened};
use crate::desktop::i18n::Lang;
use crate::desktop::theme::{density, font, space, Step, TypeSize};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{div, AnyElement, App, ClickEvent, IntoElement, ParentElement, RenderOnce, Styled, Window};
use std::cell::Cell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

/// Localized status text; Ready displays the content instead.
pub fn copy(lang: Lang, state: &PanelState) -> Option<(&'static str, &'static str)> {
    use PanelState::*;
    Some(match (lang, state) {
        (_, Ready) => return None,
        (Lang::En, Loading) => ("Loading", "The content is being loaded."),
        (Lang::ZhTw, Loading) => ("載入中", "正在載入內容。"),
        (Lang::En, Empty) => ("Nothing here yet", "There is no content to show."),
        (Lang::ZhTw, Empty) => ("尚無內容", "目前沒有可顯示的內容。"),
        (Lang::En, Stale { .. }) => ("Out of date", "The content could not be refreshed."),
        (Lang::ZhTw, Stale { .. }) => ("內容已過期", "無法更新內容。"),
        (Lang::En, Offline { .. }) => ("Offline", "The source is not reachable."),
        (Lang::ZhTw, Offline { .. }) => ("無法連線", "無法連接資料來源。"),
        (Lang::En, Corrupt { .. }) => ("Invalid content", "The content could not be read."),
        (Lang::ZhTw, Corrupt { .. }) => ("內容損壞", "無法讀取內容。"),
        (Lang::En, Failed { .. }) => ("Unable to load", "Something went wrong while loading content."),
        (Lang::ZhTw, Failed { .. }) => ("載入失敗", "載入內容時發生問題。"),
    })
}

pub fn retry_id(id: &str) -> String { format!("retry:{id}") }

fn retry_for_state(id: &str, state: &PanelState) -> Option<String> {
    if matches!(state, PanelState::Ready) { None } else { Some(retry_id(id)) }
}

static PANICKED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn first_panic(id: &str) -> bool {
    PANICKED.get_or_init(|| Mutex::new(HashSet::new()))
        .lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned())
}

/// Isolate a content builder and report its first panic per panel id.
pub fn run_content<T>(panel_id: &str, builder: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(builder)).map_err(|payload| {
        let message = payload.downcast_ref::<String>().cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "unknown panic".to_owned());
        if first_panic(panel_id) { eprintln!("panel {panel_id} panicked: {message}"); }
        message
    })
}

thread_local! { static RENDERING: Cell<Option<&'static str>> = const { Cell::new(None) }; }

pub fn rendering_panel() -> Option<&'static str> { RENDERING.with(Cell::get) }

/// Marks a content build on the current thread, restoring any enclosing panel on drop.
pub struct RenderGuard {
    previous: Option<&'static str>,
    _thread_bound: PhantomData<Rc<()>>,
}
impl RenderGuard {
    pub fn new(id: &'static str) -> Self {
        Self { previous: RENDERING.with(|current| current.replace(Some(id))), _thread_bound: PhantomData }
    }
}
impl Drop for RenderGuard {
    fn drop(&mut self) { RENDERING.with(|current| current.set(self.previous)); }
}

type PanicHook = fn(&str, &App) -> bool;
fn no_panic(_: &str, _: &App) -> bool { false }
static PANIC_HOOK: Mutex<PanicHook> = Mutex::new(no_panic);

pub fn install_panic_hook(f: fn(&str, &App) -> bool) {
    *PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner()) = f;
}
fn panic_hook() -> PanicHook { *PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner()) }

type Content = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;
type Retry = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Panel {
    id: &'static str,
    state: PanelState,
    retry: Option<Retry>,
    content: Option<Content>,
}
impl Panel {
    pub fn new(id: &'static str, state: &PanelState) -> Self {
        Self { id, state: state.clone(), retry: None, content: None }
    }
    pub fn retry(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.retry = Some(Box::new(handler));
        self
    }
    pub fn content(mut self, builder: impl FnOnce(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        self.content = Some(Box::new(builder));
        self
    }
}
impl RenderOnce for Panel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, mut state, retry, content } = self;
        if matches!(state, PanelState::Ready) {
            if let Some(content) = content {
                match run_content(id, || {
                    let _guard = RenderGuard::new(id);
                    if panic_hook()(id, cx) { panic!("injected panic {id}"); }
                    content(window, cx)
                }) {
                    Ok(element) => return element,
                    Err(reason) => state = PanelState::Failed { reason },
                }
            } else {
                state = PanelState::Failed { reason: "missing content builder".into() };
            }
        }
        let lang = *cx.global::<Lang>();
        let (title, reason) = copy(lang, &state).expect("non-ready panel has copy");
        let t = cx.tokens();
        let d = density(cx);
        let label = match lang { Lang::En => "Retry", Lang::ZhTw => "重試" };
        let mut button = Button::new(retry_for_state(id, &state).expect("non-ready panel has retry"), label, &t)
            .variant(ButtonVariant::Secondary);
        if let Some(retry) = retry { button = button.on_click(move |event, window, cx| retry(event, window, cx)); }
        div().flex().flex_col().gap(space(d, Step::Sm)).p(space(d, Step::Md))
            .text_color(t.text).child(div().text_size(font(d, TypeSize::Title)).child(title.test_text(title)))
            .child(div().text_color(t.muted).text_size(font(d, TypeSize::Body)).child(reason.test_text(reason)))
            .child(button.test_text(label)).into_any_element()
    }
}
