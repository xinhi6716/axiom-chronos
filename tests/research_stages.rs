use axiom::research_workspace::{
    index::Index,
    manifest::{locate, LocateOptions},
    read::{hash_file, Limits},
    repro::{self, ReproStatus},
    stages::{self, GateNotRun, GateResult, ResearchStatus, Retry, StageState, StaleCause},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "research-stages-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn copy(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        let d = dst.join(e.file_name());
        if p.is_dir() {
            copy(&p, &d)
        } else {
            fs::copy(p, d).unwrap();
        }
    }
}
fn put(root: &Path, path: &str, s: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn ix(root: &Path, store: Option<PathBuf>) -> Index {
    Index::build(
        Arc::new(
            locate(
                root,
                &LocateOptions {
                    store,
                    orch_home: None,
                },
            )
            .unwrap(),
        ),
        Limits::default(),
        &|_, _| {},
    )
    .unwrap()
}
fn write_manifest(root: &Path, m: &Value) {
    put(root, "research-manifest.json", &m.to_string());
}
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(base: &Path, p: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in fs::read_dir(p).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out)
            } else {
                out.insert(p.strip_prefix(base).unwrap().into(), hash_file(&p).unwrap());
            }
        }
    }
    let mut x = BTreeMap::new();
    walk(root, root, &mut x);
    x
}
fn base() -> Value {
    json!({"manifest_version":1,"research_id":"case","research_version":"1","title":"case","settings":{"limit":5},"versions":{"code":"v1","prompts":{"p":"p1"}},"artifacts":[{"id":"input","role":"other","path":"input.txt","format":"text"},{"id":"out","role":"other","path":"out.csv","format":"csv","produced":{"inputs":[],"code_version":"v1","prompt_version":"p1"}},{"id":"gate","role":"gate","path":"gate.json","format":"json"}],"stages":[{"id":"one","label":"One","outputs":["out"],"gate":{"artifact":"gate","pointer":"/value","op":"ge","setting":"/limit"}}]})
}
fn setup(root: &Path, m: &Value) {
    put(root, "input.txt", "original");
    put(root, "out.csv", "value\n1\n");
    put(root, "gate.json", "{\"value\":5}");
    write_manifest(root, m);
}

