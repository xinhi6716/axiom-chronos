use axiom::{engine::{budgets::{self, Role}, kinds, policy::{self, Models}}, session::stage::{self, Stage, Via, TurnRecord}};
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, time::Duration};

const SESSION: &str = "s";
const HEADER: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
struct Fixture { home: tempfile::TempDir, store: PathBuf }
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let store = home.path().join("orchestrate/project/w-1234567890123-abcdef");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(home.path().join("goals")).unwrap();
        let f = Self { home, store };
        f.goal("active", true);
        f
    }
    fn home(&self) -> &Path { self.home.path() }
    fn goal(&self, status: &str, project: bool) {
        fs::write(self.home().join("goals/s.json"), json!({"wish":"test", "status":status,
            "project":if project { Some("project") } else { None }, "wish_id":"w-1234567890123-abcdef"}).to_string()).unwrap();
    }
    fn put(&self, file: &str, text: &str) {
        let path = self.store.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn plans(&self) { self.put("astra/plan-final.md", "plan"); self.put("modules.md", "modules"); }
    fn gate(&self, status: &str) {
        self.put("gates.md", &format!("# Gates\n\n## g1\n- Question: q\n- Options: a,b\n- Default: a\n- Status: {status}\n{}", if status == "resolved" { "- Answer: a\n" } else { "" }));
    }
    fn units(&self, states: &[&str]) {
        let mut text = HEADER.to_string();
        for (i, state) in states.iter().enumerate() { text.push_str(&format!("u{i}\tt\t{state}\t\t\t\tbrief\n")); }
        self.put("units.tsv", &text);
    }
    fn current(&self) -> Stage { stage::current(self.home(), SESSION) }
}
fn rec(turn: u64, model: &str, via: Via, ok: bool, error: Option<&str>) -> TurnRecord {
    TurnRecord { turn, stage: "build".into(), model: model.into(), via, ok, control_error: error.map(str::to_owned) }
}

