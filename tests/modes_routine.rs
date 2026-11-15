use axiom::{
    base::{Failure, Res},
    modes::{
        launch::{self, LaunchKind, LaunchRequest, Launched, Launcher},
        routine::{
            self,
            triggers::{self, Event, EventKind},
        },
        runs, Mode,
    },
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
};

struct Fake {
    home: PathBuf,
    requests: RefCell<Vec<LaunchRequest>>,
    fail: Cell<bool>,
}
impl Fake {
    fn new(home: &Path) -> Self {
        Self {
            home: home.into(),
            requests: RefCell::new(vec![]),
            fail: Cell::new(false),
        }
    }
}
impl Launcher for Fake {
    fn launch(&self, req: &LaunchRequest) -> Res<Launched> {
        self.requests.borrow_mut().push(req.clone());
        if self.fail.get() {
            return Err(Failure::new("launch_error", "failed"));
        }
        Ok(Launched {
            run: launch::start_child(&self.home, req)?,
            detail: json!({}),
        })
    }
}
fn config(id: &str, trigger: Value, launch: &str) -> Value {
    json!({"id":id,"enabled":true,"trigger":trigger,"launch":launch,"goal":"fix it","scope":["src", "tests/file.rs"],"verify": if launch == "quick" { Some("cargo test") } else { None },"project":"project"})
}
fn save(home: &Path, configs: &[Value]) {
    fs::create_dir_all(home.join("modes")).unwrap();
    fs::write(
        home.join("modes/routines.json"),
        serde_json::to_vec(configs).unwrap(),
    )
    .unwrap();
}
fn event() -> Event {
    Event {
        kind: EventKind::Issue,
        id: "org/repo#1:opened".into(),
        repo: "org/repo".into(),
        title: "Issue".into(),
        url: None,
    }
}

#[test]
fn events_launch_children_and_deduplicate() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let fake = Fake::new(home);
    assert!(routine::fire_event(home, &event(), &fake)
        .unwrap()
        .is_empty());
    assert!(routine::tick(home, 0, &fake).unwrap().is_empty());
    let trigger = json!({"type":"event","kind":"issue","repo":"org/repo"});
    let mut disabled = config("disabled", trigger.clone(), "wish");
    disabled["enabled"] = json!(false);
    let mut default_disabled = config("default-disabled", trigger.clone(), "wish");
    default_disabled.as_object_mut().unwrap().remove("enabled");
    save(
        home,
        &[
            config("quick", trigger.clone(), "quick"),
            config("wish", trigger.clone(), "wish"),
            disabled,
            default_disabled,
            config(
                "other-kind",
                json!({"type":"event","kind":"pull_request"}),
                "wish",
            ),
            config(
                "other-repo",
                json!({"type":"event","kind":"issue","repo":"other"}),
                "wish",
            ),
        ],
    );
    let fired = routine::fire_event(home, &event(), &fake).unwrap();
    assert_eq!(fired.len(), 2);
    let requests = fake.requests.borrow();
    for (item, req) in fired.iter().zip(requests.iter()) {
        assert_eq!(item.run.mode, Mode::Routine);
        assert_eq!(item.run.reference.as_deref(), Some(item.routine.as_str()));
        assert_eq!(item.child.run.parent.as_deref(), Some(item.run.id.as_str()));
        assert_eq!(item.child.run.session, item.run.session);
        assert_eq!(req.scope, vec!["src", "tests/file.rs"]);
        assert_eq!(req.goal, "fix it");
        assert!(!req.review);
        assert!(req.context.is_none());
        assert!(req.sources.is_empty());
        let session = runs::load_session(home, &item.run.session).unwrap();
        assert_eq!(session.project, Some(PathBuf::from("project")));
        assert_eq!(session.scope, req.scope);
    }
    assert_eq!(requests[0].kind, LaunchKind::Quick);
    assert_eq!(requests[0].verify.as_deref(), Some("cargo test"));
    assert_eq!(requests[1].kind, LaunchKind::Wish);
    assert_eq!(requests[1].verify, None);
    drop(requests);
    assert!(routine::fire_event(home, &event(), &fake)
        .unwrap()
        .is_empty());
    assert_eq!(fake.requests.borrow().len(), 2);
}

#[test]
fn validation() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let good = config("id", json!({"type":"cron","expr":"* * * * *"}), "quick");
    let mut bads = vec![];
    for scope in [
        json!([]),
        json!([""]),
        json!([".."]),
        json!(["src/../secret"]),
        json!(["/absolute"]),
        json!(["C:\\absolute"]),
    ] {
        let mut bad = good.clone();
        bad["scope"] = scope;
        bads.push(bad);
    }
    let mut bad = good.clone();
    bad["verify"] = Value::Null;
    bads.push(bad);
    let mut bad = good.clone();
    bad["id"] = json!("../bad");
    bads.push(bad);
    let mut bad = good.clone();
    bad["trigger"]["expr"] = json!("60 * * * *");
    bads.push(bad);
    for bad in bads {
        save(home, &[bad]);
        assert_eq!(routine::load(home).unwrap_err().kind, "invalid_argument");
    }
}

