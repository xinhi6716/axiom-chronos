mod ux_support;

use axiom::desktop::{
    data::faults::{self, Fault, FaultPlan},
    demo,
    i18n::Lang,
    model::Screen,
    ux::{
        seams::{FakeStorage, FakeTurnPolicy, FastCost, StorageItem, TurnPolicy},
        Page,
    },
    Store,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn item(id: &str, reclaimable: bool) -> StorageItem {
    StorageItem {
        id: id.into(),
        path: format!("/data/{id}"),
        category: "cache".into(),
        project: Some("project".into()),
        wish: Some("wish".into()),
        bytes: 1024,
        measured: true,
        reclaimable,
    }
}

#[gpui_kit::test]
fn settings_and_storage_faults_retry_without_hiding_quota(cx: &mut TestAppContext) {
    for panel in ["settings.page", "storage.page"] {
        for fault in [Fault::Corrupt, Fault::Panic] {
            cx.update(|cx| faults::install(FaultPlan(HashMap::from([(panel.to_owned(), fault)])), cx));
            let mut state = demo::state();
            state.ux.seams.storage = Some(storage());
            if panel == "storage.page" { state.ux.page = Some(Page::Storage); }
            let (window, _) = ux_support::open(cx, Lang::En, state, Screen::Doctor);
            let retry = format!("retry:{panel}");
            let has = |cx: &mut TestAppContext, id: &str| cx.update_window(window, |_, window, _| {
                window.try_find(gpui_kit::ElementId::Name(id.to_owned().into())).is_some()
            }).unwrap();
            for _ in 0..20 {
                if has(cx, &retry) { break; }
                tick(cx, window, Duration::from_millis(60));
                std::thread::sleep(Duration::from_millis(55));
            }
            assert!(has(cx, &retry), "{panel} did not show retry");
            assert!(has(cx, "quota-strip"));
            cx.update(|cx| faults::install(FaultPlan::default(), cx));
            click(cx, window, retry);
            let content = if panel == "settings.page" { "settings-extreme" } else { "storage-title" };
            for _ in 0..20 {
                if has(cx, content) { break; }
                tick(cx, window, Duration::from_millis(60));
                std::thread::sleep(Duration::from_millis(55));
            }
            assert!(has(cx, content), "{panel} did not recover");
        }
    }
}
fn storage() -> Arc<FakeStorage> {
    Arc::new(FakeStorage::new(
        vec![
            item("a", true),
            item("b", true),
            item("unselected", true),
            item("fixed", false),
        ],
        None,
    ))
}
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn tick(cx: &mut TestAppContext, window: AnyWindowHandle, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
    ux_support::capture(cx, window);
}
fn shows(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str, text: &str) -> bool {
    ux_support::shows(&ux_support::capture(cx, window), id, text)
}
fn open_storage(cx: &mut TestAppContext, lang: Lang, fake: &Arc<FakeStorage>) -> AnyWindowHandle {
    let mut state = demo::state();
    state.ux.seams.storage = Some(fake.clone());
    state.ux.page = Some(Page::Storage);
    ux_support::open(cx, lang, state, Screen::Doctor).0
}
fn preview(cx: &mut TestAppContext, window: AnyWindowHandle, lang: Lang) {
    let labels = if lang == Lang::En {
        ["Category", "Project", "Wish"]
    } else {
        ["類別", "專案", "願望"]
    };
    for label in labels {
        assert!(shows(cx, window, "storage-group", label));
    }
    assert!(shows(cx, window, "storage-row", "cache"), "{:?}", ux_support::capture(cx, window));
    click(cx, window, ("storage-group-choice", 1_usize));
    assert!(shows(cx, window, "storage-row", "project"));
    click(cx, window, ("storage-group-choice", 2_usize));
    assert!(shows(cx, window, "storage-row", "wish"));
    click(cx, window, ("storage-group-choice", 0_usize));
    click(cx, window, ("storage-item", 0_usize));
    click(cx, window, ("storage-item", 1_usize));
    assert!(shows(
        cx,
        window,
        "storage-preview-label",
        if lang == Lang::En {
            "Review cleanup"
        } else {
            "檢視清理項目"
        }
    ));
    click(cx, window, "action:storage.preview");
    let lines = ux_support::capture(cx, window);
    let plans: Vec<_> = lines
        .iter()
        .filter(|line| line.path.iter().any(|id| id == "storage-plan"))
        .collect();
    assert_eq!(plans.len(), 1, "{plans:?}");
    assert_eq!(plans[0].text, "/data/a — 1.0 KiB /data/b — 1.0 KiB");
    assert!(ux_support::shows(
        &lines,
        "storage-confirm-label",
        if lang == Lang::En {
            "Delete 2 items (2.0 KiB)"
        } else {
            "刪除 2 項（2.0 KiB）"
        }
    ));
}

#[gpui_kit::test]
fn storage_cleanup_flow(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        // Preview and waiting are non-destructive; cancel returns to the list.
        let fake = storage();
        let window = open_storage(cx, lang, &fake);
        preview(cx, window, lang);
        assert!(fake.deleted().is_empty());
        tick(cx, window, Duration::from_secs(2));
        fake.step();
        assert!(fake.deleted().is_empty());
        click(cx, window, "storage-cancel");
        assert!(shows(cx, window, "storage-row", "cache"));
        assert!(fake.deleted().is_empty());
        click(cx, window, "storage-back");
        assert!(shows(
            cx,
            window,
            "settings-storage-label",
            if lang == Lang::En {
                "Storage…"
            } else {
                "儲存空間…"
            }
        ));

        let fake = storage();
        let window = open_storage(cx, lang, &fake);
        preview(cx, window, lang);
        fake.touch();
        click(cx, window, "action:storage.confirm");
        assert!(shows(cx, window, "storage-failures", "Storage changed"));
        assert!(fake.deleted().is_empty());

        for failure in [false, true] {
            let fake = storage();
            let window = open_storage(cx, lang, &fake);
            preview(cx, window, lang);
            if failure {
                fake.fail_item("a");
            }
            assert!(fake.deleted().is_empty());
            click(cx, window, "action:storage.confirm");
            fake.step();
            tick(cx, window, Duration::from_millis(400));
            assert!(shows(
                cx,
                window,
                "storage-progress",
                if lang == Lang::En {
                    "1 remaining"
                } else {
                    "剩餘 1 項"
                }
            ));
            fake.step();
            tick(cx, window, Duration::from_millis(400));
            assert!(shows(
                cx,
                window,
                "storage-progress",
                if lang == Lang::En {
                    "Cleanup complete"
                } else {
                    "清理完成"
                }
            ));
            if failure {
                assert!(shows(
                    cx,
                    window,
                    "storage-failures",
                    "/data/a: Could not remove item"
                ));
                assert_eq!(fake.deleted(), vec!["b"]);
            } else {
                assert_eq!(fake.deleted(), vec!["a", "b"]);
            }
        }
    }
}

