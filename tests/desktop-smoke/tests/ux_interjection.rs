mod ux_support;
use axiom::base::{Failure, Res};
use axiom::desktop::{
    demo,
    i18n::Lang,
    model::{RunStatus, Screen},
    ux::seams::{
        Amendment, Draft, FakeInterjections, Interjections, Receipt, ReceiptStage, RunningWork,
    },
    Store,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct CountingSeam {
    fake: FakeInterjections,
    calls: AtomicUsize,
    fail_send: AtomicBool,
    attempts: Mutex<Vec<Draft>>,
    extra: Mutex<Vec<Amendment>>,
}
impl Interjections for CountingSeam {
    fn send(&self, draft: &Draft) -> Res<Receipt> {
        self.attempts.lock().unwrap().push(draft.clone());
        if self.fail_send.load(Ordering::SeqCst) {
            return Err(Failure::new("send", "failed"));
        }
        self.fake.send(draft)
    }
    fn receipt(&self, id: &str) -> Option<Receipt> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.fake.receipt(id)
    }
    fn history(&self, session: &str) -> Vec<Amendment> {
        let mut rows = self.fake.history(session);
        rows.extend(self.extra.lock().unwrap().clone());
        rows
    }
    fn running_work(&self, session: &str) -> RunningWork {
        self.fake.running_work(session)
    }
    fn pause_batch(&self, session: &str) -> Res<()> {
        self.fake.pause_batch(session)
    }
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    click(cx, window, "interject-editor");
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn tick(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.executor().advance_clock(Duration::from_millis(600));
    ux_support::capture(cx, window);
}
fn shows(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str, text: &str) -> bool {
    ux_support::shows(&ux_support::capture(cx, window), id, text)
}

#[gpui_kit::test]
fn panel_receipts_pause_replan_and_recovery(cx: &mut TestAppContext) {
    let previous_home = std::env::var_os("AXIOM_HOME");
    for lang in [Lang::En, Lang::ZhTw] {
        let seam = Arc::new(CountingSeam::default());
        let mut state = demo::state();
        let run = &mut state.runs[state.current.unwrap()];
        run.status = RunStatus::Active;
        run.hosted = true;
        let session = run.session.to_string();
        seam.fake.set_work(
            &session,
            RunningWork {
                stage: Some("Astra".into()),
                units: vec!["U1".into()],
                batch: Some("B1".into()),
            },
        );
        state.ux.seams.interjections = Some(seam.clone());
        let (window, _) = ux_support::open(cx, lang, state, Screen::Run);
        let settings_dir = std::env::temp_dir().join(format!(
            "axiom-interjection-settings-{}-{:?}",
            std::process::id(),
            lang
        ));
        std::env::set_var("AXIOM_HOME", &settings_dir);
        cx.update(|cx| cx.set_global(Store::open(false)));
        click(cx, window, "open-interjection");
        for (id, en, zh) in [
            (
                "interject-pause-first",
                "Pause the batch first",
                "先暫停批次",
            ),
            (
                "interject-replan",
                "Re-plan with Astra + council",
                "請 Astra 與議會重新規劃",
            ),
            ("interject-preview", "Preview", "預覽"),
            ("interject-send", "Send", "送出"),
        ] {
            assert!(shows(
                cx,
                window,
                id,
                if lang == Lang::En { en } else { zh }
            ));
        }
        tick(cx, window);
        assert_eq!(seam.calls.load(Ordering::SeqCst), 0);
        assert!(shows(
            cx,
            window,
            "running-work-warning",
            if lang == Lang::En {
                "Running now:"
            } else {
                "目前正在進行："
            }
        ));
        for value in ["Astra", "U1", "B1"] {
            assert!(shows(cx, window, "running-work-warning", value));
        }
        input(cx, window, "recover my draft");
        assert!(shows(cx, window, "interject-preview", "recover my draft"));
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        ux_support::capture(cx, window);
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert!(settings_dir.join("app-settings.json").exists());
        cx.update(|cx| cx.set_global(Store::open(false)));
        click(cx, window, "open-interjection");
        assert!(shows(cx, window, "interject-preview", "recover my draft"));
        click(cx, window, "interject-pause-first");
        click(cx, window, "interject-replan");
        let failed_id = ux_support::capture(cx, window)
            .iter()
            .find(|line| {
                line.text.contains("recover my draft")
                    && line.path.iter().any(|id| id == "interject-preview")
            })
            .and_then(|line| {
                line.path
                    .iter()
                    .find_map(|id| id.strip_prefix("interject-draft:"))
            })
            .expect("draft id is attached to the preview")
            .to_owned();
        seam.fake.set_pause_failure(true);
        click(cx, window, "interject-send");
        assert!(seam.fake.sent().is_empty());
        assert!(shows(
            cx,
            window,
            "interject-error",
            if lang == Lang::En {
                "Pause failed"
            } else {
                "暫停失敗"
            }
        ));
        seam.fake.set_pause_failure(false);
        seam.fail_send.store(true, Ordering::SeqCst);
        click(cx, window, "interject-send");
        assert_eq!(seam.attempts.lock().unwrap()[0].client_id, failed_id);
        seam.fail_send.store(false, Ordering::SeqCst);
        click(cx, window, "interject-send");
        click(cx, window, "interject-send");
        let sent = seam.fake.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].client_id, failed_id);
        assert!(seam
            .attempts
            .lock()
            .unwrap()
            .iter()
            .all(|d| d.client_id == failed_id));
        assert!(sent[0].replan && sent[0].pause_first);
        let id = sent[0].client_id.clone();
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Sent"
            } else {
                "已送出"
            }
        ));
        seam.fake.advance(&id, ReceiptStage::Received);
        tick(cx, window);
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Received by the core"
            } else {
                "核心已收到"
            }
        ));
        seam.fake.advance(&id, ReceiptStage::Sent);
        tick(cx, window);
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Received by the core"
            } else {
                "核心已收到"
            }
        ));
        seam.fake.advance(
            &id,
            ReceiptStage::Applied {
                first_action: "plan".into(),
            },
        );
        tick(cx, window);
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Applied: plan"
            } else {
                "已處理：plan"
            }
        ));
        assert!(shows(cx, window, "amendment-history", "recover my draft"));
        let calls_at_final = seam.calls.load(Ordering::SeqCst);
        seam.fake.advance(
            &id,
            ReceiptStage::Rejected {
                reason: "late".into(),
            },
        );
        tick(cx, window);
        tick(cx, window);
        assert_eq!(seam.calls.load(Ordering::SeqCst), calls_at_final);
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Applied: plan"
            } else {
                "已處理：plan"
            }
        ));
        click(cx, window, "interject-editor");
        cx.update_window(window, |_, window, cx| window.press("ctrl-a", cx))
            .unwrap();
        input(cx, window, "second amendment");
        click(cx, window, "interject-send");
        let second = seam.fake.sent().last().unwrap().client_id.clone();
        seam.fake.advance(
            &second,
            ReceiptStage::Rejected {
                reason: "scope".into(),
            },
        );
        tick(cx, window);
        assert!(shows(
            cx,
            window,
            "interject-receipt",
            if lang == Lang::En {
                "Rejected: scope"
            } else {
                "未採用：scope"
            }
        ));
        let calls_at_final = seam.calls.load(Ordering::SeqCst);
        tick(cx, window);
        assert_eq!(seam.calls.load(Ordering::SeqCst), calls_at_final);
        cx.update(|cx| cx.set_global(Store::open(true)));
        std::fs::remove_dir_all(settings_dir).unwrap();
    }
    if let Some(home) = previous_home {
        std::env::set_var("AXIOM_HOME", home);
    } else {
        std::env::remove_var("AXIOM_HOME");
    }
}

