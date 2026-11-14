mod research_support;
use axiom::{
    desktop::{
        i18n::Lang,
        research::{LoadState, ResearchRef, ResearchSeams, Workbench},
    },
    research_workspace::{
        annotations::{AnnotationStore, Verdict},
        funnel::{self, Node},
    },
    seams::SystemClock,
};
use gpui_kit::{component::ThemeMode, test::TestWindowExt as _, AppContext as _, TestAppContext};
use research_support::*;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn seams(home: &Path) -> ResearchSeams {
    ResearchSeams {
        research_home: Some(home.to_owned()),
        ..Default::default()
    }
}
fn opened_workbench(
    cx: &mut TestAppContext,
    project: &Path,
    store: &Path,
) -> gpui_kit::Entity<Workbench> {
    let wb = cx.new(|cx| Workbench::new(seams(&temp_dir()), None, Arc::new(SystemClock), cx));
    wb.update(cx, |wb, cx| {
        wb.open(
            ResearchRef {
                root: project.to_owned(),
                store: Some(store.to_owned()),
            },
            cx,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if wb.read_with(cx, |wb, _| match &wb.load {
            LoadState::Ready => true,
            LoadState::Failed { kind, message } => panic!("{kind}: {message}"),
            _ => false,
        }) {
            return wb;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn wait_for(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
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
            "waiting for {id} {text}: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[gpui_kit::test]
fn sources_funnel_is_visible(cx: &mut TestAppContext) {
    let research_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/research");
    let captures_before = tree_hash(&research_dir);
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let wb = opened_workbench(cx, &copy.join("project"), &copy.join("store"));
    let research_id = wb.read_with(cx, |wb, _| wb.located.as_ref().unwrap().id.clone());
    let located = axiom::research_workspace::manifest::locate(
        &copy.join("project"),
        &axiom::research_workspace::manifest::LocateOptions {
            store: Some(copy.join("store")),
            orch_home: None,
        },
    )
    .unwrap();
    let index = axiom::research_workspace::index::Index::build(
        Arc::new(located),
        axiom::research_workspace::read::Limits::default(),
        &|_, _| {},
    )
    .unwrap();
    let f = funnel::funnel(&index).unwrap();
    assert_eq!(f.total, 40);
    for lang in [Lang::En, Lang::ZhTw] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for size in [(1100., 700.), (1920., 1200.)] {
                let home = temp_dir();
                let (window, _) =
                    open_research(cx, lang, mode, size, &copy.join("project"), seams(&home));
                click(cx, window, "research-tab-sources");
                let lines = wait_for(
                    cx,
                    window,
                    "research-sources-node-total",
                    &f.total.to_string(),
                );
                let lines: Vec<_> = lines
                    .into_iter()
                    .filter(|(path, _)| path.contains("research-sources"))
                    .collect();
                assert!(!lines.is_empty());
                assert!(shows(
                    &lines,
                    "research-sources-node-total",
                    &f.total.to_string()
                ));
                assert!(shows(
                    &lines,
                    "research-sources-node-pending",
                    &f.pending.to_string()
                ));
                assert!(shows(
                    &lines,
                    "research-sources-node-read_failed",
                    &f.read_failed.to_string()
                ));
                assert!(shows(&lines, "research-sources-row-0", "c0b3ff2c5cff"));
                assert!(shows(&lines, "research-sources-row-1", "8341ce564805"));
                assert!(
                    shows(&lines, "research-sources-row-0", "10.1234/same")
                        && shows(&lines, "research-sources-row-1", "10.1234/same")
                );
            }
        }
    }
    let home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams(&home),
    );
    click(cx, window, "research-tab-sources");
    wait_for(cx, window, "research-sources-node-total", "40");
    click(cx, window, "research-sources-node-read_failed");
    let lines = wait_for(
        cx,
        window,
        "research-sources-page",
        &format!("/ {}", f.read_failed),
    );
    assert!(shows(
        &lines,
        "research-sources-row-0",
        "Failed: read failed"
    ));
    click(cx, window, "research-sources-node-unprocessed");
    let lines = wait_for(
        cx,
        window,
        "research-sources-page",
        &format!("/ {}", f.unprocessed),
    );
    assert!(shows(&lines, "research-sources-row-0", "Unprocessed"));
    click(cx, window, "research-sources-node-pending");
    let lines = wait_for(
        cx,
        window,
        "research-sources-page",
        &format!("/ {}", f.pending),
    );
    assert!(shows(&lines, "research-sources-row-0", "Pending"));
    click(cx, window, "research-sources-sample");
    let lines = wait_for(cx, window, "research-sources-sampled-0", "");
    let seed = lines
        .iter()
        .find(|(p, _)| p.ends_with("research-sources-seed"))
        .unwrap()
        .1
        .split(": ")
        .last()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let expected = funnel::sample(&index, &Node::Pending, 5, seed).unwrap();
    for (i, id) in expected.iter().enumerate() {
        assert!(shows(&lines, &format!("research-sources-sampled-{i}"), id));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        click(cx, window, "research-spot-confirm-0");
        let spots = AnnotationStore::open(&home, &research_id)
            .unwrap()
            .spot_checks(Some(&expected[0]));
        if let Some(s) = spots.first() {
            assert_eq!(s.verdict, Verdict::Confirm);
            assert_eq!(s.node.as_deref(), Some("pending"));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    click(cx, window, "research-spot-dispute-1");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let spots = AnnotationStore::open(&home, &research_id)
            .unwrap()
            .spot_checks(Some(&expected[1]));
        if spots
            .iter()
            .any(|s| matches!(s.verdict, Verdict::Dispute { .. }))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(tree_hash(&copy), before);
    let (reopened, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams(&home),
    );
    let lines = wait_for(cx, reopened, "research-sources-sampled-0", &expected[0]);
    assert!(shows(&lines, "research-sources-seed", &seed.to_string()));
    assert!(shows(
        &lines,
        "research-sources-page",
        &format!("/ {}", f.pending)
    ));

    let raw = fixture_copy("rawonly");
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &raw,
        seams(&temp_dir()),
    );
    click(cx, window, "research-tab-sources");
    let lines = wait_for(
        cx,
        window,
        "research-sources-error",
        "does not have literature",
    );
    assert!(!visible(&lines, "research-sources-sample"));
    let corrupt = fixture_copy("rram");
    std::fs::write(
        corrupt.join("project/data/literature_inventory.csv"),
        [0xff, 0xfe],
    )
    .unwrap();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &corrupt.join("project"),
        seams(&temp_dir()),
    );
    click(cx, window, "research-tab-sources");
    wait_retry(cx, window, "retry:research.sources.panel");

    let large = fixture_copy("rram");
    let inventory = large.join("project/data/literature_inventory.csv");
    let contents = std::fs::read_to_string(&inventory).unwrap();
    let template = contents
        .lines()
        .find(|line| line.starts_with("paper39.pdf,"))
        .unwrap()
        .to_owned();
    let mut expanded = contents;
    for i in 0..201 {
        expanded.push_str(
            &template
                .replacen("paper39.pdf,", &format!("extra{i}.pdf,"), 1)
                .replace(
                    "0b14da6a1681d820f067daff920ca9965cd8cf1a35b0e52c1a5dff5dc6f68dde",
                    &format!("{i:064x}"),
                ),
        );
        expanded.push('\n');
    }
    std::fs::write(inventory, expanded).unwrap();
    let large_home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &large.join("project"),
        seams(&large_home),
    );
    click(cx, window, "research-tab-sources");
    wait_for(cx, window, "research-sources-page", "1–200 / 241");
    click(cx, window, "research-sources-next");
    wait_for(cx, window, "research-sources-page", "201–241 / 241");
    let (reopened, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &large.join("project"),
        seams(&large_home),
    );
    let lines = wait_for(cx, reopened, "research-sources-page", "201–241 / 241");
    assert!(visible(&lines, "research-sources-row-200"));
    click(cx, reopened, "research-sources-previous");
    wait_for(cx, reopened, "research-sources-page", "1–200 / 241");
    assert_eq!(
        tree_hash(&research_dir),
        captures_before,
        "research_sources must not write captures/research"
    );
}

#[gpui_kit::test]
fn split_drag_persists_and_restores(cx: &mut TestAppContext) {
    use axiom::desktop::layout::persist;
    use gpui_kit::px;
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    cx.update(|cx| persist::init(&home, cx));
    let configuration = seams(&home);
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        configuration.clone(),
    );
    click(cx, window, "research-tab-sources");
    wait_for(cx, window, "research-sources-node-total", "");
    let original = cx
        .update_window(window, |_, window, _| {
            window.find("research-sources-split-frame").bounds()
        })
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        let start = original.origin + gpui_kit::point(original.size.width / 2., px(10.));
        window.drag(start, start + gpui_kit::point(px(4.), px(0.)), cx);
    })
    .unwrap();
    frame(cx, window);
    let moved = cx
        .update_window(window, |_, window, _| {
            window.find("research-sources-split-frame").bounds()
        })
        .unwrap();
    assert!(moved.origin.x > original.origin.x);
    let saved = cx
        .update(|cx| {
            persist::flush(cx);
            persist::sizes("research.sources", cx)
        })
        .unwrap();
    assert_eq!(
        persist::load(&home).panels.get("research.sources"),
        Some(&saved)
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    let (again, _) = open_shell(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        configuration,
        true,
    );
    wait_load(cx, again);
    click(cx, again, "research-tab-sources");
    wait_for(cx, again, "research-sources-node-total", "");
    let restored = cx
        .update_window(again, |_, window, _| {
            window.find("research-sources-split-frame").bounds()
        })
        .unwrap();
    assert_eq!(restored.origin.x, moved.origin.x);
    assert_eq!(tree_hash(&copy), before);
}

#[gpui_kit::test]
fn feed_failure_is_local_to_tab(cx: &mut TestAppContext) {
    use axiom::desktop::data::faults::{self, Fault, FaultPlan};
    use std::collections::HashMap;
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let home = temp_dir();
    let configuration = seams(&home);
    cx.update(|cx| {
        faults::install(
            FaultPlan(HashMap::from([(
                "research.sources.panel".into(),
                Fault::Corrupt,
            )])),
            cx,
        )
    });
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        configuration,
    );
    click(cx, window, "research-tab-sources");
    wait_retry(cx, window, "retry:research.sources.panel");
    click(cx, window, "research-tab-map");
    wait_for(cx, window, "research-map-status", "");
    cx.update(|cx| faults::install(FaultPlan::default(), cx));
    assert_eq!(tree_hash(&copy), before);
}
