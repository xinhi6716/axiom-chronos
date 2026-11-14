mod research_support;

use axiom::{
    base::{Failure, Res},
    desktop::{
        i18n::{self, Lang},
        research::{
            actions::{self, Actions},
            LoadState, ResearchRef, ResearchSeams, Workbench,
        },
        theme,
        ux::seams::{
            Amendment, Draft, FakeInterjections, Interjections, Receipt, ReceiptStage, RunningWork,
        },
    },
    research_workspace::annotations::{
        register_snapshot, AnnotationStore, Correction, Reverify, SnapshotEntry, Verdict,
    },
    seams::SystemClock,
};
use gpui_kit::{
    component::{v_flex, ThemeMode},
    px, size,
    test::TestWindowExt as _,
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _,
    Point, Render, TestAppContext, Window, WindowBounds, WindowOptions,
};
use research_support::*;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const TARGET: &str = "evidence:RE1";
const AFTER: &str = "runs/device_A.csv";
const WHY: &str = "source review";

struct Host {
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
}
impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.wb.read(cx);
        v_flex()
            .child(actions::spot_check_buttons(
                &self.actions,
                TARGET,
                Some("evidence"),
                cx,
            ))
            .child(actions::correction_form(&self.actions, window, cx))
            .child(actions::receipts(&self.actions, cx))
    }
}