struct Home {
    path: PathBuf,
}
impl Home {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("axiom-ux-storage-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

#[gpui_kit::test]
fn settings_extreme_and_storage_entry(cx: &mut TestAppContext) {
    let home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        for cost in [Some(FastCost { multiplier: 2.0 }), None] {
            let policy = Arc::new(FakeTurnPolicy::new(cost));
            let mut state = demo::state();
            state.ux.seams.storage = None;
            state.ux.seams.turn_policy = Some(policy.clone());
            let (window, state) = ux_support::open(cx, lang, state, Screen::Doctor);
            let lines = ux_support::capture(cx, window);
            assert!(cx
                .update_window(window, |_, window, _| window
                    .try_find("settings-storage")
                    .is_none())
                .unwrap());
            assert!(ux_support::shows(
                &lines,
                "settings-extreme",
                if lang == Lang::En {
                    "Applies from the next Codex turn; a turn already running keeps its setting."
                } else {
                    "從下一個 Codex 回合起生效；進行中的回合維持原設定。"
                }
            ));
            let cost_text = match (lang, cost.is_some()) {
                (Lang::En, true) => "Fast usage: 2× Codex quota",
                (Lang::En, false) => "Fast usage cost: —",
                (Lang::ZhTw, true) => "快速用量：Codex 額度 2 倍",
                (Lang::ZhTw, false) => "快速用量成本：—",
            };
            assert!(ux_support::shows(&lines, "settings-extreme", cost_text));
            // Open an isolated persistent store after ux_support initialized the real Shell.
            let settings_path = home.path.join("app-settings.json");
            if settings_path.exists() {
                std::fs::remove_file(&settings_path).unwrap();
            }
            cx.update(|cx| cx.set_global(Store::at(Some(settings_path.clone()))));
            let before = policy.begin_turn();
            assert!(!before.extreme);
            click(cx, window, "action:settings.toggle-extreme");
            assert!(cx.update(|cx| Store::current(cx).ux.extreme));
            assert!(policy.current().extreme);
            let after = policy.begin_turn();
            assert!(after.extreme);
            assert_eq!(policy.turns(), vec![before, after]);
            tick(cx, window, Duration::from_secs(1));
            cx.update(|cx| cx.set_global(Store::at(Some(settings_path.clone()))));
            assert!(
                cx.update(|cx| Store::current(cx).ux.extreme),
                "saved extreme survives reopening"
            );

            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    state.ux.seams.storage = Some(storage());
                    cx.notify();
                })
            });
            assert!(shows(
                cx,
                window,
                "settings-storage-label",
                if lang == Lang::En {
                    "Storage…"
                } else {
                    "儲存空間…"
                }
            ));
            click(cx, window, "settings-storage");
            assert_eq!(
                state.read_with(cx, |state, _| state.ux.page.clone()),
                Some(Page::Storage)
            );
            assert!(shows(
                cx,
                window,
                "storage-page",
                if lang == Lang::En {
                    "Storage"
                } else {
                    "儲存空間"
                }
            ));
        }
    }
}