#[gpui_kit::test]
fn editor_grows_and_history_shows_timestamp_and_affected(cx: &mut TestAppContext) {
    let seam = Arc::new(CountingSeam::default());
    seam.extra.lock().unwrap().push(Amendment {
        client_id: None,
        text: "manual history".into(),
        at_ms: Some(1_700_000_000_000),
        stage: Some(ReceiptStage::Applied {
            first_action: "plan".into(),
        }),
        affected: vec!["unit-42".into(), "plan-7".into()],
        replan: false,
    });
    let mut state = demo::state();
    let run = &mut state.runs[state.current.unwrap()];
    run.status = RunStatus::Active;
    run.hosted = true;
    state.ux.seams.interjections = Some(seam);
    let (window, _) = ux_support::open(cx, Lang::En, state, Screen::Run);
    click(cx, window, "open-interjection");
    let lines = ux_support::capture(cx, window);
    let row = lines
        .iter()
        .find(|line| {
            line.path.iter().any(|id| id == "amendment-history")
                && line.text.contains("manual history")
        })
        .unwrap();
    let time = row.text.split(" · ").next().unwrap();
    assert_eq!(time.len(), 8);
    assert!(time.chars().enumerate().all(|(i, c)| if i == 2 || i == 5 {
        c == ':'
    } else {
        c.is_ascii_digit()
    }));
    assert!(row.text.contains("unit-42") && row.text.contains("plan-7"));
    let before = cx
        .update_window(window, |_, window, _| {
            window.find("interject-editor").bounds().size.height
        })
        .unwrap();
    input(
        cx,
        window,
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine",
    );
    let after = cx
        .update_window(window, |_, window, _| {
            window.find("interject-editor").bounds().size.height
        })
        .unwrap();
    assert!(
        after > before,
        "editor should grow: {before:?} -> {after:?}"
    );
}

