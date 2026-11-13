use super::{
    grid, persist,
    split::{arrange, Arrangement, Pane, Split},
    window, Breakpoint,
};
use crate::desktop::{
    settings::ColumnWidths,
    theme::{space, Density, Step},
};
use gpui_kit::{point, px, size, Bounds};

fn split() -> Split {
    Split {
        id: "test",
        panes: vec![
            Pane {
                min: 100.,
                max: 400.,
                default: 300.,
                collapsible: false,
            },
            Pane {
                min: 200.,
                max: 500.,
                default: 400.,
                collapsible: false,
            },
            Pane {
                min: 100.,
                max: 300.,
                default: 200.,
                collapsible: true,
            },
        ],
    }
}
#[test]
fn breakpoint_edges_and_grid_tokens() {
    use Breakpoint::*;
    for (width, expected) in [
        (0., Narrow),
        (959.99, Narrow),
        (960., Compact),
        (1279.99, Compact),
        (1280., Regular),
        (1919.99, Regular),
        (1920., Wide),
        (2559.99, Wide),
        (2560., Ultra),
        (f32::NAN, Narrow),
        (f32::INFINITY, Narrow),
        (f32::NEG_INFINITY, Narrow),
    ] {
        assert_eq!(Breakpoint::of(width), expected);
    }
    for (bp, cols, gutter, margin) in [
        (Narrow, 4, Step::Md, Step::Lg),
        (Compact, 8, Step::Md, Step::Lg),
        (Regular, 12, Step::Lg, Step::Xl),
        (Wide, 12, Step::Lg, Step::Xl),
        (Ultra, 16, Step::Lg, Step::Xl),
    ] {
        assert_eq!(grid::columns(bp), cols);
        for d in [Density::Dense, Density::Comfortable] {
            assert_eq!(grid::gutter(bp, d), space(d, gutter));
            assert_eq!(grid::margin(bp, d), space(d, margin));
        }
    }
}
#[test]
fn split_defaults_shrink_and_collapse() {
    let s = split();
    for saved in [
        None,
        Some(&[1., 2.][..]),
        Some(&[f32::NAN, 300., 200.][..]),
        Some(&[99., 300., 200.][..]),
        Some(&[401., 300., 200.][..]),
    ] {
        assert_eq!(s.sizes(1200., saved), vec![300., 400., 200.]);
    }
    let reduced = s.sizes(650., None);
    for (a, b) in reduced.iter().zip([200., 300., 150.]) {
        assert!((a - b).abs() < 0.001);
    }
    let collapsed = s.sizes(350., None);
    assert_eq!(collapsed, vec![100., 200., 0.]);
    assert_eq!(s.resize(&collapsed, 1, f32::NAN, 350.), collapsed);
    assert_eq!(s.resize(&collapsed, 1, 10., 350.), collapsed);
    let impossible = s.sizes(150., None);
    assert!(impossible.iter().sum::<f32>() <= 150. && impossible[2] == 0.);
    assert_eq!(s.resize(&impossible, 0, 20., 150.), impossible);
    assert_eq!(s.resize(&impossible, 1, -20., 150.), impossible);
    assert_eq!(s.sizes(f32::NAN, None), vec![0., 0., 0.]);
}
#[test]
fn placement_keeps_visible_overhangs_and_windows_on_other_displays() {
    let rect = |x, y, w, h| Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w), px(h)),
    };
    let displays = [rect(0., 0., 1920., 1080.), rect(1920., 0., 1920., 1080.)];
    let default = size(px(1440.), px(900.));
    let min = size(px(800.), px(600.));
    let center = rect(240., 90., 1440., 900.);
    for saved in [
        rect(-8., -8., 1936., 1096.),
        rect(2100., 100., 1000., 700.),
        rect(1400., 100., 1000., 700.),
        rect(1420., 100., 1000., 700.), // exactly half on each display
        rect(1421., 100., 1000., 700.), // title on first, majority on second
    ] {
        assert_eq!(window::recover(Some(saved), &displays, default, min), saved);
    }
    for saved in [
        rect(-501., 100., 1000., 700.),  // less than half visible
        rect(-100., -200., 1000., 700.), // title area is off-screen
    ] {
        assert_eq!(
            window::recover(Some(saved), &displays, default, min),
            center
        );
    }
    // Visibility split among displays does not count as half on any one display.
    let narrow_displays = [
        rect(0., 0., 400., 1080.),
        rect(400., 0., 400., 1080.),
        rect(800., 0., 400., 1080.),
    ];
    assert_eq!(
        window::recover(
            Some(rect(0., 0., 1200., 700.)),
            &narrow_displays,
            default,
            min
        ),
        rect(-520., 90., 1440., 900.)
    );
}
#[test]
fn resize_clamps_both_panes_and_preserves_sum() {
    let s = split();
    assert_eq!(
        s.resize(&[300., 400., 200.], 0, 1000., 1000.),
        vec![400., 300., 200.]
    );
    assert_eq!(
        s.resize(&[300., 400., 200.], 0, -1000., 1000.),
        vec![200., 500., 200.]
    );
    assert_eq!(
        s.resize(&[300., 400., 200.], 1, 1000., 1000.),
        vec![300., 500., 100.]
    );
    for delta in [f32::NAN, f32::INFINITY] {
        assert_eq!(
            s.resize(&[300., 400., 200.], 0, delta, 1000.),
            vec![300., 400., 200.]
        );
    }
    assert_eq!(
        s.resize(&[300., 400., 200.], 8, 10., 1000.),
        vec![300., 400., 200.]
    );
}
#[test]
fn arrangement_checks_compact_available_width() {
    let mut s = split();
    assert_eq!(arrange(Breakpoint::Narrow, &s), Arrangement::Tabs);
    s.panes.pop();
    assert_eq!(arrange(Breakpoint::Narrow, &s), Arrangement::Stacked);
    s.panes[0].min = 500.;
    s.panes[1].min = 429.;
    assert_eq!(arrange(Breakpoint::Compact, &s), Arrangement::Stacked);
    s.panes[1].min = 428.;
    assert_eq!(arrange(Breakpoint::Compact, &s), Arrangement::Side);
    assert_eq!(arrange(Breakpoint::Regular, &s), Arrangement::Side);
}
#[test]
fn placement_recovery() {
    let display = Bounds {
        origin: point(px(100.), px(50.)),
        size: size(px(1920.), px(1080.)),
    };
    let default = size(px(1440.), px(900.));
    let min = size(px(800.), px(600.));
    let center = Bounds {
        origin: point(px(340.), px(140.)),
        size: default,
    };
    let good = Bounds {
        origin: point(px(200.), px(100.)),
        size: size(px(1000.), px(700.)),
    };
    assert_eq!(window::recover(Some(good), &[display], default, min), good);
    for bad in [
        Bounds {
            origin: point(px(-3000.), px(0.)),
            size: default,
        },
        Bounds {
            origin: point(px(f32::NAN), px(0.)),
            size: default,
        },
        Bounds {
            origin: point(px(200.), px(100.)),
            size: size(px(799.), px(700.)),
        },
    ] {
        assert_eq!(window::recover(Some(bad), &[display], default, min), center);
    }
    assert_eq!(window::recover(None, &[display], default, min), center);
}
#[test]
fn persistence_validation_and_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop/layout.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for json in [
        "{",
        r#"{"version":2,"panels":{"run.columns":[1,2]}}"#,
        r#"{"version":1,"panels":{"BAD":[1,2]}}"#,
        r#"{"version":1,"panels":{"run.columns":[-1]}}"#,
        r#"{"version":1,"panels":{"run.columns":[]}}"#,
    ] {
        std::fs::write(&path, json).unwrap();
        assert_eq!(persist::load(dir.path()), persist::PanelSizes::default());
    }
    for id in ["", "Upper", "a_b", "a/b", &"x".repeat(49)] {
        assert!(!persist::valid_id(id));
    }
    assert!(persist::valid_id(&"x".repeat(48)));
    assert!(persist::valid_id("run.columns-2"));
    let mut data = persist::PanelSizes::default();
    persist::migrate_run_columns(
        &mut data,
        ColumnWidths {
            checklist: 220.,
            details: 330.,
        },
    );
    persist::migrate_run_columns(
        &mut data,
        ColumnWidths {
            checklist: 1.,
            details: 2.,
        },
    );
    assert_eq!(data.panels["run.columns"], vec![220., 330.]);
}
#[test]
fn state_init_loads_and_migrates_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop/layout.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"version":1,"panels":{"other":[10,20]}}"#).unwrap();
    let legacy = ColumnWidths {
        checklist: 220.,
        details: 330.,
    };
    let state = persist::init_state(dir.path(), Some(legacy));
    assert_eq!(persist::sizes_of(&state, "other"), Some(vec![10., 20.]));
    assert_eq!(
        persist::sizes_of(&state, "run.columns"),
        Some(vec![220., 330.])
    );

    std::fs::write(&path, r#"{"version":1,"panels":{"run.columns":[1,2]}}"#).unwrap();
    let state = persist::init_state(dir.path(), Some(legacy));
    assert_eq!(persist::sizes_of(&state, "run.columns"), Some(vec![1., 2.]));
}

