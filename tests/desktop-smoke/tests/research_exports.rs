mod research_support;
use axiom::{
    desktop::{i18n::Lang, research::ResearchSeams},
    research_workspace::{
        index::{Filter, Op},
        manifest::{locate, LocateOptions},
        seams::{FakeSnapshotConsumer, FakeVault},
        session::{self, SessionPrefs},
    },
};
use gpui_kit::{component::ThemeMode, test::TestWindowExt as _, AppContext as _, TestAppContext};
use research_support::*;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn wait_for(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    id: &str,
    needle: &str,
) -> Vec<(String, String)> {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let lines = capture(cx, window);
        if shows(&lines, id, needle) {
            return lines;
        }
        assert!(Instant::now() < end, "waiting for {id} {needle}: {lines:?}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn setup(
    home: &Path,
    raw: bool,
    wired: bool,
) -> (ResearchSeams, Arc<FakeSnapshotConsumer>, Arc<FakeVault>) {
    let consumer = Arc::new(FakeSnapshotConsumer::new());
    let vault = Arc::new(FakeVault::new());
    if raw {
        let mut prefs = SessionPrefs::default();
        let view = prefs
            .views
            .entry("rawonly".into())
            .or_default()
            .tabs
            .entry("dataset".into())
            .or_default();
        view.node = Some("raw".into());
        view.filters = vec![Filter {
            column: "row_id".into(),
            op: Op::Eq("R1".into()),
        }];
        session::save(home, &prefs).unwrap();
    }
    (
        ResearchSeams {
            research_home: Some(home.to_owned()),
            snapshot_consumer: wired.then(|| consumer.clone() as _),
            vault: wired.then(|| vault.clone() as _),
            ..Default::default()
        },
        consumer,
        vault,
    )
}
fn scroll(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle, amount: f32) {
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "research-page",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(amount))),
            cx,
        );
    })
    .unwrap();
    frame(cx, window);
}
fn result(lines: &[(String, String)], id: &str) -> (String, String) {
    let text = &lines
        .iter()
        .find(|(p, _)| p.split('/').any(|s| s == id))
        .unwrap()
        .1;
    let parts: Vec<_> = text.split(" · ").collect();
    assert_eq!(parts.len(), 3, "{text}");
    assert_eq!(parts[2].len(), 12);
    assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    (parts[0].into(), parts[2].into())
}
fn run(
    cx: &mut TestAppContext,
    fixture: &str,
    lang: Lang,
    mode: ThemeMode,
    size: (f32, f32),
    wired: bool,
) {
    let copy = fixture_copy(fixture);
    let home = temp_dir();
    let dest = temp_dir();
    let before = tree_hash(&copy);
    let (seams, consumer, vault) = setup(&home, fixture == "rawonly", wired);
    let root = if fixture == "rram" {
        copy.join("project")
    } else {
        copy.clone()
    };
    let research_id = locate(
        &root,
        &LocateOptions {
            store: (fixture == "rram").then(|| copy.join("store")),
            orch_home: None,
        },
    )
    .unwrap()
    .manifest
    .research_id;
    let (window, _) = open_research(cx, lang, mode, size, &root, seams);
    click(cx, window, "research-tab-exports");
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-exports-dest").unwrap().visible());
        assert!(window
            .try_find("research-exports-snapshot")
            .unwrap()
            .visible());
    })
    .unwrap();
    wait_for(
        cx,
        window,
        "research-exports-repro",
        if lang == Lang::En {
            "Repro:"
        } else {
            "重現性"
        },
    );
    click(cx, window, "research-exports-dest");
    type_text(cx, window, dest.to_str().unwrap());
    click(cx, window, "research-exports-csv");
    let lines = wait_for(cx, window, "research-exports-result-0", " · ");
    let csv = result(&lines, "research-exports-result-0").0;
    assert!(Path::new(&csv).is_file());
    if fixture == "rawonly" {
        assert_eq!(Path::new(&csv).file_name().unwrap(), "dataset-raw.csv");
        assert_eq!(std::fs::read_to_string(&csv).unwrap().lines().count(), 2);
        assert!(shows(&lines, "research-exports-result-0", " · 1 · "));
    }
    click(cx, window, "research-exports-bundle");
    let lines = wait_for(cx, window, "research-exports-result-1", " · ");
    let bundle = result(&lines, "research-exports-result-1").0;
    assert!(Path::new(&bundle).is_file());
    if fixture == "rawonly" {
        assert!(shows(&lines, "research-exports-result-1", " · 1 · "));
        let contents = std::fs::read_to_string(&bundle).unwrap();
        assert!(contents.contains("\"evidence_id\":\"RE1\""));
        assert!(!contents.contains("\"evidence_id\":\"RE2\""));
        let unfiltered = std::fs::read_to_string(copy.join("data/raw_measurements.csv")).unwrap();
        assert_eq!(unfiltered.lines().count() - 1, 2);
        assert!(unfiltered.contains("RE1") && unfiltered.contains("RE2"));
    }
    click(cx, window, "research-exports-snapshot");
    scroll(cx, window, -250.);
    let lines = wait_for(cx, window, "research-exports-result-2", " · ");
    let first = result(&lines, "research-exports-result-2").0;
    assert!(Path::new(&first).join("snapshot.json").is_file());
    let expected = if wired {
        if lang == Lang::En {
            "Delivered"
        } else {
            "已交付"
        }
    } else if lang == Lang::En {
        "Paper writing is not connected yet, so the snapshot stays here for you to share"
    } else {
        "論文寫作尚未連接"
    };
    let handoff = wait_for(cx, window, "research-exports-handoff", expected);
    if !wired {
        assert!(shows(
            &handoff,
            "research-exports-handoff",
            if lang == Lang::En {
                "Vault is not connected yet"
            } else {
                "知識庫尚未連接"
            }
        ));
    }
    if wired {
        let calls = consumer.calls();
        let links = vault.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(links.len(), 1);
        assert_eq!(calls[0].research_id, research_id);
        assert_eq!(
            calls[0].snapshot_id,
            Path::new(&first)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replacen("snapshot-", "snapshot:", 1)
        );
        assert_eq!(calls[0].research_id, links[0].research_id);
        assert_eq!(Some(&calls[0].snapshot_id), links[0].snapshot_id.as_ref());
        assert_eq!(calls[0].path, Path::new(&first));
    }
    scroll(cx, window, 250.);
    click(cx, window, "research-exports-snapshot");
    scroll(cx, window, -250.);
    let lines = wait_for(cx, window, "research-exports-result-3", " · ");
    assert_eq!(first, result(&lines, "research-exports-result-3").0);
    assert_eq!(tree_hash(&copy), before);
    if fixture == "rawonly" && wired {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .append(true)
            .open(copy.join("data/raw_measurements.csv"))
            .unwrap()
            .write_all(b"x")
            .unwrap();
        assert_ne!(tree_hash(&copy), before);
    }
}
#[gpui_kit::test]
fn exports_on_both_fixtures_and_layouts(cx: &mut TestAppContext) {
    run(cx, "rram", Lang::En, ThemeMode::Dark, (1100., 700.), true);
    run(
        cx,
        "rram",
        Lang::ZhTw,
        ThemeMode::Light,
        (1920., 1200.),
        false,
    );
    run(
        cx,
        "rawonly",
        Lang::En,
        ThemeMode::Light,
        (1920., 1200.),
        true,
    );
    run(
        cx,
        "rawonly",
        Lang::ZhTw,
        ThemeMode::Dark,
        (1100., 700.),
        false,
    );
}
#[gpui_kit::test]
fn source_destination_is_rejected(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    let (seams, _, _) = setup(&home, false, false);
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy.join("project"),
        seams,
    );
    click(cx, window, "research-tab-exports");
    click(cx, window, "research-exports-csv");
    wait_for(cx, window, "research-exports-error", "Enter a destination");
    assert_eq!(tree_hash(&copy), before);
    click(cx, window, "research-exports-dest");
    let forbidden = copy.join("project/forbidden");
    type_text(cx, window, forbidden.to_str().unwrap());
    click(cx, window, "research-exports-csv");
    wait_for(cx, window, "research-exports-error", "dest_in_source:");
    assert!(!forbidden.exists());
    assert_eq!(tree_hash(&copy), before);
}
#[test]
fn tree_hash_detects_source_modification() {
    let copy = fixture_copy("rawonly");
    let before = tree_hash(&copy);
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(copy.join("data/raw_measurements.csv"))
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert_ne!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn no_research_has_no_export_actions(cx: &mut TestAppContext) {
    let (window, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        ResearchSeams::default(),
        true,
    );
    click(cx, window, "research-tab-exports");
    assert!(shows(
        &capture(cx, window),
        "research-exports-panel",
        "No research open"
    ));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("research-exports-csv").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_filter_shows_message_and_writes_nothing(cx: &mut TestAppContext) {
    let copy = fixture_copy("rawonly");
    let home = temp_dir();
    let dest = temp_dir();
    let mut prefs = SessionPrefs::default();
    let view = prefs
        .views
        .entry("rawonly".into())
        .or_default()
        .tabs
        .entry("dataset".into())
        .or_default();
    view.node = Some("raw".into());
    view.filters = vec![Filter {
        column: "row_id".into(),
        op: Op::Eq("missing".into()),
    }];
    session::save(&home, &prefs).unwrap();
    let seams = ResearchSeams {
        research_home: Some(home),
        ..Default::default()
    };
    let (window, _) = open_research(cx, Lang::En, ThemeMode::Dark, (1100., 700.), &copy, seams);
    click(cx, window, "research-tab-exports");
    click(cx, window, "research-exports-dest");
    type_text(cx, window, dest.to_str().unwrap());
    click(cx, window, "research-exports-bundle");
    wait_for(
        cx,
        window,
        "research-exports-error",
        "No evidence in the current dataset filter",
    );
    assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
}

#[gpui_kit::test]
fn snapshot_without_home_shows_message(cx: &mut TestAppContext) {
    let copy = fixture_copy("rawonly");
    let dest = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1100., 700.),
        &copy,
        ResearchSeams::default(),
    );
    click(cx, window, "research-tab-exports");
    click(cx, window, "research-exports-dest");
    type_text(cx, window, dest.to_str().unwrap());
    click(cx, window, "research-exports-snapshot");
    wait_for(
        cx,
        window,
        "research-exports-error",
        "Research home is not configured",
    );
    assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
}

#[gpui_kit::test]
fn snapshot_reports_failed_vault_handoff(cx: &mut TestAppContext) {
    let copy = fixture_copy("rawonly");
    let home = temp_dir();
    let dest = temp_dir();
    let (seams, consumer, vault) = setup(&home, false, true);
    vault.fail_next("429");
    let (window, _) = open_research(cx, Lang::En, ThemeMode::Dark, (1100., 700.), &copy, seams);
    click(cx, window, "research-tab-exports");
    click(cx, window, "research-exports-dest");
    type_text(cx, window, dest.to_str().unwrap());
    click(cx, window, "research-exports-snapshot");
    let lines = wait_for(
        cx,
        window,
        "research-exports-handoff",
        "Delivery failed: 429: scripted failure",
    );
    assert!(shows(
        &lines,
        "research-exports-handoff",
        "Delivered: fake_snapshot_consumer"
    ));
    assert_eq!(consumer.calls().len(), 1);
    assert_eq!(vault.calls().len(), 1);
}
