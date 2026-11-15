mod ux_support;

use axiom::desktop::{
    demo,
    i18n::Lang,
    model::{AppState, LogLevel, LogLine, Screen, Selection, UnitState},
    ux::{
        seams::{
            ApprovalCard, ApprovalService, FakeApprovals, FakeSessions, FakeUnitActions,
            SessionKind, UnitEvidence, WorkerSession,
        },
        Page,
    },
};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use std::sync::Arc;

fn click(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
    cx.update_window(w, |_, w, cx| w.click(id, cx)).unwrap();
    ux_support::capture(cx, w);
}
fn press(cx: &mut TestAppContext, w: AnyWindowHandle, key: &str) {
    cx.update_window(w, |_, w, cx| w.press(key, cx)).unwrap();
    ux_support::capture(cx, w);
}
fn input(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str, text: &str) {
    click(cx, w, id);
    cx.update_window(w, |_, w, cx| w.input(text, cx)).unwrap();
    ux_support::capture(cx, w);
    let value = cx
        .update_window(w, |_, w, _| {
            w.elements()
                .into_iter()
                .find(|e| e.path().contains(&id.into()) && e.value().is_some())
                .and_then(|e| e.value().map(str::to_owned))
        })
        .unwrap();
    assert!(
        value.as_deref().is_some_and(|v| v.contains(text)),
        "input {id} value {value:?}"
    );
}
fn shows(cx: &mut TestAppContext, w: AnyWindowHandle, id: &str, text: &str) {
    let lines = ux_support::capture(cx, w);
    assert!(ux_support::shows(&lines, id, text), "missing {id}: {text}");
}
fn disabled(cx: &mut TestAppContext, w: AnyWindowHandle, id: &str, expected: bool) {
    ux_support::capture(cx, w);
    let id: ElementId = id.to_owned().into();
    let (found, is_disabled) = cx
        .update_window(w, |_, w, _| {
            let elements = w.elements();
            let matches: Vec<_> = elements.iter().filter(|e| e.path().contains(&id)).collect();
            (
                !matches.is_empty(),
                matches.iter().any(|e| e.disabled() == Some(true)),
            )
        })
        .unwrap();
    assert!(found, "missing {id:?}");
    assert_eq!(is_disabled, expected, "{id:?} disabled state");
}
fn clean() -> AppState {
    let mut s = demo::state();
    let current = s.current.unwrap();
    let run = s.runs[current].clone();
    s.runs = vec![run];
    s.current = Some(0);
    s.ux.activity.clear();
    s.ux.notices.clear();
    s.ux.seams.interjections = None;
    let run = &mut s.runs[0];
    run.needs.clear();
    run.warnings.clear();
    run.decisions.clear();
    run.log.clear();
    s.selection = Selection::None;
    s
}
fn card(s: &AppState, id: &str) -> ApprovalCard {
    ApprovalCard {
        action_id: id.into(),
        version: 1,
        session: s.run().unwrap().session.to_string(),
        action: "Publish".into(),
        target: "Report".into(),
        scope: "Report only".into(),
        cost_usd: Some(1.0),
    }
}
fn tr(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    if lang == Lang::En {
        en
    } else {
        zh
    }
}

#[gpui_kit::test]
fn recent_events_are_summaries_with_expandable_evidence(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = clean();
        s.ux.seams.approvals = None;
        s.runs[s.current.unwrap()].log = vec![LogLine {
            time: "10:00".into(),
            source: "chronos".into(),
            text: r#"{"type":"chronos","subtype":"max_turns","turn_limit":40}"#.into(),
            level: LogLevel::Warn,
        }];
        let (w, _) = ux_support::open(cx, lang, s, Screen::Run);
        let lines = ux_support::capture(cx, w);
        assert!(
            ux_support::shows(
                &lines,
                "event-summary",
                tr(lang, "Stopped at the turn limit", "已達回合上限而停止")
            ),
            "{lines:#?}"
        );
        assert!(!lines
            .iter()
            .any(|l| l.path.iter().any(|id| id == "event") && l.text.starts_with("{\"")));
        assert!(!ux_support::shows(&lines, "event-raw", "turn_limit"));
        click(cx, w, "event-raw-toggle");
        shows(cx, w, "event-raw", "turn_limit");
        click(cx, w, "event-raw-toggle");
        assert!(!ux_support::shows(
            &ux_support::capture(cx, w),
            "event-raw",
            "turn_limit"
        ));
    }
}