#[gpui_kit::test]
fn restarted_history_polls_all_unfinished_rows_until_final(cx: &mut TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "axiom-interjection-poll-{}.json",
        std::process::id()
    ));
    let mut state = demo::state();
    let run = &mut state.runs[state.current.unwrap()];
    run.status = RunStatus::Active;
    run.hosted = true;
    let session = run.session.to_string();
    let fake = FakeInterjections::at(&path);
    for id in ["old-one", "old-two"] {
        fake.send(&Draft {
            session: session.clone(),
            client_id: id.into(),
            text: id.into(),
            pause_first: false,
            replan: false,
        })
        .unwrap();
    }
    drop(fake);
    let seam = Arc::new(CountingSeam {
        fake: FakeInterjections::at(&path),
        ..Default::default()
    });
    state.ux.seams.interjections = Some(seam.clone());
    let (window, _) = ux_support::open(cx, Lang::En, state, Screen::Run);
    click(cx, window, "open-interjection");
    input(cx, window, "new amendment");
    click(cx, window, "interject-send");
    let latest = seam.fake.sent()[0].client_id.clone();
    seam.fake.advance(
        &latest,
        ReceiptStage::Applied {
            first_action: "new done".into(),
        },
    );
    seam.fake.advance("old-one", ReceiptStage::Received);
    seam.fake.advance(
        "old-two",
        ReceiptStage::Applied {
            first_action: "done".into(),
        },
    );
    tick(cx, window);
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "Received by the core · old-one"
    ));
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "Applied: done · old-two"
    ));
    seam.fake.advance("old-one", ReceiptStage::Sent);
    seam.fake.advance(
        "old-two",
        ReceiptStage::Rejected {
            reason: "late".into(),
        },
    );
    tick(cx, window);
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "Received by the core · old-one"
    ));
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "Applied: done · old-two"
    ));
    seam.fake.advance(
        "old-one",
        ReceiptStage::Rejected {
            reason: "finished".into(),
        },
    );
    tick(cx, window);
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "Rejected: finished · old-one"
    ));
    let calls = seam.calls.load(Ordering::SeqCst);
    tick(cx, window);
    tick(cx, window);
    assert_eq!(seam.calls.load(Ordering::SeqCst), calls);
    std::fs::remove_file(path).unwrap();
}

#[gpui_kit::test]
fn history_is_recovered_and_blocked_opens_panel(cx: &mut TestAppContext) {
    let path = std::env::temp_dir().join(format!("axiom-interjection-{}.json", std::process::id()));
    let mut state = demo::state();
    let run = &mut state.runs[state.current.unwrap()];
    run.status = RunStatus::Blocked;
    let session = run.session.to_string();
    let seam = FakeInterjections::at(&path);
    let draft = Draft {
        session: session.clone(),
        client_id: "restart-id".into(),
        text: "persisted amendment".into(),
        pause_first: false,
        replan: false,
    };
    let first = seam.send(&draft).unwrap();
    assert_eq!(first, seam.send(&draft).unwrap());
    assert_eq!(seam.sent().len(), 1);
    seam.advance(
        &draft.client_id,
        ReceiptStage::Rejected {
            reason: "scope".into(),
        },
    );
    drop(seam);
    state.ux.seams.interjections = Some(Arc::new(FakeInterjections::at(&path)));
    let (window, _) = ux_support::open(cx, Lang::ZhTw, state, Screen::Run);
    click(cx, window, "open-interjection");
    assert!(shows(
        cx,
        window,
        "amendment-history",
        "persisted amendment"
    ));
    assert!(shows(cx, window, "amendment-history", "未採用：scope"));
    std::fs::remove_file(path).unwrap();
}

#[gpui_kit::test]
fn live_history_falls_back_to_amendments(cx: &mut TestAppContext) {
    let mut state = demo::state();
    let run = &mut state.runs[state.current.unwrap()];
    run.status = RunStatus::Active;
    run.hosted = true;
    let session = run.session.clone();
    state.ux.seams.interjections = None;
    state
        .ux
        .amendments
        .insert(session, vec!["original amendment".into()]);
    let (window, _) = ux_support::open(cx, Lang::En, state, Screen::Run);
    click(cx, window, "open-interjection");
    assert!(shows(cx, window, "amendment-history", "original amendment"));
    assert!(shows(cx, window, "amendment-history", "—"));
}