fn open(
    cx: &mut TestAppContext,
    lang: Lang,
    root: &Path,
    home: &Path,
    bridge: Arc<dyn Interjections>,
) -> (AnyWindowHandle, Entity<Workbench>, Entity<Actions>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(lang, cx);
    });
    let seams = ResearchSeams {
        research_home: Some(home.to_owned()),
        goals_home: Some(temp_dir()),
        ..Default::default()
    };
    let wb = cx.new(|cx| Workbench::new(seams, Some(bridge), Arc::new(SystemClock), cx));
    wb.update(cx, |wb, cx| {
        wb.open(
            ResearchRef {
                root: root.to_owned(),
                store: None,
            },
            cx,
        )
    });
    let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(1100.), px(700.)),
        })),
        ..Default::default()
    };
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| {
                    cx.observe(&wb, |_, _, cx| cx.notify()).detach();
                    cx.observe(&actions, |_, _, cx| cx.notify()).detach();
                    Host {
                        wb: wb.clone(),
                        actions: actions.clone(),
                    }
                })
            })
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        frame(cx, window);
        let ready = wb.read_with(cx, |wb, _| match &wb.load {
            LoadState::Ready => true,
            LoadState::Failed { kind, message } => panic!("{kind}: {message}"),
            _ => false,
        });
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "workbench timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    (window, wb, actions)
}
fn wait_for(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: &str,
    text: &str,
) -> Vec<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, window);
        if shows(&lines, id, text) {
            return lines;
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {id} / {text}: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn store(home: &Path) -> AnnotationStore {
    AnnotationStore::open(home, "rawonly").unwrap()
}
fn wait_spot(cx: &mut TestAppContext, window: AnyWindowHandle, home: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        click(cx, window, "research-spot-confirm-0");
        let spots = store(home).spot_checks(Some(TARGET));
        if let Some(spot) = spots.first() {
            assert_eq!(spot.verdict, Verdict::Confirm);
            assert_eq!(spot.node.as_deref(), Some("evidence"));
            break;
        }
        assert!(Instant::now() < deadline, "spot-check did not persist");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn submit(cx: &mut TestAppContext, window: AnyWindowHandle, actions: &Entity<Actions>) {
    actions.update(cx, |a, cx| a.begin_correction(TARGET, "raw_file", cx));
    frame(cx, window);
    click(cx, window, "research-correct-after");
    type_text(cx, window, AFTER);
    click(cx, window, "research-correct-why");
    type_text(cx, window, WHY);
    click(cx, window, "research-correct-submit");
}
fn correction(home: &Path) -> Correction {
    let corrections = store(home).corrections(Some(TARGET));
    assert_eq!(corrections.len(), 1);
    corrections.into_iter().next().unwrap()
}
fn session_fixture() -> std::path::PathBuf {
    let root = fixture_copy("rawonly");
    let path = root.join("research-manifest.json");
    let manifest = std::fs::read_to_string(&path).unwrap().replacen(
        "\"research_id\": \"rawonly\",",
        "\"research_id\": \"rawonly\",\n  \"session\": \"s1\",",
        1,
    );
    assert!(manifest.contains("\"session\": \"s1\""));
    std::fs::write(path, manifest).unwrap();
    root
}

#[derive(Default)]
struct SwitchBridge {
    fail: AtomicBool,
    fake: FakeInterjections,
    attempts: Mutex<Vec<Draft>>,
}
impl Interjections for SwitchBridge {
    fn send(&self, d: &Draft) -> Res<Receipt> {
        self.attempts.lock().unwrap().push(d.clone());
        if self.fail.load(Ordering::SeqCst) {
            Err(Failure::new("offline", "offline"))
        } else {
            self.fake.send(d)
        }
    }
    fn receipt(&self, id: &str) -> Option<Receipt> {
        self.fake.receipt(id)
    }
    fn history(&self, id: &str) -> Vec<Amendment> {
        self.fake.history(id)
    }
    fn running_work(&self, id: &str) -> RunningWork {
        self.fake.running_work(id)
    }
    fn pause_batch(&self, id: &str) -> Res<()> {
        self.fake.pause_batch(id)
    }
}

fn exercise(cx: &mut TestAppContext, lang: Lang) {
    let (
        confirm,
        dispute,
        who,
        why,
        version,
        verify,
        passed,
        receipt,
        sent,
        received,
        applied,
        invalidated,
        kept,
        no_session,
        not_requested,
    ) = match lang {
        Lang::En => (
            "Confirm",
            "Dispute",
            "Who",
            "Why",
            "Version",
            "Re-verification",
            "Passed",
            "Receipt",
            "Sent",
            "Received",
            "Applied",
            "Invalidated",
            "Not sent; your request is kept",
            "There is no active session for this research.",
            "Not requested",
        ),
        _ => (
            "確認",
            "提出異議",
            "執行者",
            "原因",
            "版本",
            "重新驗證",
            "通過",
            "回執",
            "已送出",
            "已收到",
            "已套用",
            "受影響項目",
            "尚未送出；已保留您的請求",
            "此研究沒有進行中的工作階段。",
            "未請求",
        ),
    };
    let root = session_fixture();
    let hash = tree_hash(&root);
    let home = temp_dir();
    register_snapshot(
        &home,
        "rawonly",
        &SnapshotEntry {
            snapshot_id: "snapshot-evidence".into(),
            artifacts: vec!["evidence".into()],
            sha256: "hash".into(),
        },
    )
    .unwrap();
    let fake = Arc::new(FakeInterjections::new());
    let (window, _, actions) = open(cx, lang, &root, &home, fake.clone());
    let lines = capture(cx, window);
    assert!(
        shows(&lines, "research-spot-confirm-0", confirm)
            && shows(&lines, "research-spot-dispute-0", dispute),
        "{lines:?}"
    );
    wait_spot(cx, window, &home);
    submit(cx, window, &actions);
    let lines = wait_for(cx, window, "research-receipt-0", sent);
    let c = correction(&home);
    assert_eq!(
        (&*c.after, &*c.why, &*c.research_version),
        (AFTER, WHY, "1")
    );
    assert_eq!(c.reverify, Reverify::Passed);
    assert!(!c.who.is_empty());
    assert_eq!(c.invalidates, vec!["snapshot-evidence"]);
    for text in [
        format!("{who}: {}", c.who),
        format!("{why}: {WHY}"),
        format!("{version}: 1"),
        format!("{verify}: {passed}"),
        format!("{receipt}: {sent}"),
        format!("{invalidated}: {}", c.invalidates.len()),
    ] {
        assert!(
            shows(&lines, "research-receipt-0", &text),
            "{text}: {lines:?}"
        );
    }
    let id = c.receipt.unwrap().client_id;
    assert_eq!(fake.sent().len(), 1);
    for (stage, text) in [
        (ReceiptStage::Received, received),
        (
            ReceiptStage::Applied {
                first_action: "review".into(),
            },
            applied,
        ),
    ] {
        fake.advance(&id, stage);
        actions.update(cx, |a, cx| a.poll_receipts(cx));
        wait_for(cx, window, "research-receipt-0", text);
    }
    assert_eq!(
        correction(&home).receipt.unwrap().detail.as_deref(),
        Some("review")
    );
    assert_eq!(tree_hash(&root), hash);

    let root = session_fixture();
    let hash = tree_hash(&root);
    let home = temp_dir();
    let bridge = Arc::new(SwitchBridge::default());
    bridge.fail.store(true, Ordering::SeqCst);
    let (window, _, actions) = open(cx, lang, &root, &home, bridge.clone());
    wait_spot(cx, window, &home);
    submit(cx, window, &actions);
    wait_for(cx, window, "research-receipts", kept);
    let c = correction(&home);
    assert!(c.draft.as_ref().is_some_and(|d| !d.is_empty()) && c.receipt.is_none());
    assert_eq!(
        c.draft.as_deref(),
        Some(bridge.attempts.lock().unwrap()[0].text.as_str())
    );
    bridge.fail.store(false, Ordering::SeqCst);
    click(cx, window, "research-retry-0");
    wait_for(cx, window, "research-receipt-0", sent);
    let attempts = bridge.attempts.lock().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    assert_eq!(
        correction(&home).receipt.unwrap().client_id,
        attempts[0].client_id
    );
    assert_eq!(tree_hash(&root), hash);

    let root = fixture_copy("rawonly");
    let hash = tree_hash(&root);
    let home = temp_dir();
    let fake = Arc::new(FakeInterjections::new());
    let (window, _, actions) = open(cx, lang, &root, &home, fake.clone());
    wait_spot(cx, window, &home);
    actions.update(cx, |a, cx| a.begin_correction(TARGET, "raw_file", cx));
    let lines = capture(cx, window);
    assert!(
        lines.iter().any(|(_, t)| t.contains(no_session)),
        "{lines:?}"
    );
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-retry-0").is_none())
    })
    .unwrap();
    submit(cx, window, &actions);
    wait_for(cx, window, "research-receipt-0", not_requested);
    let c = correction(&home);
    assert!(c.receipt.is_none() && c.draft.is_none() && fake.sent().is_empty());
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-retry-0").is_none());
    })
    .unwrap();
    assert_eq!(tree_hash(&root), hash);
}

#[gpui_kit::test]
fn actions_en(cx: &mut TestAppContext) {
    exercise(cx, Lang::En);
}
#[gpui_kit::test]
fn actions_zh_tw(cx: &mut TestAppContext) {
    exercise(cx, Lang::ZhTw);
}
