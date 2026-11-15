//! Exercises graph references through the real live worker and desktop state.
use axiom::desktop::{demo, model::smoke_read};
use axiom::graph::{
    desktop::memory_refs, Batch, EdgeSpec, GraphStore, NodeRef, NodeSpec, SourceSpec,
};
use axiom::memory::Memory;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[gpui_kit::test]
fn phase_six_note_is_visible_in_both_languages(cx: &mut gpui_kit::TestAppContext) {
    use axiom::desktop::{i18n, model::Screen, theme, Shell, Store};
    use gpui_kit::component::ThemeMode;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, WindowOptions};

    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let mut state = demo::state();
    state.view = Screen::Memory;
    let state = cx.new(|_| state);
    let (window, _) = cx.update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| Shell::new(state, false, window, cx))
    })).unwrap();
    for (lang, expected) in [
        (i18n::Lang::En, "Phase 6 memory and Obsidian features are unavailable"),
        (i18n::Lang::ZhTw, "第 6 階段的記憶與 Obsidian 功能尚未提供"),
    ] {
        cx.update(|cx| i18n::set(lang, cx));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.refresh();
            window.render_frame(cx);
            assert!(gpui_kit::base::test_support::texts(window).iter().any(|text| text.visible() && text.text() == expected));
        }).unwrap();
    }
}

struct TempHome(PathBuf);
impl TempHome {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("axiom-memory-graph-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn live_memory_and_runs_carry_graph_references() {
    let temp = TempHome::new();
    let home = temp.0.join("home");
    let repo = temp.0.join("repo");
    let wish = home.join("orchestrate/proj");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&wish).unwrap();
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(
        home.join("goals/s1.json"),
        format!(
            r#"{{"wish":"test wish","status":"done","project":"proj","cwd":{:?}}}"#,
            repo.to_string_lossy()
        ),
    )
    .unwrap();
    Memory::open(&home).unwrap();
    let graph = GraphStore::open(&home).unwrap();
    graph.with_tx(|tx| {
        tx.execute("INSERT INTO entries (id,project,unit,title,summary,body,symbols,wish,files,project_dir,created,confirmed) VALUES (1,'proj','u','entry','summary','body','','wish','[]',?1,1,1)", [repo.to_string_lossy().as_ref()]).unwrap();
        tx.execute("INSERT INTO entries (id,project,unit,title,summary,body,symbols,wish,files,project_dir,created,confirmed) VALUES (2,'proj','u','missing','summary','body','','wish','[]',?1,1,1)", [repo.to_string_lossy().as_ref()]).unwrap();
        Ok(())
    }).unwrap();
    let scope = graph.node_by_entry(1).unwrap().unwrap().scope;
    graph
        .with_tx(|tx| {
            tx.execute("DELETE FROM g_nodes WHERE entry_id=2", [])
                .unwrap();
            Ok(())
        })
        .unwrap();
    let reference = |key: &str| NodeRef {
        scope,
        key: key.into(),
    };
    let node = |key: &str, kind: &str, label: &str| NodeSpec {
        key: key.into(),
        kind: kind.into(),
        label: label.into(),
        summary: String::new(),
        attrs: "{}".parse().unwrap(),
        entry_id: None,
        state: "active".into(),
    };
    graph
        .write(&Batch {
            scope,
            source: SourceSpec {
                kind: "test".into(),
                locator: "test".into(),
                sha256: "hash".into(),
                version: "1".into(),
                state: "active".into(),
            },
            nodes: vec![
                node("file:src/a.rs", "file", "a.rs"),
                node("module:proj/app", "module", "app"),
            ],
            edges: vec![EdgeSpec {
                src: reference("entry:1"),
                dst: reference("file:src/a.rs"),
                kind: "about".into(),
                locator: String::new(),
                weight: 100,
                state: "active".into(),
                attrs: "{}".parse().unwrap(),
            }],
            replace_source: false,
        })
        .unwrap();
    let revision = graph.revision().unwrap();
    let mut state = demo::state();
    state.runs.clear();
    state.current = None;
    state.memory.clear();
    smoke_read(&home, &mut state).unwrap();
    let entry = state.memory.iter().find(|e| e.id == 1).unwrap();
    let found = entry.graph.as_ref().unwrap();
    assert_eq!((&*found.status, found.revision), ("active", revision));
    assert_eq!(found.neighbours.len(), 1);
    let neighbour = &found.neighbours[0];
    assert_eq!(
        (&*neighbour.key, &*neighbour.edge, neighbour.outgoing),
        ("file:src/a.rs", "about", true)
    );
    let missing = state
        .memory
        .iter()
        .find(|e| e.id == 2)
        .unwrap()
        .graph
        .as_ref();
    assert!(missing.is_none_or(|g| g.status == "missing" && g.neighbours.is_empty()));
    assert_eq!(state.mesh_graph["module:proj/app"].revision, revision);

    let memory = Memory::open(&home).unwrap();
    let prior = memory.prior(1).unwrap();
    memory.correct(1, "corrected body").unwrap();
    smoke_read(&home, &mut state).unwrap();
    let corrected = state.memory.iter().find(|e| e.id == 1).unwrap();
    assert!(corrected.corrected);
    assert_eq!(corrected.body.as_ref(), "corrected body");
    let corrected_revision = corrected.graph.as_ref().unwrap().revision;
    assert!(corrected_revision > revision);

    memory.restore(1, &prior).unwrap();
    smoke_read(&home, &mut state).unwrap();
    let restored = state.memory.iter().find(|e| e.id == 1).unwrap();
    assert!(!restored.corrected);
    assert_eq!(restored.body.as_ref(), "body");
    let restored_revision = restored.graph.as_ref().unwrap().revision;
    assert!(restored_revision > corrected_revision);

    memory.delete(1).unwrap();
    smoke_read(&home, &mut state).unwrap();
    assert!(state.memory.iter().all(|e| e.id != 1));
    assert!(
        state
            .memory
            .iter()
            .find(|e| e.id == 2)
            .unwrap()
            .graph
            .as_ref()
            .unwrap()
            .revision
            > restored_revision
    );
    let deleted = memory_refs(&home, &[1]);
    assert!(deleted.get(&1).is_none_or(|g| g.status == "tombstone"));
}