#[test]
fn cron_schedules_and_tick_deduplication() {
    let monday = 1_704_067_200i64; // 2024-01-01 00:00 UTC
    let cron = triggers::parse_cron("*/15 * * * *").unwrap();
    assert!(cron.due(monday));
    assert!(!cron.due(monday + 60));
    assert!(cron.due(monday + 900));
    let cron = triggers::parse_cron("30 2 * * 1").unwrap();
    assert!(cron.due(monday + 9000));
    assert!(!cron.due(monday + 86_400 + 9000));
    let cron = triggers::parse_cron("0,30 2-4 2 * 1").unwrap();
    assert!(cron.due(monday + 9000));
    assert!(cron.due(monday + 86_400 + 4 * 3600));
    assert!(!cron.due(monday + 2 * 86_400 + 9000));
    assert!(triggers::parse_cron("59 23 * * 3").unwrap().due(-1));
    for expr in [
        "* * * *",
        "*/0 * * * *",
        "60 * * * *",
        "* 24 * * *",
        "* * 0 * *",
        "* * * 13 *",
        "* * * * 7",
        "1-0 * * * *",
        "1,,2 * * * *",
    ] {
        assert_eq!(
            triggers::parse_cron(expr).unwrap_err().kind,
            "invalid_argument",
            "{expr}"
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let fake = Fake::new(home);
    save(
        home,
        &[config(
            "cron",
            json!({"type":"cron","expr":"*/15 * * * *"}),
            "wish",
        )],
    );
    assert!(routine::tick(home, monday + 60, &fake).unwrap().is_empty());
    assert_eq!(routine::tick(home, monday, &fake).unwrap().len(), 1);
    assert!(routine::tick(home, monday + 59, &fake).unwrap().is_empty());
    assert_eq!(routine::tick(home, monday + 900, &fake).unwrap().len(), 1);
}

#[test]
fn failed_launch_is_finished_and_retryable() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let fake = Fake::new(home);
    save(
        home,
        &[
            config("event", json!({"type":"event","kind":"issue"}), "wish"),
            config("cron", json!({"type":"cron","expr":"* * * * *"}), "wish"),
        ],
    );
    fake.fail.set(true);
    assert_eq!(
        routine::fire_event(home, &event(), &fake).unwrap_err().kind,
        "launch_error"
    );
    assert_eq!(
        routine::tick(home, 0, &fake).unwrap_err().kind,
        "launch_error"
    );
    for req in fake.requests.borrow().iter() {
        let run = runs::load_run(home, &req.parent_run).unwrap();
        assert!(run.finished.is_some());
        assert!(run.outcome.unwrap().contains("failed"));
    }
    fake.fail.set(false);
    assert_eq!(routine::fire_event(home, &event(), &fake).unwrap().len(), 1);
    assert_eq!(routine::tick(home, 0, &fake).unwrap().len(), 1);
}

#[test]
fn github_mapping() {
    let issue = json!({"repository":{"full_name":"org/repo"},"action":"opened","number":7,"issue":{"title":"Problem","html_url":"https://example/7"}});
    let e = triggers::from_github("issues", &issue).unwrap();
    assert_eq!(e.kind, EventKind::Issue);
    assert_eq!(e.id, "org/repo#7:opened");
    assert_eq!(e.title, "Problem");
    assert_eq!(e.url.as_deref(), Some("https://example/7"));
    assert_eq!(e, triggers::from_github("issues", &issue).unwrap());
    let pr = json!({"repository":{"full_name":"org/repo"},"action":"closed","number":8,"pull_request":{"title":"PR"}});
    let e = triggers::from_github("pull_request", &pr).unwrap();
    assert_eq!(e.kind, EventKind::PullRequest);
    assert_eq!(e.id, "org/repo#8:closed");
    for name in ["workflow_run", "check_run"] {
        let payload = json!({"repository":{"full_name":"org/repo"}, name: {"id":42,"name":"Build","conclusion":"failure","html_url":"https://example/run"}});
        let e = triggers::from_github(name, &payload).unwrap();
        assert_eq!(e.kind, EventKind::CiFailure);
        assert_eq!(e.id, "org/repo:run:42");
        assert_eq!(e.title, "Build");
        let mut success = payload.clone();
        success[name]["conclusion"] = json!("success");
        assert_eq!(
            triggers::from_github(name, &success).unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert_eq!(
        triggers::from_github("push", &issue).unwrap_err().kind,
        "invalid_argument"
    );
}
