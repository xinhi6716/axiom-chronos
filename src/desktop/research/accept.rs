use super::*;
use std::prelude::v1::test;

#[test]
fn restored_view_text_localizes_and_omits_default_parts() {
    use crate::research_workspace::index::Op;

    let mut view = TabView::default();
    for lang in [Lang::En, Lang::ZhTw] {
        assert_eq!(view_state_text(&view, lang), None);
    }
    view.filters = vec![
        Filter {
            column: "device".into(),
            op: Op::Eq("RRAM".into()),
        },
        Filter {
            column: "kind".into(),
            op: Op::In(vec!["a".into(), "b".into()]),
        },
    ];
    assert_eq!(
        view_state_text(&view, Lang::En).as_deref(),
        Some("Restored view: 2 filter(s): device = RRAM, kind = a, b")
    );
    assert_eq!(
        view_state_text(&view, Lang::ZhTw).as_deref(),
        Some("已還原檢視：2 個篩選條件：device = RRAM, kind = a, b")
    );
    view.offset = 42;
    assert_eq!(
        view_state_text(&view, Lang::En).as_deref(),
        Some("Restored view: 2 filter(s): device = RRAM, kind = a, b ; starting at item 43")
    );
    assert_eq!(
        view_state_text(&view, Lang::ZhTw).as_deref(),
        Some("已還原檢視：2 個篩選條件：device = RRAM, kind = a, b；從第 43 項開始")
    );
    view.filters.clear();
    assert_eq!(
        view_state_text(&view, Lang::En).as_deref(),
        Some("Restored view: starting at item 43")
    );
    assert_eq!(
        view_state_text(&view, Lang::ZhTw).as_deref(),
        Some("已還原檢視：從第 43 項開始")
    );
}

#[test]
fn tab_keys_round_trip() {
    for tab in [
        Tab::Map,
        Tab::Sources,
        Tab::Evidence,
        Tab::Dataset,
        Tab::Exports,
    ] {
        assert_eq!(Tab::from_key(tab.key()), Some(tab));
    }
    assert_eq!(Tab::from_key("unknown"), None);
}

#[test]
fn public_tab_and_load_state_are_the_seam_types() {
    use crate::desktop::ux::seams::research;

    let tab: research::Tab = Tab::Sources;
    let state: research::LoadState = LoadState::Indexing { done: 1, total: 2 };
    assert_eq!(tab.key(), "sources");
    assert_eq!(state, research::LoadState::Indexing { done: 1, total: 2 });
}

#[test]
fn production_seams_use_research_paths() {
    let home = std::path::Path::new("test-home");
    let seams = ResearchSeams::production(home);
    assert_eq!(seams.research_home, Some(home.join("research")));
    assert_eq!(seams.orch_home, Some(home.join("orchestrate")));
    assert_eq!(seams.goals_home, Some(home.to_path_buf()));
    assert_eq!(seams.page_cache_bytes, 256 * 1024 * 1024);
    assert!(seams.renderer.is_some());
}

#[test]
fn palette_only_offers_research_when_configured() {
    use crate::desktop::{
        demo,
        i18n::Lang,
        palette::{self, Command, Target},
        ux::prefs::UxPrefs,
    };
    let mut state = demo::state();
    state.ux.research = ResearchSeams::default();
    let find = |s: &crate::desktop::model::AppState, lang| {
        palette::index(s, lang, true, &UxPrefs::default())
            .into_iter()
            .find(|item| item.target == Target::Command(Command::Research))
    };
    assert!(find(&state, Lang::En).is_none());
    state.ux.research.research_home = Some("research".into());
    assert_eq!(
        find(&state, Lang::En).unwrap().title.as_ref(),
        "Research workbench"
    );
    assert_eq!(
        find(&state, Lang::ZhTw).unwrap().title.as_ref(),
        "研究查核台"
    );
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "axiom-research-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn background_index_opens_real_fixture_and_reports_bad_manifest() {
    let tmp = Temp::new();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram"),
        &tmp.0,
    );
    let path = tmp.0.join("project/data/measurements.csv");
    let input = std::fs::read_to_string(&path).unwrap();
    let mut lines = input.lines();
    let header = lines.next().unwrap();
    let row = lines.next().unwrap();
    let mut data = format!("{header}\n");
    for i in 0..421 {
        data.push_str(&format!("M{i},{}\n", row.split_once(',').unwrap().1));
    }
    std::fs::write(path, data).unwrap();
    let r = ResearchRef {
        root: tmp.0.join("project"),
        store: Some(tmp.0.join("store")),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        index_research(r, None, 0, SessionPrefs::default(), tx);
    });
    let events: Vec<_> = rx.iter().collect();
    let first_progress = events
        .iter()
        .position(|e| matches!(e, WorkEvent::Progress(..)))
        .unwrap();
    assert!(
        first_progress > 1,
        "a dataset preview must arrive between Located and indexing progress"
    );
    let rows = match &events[1] {
        WorkEvent::Preview(rows, false) => rows,
        _ => panic!("expected preview after Located"),
    };
    assert_eq!(rows.len(), 200);
    let index = events
        .iter()
        .find_map(|e| match e {
            WorkEvent::Ready(index, _) => Some(index),
            _ => None,
        })
        .unwrap();
    let page = dataset::load_page(
        index,
        crate::research_workspace::dataset::DataLayer::Extracted,
        &[],
        0,
    )
    .unwrap()
    .0;
    assert_eq!(page.rows.len(), 200);
    assert_eq!(rows[0].fields["variable"], page.rows[0].variable);
    assert!(events.iter().any(|e| matches!(e, WorkEvent::Progress(..))));
    assert!(events
        .iter()
        .any(|e| matches!(e, WorkEvent::Ready(_, map) if stages::blocker(map).is_some())));
    std::fs::write(tmp.0.join("project/research-manifest.json"), "{").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    index_research(
        ResearchRef {
            root: tmp.0.join("project"),
            store: None,
        },
        None,
        0,
        SessionPrefs::default(),
        tx,
    );
    assert!(
        matches!(rx.recv().unwrap(), WorkEvent::Failed(Failure { kind, .. }) if kind == "invalid_manifest")
    );
}

