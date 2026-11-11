use super::*;
use std::prelude::v1::test;

// Root lib tests lack gpui-kit test-support: setter/restore, Store::edit scheduling,
// window, theme, language, plain-text and GPUI side effects are not exercised here.
// Their settings mutations and wrapper decisions are covered through pure helpers.

#[test]
fn system_motion_is_captured_before_a_stored_override_and_never_overwritten() {
    for system in [false, true] {
        let mut settings = Settings::parse(&serde_json::json!({"reduce_motion": !system}).to_string());
        let captured = capture_system_motion(None, system);
        let captured = capture_system_motion(captured, !system);
        assert_eq!(captured, Some(system));
        let prior = settings.set_reduce_motion(None);
        assert_eq!(prior, SettingPrior::ReduceMotion(Some(!system)));
        assert_eq!(resolve_none_motion(captured, !system), system);
        settings.restore(prior);
        assert_eq!(settings.reduce_motion, Some(!system));
    }
    assert!(!resolve_none_motion(None, true));
    assert!(!resolve_none_motion(None, false));
}

#[test]
fn view_strings_are_limited_by_characters_when_parsed() {
    let ascii = "a".repeat(200);
    let cjk = "界".repeat(150);
    let long = "x".repeat(201);
    let source = serde_json::json!({ "views": { "run": {
        "filter": ascii, "sort": cjk, "selected": long
    }}});
    let view = Settings::parse(&source.to_string()).view_memory("run");
    assert_eq!(view.filter, Some(ascii));
    assert_eq!(view.sort, Some(cjk));
    assert_eq!(view.selected, None);
}

#[test]
fn column_and_view_memory_helpers_match_wrapper_behavior() {
    let mut s = Settings::default();
    s.with_columns(ColumnWidths { checklist: f32::NAN, details: -1.0 });
    assert_eq!(s.columns, Some(ColumnWidths { checklist: 0.0, details: 0.0 }));
    s.with_columns(ColumnWidths { checklist: 123.0, details: 456.0 });
    assert_eq!(s.columns, Some(ColumnWidths { checklist: 123.0, details: 456.0 }));
    assert_eq!(s.view_memory("run"), ViewMemory::default());
    s.with_view("run", |view| {
        view.filter = Some("open".into());
        view.scroll = 23.0;
    });
    assert_eq!(s.view_memory("run"), ViewMemory {
        filter: Some("open".into()), scroll: 23.0, ..ViewMemory::default()
    });
    assert_eq!(Settings::parse(&s.to_json()).view_memory("run"), s.view_memory("run"));
}

#[test]
fn old_file_gets_all_new_defaults() {
    let old = r#"{"language":"system","theme":"system","plain_words":true,"reduce_motion":null,"window":null,"last_screen":"run","recent_projects":[],"first_day_done":false}"#;
    assert_eq!(Settings::parse(old), Settings::default());
}

#[test]
fn all_new_fields_round_trip() {
    let mut s = Settings::default();
    s.density = Density::Dense;
    s.mode = WindowMode::Focus(FocusPanel::Checklist);
    s.mini_on_top = true;
    s.columns = Some(ColumnWidths {
        checklist: 250.5,
        details: 300.0,
    });
    s.with_view("run-screen", |v| {
        v.filter = Some("中文".into());
        v.sort = Some("new".into());
        v.scroll = 15.5;
        v.selected = Some("item".into());
    });
    s.board = BoardChoice::Off;
    s.board_tokens = BoardTokens {
        input: u64::MAX,
        output: 50,
        unreported: 3,
    };
    assert_eq!(Settings::parse(&s.to_json()), s);
    for (mode, key) in [
        (WindowMode::Full, "full"),
        (WindowMode::Focus(FocusPanel::Graph), "focus-graph"),
        (WindowMode::Focus(FocusPanel::Checklist), "focus-checklist"),
        (WindowMode::Mini, "mini"),
    ] {
        s.mode = mode;
        assert!(s.to_json().contains(key));
        assert_eq!(Settings::parse(&s.to_json()).mode, mode);
    }
    for board in [BoardChoice::Luna, BoardChoice::Haiku, BoardChoice::Off] {
        s.board = board;
        assert_eq!(Settings::parse(&s.to_json()).board, board);
    }
}

