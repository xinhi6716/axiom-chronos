mod ux_support;
use axiom::base::{Failure, Res};
use axiom::desktop::model::{Backend, WishRequest};
use axiom::desktop::{
    demo,
    i18n::Lang,
    model::Screen,
    ux::{
        seams::{FakeGh, FakeOptimizer, GhOutcome, Proposal},
        Page,
    },
};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, w, cx| w.click(id, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn value(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> String {
    cx.update_window(window, |_, w, _| {
        w.elements()
            .into_iter()
            .find(|e| e.path().contains(&id.into()) && e.value().is_some())
            .and_then(|e| e.value().map(str::to_owned))
            .unwrap_or_default()
    })
    .unwrap()
}
fn set(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str, text: &str) {
    click(cx, window, id);
    cx.update_window(window, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.input(text, cx);
    })
    .unwrap();
    ux_support::capture(cx, window);
}
fn wait_for(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    for _ in 0..100 {
        ux_support::capture(cx, window);
        if cx
            .update_window(window, |_, w, _| w.try_find(id).is_some())
            .unwrap()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("missing {id}");
}
fn proposal() -> Proposal {
    Proposal {
        goal: "Verified goal".into(),
        scope: "Scope".into(),
        limits: "Limits".into(),
        acceptance: "Acceptance".into(),
        ambiguities: vec!["Which outcome?".into()],
    }
}
fn gh_success() -> GhOutcome {
    GhOutcome::Exit { code: 0, stdout: r#"{"number":42,"title":"Issue title","body":"Issue body","url":"https://github.com/acme/repo/issues/42","labels":[]}"#.into(), stderr: String::new() }
}
fn open(
    cx: &mut TestAppContext,
    lang: Lang,
    state: axiom::desktop::model::AppState,
) -> (
    AnyWindowHandle,
    gpui_kit::Entity<axiom::desktop::model::AppState>,
) {
    let (window, state) = ux_support::open(cx, lang, state, Screen::Wishes);
    state.update(cx, |s, cx| s.set_composing(true, cx));
    ux_support::capture(cx, window);
    (window, state)
}

#[gpui_kit::test]
fn composer_workflows_in_both_languages(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let optimizer = Arc::new(FakeOptimizer::new(vec![
            Ok(proposal()),
            Ok(proposal()),
            Err(Failure::new("timeout", "timeout")),
            Ok(proposal()),
            Err(Failure::new("failed", "unavailable")),
            Ok(proposal()),
        ]));
        let gh = Arc::new(FakeGh::new(vec![
            gh_success(),
            GhOutcome::Exit {
                code: 1,
                stdout: String::new(),
                stderr: "gh auth login".into(),
            },
        ]));
        let mut state = demo::state();
        state.ux.seams.optimizer = Some(optimizer.clone());
        state.ux.seams.github = Some(gh.clone());
        state.ux.composer_prefill = Some("original".into());
        let (window, state) = open(cx, lang, state);
        assert_eq!(optimizer.calls(), 0);
        assert_eq!(value(cx, window, "composer-wish"), "original");
        assert!(state.read_with(cx, |s, _| s.ux.composer_prefill.is_none()));
        click(cx, window, "optimizer-run");
        wait_for(cx, window, "optimizer-diff");
        set(cx, window, "composer-wish", "must not replace original");
        assert_eq!(value(cx, window, "composer-wish"), "original");
        cx.update_window(window, |_, w, cx| {
            w.click(("optimizer-answer", 0_usize), cx);
            w.input("Chosen outcome", cx);
        })
        .unwrap();
        ux_support::capture(cx, window);
        click(cx, window, "optimizer-adopt");
        assert!(value(cx, window, "composer-wish").contains("Chosen outcome"));
        assert!(value(cx, window, "composer-wish").contains("Verified goal"));
        set(cx, window, "composer-wish", "keep this");
        click(cx, window, "optimizer-run");
        wait_for(cx, window, "optimizer-diff");
        click(cx, window, "optimizer-cancel");
        assert_eq!(value(cx, window, "composer-wish"), "keep this");
        click(cx, window, "optimizer-run");
        wait_for(cx, window, "optimizer-retry");
        click(cx, window, "optimizer-retry");
        wait_for(cx, window, "optimizer-diff");
        click(cx, window, "optimizer-cancel");
        click(cx, window, "optimizer-run");
        wait_for(cx, window, "optimizer-retry");
        click(cx, window, "optimizer-retry");
        wait_for(cx, window, "optimizer-diff");
        click(cx, window, "optimizer-cancel");
        click(cx, window, "wish-templates");
        click(cx, window, "fix-bug");
        assert_eq!(value(cx, window, "composer-wish"), "keep this");
        click(cx, window, "ask-back");
        assert_eq!(value(cx, window, "composer-wish"), "keep this");
        click(cx, window, "wish-templates");
        click(cx, window, "fix-bug");
        click(cx, window, "ask-confirm");
        assert_ne!(value(cx, window, "composer-wish"), "keep this");
        click(cx, window, "github-import");
        set(cx, window, "github-import-field", "acme/repo#42");
        click(cx, window, "github-import-go");
        for _ in 0..100 {
            ux_support::capture(cx, window);
            if value(cx, window, "composer-wish").contains("Issue title") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(value(cx, window, "composer-wish").contains("Issue body"));
        assert_eq!(
            gh.calls()[0],
            [
                "issue",
                "view",
                "42",
                "--repo",
                "acme/repo",
                "--json",
                "number,title,body,url,labels"
            ]
        );
        click(cx, window, "github-import");
        set(cx, window, "github-import-field", "acme/repo#42");
        click(cx, window, "github-import-go");
        wait_for(cx, window, "github-import-error");
        assert!(ux_support::shows(
            &ux_support::capture(cx, window),
            "github-import-error",
            if lang == Lang::En {
                "Sign in with gh auth login."
            } else {
                "請使用 gh auth login 登入。"
            }
        ));
        for bad in ["acme/repo#42;pwd", "acme/repo#42&pwd", "acme/repo#$(pwd)"] {
            set(cx, window, "github-import-field", bad);
            click(cx, window, "github-import-go");
        }
        assert_eq!(gh.calls().len(), 2);
        set(cx, window, "composer-wish", "carry this");
        click(cx, window, "mode-chat");
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Chat)
        );
        assert!(!state.read_with(cx, |s, _| s.composing));
    }
}

#[gpui_kit::test]
fn live_seams_hide_mode_picker(cx: &mut TestAppContext) {
    let mut state = demo::state();
    state.ux.seams.optimizer = None;
    state.ux.seams.github = None;
    state.ux.seams.modes = None;
    let (window, _) = open(cx, Lang::En, state);
    assert!(cx
        .update_window(window, |_, w, _| w.try_find("mode-picker").is_none())
        .unwrap());
}

#[derive(Clone)]
struct RecordingBackend(Rc<RefCell<Vec<WishRequest>>>);
impl Backend for RecordingBackend {
    fn start_wish(&self, request: WishRequest) -> Res<()> {
        self.0.borrow_mut().push(request);
        Ok(())
    }
    fn interject(&self, _: &str, _: &str) -> Res<()> {
        Ok(())
    }
    fn stop(&self, _: &str) -> Res<()> {
        Ok(())
    }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> {
        Ok(())
    }
    fn delete_memory(&self, _: i64) -> Res<()> {
        Ok(())
    }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> {
        Ok(())
    }
    fn refresh(&self) -> Res<()> {
        Ok(())
    }
    fn recheck(&self) -> Res<()> {
        Ok(())
    }
    fn follow(&self, _: &str) -> Res<()> {
        Ok(())
    }
}

#[gpui_kit::test]
fn start_submits_current_box_after_each_workflow(cx: &mut TestAppContext) {
    for workflow in ["adopt", "github", "template"] {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let mut state = demo::state();
        state.backend = Some(Box::new(RecordingBackend(requests.clone())));
        let folder =
            std::env::temp_dir().join(format!("axiom-composer-{}-{workflow}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let index = state.current.unwrap();
        state.runs[index].cwd = Some(folder.clone());
        state.ux.composer_prefill = Some("  original wish  ".into());
        state.ux.seams.optimizer = Some(Arc::new(FakeOptimizer::new(vec![Ok(proposal())])));
        state.ux.seams.github = Some(Arc::new(FakeGh::new(vec![gh_success()])));
        let (window, _) = open(cx, Lang::En, state);
        match workflow {
            "adopt" => {
                click(cx, window, "optimizer-run");
                wait_for(cx, window, "optimizer-diff");
                click(cx, window, "optimizer-adopt");
            }
            "github" => {
                click(cx, window, "github-import");
                set(cx, window, "github-import-field", "acme/repo#42");
                click(cx, window, "github-import-go");
                for _ in 0..100 {
                    ux_support::capture(cx, window);
                    if value(cx, window, "composer-wish").contains("Issue title") {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(value(cx, window, "composer-wish").contains("Issue title"));
            }
            _ => {
                click(cx, window, "wish-templates");
                click(cx, window, "fix-bug");
                click(cx, window, "ask-confirm");
            }
        }
        let box_text = value(cx, window, "composer-wish");
        assert!(!box_text.trim().is_empty());
        for _ in 0..200 {
            ux_support::capture(cx, window);
            let enabled = cx
                .update_window(window, |_, w, _| {
                    w.elements().iter().any(|e| {
                        e.path().contains(&"composer-start".into()) && e.disabled() == Some(false)
                    })
                })
                .unwrap();
            if enabled {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        click(cx, window, "composer-start");
        assert_eq!(
            requests.borrow().len(),
            1,
            "{workflow}: start did not reach backend"
        );
        assert_eq!(requests.borrow()[0].wish, box_text.trim(), "{workflow}");
        std::fs::remove_dir(&folder).unwrap();
    }
}

#[gpui_kit::test]
fn other_mode_choices_close_composer_and_wish_stays(cx: &mut TestAppContext) {
    for (id, page) in [("mode-quick", Page::QuickTask), ("mode-plaza", Page::Plaza)] {
        let (window, state) = open(cx, Lang::En, demo::state());
        set(cx, window, "composer-wish", "carry this");
        click(cx, window, "mode-wish");
        assert!(state.read_with(cx, |s, _| s.composing));
        assert_eq!(value(cx, window, "composer-wish"), "carry this");
        click(cx, window, id);
        assert_eq!(state.read_with(cx, |s, _| s.ux.page.clone()), Some(page));
        assert!(!state.read_with(cx, |s, _| s.composing));
    }
}

#[gpui_kit::test]
fn blank_wish_disables_optimizer(cx: &mut TestAppContext) {
    let optimizer = Arc::new(FakeOptimizer::new(vec![Ok(proposal())]));
    let mut state = demo::state();
    state.ux.seams.optimizer = Some(optimizer.clone());
    let (window, _) = open(cx, Lang::En, state);
    set(cx, window, "composer-wish", "   ");
    let disabled = cx
        .update_window(window, |_, w, _| {
            w.elements()
                .iter()
                .any(|e| e.path().contains(&"optimizer-run".into()) && e.disabled() == Some(true))
        })
        .unwrap();
    assert!(disabled);
    click(cx, window, "optimizer-run");
    assert_eq!(optimizer.calls(), 0);
}