#[test]
fn empty_store_is_planning() {
    let f = Fixture::new();
    assert_eq!(f.current(), Stage::Planning);
    assert_eq!(Stage::Planning.as_str(), "planning");
}
#[test]
fn one_plan_file_is_still_planning() {
    let f = Fixture::new();
    f.put("astra/plan-final.md", "plan");
    assert_eq!(f.current(), Stage::Planning);
    fs::remove_file(f.store.join("astra/plan-final.md")).unwrap();
    f.put("modules.md", "modules");
    assert_eq!(f.current(), Stage::Planning);
}
#[test]
fn both_plan_files_is_build() {
    let f = Fixture::new(); f.plans();
    assert_eq!(f.current(), Stage::Build);
    assert_eq!(Stage::Build.as_str(), "build");
}
#[test]
fn open_gate_is_escalation() {
    let f = Fixture::new(); f.plans(); f.gate("open");
    assert_eq!(f.current(), Stage::Escalation);
    f.gate("resolved");
    assert_eq!(f.current(), Stage::Build);
    assert_eq!(Stage::Escalation.as_str(), "escalation");
}
#[test]
fn pending_adjudication_marker() {
    let f = Fixture::new(); f.plans(); f.put("adjudication.pending", "pending");
    assert_eq!(f.current(), Stage::Escalation);
    f.put("adjudication.pending", "");
    assert_eq!(f.current(), Stage::Build);
}
#[test]
fn all_units_verified_is_close() {
    let f = Fixture::new(); f.plans(); f.units(&["verified", "verified", "superseded:u0", "superseded"]);
    assert_eq!(f.current(), Stage::Close);
    assert_eq!(Stage::Close.as_str(), "close");
    f.units(&["verified", "built"]); assert_eq!(f.current(), Stage::Build);
    f.units(&[]); assert_eq!(f.current(), Stage::Build);
    f.units(&["superseded:u0"]); assert_eq!(f.current(), Stage::Build);
}
#[test]
fn done_goal_is_close() {
    let f = Fixture::new(); f.gate("open"); f.goal("done", true);
    assert_eq!(f.current(), Stage::Close);
    f.goal("done", false); assert_eq!(f.current(), Stage::Close);
}
#[test]
fn close_beats_escalation_beats_build() {
    let f = Fixture::new(); f.plans(); f.gate("open"); f.put("adjudication.pending", "pending"); f.units(&["verified"]);
    assert_eq!(f.current(), Stage::Close);
    f.units(&["built"]); assert_eq!(f.current(), Stage::Escalation);
    f.gate("resolved"); assert_eq!(f.current(), Stage::Escalation);
    f.put("adjudication.pending", ""); assert_eq!(f.current(), Stage::Build);
    fs::remove_file(f.store.join("modules.md")).unwrap(); assert_eq!(f.current(), Stage::Planning);
}
#[test]
fn unreadable_state_degrades() {
    let f = Fixture::new();
    assert_eq!(stage::current(f.home(), "unknown"), Stage::Planning);
    f.goal("active", false); assert_eq!(f.current(), Stage::Planning);
    f.goal("active", true); f.plans(); f.put("gates.md", "broken\n- Status: open\n");
    assert_eq!(f.current(), Stage::Build);
    f.put("units.tsv", "invalid"); assert_eq!(f.current(), Stage::Build);
    f.put("units.tsv", &format!("{HEADER}broken\n")); assert_eq!(f.current(), Stage::Build);
    fs::write(f.home().join("goals/s.json"), "{").unwrap(); assert_eq!(f.current(), Stage::Planning);
    f.goal("active", true); fs::remove_dir_all(&f.store).unwrap(); assert_eq!(f.current(), Stage::Planning);
}
#[test]
fn model_defaults_and_rereads() {
    let f = Fixture::new();
    for (s, expected) in [(Stage::Planning,"opus"),(Stage::Build,"sonnet"),(Stage::Escalation,"opus"),(Stage::Close,"opus")] {
        assert_eq!(stage::model_for(f.home(), SESSION, s).unwrap(), expected);
        assert_eq!(stage::model_for(f.home(), "unknown", s).unwrap(), expected);
    }
    let mut models = Models::default(); models.core.build = "global-build".into();
    policy::save_global_models(f.home(), &models).unwrap();
    assert_eq!(stage::model_for(f.home(), SESSION, Stage::Build).unwrap(), "global-build");
    assert_eq!(stage::model_for(f.home(), "unknown", Stage::Build).unwrap(), "global-build");
    f.goal("active", false);
    assert_eq!(stage::model_for(f.home(), SESSION, Stage::Build).unwrap(), "global-build");
    f.goal("active", true);
    f.put("policy.json", r#"{"models":{"core":{"planning":"p","build":"b","escalation":"e","close":"c"}}}"#);
    for (s, expected) in [(Stage::Planning,"p"),(Stage::Build,"b"),(Stage::Escalation,"e"),(Stage::Close,"c")] {
        assert_eq!(stage::model_for(f.home(), SESSION, s).unwrap(), expected);
    }
    f.put("policy.json", r#"{"models":{"core":{"build":"changed"}}}"#);
    assert_eq!(stage::model_for(f.home(), SESSION, Stage::Build).unwrap(), "changed");
    f.put("policy.json", "{}"); models.core.build = "global-new".into();
    policy::save_global_models(f.home(), &models).unwrap();
    assert_eq!(stage::model_for(f.home(), SESSION, Stage::Build).unwrap(), "global-new");
}
#[test]
fn turn_log_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    assert!(stage::read_turns(home.path(), SESSION).unwrap().is_empty());
    stage::log_turn(home.path(), SESSION, &rec(1,"opus",Via::Initial,true,None)).unwrap();
    stage::log_turn(home.path(), SESSION, &rec(2,"sonnet",Via::Control,false,Some("denied"))).unwrap();
    let path = home.path().join("goals/s.turns.jsonl");
    assert!(path.is_file());
    let text = fs::read_to_string(path).unwrap(); assert_eq!(text.lines().count(), 2); assert!(text.ends_with('\n'));
    let values: Vec<Value> = text.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    assert_eq!(values[0], json!({"turn":1,"stage":"build","model":"opus","via":"initial","ok":true}));
    assert_eq!(values[1], json!({"turn":2,"stage":"build","model":"sonnet","via":"control","ok":false,"control_error":"denied"}));
    let records = stage::read_turns(home.path(), SESSION).unwrap(); assert_eq!(records.len(), 2);
    assert_eq!(serde_json::to_value(&records[0]).unwrap(), values[0]);
    assert_eq!(serde_json::to_value(&records[1]).unwrap(), values[1]);
    let resume: TurnRecord = serde_json::from_value(json!({"turn":3,"stage":"close","model":"opus","via":"resume","ok":true})).unwrap();
    assert_eq!(serde_json::to_value(resume).unwrap()["via"], "resume");
}
#[test]
fn turn_log_malformed_line_errors() {
    let f = Fixture::new();
    for bad in ["not json", "{}", ""] {
        let valid = serde_json::to_string(&rec(1,"opus",Via::Resume,true,None)).unwrap();
        fs::write(f.home().join("goals/s.turns.jsonl"), format!("{valid}\n{bad}\n{valid}\n")).unwrap();
        let result = stage::read_turns(f.home(), SESSION);
        assert!(result.is_err()); assert_eq!(result.err().unwrap().kind, "invalid");
    }
}
#[test]
fn set_model_request_wire() {
    let text = stage::set_model_request("id\"\n", "model\\\n");
    assert_eq!(text.lines().count(), 1); assert!(text.ends_with('\n')); assert!(!text.ends_with("\n\n"));
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({"type":"control_request","request_id":"id\"\n","request":{"subtype":"set_model","model":"model\\\n"}}));
    assert_eq!(stage::CONTROL_TIMEOUT, Duration::from_secs(10));
}
#[test]
fn control_response_parsing() {
    let event = |subtype: &str| json!({"type":"control_response","response":{"request_id":"r","subtype":subtype}});
    assert_eq!(stage::parse_control_response(&event("success"), "r"), Some(Ok(())));
    let mut error = event("error"); error["response"]["error"] = json!("denied");
    assert_eq!(stage::parse_control_response(&error,"r"), Some(Err("denied".into())));
    assert_eq!(stage::parse_control_response(&event("error"),"r"), Some(Err("control error".into())));
    assert!(matches!(stage::parse_control_response(&event("other"),"r"), Some(Err(_))));
    assert!(matches!(stage::parse_control_response(&json!({"type":"control_response","response":{"request_id":"r"}}),"r"), Some(Err(_))));
    assert_eq!(stage::parse_control_response(&event("success"),"different"), None);
    assert_eq!(stage::parse_control_response(&json!({"type":"assistant"}),"r"), None);
    assert_eq!(stage::parse_control_response(&json!({}),"r"), None);
}
#[test]
fn trace_groups_by_model_and_role() {
    let f = Fixture::new();
    for (turn, model, ok) in [(1,"opus",true),(2,"opus",true),(3,"sonnet",false),(4,"sonnet",true)] {
        stage::log_turn(f.home(), SESSION, &rec(turn,model,Via::Initial,ok,None)).unwrap();
    }
    for (role, model) in [(Role::Reviewer,"astra"),(Role::Reviewer,"astra"),(Role::Dispatcher,"astra"),(Role::Reviewer,"opus"),(Role::Core,"ignored")] {
        budgets::record(&f.store,role,model,Some(0.1),1,1,"call").unwrap();
    }
    axiom::orch::events::append_event(&f.store, kinds::ROLE_SPEND, None,None,json!({"role":"core","model":"opus"})).unwrap();
    axiom::orch::events::append_event(&f.store, "other",None,None,json!({"role":"reviewer","model":"ignored"})).unwrap();
    let rows = stage::trace(f.home(), SESSION).unwrap();
    let rows: Vec<_> = rows.iter().map(|r| (r.model.as_str(),r.role.as_str(),r.calls)).collect();
    assert_eq!(rows, vec![("astra","dispatcher",1),("astra","reviewer",2),("opus","core",2),("opus","reviewer",1),("sonnet","core",1)]);
    f.goal("active",false);
    assert_eq!(stage::trace(f.home(),SESSION).unwrap().len(), 2);
    assert!(stage::trace(f.home(),"unknown").unwrap().is_empty());
}
#[test]
fn stage_is_not_cached() {
    let f = Fixture::new(); assert_eq!(f.current(), Stage::Planning);
    f.plans(); assert_eq!(f.current(), Stage::Build);
    f.put("adjudication.pending","pending"); assert_eq!(f.current(), Stage::Escalation);
    fs::remove_file(f.store.join("adjudication.pending")).unwrap(); assert_eq!(f.current(), Stage::Build);
}