#[test]
fn damaged_fields_are_isolated() {
    let long = "界".repeat(201);
    let source = serde_json::json!({
        "density":"invalid", "mode":4, "board":"gpt", "mini_on_top":true,
        "columns":{"checklist":"NaN", "details":230},
        "views": {"bad_key":{"filter":"lost"}, "ok":{"filter":long,"sort":"valid","scroll":-5,"selected":"kept"}},
        "board_tokens":{"input":"9","output":8,"unreported":-1}
    });
    let s = Settings::parse(&source.to_string());
    assert_eq!(s.density, Density::Comfortable);
    assert_eq!(s.mode, WindowMode::Full);
    assert_eq!(s.board, BoardChoice::Luna);
    assert!(s.mini_on_top);
    assert_eq!(
        s.columns,
        Some(ColumnWidths {
            checklist: 0.0,
            details: 230.0
        })
    );
    assert!(!s.views.contains_key("bad_key"));
    assert_eq!(
        s.views["ok"],
        ViewMemory {
            filter: None,
            sort: Some("valid".into()),
            scroll: 0.0,
            selected: Some("kept".into())
        }
    );
    assert_eq!(
        s.board_tokens,
        BoardTokens {
            input: 0,
            output: 8,
            unreported: 0
        }
    );
    for bad in ["\"oops\"", "\"NaN\"", "-1", "1e100"] {
        let source = format!(
            r#"{{"columns":{{"checklist":{bad},"details":42}},"views":{{"ok":{{"scroll":{bad},"sort":"ok"}}}}}}"#
        );
        let s = Settings::parse(&source);
        assert_eq!(
            s.columns,
            Some(ColumnWidths {
                checklist: 0.0,
                details: 42.0
            }),
            "{bad}"
        );
        assert_eq!(s.views["ok"].scroll, 0.0, "{bad}");
        assert_eq!(s.views["ok"].sort.as_deref(), Some("ok"));
    }
    let mut s = Settings::default();
    s.with_view("bad_key", |_| panic!("invalid key must be ignored"));
    s.with_view("valid", |v| {
        v.filter = Some("a".repeat(200));
        v.sort = Some("b".repeat(201));
        v.selected = Some("c".repeat(201));
        v.scroll = f32::INFINITY;
    });
    assert_eq!(s.views.len(), 1);
    assert_eq!(s.views["valid"].filter.as_ref().unwrap().len(), 200);
    assert_eq!(s.views["valid"].sort, None);
    assert_eq!(s.views["valid"].selected, None);
    assert_eq!(s.views["valid"].scroll, 0.0);
}

#[test]
fn mode_transitions_cover_every_start() {
    for mode in [
        WindowMode::Full,
        WindowMode::Focus(FocusPanel::Graph),
        WindowMode::Focus(FocusPanel::Checklist),
        WindowMode::Mini,
    ] {
        for panel in [FocusPanel::Graph, FocusPanel::Checklist] {
            assert_eq!(
                mode.toggle_focus(panel),
                if mode == WindowMode::Focus(panel) {
                    WindowMode::Full
                } else {
                    WindowMode::Focus(panel)
                }
            );
        }
        assert_eq!(
            mode.toggle_mini(),
            if mode == WindowMode::Mini {
                WindowMode::Full
            } else {
                WindowMode::Mini
            }
        );
        assert_eq!(mode.back_to_full(), WindowMode::Full);
    }
}