#[test]
fn state_remember_validates_and_flushes() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = persist::init_state(dir.path(), None);
    for (id, sizes) in [
        ("x".repeat(49), vec![1.]),
        ("Upper".into(), vec![1.]),
        ("run.columns".into(), vec![f32::NAN]),
        ("run.columns".into(), vec![-1.]),
        ("run.columns".into(), vec![]),
    ] {
        assert!(!persist::remember_state(&mut state, &id, sizes));
    }
    persist::flush_state(&state);
    assert!(!dir.path().join("desktop/layout.json").exists());
    assert!(persist::remember_state(
        &mut state,
        "run.columns",
        vec![0., 330.]
    ));
    assert_eq!(
        persist::sizes_of(&state, "run.columns"),
        Some(vec![0., 330.])
    );
    persist::flush_state(&state);
    assert_eq!(
        persist::load(dir.path()).panels["run.columns"],
        vec![0., 330.]
    );
}

#[test]
fn state_remember_rewrites_corrupt_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("desktop/layout.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{").unwrap();
    let mut state = persist::init_state(dir.path(), None);
    assert!(persist::remember_state(
        &mut state,
        "run.columns",
        vec![1., 2.]
    ));
    persist::flush_state(&state);
    assert_eq!(
        persist::load(dir.path()).panels["run.columns"],
        vec![1., 2.]
    );
}
#[test]
fn background_flush_writes_and_replaces_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let worker = persist::worker(dir.path().to_path_buf());
    let mut data = persist::PanelSizes::default();
    data.panels.insert("run.columns".into(), vec![220., 330.]);
    worker.send(persist::Message::Write(data.clone())).unwrap();
    persist::flush_sender(&worker);
    assert_eq!(persist::load(dir.path()), data);
    data.panels.insert("run.columns".into(), vec![240., 350.]);
    worker.send(persist::Message::Write(data.clone())).unwrap();
    persist::flush_sender(&worker);
    assert_eq!(persist::load(dir.path()), data);
    assert_eq!(
        std::fs::read_dir(dir.path().join("desktop"))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn background_writes_are_debounced() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let worker = persist::worker(dir.path().to_path_buf());
    let mut data = persist::PanelSizes::default();
    data.panels.insert("run.columns".into(), vec![1., 2.]);
    worker.send(persist::Message::Write(data.clone())).unwrap();
    data.panels.insert("run.columns".into(), vec![3., 4.]);
    worker.send(persist::Message::Write(data.clone())).unwrap();
    let path = dir.path().join("desktop/layout.json");
    assert!(!path.exists());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "debounced write did not complete");
    assert_eq!(persist::load(dir.path()), data);
}
