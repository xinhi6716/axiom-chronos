use super::adapter::map_format::{engineering_text, map_lines, repro_text, status_text, Tone};
use crate::{
    desktop::i18n::Lang,
    research_workspace::{
        repro::{ReproResult, ReproStatus},
        stages::{
            Engineering, GateNotRun, GateResult, Progress, ResearchMap, ResearchStatus, Retry,
            StageState, StageView, StaleCause, Staleness,
        },
    },
};

#[test]
fn map_funnel_uses_actual_viz_id_and_display_counts() {
    use crate::desktop::{
        theme,
        ux::seams::research::{FunnelNode, MapDisplay},
    };
    use gpui_kit::{Element as _, IntoElement as _};
    let map = MapDisplay {
        funnel: vec![
            FunnelNode {
                label: "Total".into(),
                count: 42,
                ..Default::default()
            },
            FunnelNode {
                label: "Included".into(),
                count: 7,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let stages = super::map::funnel_stages(&map);
    assert_eq!(
        stages
            .iter()
            .map(|s| (s.label.as_str(), s.count))
            .collect::<Vec<_>>(),
        vec![("Total", 42), ("Included", 7)]
    );
    assert_eq!(
        super::map::map_funnel(&map, &theme::night())
            .into_element()
            .id(),
        Some("research-map-funnel".into())
    );
}

fn map(state: StageState, gate: GateResult) -> ResearchMap {
    ResearchMap {
        status: ResearchStatus::Paused {
            reason: "waiting".into(),
        },
        stages: vec![StageView {
            id: "s".into(),
            label: "stage".into(),
            state,
            progress: None,
            gate,
            stale: vec![],
        }],
        stale: vec![],
        engineering: None,
        repro: None,
    }
}
fn repro(status: ReproStatus) -> ReproResult {
    ReproResult {
        status,
        pilot_groups: 1,
        acceptance_groups: 1,
        pilot_sha256: None,
        acceptance_sha256: None,
        rule: String::new(),
    }
}
#[test]
fn build_verification_is_not_research_pass() {
    let mut m = map(StageState::Done, GateResult::NotDeclared);
    m.engineering = Some(Engineering {
        verified: 12,
        superseded: 0,
        other: 0,
        total: 12,
    });
    assert!(engineering_text(m.engineering.as_ref().unwrap(), Lang::En)
        .contains("12 of 12 units verified (this is not a research result)"));
    for lang in [Lang::En, Lang::ZhTw] {
        let forbidden = if lang == Lang::En { "passed" } else { "通過" };
        let lines = map_lines(&m, lang);
        assert!(!format!("{:?} {}", lines, status_text(&m.status, lang))
            .to_lowercase()
            .contains(forbidden));
        m.stages[0].gate = GateResult::Passed;
        assert!(map_lines(&m, lang)[0]
            .gate_text
            .to_lowercase()
            .contains(forbidden));
        assert!(status_text(&ResearchStatus::Passed, lang)
            .to_lowercase()
            .contains(forbidden));
        m.stages[0].gate = GateResult::NotDeclared;
    }
}
#[test]
fn stale_input_and_progress() {
    let mut m = map(StageState::Done, GateResult::NotDeclared);
    m.stages[0].stale.push(Staleness {
        artifact: "output".into(),
        cause: StaleCause::InputChanged {
            input: "source".into(),
            recorded: "old".into(),
            current: "new".into(),
        },
    });
    m.stages[0].progress = Some(Progress {
        done: 152,
        total: 172,
        failed: 3,
        pending: 17,
    });
    let l = &map_lines(&m, Lang::En)[0];
    assert_eq!(l.tone, Tone::Stale);
    assert_eq!(l.progress_text.as_deref(), Some("152/172"));
    for part in ["output", "source", "old", "new"] {
        assert!(l.stale_text[0].contains(part));
    }
    m.stages[0].state = StageState::Paused { reason: "x".into() };
    assert_eq!(map_lines(&m, Lang::En)[0].tone, Tone::Blocked);
}
#[test]
fn rate_limit_retry_is_explicit() {
    let m = map(
        StageState::RateLimited {
            retry: Retry::Unknown,
        },
        GateResult::NotDeclared,
    );
    assert_eq!(
        map_lines(&m, Lang::En)[0].state_text,
        "rate limited; retry time unknown"
    );
    assert!(map_lines(&m, Lang::ZhTw)[0]
        .state_text
        .contains("重試時間未知"));
    let at = ResearchStatus::RateLimited {
        retry: Retry::At("tomorrow".into()),
    };
    assert!(status_text(&at, Lang::En).contains("tomorrow"));
    assert!(status_text(&at, Lang::ZhTw).contains("tomorrow"));
}
#[test]
fn repro_guard_reports_result_and_overlap() {
    let fail = repro_text(
        &repro(ReproStatus::Fail {
            overlap: vec!["a".into(), "b".into()],
        }),
        Lang::En,
    );
    assert!(fail.contains("fail") && fail.contains("a, b"));
    assert!(repro_text(&repro(ReproStatus::Pass), Lang::En).contains("pass"));
    assert!(repro_text(
        &repro(ReproStatus::NotApplicable {
            reason: "missing".into()
        }),
        Lang::En
    )
    .contains("missing"));
}
#[test]
fn chinese_state_words_are_localized() {
    let states = [
        StageState::NotStarted,
        StageState::Running,
        StageState::Paused { reason: "x".into() },
        StageState::Blocked { reason: "x".into() },
        StageState::RateLimited {
            retry: Retry::Unknown,
        },
        StageState::Stalled { since: "x".into() },
        StageState::Done,
        StageState::Failed { reason: "x".into() },
    ];
    for state in states {
        let m = map(state, GateResult::NotDeclared);
        let en = map_lines(&m, Lang::En)[0]
            .state_text
            .split(':')
            .next()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_lowercase();
        let zh = map_lines(&m, Lang::ZhTw)[0].state_text.to_lowercase();
        assert!(!zh.contains(&en), "{en}: {zh}");
    }
}

#[test]
fn gate_not_run_reasons_are_localized() {
    let cases = [
        (
            GateNotRun::ArtifactUnreadable {
                artifact: "artifact-id".into(),
            },
            Some("artifact-id"),
            "Gate not run: gate artifact artifact-id missing or corrupt",
        ),
        (
            GateNotRun::MissingSetting {
                pointer: "/setting-name".into(),
            },
            Some("/setting-name"),
            "Gate not run: missing setting /setting-name",
        ),
        (
            GateNotRun::MissingThreshold,
            None,
            "Gate not run: missing gate threshold",
        ),
        (
            GateNotRun::MissingPointer {
                pointer: "/pointer-name".into(),
            },
            Some("/pointer-name"),
            "Gate not run: missing gate pointer /pointer-name",
        ),
        (
            GateNotRun::NonNumeric,
            None,
            "Gate not run: gate requires numeric values",
        ),
    ];
    for (reason, value, english) in cases {
        let m = map(StageState::NotStarted, GateResult::NotRun { reason });
        assert_eq!(map_lines(&m, Lang::En)[0].gate_text, english);
        let zh = &map_lines(&m, Lang::ZhTw)[0].gate_text;
        let without_value = if let Some(value) = value {
            assert!(zh.contains(value), "{zh}");
            zh.replace(value, "")
        } else {
            zh.clone()
        };
        assert!(
            !without_value.chars().any(|c| c.is_ascii_alphabetic()),
            "{zh}"
        );
    }
}
