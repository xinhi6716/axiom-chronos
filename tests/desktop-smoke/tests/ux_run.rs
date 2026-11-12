mod ux_support;

use axiom::{
    base::Res,
    desktop::{
        demo,
        i18n::Lang,
        model::{AppState, Backend, Phase, RunStatus, Screen, Selection, UnitState, WishRequest},
        ux::{
            activity::{OpenActivity, Projection},
            events::{Notice, NoticeKind},
            seams::{FakeSessions, SessionKind, WorkerSession},
            Page,
        },
    },
};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use std::{cell::RefCell, rc::Rc, sync::Arc};

const NOW: i64 = 1_000_000;
fn seed() -> AppState {
    let mut s = demo::state();
    s.ux.clock_ms = Some(NOW);
    let run = &mut s.runs[s.current.unwrap()];
    run.status = RunStatus::Active;
    for u in &mut run.units {
        u.state = UnitState::Built;
    }
    for stage in &mut run.stages {
        stage.phase = Phase::Waiting;
    }
    let key = run.session.clone();
    s.ux.notices.clear();
    s.ux.council.clear();
    s.ux.activity.insert(
        key,
        Projection {
            last_seen_ms: Some(NOW - 1000),
            last_activity_ms: Some(NOW - 12_000),
            open: vec![OpenActivity {
                id: "a".into(),
                module: "core".into(),
                actor: "core".into(),
                phase: "review".into(),
                since_ms: NOW - 12_000,
            }],
            ..Default::default()
        },
    );
    s
}
fn has(cx: &mut TestAppContext, w: AnyWindowHandle, id: &str) -> bool {
    let id: ElementId = id.to_owned().into();
    cx.update_window(w, |_, w, _| {
        w.elements().iter().any(|e| e.path().contains(&id))
    })
    .unwrap()
}
fn click_in(cx: &mut TestAppContext, w: AnyWindowHandle, scope: &str, button: &str) {
    let scope: ElementId = scope.to_owned().into();
    cx.update_window(w, |_, w, cx| {
        let b = w
            .elements()
            .iter()
            .find(|e| {
                e.path().contains(&scope)
                    && e.path().iter().any(|id| match id {
                        ElementId::Name(n) | ElementId::NamedInteger(n, _) => n.as_ref() == button,
                        _ => false,
                    })
            })
            .expect("button")
            .bounds();
        w.pointer_down(b.center(), gpui_kit::MouseButton::Left, cx);
        w.pointer_up(b.center(), gpui_kit::MouseButton::Left, cx);
    })
    .unwrap();
    ux_support::capture(cx, w);
}
fn shows(cx: &mut TestAppContext, w: AnyWindowHandle, id: &str, text: &str) -> bool {
    let lines = ux_support::capture(cx, w);
    let ok = ux_support::shows(&lines, id, text);
    if !ok {
        eprintln!(
            "MISSING {id}: {text}
{lines:#?}"
        );
    }
    ok
}
#[gpui_kit::test]
fn live_activity_and_watch(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let (w, state) = ux_support::open(cx, lang, seed(), Screen::Run);
        assert!(shows(
            cx,
            w,
            "run-heartbeat",
            if lang == Lang::En {
                "12 s ago"
            } else {
                "12 秒前"
            }
        ));
        assert!(shows(
            cx,
            w,
            "run-live-stage",
            if lang == Lang::En {
                "Astra planning"
            } else {
                "Astra 規劃中"
            }
        ));
        assert!(!shows(cx, w, "run-live-stage", "waiting"));
        assert!(shows(
            cx,
            w,
            "run-claude-activity",
            if lang == Lang::En {
                "Core · reviewing"
            } else {
                "主控 · 審查中"
            }
        ));
        assert!(shows(
            cx,
            w,
            "who-running",
            if lang == Lang::En {
                "Working now"
            } else {
                "正在工作"
            }
        ));
        assert!(shows(cx, w, "who-row", "Claude"));
        state.update(cx, |s, cx| {
            let key = s.run().unwrap().session.clone();
            let p = s.ux.activity.get_mut(&key).unwrap();
            p.last_seen_ms = None;
            cx.notify();
        });
        assert!(shows(
            cx,
            w,
            "run-heartbeat",
            if lang == Lang::En {
                "unknown"
            } else {
                "未知"
            }
        ));
        state.update(cx, |s, cx| {
            let key = s.run().unwrap().session.clone();
            s.ux.activity.get_mut(&key).unwrap().last_seen_ms = Some(NOW - 180_000);
            cx.notify();
        });
        assert!(shows(
            cx,
            w,
            "run-heartbeat",
            if lang == Lang::En {
                "No signal for 3 min"
            } else {
                "3 分鐘沒有訊號"
            }
        ));
        state.update(cx, |s, cx| {
            let run = &mut s.runs[s.current.unwrap()];
            run.units[0].state = UnitState::Running;
            let key = run.session.to_string();
            let unit = run.units[0].id.to_string();
            s.ux.seams.sessions = Some(Arc::new(FakeSessions::new(vec![
                WorkerSession {
                    id: "wrong-run".into(),
                    run: "another-run".into(),
                    unit: Some(unit.clone()),
                    title: "wrong".into(),
                    kind: SessionKind::Terminal,
                },
                WorkerSession {
                    id: "watch-me".into(),
                    run: key,
                    unit: Some(unit),
                    title: "test".into(),
                    kind: SessionKind::Terminal,
                },
            ])));
            cx.notify();
        });
        assert!(!has(cx, w, "run-claude-activity"));
        assert!(shows(cx, w, "who-row", "Codex"));
        assert!(shows(
            cx,
            w,
            "watch-session",
            if lang == Lang::En { "Watch" } else { "觀看" }
        ));
        click_in(cx, w, "who-running", "watch-session");
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Session("watch-me".into()))
        );
        state.update(cx, |s, cx| {
            s.ux.page = None;
            s.ux.seams.sessions = None;
            cx.notify();
        });
        ux_support::capture(cx, w);
        assert!(!has(cx, w, "watch-session"));
        state.update(cx, |s, cx| {
            s.ux.activity.clear();
            s.ux.notices.clear();
            cx.notify();
        });
        ux_support::capture(cx, w);
        for id in [
            "run-heartbeat",
            "run-live-stage",
            "run-claude-activity",
            "who-running",
            "notice-max-turns",
            "notice-degraded-council",
            "notice-scope-failure",
        ] {
            assert!(!has(cx, w, id), "{id}");
        }
    }
}
struct BackendSpy(Rc<RefCell<Vec<(String, Option<String>)>>>);
impl Backend for BackendSpy {
    fn start_wish(&self, _: WishRequest) -> Res<()> {
        Ok(())
    }
    fn interject(&self, _: &str, _: &str) -> Res<()> {
        Ok(())
    }
    fn stop(&self, _: &str) -> Res<()> {
        Ok(())
    }
    fn resume(&self, session: &str, note: Option<&str>) -> Res<()> {
        self.0
            .borrow_mut()
            .push((session.into(), note.map(str::to_owned)));
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
fn notices_and_actions(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut s = seed();
        s.backend = Some(Box::new(BackendSpy(calls.clone())));
        let run = &mut s.runs[s.current.unwrap()];
        run.modules[0].id = "council".into();
        let key = run.session.clone();
        let unit = run.units[1].id.to_string();
        let notices = vec![
            Notice {
                key: "max".into(),
                kind: NoticeKind::MaxTurns {
                    turn_limit: Some(40),
                },
                at_ms: None,
            },
            Notice {
                key: "council".into(),
                kind: NoticeKind::DegradedCouncil {
                    member: "Fable".into(),
                    phase: "review".into(),
                    round: 1,
                    status: "failed".into(),
                    reason: "offline".into(),
                },
                at_ms: None,
            },
            Notice {
                key: "scope".into(),
                kind: NoticeKind::ScopeFailure {
                    unit,
                    allowed: vec!["src/a".into()],
                    outside: vec!["src/b".into()],
                    outside_count: 1,
                    ignored: vec![],
                },
                at_ms: None,
            },
        ];
        s.ux.activity.clear();
        s.ux.notices.insert(key.clone(), vec![notices[0].clone()]);
        let (w, state) = ux_support::open(cx, lang, s, Screen::Run);
        for (i, (id, title, action)) in [
            (
                "notice-max-turns",
                if lang == Lang::En {
                    "Stopped at the turn limit"
                } else {
                    "已達回合上限而停止"
                },
                if lang == Lang::En {
                    "Resume with more turns"
                } else {
                    "增加回合後接續"
                },
            ),
            (
                "notice-degraded-council",
                if lang == Lang::En {
                    "Council continued without Fable"
                } else {
                    "議會在缺少 Fable"
                },
                if lang == Lang::En {
                    "Open council"
                } else {
                    "開啟議會"
                },
            ),
            (
                "notice-scope-failure",
                if lang == Lang::En {
                    "Outside the allowed paths"
                } else {
                    "超出允許路徑"
                },
                if lang == Lang::En {
                    "Open unit"
                } else {
                    "開啟工作單元"
                },
            ),
        ]
        .into_iter()
        .enumerate()
        {
            state.update(cx, |s, cx| {
                s.ux.notices.insert(key.clone(), vec![notices[i].clone()]);
                cx.notify();
            });
            assert!(shows(cx, w, id, title));
            assert!(shows(cx, w, id, action));
            let (cause, effect) = match (i, lang) {
                (0, Lang::En) => ("40 turns", "The session stopped"),
                (0, _) => ("40 回合", "工作階段已停止"),
                (1, Lang::En) => ("offline", "remaining members"),
                (1, _) => ("offline", "其餘成員"),
                (_, Lang::En) => ("src/b", "unit needs attention"),
                (_, _) => ("src/b", "工作單元需要處理"),
            };
            assert!(shows(cx, w, id, cause));
            assert!(shows(cx, w, id, effect));
            click_in(cx, w, id, "notice-action");
            match i {
                0 => assert_eq!(&*calls.borrow(), &[(key.to_string(), None)]),
                1 => assert_eq!(
                    state.read_with(cx, |s, _| s.selection.clone()),
                    Selection::Module(0)
                ),
                _ => assert_eq!(
                    state.read_with(cx, |s, _| s.selection.clone()),
                    Selection::Unit(1)
                ),
            }
        }
        state.update(cx, |s, cx| {
            s.runs[s.current.unwrap()].modules.clear();
            s.selection = Selection::Unit(0);
            s.ux.notices.insert(key.clone(), vec![notices[1].clone()]);
            cx.notify();
        });
        ux_support::capture(cx, w);
        click_in(cx, w, "notice-degraded-council", "notice-action");
        assert_eq!(
            state.read_with(cx, |s, _| s.selection.clone()),
            Selection::None
        );
    }
}