#[test]
fn widths_respect_bounds_and_graph_space() {
    for a in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -1.0,
        100.0,
        200.0,
        900.0,
    ] {
        for b in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -1.0,
            120.0,
            250.0,
            900.0,
        ] {
            for available in [f32::NAN, f32::INFINITY, -100.0, 350.0, 500.0, 1000.0] {
                let w = ColumnWidths {
                    checklist: a,
                    details: b,
                }
                .clamped(available, 100.0..300.0, 120.0..400.0, 100.0);
                assert!((100.0..300.0).contains(&w.checklist));
                assert!((120.0..400.0).contains(&w.details));
                if available.is_finite() && available >= 320.0 {
                    assert!(w.checklist + w.details + 100.0 <= available + 0.001);
                }
            }
        }
    }
    let w = ColumnWidths {
        checklist: 180.0,
        details: 200.0,
    };
    assert_eq!(w.clamped(1000.0, 100.0..300.0, 120.0..400.0, 100.0), w);
    assert_eq!(
        ColumnWidths {
            checklist: f32::NAN,
            details: f32::INFINITY
        }
        .clamped(1000.0, 100.0..300.0, 120.0..400.0, 100.0),
        ColumnWidths {
            checklist: 100.0,
            details: 120.0
        }
    );
}

#[test]
fn setters_return_priors_that_restore_exact_settings() {
    let original = Settings::default();
    let cases: [fn(&mut Settings) -> SettingPrior; 8] = [
        |s| s.set_theme(ThemeChoice::Dark),
        |s| s.set_language(LanguageChoice::English),
        |s| s.set_reduce_motion(Some(true)),
        |s| s.set_plain_words(false),
        |s| s.set_density(Density::Dense),
        |s| s.set_mode(WindowMode::Mini),
        |s| s.set_mini_on_top(true),
        |s| s.set_board(BoardChoice::Haiku),
    ];
    let priors = [
        SettingPrior::Theme(ThemeChoice::System),
        SettingPrior::Language(LanguageChoice::System),
        SettingPrior::ReduceMotion(None),
        SettingPrior::PlainWords(true),
        SettingPrior::Density(Density::Comfortable),
        SettingPrior::Mode(WindowMode::Full),
        SettingPrior::MiniOnTop(false),
        SettingPrior::Board(BoardChoice::Luna),
    ];
    for (change, expected) in cases.into_iter().zip(priors) {
        let mut s = original.clone();
        let prior = change(&mut s);
        assert_eq!(prior, expected);
        assert_ne!(s, original);
        s.restore(prior);
        assert_eq!(s, original);
    }
}

#[test]
fn missing_usage_and_saturation() {
    let mut s = Settings::default();
    s.add_board_tokens(None, None);
    assert_eq!(
        s.board_tokens,
        BoardTokens {
            input: 0,
            output: 0,
            unreported: 1
        }
    );
    s.add_board_tokens(Some(7), None);
    assert_eq!(
        s.board_tokens,
        BoardTokens {
            input: 7,
            output: 0,
            unreported: 1
        }
    );
    s.board_tokens = BoardTokens {
        input: u64::MAX - 1,
        output: u64::MAX,
        unreported: u64::MAX,
    };
    s.add_board_tokens(Some(2), Some(1));
    s.add_board_tokens(None, None);
    assert_eq!(
        s.board_tokens,
        BoardTokens {
            input: u64::MAX,
            output: u64::MAX,
            unreported: u64::MAX
        }
    );
}

#[test]
fn ephemeral_edits_only_change_memory() {
    let mut store = Store::at(None);
    assert!(store.path.is_none());
    assert!(!store.edit_memory(|s| {
        s.set_mode(WindowMode::Mini);
    }));
    assert_eq!(store.settings.mode, WindowMode::Mini);
    assert!(store.path.is_none());
    assert_eq!(store.version, 0);
    assert!(store.pending.is_none());
    assert_eq!(*store.written.lock().unwrap(), 0);
    assert!(Store::open(true).path.is_none());
    let dir = tempfile::tempdir().unwrap();
    let mut persistent = Store::at(Some(dir.path().join(FILE_NAME)));
    assert!(persistent.edit_memory(|s| { s.set_mode(WindowMode::Mini); }));
    assert!(!persistent.edit_memory(|s| { s.set_mode(WindowMode::Mini); }));
    assert!(!dir.path().join(FILE_NAME).exists());
}
