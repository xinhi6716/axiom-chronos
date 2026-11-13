mod research_support;
use axiom::{
    desktop::{
        i18n::Lang,
        research::{LoadState, ResearchSeams, Workbench},
    },
    research_workspace::{manifest::Role, session::ResearchRef, stages::StageState},
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
    let home = temp_dir();
    let wb = cx.new(|cx| Workbench::new(seams(&home), None, Arc::new(SystemClock), cx));
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
        assert!(Instant::now() < deadline, "workbench timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[gpui_kit::test]
fn map_displays_truth_across_languages_themes_and_sizes(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for size in [(1100., 700.), (1920., 1200.)] {
                let copy = fixture_copy("rram");
                let before = tree_hash(&copy);
                let wb = opened_workbench(cx, &copy.join("project"), &copy.join("store"));
                let (stage_index, progress, has_repro) = wb.read_with(cx, |wb, _| {
                    let m = wb.map.as_ref().unwrap();
                    let (i, s) = m
                        .stages
                        .iter()
                        .enumerate()
                        .find(|(_, s)| {
                            matches!(s.state, StageState::Paused { .. }) && s.progress.is_some()
                        })
                        .expect("paused stage with progress");
                    let p = s.progress.as_ref().unwrap();
                    assert!(m.engineering.is_some());
                    (i, format!("{}/{}", p.done, p.total), m.repro.is_some())
                });
                let home = temp_dir();
                let (window, _) =
                    open_research(cx, lang, mode, size, &copy.join("project"), seams(&home));
                let lines = capture(cx, window);
                let (paused, rate, disclaimer) = match lang {
                    Lang::En => ("Paused", "rate limited", "not a research result"),
                    Lang::ZhTw => ("已暫停", "rate limited", "不是研究結果"),
                };
                let stage_id = format!("research-map-stage-{stage_index}");
                assert!(shows(&lines, "research-map-status", paused), "{lines:?}");
                assert!(
                    shows(&lines, &stage_id, paused)
                        && shows(&lines, &stage_id, rate)
                        && shows(&lines, &stage_id, &progress),
                    "{lines:?}"
                );
                assert!(
                    shows(&lines, "research-map-engineering", disclaimer),
                    "{lines:?}"
                );
                assert!(
                    has_repro && visible(&lines, "research-map-repro"),
                    "{lines:?}"
                );
                assert!(visible(&lines, "research-map-current"), "{lines:?}");
                assert_eq!(tree_hash(&copy), before);
            }
        }
    }
}
#[gpui_kit::test]
fn output_link_opens_role_tab_without_changing_fixture(cx: &mut TestAppContext) {
    let copy = fixture_copy("rram");
    let before = tree_hash(&copy);
    let wb = opened_workbench(cx, &copy.join("project"), &copy.join("store"));
    let (i, target) = wb.read_with(cx, |wb, _| {
        let l = wb.located.as_ref().unwrap();
        let map = wb.map.as_ref().unwrap();
        let (i, role) = map
            .stages
            .iter()
            .enumerate()
            .find_map(|(i, s)| {
                let output = l
                    .manifest
                    .stages
                    .iter()
                    .find(|d| d.id == s.id)?
                    .outputs
                    .first()?;
                let role = l.artifact(output).map(|a| a.role.clone());
                matches!(role, Some(Role::Measurements | Role::RawMeasurements))
                    .then_some((i, role))
            })
            .unwrap();
        let target = match role {
            Some(Role::Measurements | Role::RawMeasurements) => "research-dataset-panel",
            Some(Role::Evidence) => "research-evidence-panel",
            _ => "research-sources-panel",
        };
        (i, target)
    });
    let home = temp_dir();
    let (window, _) = open_research(
        cx,
        Lang::En,
        ThemeMode::Dark,
        (1920., 1200.),
        &copy.join("project"),
        seams(&home),
    );
    assert!(visible(
        &capture(cx, window),
        &format!("research-map-open-{i}")
    ));
    click(cx, window, &format!("research-map-open-{i}"));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find(target).is_some(), "missing {target}");
        assert!(window.try_find("research-map-panel").is_none());
    })
    .unwrap();
    assert_eq!(tree_hash(&copy), before);
}