#[test]
fn session_queue_restores_tab_filters_and_offset_after_reopen() {
    use crate::research_workspace::index::{Filter, Op};
    let temp = Temp::new();
    let queue = session_queue(&temp.0);
    let filter = Filter {
        column: "device".into(),
        op: Op::Eq("RRAM".into()),
    };
    let mut prefs = SessionPrefs::default();
    prefs.views.entry("study".into()).or_default().tab = Tab::Dataset.key().into();
    update_view(&mut prefs, "study", Tab::Dataset, |v| {
        v.filters = vec![filter.clone()];
        v.offset = 42;
    });
    queue.send(SessionCommand::Save(prefs)).unwrap();
    let reopened = session_queue(&temp.0);
    let (tx, rx) = std::sync::mpsc::channel();
    reopened.send(SessionCommand::Load(tx)).unwrap();
    let saved = rx.recv().unwrap();
    assert_eq!(restore_tab(&saved, "study"), Tab::Dataset);
    let view = &saved.views["study"].tabs["dataset"];
    assert_eq!(view.filters, vec![filter]);
    assert_eq!(view.offset, 42);
}

#[test]
fn primary_action_tracks_open_failure_blocker_and_ready() {
    assert_eq!(
        primary_action(&LoadState::NoResearch, false, false),
        PrimaryAction::Open
    );
    assert_eq!(
        primary_action(&LoadState::Opening, true, false),
        PrimaryAction::Open
    );
    assert_eq!(
        primary_action(&LoadState::Indexing { done: 1, total: 2 }, true, false),
        PrimaryAction::Open
    );
    assert_eq!(
        primary_action(&LoadState::Ready, false, false),
        PrimaryAction::Open
    );
    assert_eq!(
        primary_action(
            &LoadState::Failed {
                kind: "bad".into(),
                message: "broken".into()
            },
            true,
            true
        ),
        PrimaryAction::Retry
    );
    assert_eq!(
        primary_action(&LoadState::Ready, true, true),
        PrimaryAction::Blocker
    );
    assert_eq!(
        primary_action(&LoadState::Ready, true, false),
        PrimaryAction::Freeze
    );
}

#[test]
fn open_form_rejects_blank_root_and_uses_optional_store() {
    assert!(form_ref("  ", "store").is_none());
    let reference = form_ref(" project ", "  store  ").unwrap();
    assert_eq!(reference.root, std::path::PathBuf::from("project"));
    assert_eq!(reference.store, Some(std::path::PathBuf::from("store")));
    assert_eq!(form_ref("project", "  ").unwrap().store, None);
}

#[test]
fn load_line_reports_empty_and_index_progress_in_both_languages() {
    assert_eq!(
        load_text(&LoadState::NoResearch, Lang::En),
        "No research open"
    );
    assert_eq!(
        load_text(&LoadState::NoResearch, Lang::ZhTw),
        "尚未開啟研究"
    );
    assert_eq!(
        load_text(&LoadState::Indexing { done: 2, total: 5 }, Lang::En),
        "Indexing 2/5"
    );
    assert_eq!(
        load_text(
            &LoadState::Failed {
                kind: "index".into(),
                message: "indexer stopped".into()
            },
            Lang::ZhTw
        ),
        "失敗: index — 索引器已停止"
    );
}