#[test]
fn rram_driver_progress_engineering_and_read_only() {
    let t = Temp::new();
    copy(Path::new("tests/fixtures/research/rram"), &t.0);
    let root = t.0.join("project");
    let store = t.0.join("store");
    let before = tree(&t.0);
    let index = ix(&root, Some(store.clone()));
    let map = stages::research_map(&index, 0);
    assert_eq!(
        map.status,
        ResearchStatus::Paused {
            reason: "rate limited".into()
        }
    );
    let screen = map.stages.iter().find(|s| s.id == "screen").unwrap();
    assert_eq!(
        screen.state,
        StageState::Paused {
            reason: "rate limited".into()
        }
    );
    let p = screen.progress.as_ref().unwrap();
    assert_eq!((p.done, p.total, p.failed, p.pending), (152, 172, 0, 20));
    assert_eq!(map.engineering.unwrap().verified, 1);
    assert_eq!(tree(&t.0), before);
    put(&root, "state/progress/driver.json", "{}");
    let m = stages::research_map(&ix(&root, Some(store.clone())), 0);
    assert_eq!(
        m.stages.iter().find(|s| s.id == "screen").unwrap().state,
        StageState::RateLimited {
            retry: Retry::Unknown
        }
    );
    put(
        &root,
        "state/progress/screen.json",
        r#"{"total":172,"done":152,"failed":0,"pending":20,"rate_limited_until":"2026-11-14T00:00:00Z"}"#,
    );
    let m = stages::research_map(&ix(&root, Some(store.clone())), 0);
    assert_eq!(
        m.stages.iter().find(|s| s.id == "screen").unwrap().state,
        StageState::RateLimited {
            retry: Retry::At("2026-11-14T00:00:00Z".into())
        }
    );
    put(
        &store,
        "units.tsv",
        "id\ttrack\tstate\n1\tx\tverified\n2\tx\tsuperseded:1\n3\tx\tworking\n",
    );
    let e = stages::research_map(&ix(&root, Some(store)), 0)
        .engineering
        .unwrap();
    assert_eq!((e.verified, e.superseded, e.other, e.total), (1, 1, 1, 3));
}
#[test]
fn rate_limit_requires_unresolved_status() {
    let t = Temp::new();
    let mut m = base();
    m["artifacts"].as_array_mut().unwrap().extend([
        json!({"id":"progress","role":"progress","path":"progress.json","format":"json"}),
        json!({"id":"driver","role":"driver","path":"driver.json","format":"json"}),
        json!({"id":"completed","role":"completed","path":"completed.jsonl","format":"jsonl"}),
    ]);
    m["stages"][0]["progress"] = json!("progress");
    setup(&t.0, &m);
    put(
        &t.0,
        "progress.json",
        r#"{"total":172,"done":152,"failed":0,"pending":20,"rate_limited_until":null}"#,
    );
    put(
        &t.0,
        "driver.json",
        r#"{"stage":"one","state":"rate_limited"}"#,
    );
    put(&t.0, "completed.jsonl", "");
    let state = || {
        stages::research_map(&ix(&t.0, None), 0).stages[0]
            .state
            .clone()
    };
    assert_eq!(
        state(),
        StageState::RateLimited {
            retry: Retry::Unknown
        }
    );
    put(
        &t.0,
        "progress.json",
        r#"{"total":172,"done":152,"failed":0,"pending":20,"rate_limited_until":"unknown"}"#,
    );
    assert_eq!(
        state(),
        StageState::RateLimited {
            retry: Retry::Unknown
        }
    );
    put(
        &t.0,
        "driver.json",
        r#"{"stage":"one","state":"rate_limited","retry_at":"2026-11-14T00:00:00Z"}"#,
    );
    assert_eq!(
        state(),
        StageState::RateLimited {
            retry: Retry::At("2026-11-14T00:00:00Z".into())
        }
    );
    put(&t.0, "driver.json", "{}");
    put(&t.0, "completed.jsonl", "{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"rate_limited\",\"retry_after\":\"2026-11-15T00:00:00Z\"}\n");
    assert_eq!(
        state(),
        StageState::RateLimited {
            retry: Retry::At("2026-11-15T00:00:00Z".into())
        }
    );
    put(&t.0, "completed.jsonl", "{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"rate_limited\"}\n{\"stage\":\"one\",\"sha256\":\"b\",\"status\":\"done\"}\n");
    assert_eq!(
        state(),
        StageState::RateLimited {
            retry: Retry::Unknown
        }
    );
    put(&t.0, "completed.jsonl", "{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"rate_limited\"}\n{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"done\"}\n");
    put(
        &t.0,
        "progress.json",
        r#"{"total":172,"done":152,"failed":0,"pending":20,"rate_limited_until":"2026-11-14T00:00:00Z"}"#,
    );
    assert_eq!(state(), StageState::Running);
    put(&t.0, "completed.jsonl", "{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"done\"}\n{\"stage\":\"one\",\"sha256\":\"a\",\"status\":\"rate_limited\"}\n");
    assert!(matches!(state(), StageState::RateLimited { .. }));
    put(
        &t.0,
        "progress.json",
        r#"{"total":172,"done":172,"failed":0,"pending":0,"rate_limited_until":"2026-11-14T00:00:00Z"}"#,
    );
    assert_eq!(state(), StageState::Done);
}
#[test]
fn stale_pins_and_versions() {
    let t = Temp::new();
    let mut m = base();
    setup(&t.0, &m);
    let old = hash_file(&t.0.join("input.txt")).unwrap();
    m["artifacts"][1]["produced"]["inputs"] = json!([{ "artifact":"input","sha256":old }]);
    write_manifest(&t.0, &m);
    let map = stages::research_map(&ix(&t.0, None), 0);
    assert_eq!(map.status, ResearchStatus::Passed);
    assert!(map.stale.is_empty());
    put(&t.0, "input.txt", "changed");
    let current = hash_file(&t.0.join("input.txt")).unwrap();
    assert_eq!(stages::staleness(&ix(&t.0, None)).len(), 1);
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 0).status,
        ResearchStatus::Stale
    );
    let map = stages::research_map(&ix(&t.0, None), 0);
    assert_eq!(
        map.stale[0].cause,
        StaleCause::InputChanged {
            input: "input".into(),
            recorded: old.clone(),
            current: current.clone()
        }
    );
    put(&t.0, "input.txt", "original");
    m["versions"]["code"] = json!("v2");
    write_manifest(&t.0, &m);
    let map = stages::research_map(&ix(&t.0, None), 0);
    assert_eq!(map.status, ResearchStatus::Stale);
    assert_eq!(map.stale.len(), 1);
    assert_eq!(
        map.stale[0].cause,
        StaleCause::CodeChanged {
            recorded: "v1".into(),
            current: "v2".into(),
        }
    );
    m["versions"]["code"] = json!("v1");
    m["versions"]["prompts"]["p"] = json!("p2");
    write_manifest(&t.0, &m);
    let map = stages::research_map(&ix(&t.0, None), 0);
    assert_eq!(map.status, ResearchStatus::Stale);
    assert_eq!(map.stale.len(), 1);
    assert_eq!(
        map.stale[0].cause,
        StaleCause::PromptChanged {
            recorded: "p1".into(),
            current: "p2".into(),
        }
    );
    put(&t.0, "input.txt", "changed");
    put(&t.0, "pin.txt", &old);
    m["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"pin","role":"other","path":"pin.txt","format":"text"}));
    m["artifacts"][1]["produced"]["inputs"] = json!([{ "artifact":"input","sha256_from":"pin" }]);
    write_manifest(&t.0, &m);
    assert!(stages::staleness(&ix(&t.0, None))
        .iter()
        .any(|s| matches!(s.cause, StaleCause::InputChanged { .. })));
}
#[test]
fn stall_and_blocker() {
    let t = Temp::new();
    let mut m = base();
    m["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"progress","role":"progress","path":"progress.json","format":"json"}));
    m["stages"][0]["progress"] = json!("progress");
    m["stall_after_s"] = json!(10);
    setup(&t.0, &m);
    put(
        &t.0,
        "progress.json",
        r#"{"total":10,"done":3,"failed":0,"pending":7,"updated":"2024-01-01T00:00:00Z"}"#,
    );
    let recent = stages::research_map(&ix(&t.0, None), 1704067205000);
    assert_eq!(recent.stages[0].state, StageState::Running);
    assert_eq!(stages::blocker(&recent), None);
    let old = stages::research_map(&ix(&t.0, None), 1704067211000);
    assert!(matches!(old.status, ResearchStatus::Stalled));
    assert_eq!(stages::blocker(&old).unwrap().0, "one");
    put(
        &t.0,
        "progress.json",
        r#"{"total":10,"done":3,"failed":1,"pending":6,"updated":"2024-01-01T00:00:00Z"}"#,
    );
    let recent = stages::research_map(&ix(&t.0, None), 1704067205000);
    assert_eq!(
        recent.stages[0].state,
        StageState::Failed {
            reason: "1 failed".into()
        }
    );
    assert_eq!(recent.status, ResearchStatus::InProgress);
    let old = stages::research_map(&ix(&t.0, None), 1704067211000);
    assert_eq!(
        old.stages[0].state,
        StageState::Stalled {
            since: "2024-01-01T00:00:00Z".into()
        }
    );
    assert_eq!(old.status, ResearchStatus::Stalled);
}
#[test]
fn gate_comparisons_and_status() {
    let t = Temp::new();
    let mut m = base();
    setup(&t.0, &m);
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 0).status,
        ResearchStatus::Passed
    );
    for (op, value, expected) in [
        ("ge", 4, true),
        ("ge", 6, false),
        ("le", 6, true),
        ("le", 4, false),
        ("eq", 5, true),
        ("eq", 4, false),
    ] {
        m["stages"][0]["gate"]["op"] = json!(op);
        m["settings"]["limit"] = json!(value);
        write_manifest(&t.0, &m);
        let map = stages::research_map(&ix(&t.0, None), 0);
        assert_eq!(
            matches!(map.stages[0].gate, GateResult::Passed),
            expected,
            "{op} {value}"
        );
        if !expected {
            assert_eq!(map.status, ResearchStatus::InProgress);
        }
    }
    m["stages"][0]["gate"]["setting"] = json!("/missing");
    write_manifest(&t.0, &m);
    assert!(matches!(
        stages::research_map(&ix(&t.0, None), 0).stages[0].gate,
        GateResult::NotRun { .. }
    ));
    m["stages"][0]["gate"]["setting"] = json!("/limit");
    write_manifest(&t.0, &m);
    fs::remove_file(t.0.join("gate.json")).unwrap();
    assert!(matches!(
        stages::research_map(&ix(&t.0, None), 0).stages[0].gate,
        GateResult::NotRun { .. }
    ));
    m["stages"][0]["gate"] = Value::Null;
    write_manifest(&t.0, &m);
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 0).stages[0].gate,
        GateResult::NotDeclared
    );
}
#[test]
fn gate_not_run_reasons_are_typed() {
    let t = Temp::new();
    let mut m = base();
    setup(&t.0, &m);
    let check = |m: &Value, expected| {
        write_manifest(&t.0, m);
        assert_eq!(
            stages::research_map(&ix(&t.0, None), 0).stages[0].gate,
            GateResult::NotRun { reason: expected }
        );
    };
    m["stages"][0]["gate"]["setting"] = json!("/missing");
    check(
        &m,
        GateNotRun::MissingSetting {
            pointer: "/missing".into(),
        },
    );
    m["stages"][0]["gate"]["setting"] = Value::Null;
    check(&m, GateNotRun::MissingThreshold);
    m["stages"][0]["gate"]["value"] = json!(5);
    put(&t.0, "gate.json", "invalid json");
    check(
        &m,
        GateNotRun::ArtifactUnreadable {
            artifact: "gate".into(),
        },
    );
    fs::remove_file(t.0.join("gate.json")).unwrap();
    check(
        &m,
        GateNotRun::ArtifactUnreadable {
            artifact: "gate".into(),
        },
    );
    put(&t.0, "gate.json", r#"{"other":5}"#);
    check(
        &m,
        GateNotRun::MissingPointer {
            pointer: "/value".into(),
        },
    );
    put(&t.0, "gate.json", r#"{"value":"not numeric"}"#);
    for op in ["ge", "le"] {
        m["stages"][0]["gate"]["op"] = json!(op);
        check(&m, GateNotRun::NonNumeric);
    }
}
#[test]
fn repro_groups_and_missing() {
    let t = Temp::new();
    copy(Path::new("tests/fixtures/research/rawonly"), &t.0);
    let before = tree(&t.0);
    let r = repro::guard(&ix(&t.0, None));
    assert_eq!(r.status, ReproStatus::Pass);
    assert_eq!((r.pilot_groups, r.acceptance_groups), (1, 1));
    assert_eq!(tree(&t.0), before);
    let pilot = fs::read_to_string(t.0.join("data/pilot.csv")).unwrap();
    put(&t.0, "data/acceptance.csv", &pilot);
    assert_eq!(
        repro::guard(&ix(&t.0, None)).status,
        ReproStatus::Fail {
            overlap: vec!["lab-a".into()]
        }
    );
    put(&t.0, "data/pilot.csv", "\"broken");
    assert!(matches!(
        repro::guard(&ix(&t.0, None)).status,
        ReproStatus::NotApplicable { .. }
    ));
    fs::remove_file(t.0.join("data/pilot.csv")).unwrap();
    assert!(matches!(
        repro::guard(&ix(&t.0, None)).status,
        ReproStatus::NotApplicable { .. }
    ));
    let mut m: Value =
        serde_json::from_str(&fs::read_to_string(t.0.join("research-manifest.json")).unwrap())
            .unwrap();
    m["groups"] = Value::Null;
    write_manifest(&t.0, &m);
    assert!(matches!(
        repro::guard(&ix(&t.0, None)).status,
        ReproStatus::NotApplicable { .. }
    ));
}
#[test]
fn study_groups_overlap() {
    let t = Temp::new();
    copy(Path::new("tests/fixtures/research/rram"), &t.0);
    let root = t.0.join("project");
    let i = ix(&root, None);
    let g = repro::study_groups(&i);
    assert_eq!(g["P1"], "P0");
    assert_eq!(g["P4"], "P0");
    assert_eq!(
        repro::guard(&i).status,
        ReproStatus::Fail {
            overlap: vec!["P0".into()]
        }
    );
}
#[test]
fn passing_project_repro_failure_and_first_blocker() {
    let t = Temp::new();
    let mut m = base();
    m["artifacts"].as_array_mut().unwrap().extend([
        json!({"id":"pilot","role":"pilot","path":"pilot.csv","format":"csv"}),
        json!({"id":"acceptance","role":"acceptance","path":"acceptance.csv","format":"csv"}),
    ]);
    m["groups"] = json!({"pilot":"pilot","acceptance":"acceptance","by":{"column":"lab"}});
    setup(&t.0, &m);
    put(&t.0, "pilot.csv", "lab\na\n");
    put(&t.0, "acceptance.csv", "lab\nb\n");
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 0).status,
        ResearchStatus::Passed
    );
    put(&t.0, "acceptance.csv", "lab\na\n");
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 0).status,
        ResearchStatus::InProgress
    );
    m["stages"].as_array_mut().unwrap().extend([
        json!({"id":"two","label":"Two","outputs":["missing"]}),
        json!({"id":"three","label":"Three","outputs":["missing"]}),
    ]);
    write_manifest(&t.0, &m);
    let map = stages::research_map(&ix(&t.0, None), 0);
    assert_eq!(
        stages::blocker(&map),
        Some(("two".into(), StageState::NotStarted))
    );
}
#[test]
fn pin_json_pointer_and_offset_stall() {
    let t = Temp::new();
    let mut m = base();
    setup(&t.0, &m);
    let old = hash_file(&t.0.join("input.txt")).unwrap();
    put(&t.0, "pin.json", &json!({"sha":old}).to_string());
    m["artifacts"].as_array_mut().unwrap().extend([
        json!({"id":"pin","role":"other","path":"pin.json","format":"json"}),
        json!({"id":"progress","role":"progress","path":"progress.json","format":"json"}),
    ]);
    m["artifacts"][1]["produced"]["inputs"] =
        json!([{ "artifact":"input","sha256_from":"pin#/sha" }]);
    m["stages"][0]["progress"] = json!("progress");
    m["stall_after_s"] = json!(10);
    write_manifest(&t.0, &m);
    put(
        &t.0,
        "progress.json",
        r#"{"total":2,"done":1,"failed":0,"pending":1,"updated":"2024-01-01T08:00:00+08:00"}"#,
    );
    assert!(stages::staleness(&ix(&t.0, None)).is_empty());
    assert_eq!(
        stages::research_map(&ix(&t.0, None), 1704067211000).status,
        ResearchStatus::Stalled
    );
    put(&t.0, "input.txt", "different");
    assert!(stages::staleness(&ix(&t.0, None))
        .iter()
        .any(|s| matches!(s.cause, StaleCause::InputChanged { .. })));
}
#[test]
fn constructed_study_groups_join_supplements_and_duplicates() {
    let t = Temp::new();
    let mut m = base();
    m["artifacts"].as_array_mut().unwrap().extend([
        json!({"id":"inventory","role":"inventory","path":"inventory.csv","format":"csv"}),
        json!({"id":"pilot","role":"pilot","path":"pilot.csv","format":"csv"}),
        json!({"id":"acceptance","role":"acceptance","path":"acceptance.csv","format":"csv"}),
    ]);
    m["groups"] = json!({"pilot":"pilot","acceptance":"acceptance","by":"study_groups"});
    setup(&t.0, &m);
    put(&t.0, "inventory.csv", "path,paper_id,duplicate_group,supplement_of,erratum_of\na,A,dup,,\nb,B,dup,,\nc,C,,A,\nd,D,,,C\n");
    put(&t.0, "pilot.csv", "paper_id\nB\n");
    put(&t.0, "acceptance.csv", "paper_id\nD\n");
    let i = ix(&t.0, None);
    let groups = repro::study_groups(&i);
    assert_eq!(groups["D"], "A");
    assert_eq!(
        repro::guard(&i).status,
        ReproStatus::Fail {
            overlap: vec!["A".into()]
        }
    );
}