#[gpui_kit::test]
#[ignore = "3.0.0-beta.1: the unfinished run page clips the approval receipt out of view; the official 3.0.0 re-enables this test"]
fn approvals_are_idempotent_and_require_fresh_review(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let mut s = clean();
        let approvals = Arc::new(FakeApprovals::new(vec![card(&s, "approve")]));
        s.ux.seams.approvals = Some(approvals.clone());
        let (w, state) = ux_support::open(cx, lang, s, Screen::Run);
        shows(cx, w, "approval-card", "Publish");
        click(cx, w, "approval-approve");
        press(cx, w, "enter");
        shows(
            cx,
            w,
            "approval-receipt",
            tr(lang, "Done: Publish: Report", "已執行：Publish: Report"),
        );
        click(cx, w, "approval-approve");
        assert_eq!(approvals.executions("approve"), 1);
        let index = state.read_with(cx, |s, _| {
            s.run()
                .unwrap()
                .units
                .iter()
                .position(|u| u.state == UnitState::Failed)
                .unwrap()
        });
        state.update(cx, |s, cx| s.select(Selection::Unit(index), cx));
        ux_support::capture(cx, w);
        click(cx, w, "detail-back");
        shows(cx, w, "approval-receipt", tr(lang, "Done:", "已執行："));
        click(cx, w, "approval-approve");
        assert_eq!(approvals.executions("approve"), 1);

        let mut s = clean();
        let reject = Arc::new(FakeApprovals::new(vec![card(&s, "reject")]));
        s.ux.seams.approvals = Some(reject.clone());
        let (w, _) = ux_support::open(cx, lang, s, Screen::Run);
        disabled(cx, w, "approval-reject", true);
        click(cx, w, "approval-reject");
        assert_eq!(reject.pending().len(), 1);
        input(cx, w, "approval-reason", "not yet");
        disabled(cx, w, "approval-reject", false);
        click(cx, w, "approval-reject");
        shows(cx, w, "approval-receipt", tr(lang, "Rejected", "已拒絕"));
        assert_eq!(reject.executions("reject"), 0);

        let mut s = clean();
        let stale = Arc::new(FakeApprovals::new(vec![card(&s, "stale")]));
        s.ux.seams.approvals = Some(stale.clone());
        let (w, _) = ux_support::open(cx, lang, s, Screen::Run);
        shows(cx, w, "approval-card", "Publish");
        stale.bump_version("stale");
        click(cx, w, "approval-approve");
        shows(
            cx,
            w,
            "approval-receipt",
            tr(
                lang,
                "Changed since you looked. Review it again.",
                "內容已變更，請重新檢視。",
            ),
        );
        assert_eq!(stale.executions("stale"), 0);
        assert_eq!(stale.pending()[0].version, 2);
        click(cx, w, "approval-approve");
        shows(cx, w, "approval-receipt", tr(lang, "Done:", "已執行："));
        assert_eq!(stale.executions("stale"), 1);
    }
}

fn failed_state() -> (AppState, Arc<FakeUnitActions>, String) {
    let mut s = clean();
    let run = s.run().unwrap();
    let index = run
        .units
        .iter()
        .position(|u| u.state == UnitState::Failed)
        .unwrap();
    let unit = run.units[index].id.to_string();
    let session = run.session.to_string();
    let actions = Arc::new(FakeUnitActions::new());
    actions.set_evidence(
        &session,
        &unit,
        UnitEvidence {
            failure: Some("verification failed".into()),
            ..Default::default()
        },
    );
    s.selection = Selection::Unit(index);
    s.ux.seams.unit_actions = Some(actions.clone());
    (s, actions, unit)
}

#[gpui_kit::test]
fn failed_unit_retry_and_waiver(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let (s, actions, unit) = failed_state();
        let (w, _) = ux_support::open(cx, lang, s, Screen::Run);
        shows(cx, w, "unit-evidence", "verification failed");
        click(cx, w, "unit-retry");
        shows(cx, w, "unit-action-receipt", "retry accepted");
        click(cx, w, "unit-retry");
        assert_eq!(actions.count(&unit, "retry"), 1);

        let (s, actions, unit) = failed_state();
        let (w, _) = ux_support::open(cx, lang, s, Screen::Run);
        click(cx, w, "unit-waive");
        disabled(cx, w, "unit-waive-confirm", true);
        click(cx, w, "unit-waive-confirm");
        assert_eq!(actions.count(&unit, "waive"), 0);
        input(cx, w, "unit-waive-reason", " ");
        disabled(cx, w, "unit-waive-confirm", true);
        input(cx, w, "unit-waive-reason", "accepted risk");
        disabled(cx, w, "unit-waive-confirm", false);
        click(cx, w, "unit-waive-confirm");
        shows(cx, w, "unit-action-receipt", "waive accepted");
        assert_eq!(actions.count(&unit, "waive"), 1);
    }
}

fn watch_session_opens_the_selected_worker_session(cx: &mut TestAppContext, lang: Lang) {
    let mut s = clean();
    let run = s.run().unwrap();
    let index = run
        .units
        .iter()
        .position(|u| u.state == UnitState::Running)
        .unwrap();
    let session = WorkerSession {
        id: "terminal".into(),
        run: run.session.to_string(),
        unit: Some(run.units[index].id.to_string()),
        title: "Terminal".into(),
        kind: SessionKind::Terminal,
    };
    let sessions = Arc::new(FakeSessions::new(vec![session]));
    s.selection = Selection::Unit(index);
    s.ux.seams.sessions = Some(sessions);
    let (w, state) = ux_support::open(cx, lang, s, Screen::Run);
    shows(cx, w, "watch-session", tr(lang, "Watch session", "觀看工作階段"));
    click(cx, w, "watch-session");
    assert_eq!(
        state.read_with(cx, |s, _| s.ux.page.clone()),
        Some(Page::Session("terminal".into()))
    );
}

#[gpui_kit::test]
fn session_en(cx: &mut TestAppContext) {
    watch_session_opens_the_selected_worker_session(cx, Lang::En);
}
#[gpui_kit::test]
fn session_zh_tw(cx: &mut TestAppContext) {
    watch_session_opens_the_selected_worker_session(cx, Lang::ZhTw);
}
