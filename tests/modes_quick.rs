mod common;
use axiom::{
    base::Res,
    modes::{
        self,
        launch::{LaunchKind, LaunchRequest, Launcher},
        quick::{Brief, QuickConfig, QuickLauncher, ReviewOutcome, Reviewer},
        receipts::{self, Cost, Part},
        Message, Mode, Snapshot,
    },
    orch,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

struct FakeReview {
    calls: Arc<AtomicUsize>,
    passed: bool,
}
impl Reviewer for FakeReview {
    fn review(&self, _: &Path, _: &Brief, _: &[String]) -> Res<ReviewOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ReviewOutcome {
            passed: self.passed,
            summary: "fake review".into(),
            part: Some(Part {
                label: "review".into(),
                model: "fake".into(),
                usage: None,
                cost: Cost::Usd(0.5),
                wall_ms: 50,
            }),
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    home: PathBuf,
    log: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let project = temp.path().join("project");
        let home = temp.path().join("home");
        let log = temp.path().join("codex.log");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/mod.py"), "old\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&project)
                .status()
                .unwrap()
                .success());
        }
        Self {
            _temp: temp,
            project,
            home,
            log,
        }
    }
    fn config(&self, calls: Arc<AtomicUsize>, passed: bool) -> QuickConfig {
        let wrapper = self._temp.path().join("fake.py");
        fs::write(&wrapper, "import os, sys\nos.environ['FAKE_CODEX_LOG'] = sys.argv[1]\np = sys.argv[2]\nsys.argv = [p] + sys.argv[3:]\nsource = open(p, encoding='utf-8').read()\nsource = source.replace(\"paths = ['src/mod.py']\", \"paths = ['src/mod.py']; (cwd / paths[0]).write_text('fake-written', encoding='utf-8')\")\nexec(compile(source, p, 'exec'), {'__name__': '__main__', '__file__': p})\n").unwrap();
        let tick = Arc::new(AtomicU64::new(1000));
        QuickConfig {
            home: self.home.clone(),
            project: self.project.clone(),
            codex: vec![
                common::python(),
                "-I".into(),
                wrapper.to_string_lossy().into_owned(),
                self.log.to_string_lossy().into_owned(),
                common::repo_root()
                    .join("tests/fakes/fake_codex.py")
                    .to_string_lossy()
                    .into_owned(),
            ],
            model: "gpt-6.1-terra".into(),
            slots_home: self._temp.path().join("slots"),
            timeout_s: 30,
            now_ms: Box::new(move || tick.fetch_add(100, Ordering::SeqCst)),
            reviewer: Some(Box::new(FakeReview { calls, passed })),
        }
    }
    fn methods(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
}
fn brief() -> Brief {
    Brief {
        goal: "Implement a change".into(),
        scope: vec!["src/mod.py".into()],
        verify: "exit 0".into(),
        review: false,
    }
}

#[test]
fn end_to_end_and_receipt() {
    let f = Fixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let cfg = f.config(calls.clone(), true);
    let snapshot = Snapshot {
        session: "session".into(),
        upto_seq: 1,
        thread: None,
        messages: vec![Message {
            seq: 1,
            ts: "now".into(),
            author: "user".into(),
            kind: "text".into(),
            text: "use this context".into(),
            thread: None,
            data: serde_json::Value::Null,
        }],
    };
    let out = modes::quick::run_quick(&cfg, None, None, &brief(), Some(&snapshot)).unwrap();
    assert!(out.passed);
    assert_eq!(out.verify_exit, Some(0));
    assert_eq!(out.run.outcome.as_deref(), Some("passed"));
    assert_eq!(out.run.mode, Mode::Quick);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(out.review.is_none());
    assert!(!out.store.join("modules.md").exists());
    let units = orch::read_units(&out.store).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "quick");
    assert_eq!(
        orch::evidence(&out.store, "quick")
            .unwrap()
            .iter()
            .filter(|r| r.kind == "run")
            .count(),
        1
    );
    let methods = f.methods();
    assert_eq!(methods.iter().filter(|m| *m == "turn/start").count(), 1);
    assert_eq!(out.receipt.mode, Mode::Quick);
    assert!(out.receipt.wall_ms > 0);
    assert!(matches!(out.receipt.cost,Cost::Usd(x) if x>0.0));
    assert_eq!(receipts::load(&f.home, &out.run.id).unwrap(), out.receipt);
    let text = fs::read_to_string(out.store.join("quick.brief")).unwrap();
    assert!(text.contains("use this context"));
}
#[test]
fn failure_rejection_and_review_gate() {
    for (verify, scope, review, review_pass, expected, review_calls) in [
        ("exit 1", "src/mod.py", true, true, "failed", 0),
        ("exit 0", "src/other.py", true, true, "rejected", 0),
        ("exit 0", "src/mod.py", true, true, "passed", 1),
        ("exit 0", "src/mod.py", true, false, "failed", 1),
    ] {
        let f = Fixture::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let cfg = f.config(calls.clone(), review_pass);
        let mut b = brief();
        b.verify = verify.into();
        b.scope = vec![scope.into()];
        b.review = review;
        let out = modes::quick::run_quick(&cfg, None, None, &b, None).unwrap();
        assert_eq!(out.run.outcome.as_deref(), Some(expected));
        assert_eq!(out.passed, expected == "passed");
        assert_eq!(calls.load(Ordering::SeqCst), review_calls);
        if expected == "rejected" {
            assert_eq!(out.outside_scope, vec!["src/mod.py"]);
            assert_eq!(
                fs::read_to_string(f.project.join("src/mod.py")).unwrap(),
                "fake-written"
            );
        }
        if review_calls == 1 {
            assert_eq!(out.receipt.parts.len(), 2);
            assert!(matches!(out.receipt.cost,Cost::Usd(x) if x>0.5));
        }
        assert_eq!(f.methods().iter().filter(|m| *m == "turn/start").count(), 1);
    }
}
#[test]
fn validation_happens_before_creation_and_text_roundtrips() {
    let f = Fixture::new();
    let cfg = f.config(Arc::new(AtomicUsize::new(0)), true);
    let b = brief();
    let p = orch::parse_brief(&b.to_text(Some("background"))).unwrap();
    assert_eq!(
        (p.goal, p.scope, p.verify, p.context),
        (
            b.goal.clone(),
            b.scope.clone(),
            b.verify.clone(),
            Some("background".into())
        )
    );
    let mut bad = Vec::new();
    let mut x = b.clone();
    x.goal = " ".into();
    bad.push(x);
    let mut x = b.clone();
    x.verify = "".into();
    bad.push(x);
    let mut x = b.clone();
    x.scope.clear();
    bad.push(x);
    for path in [
        "../bad",
        "a/../bad",
        "/absolute",
        "C:\\absolute",
        "a\\..\\bad",
        "",
    ] {
        let mut x = b.clone();
        x.scope = vec![path.into()];
        bad.push(x);
    }
    for x in bad {
        let e = modes::quick::run_quick(&cfg, None, None, &x, None).unwrap_err();
        assert_eq!(e.kind, "invalid_argument");
        assert!(!f.home.exists());
        assert!(!f.log.exists());
    }
}
#[test]
fn launcher_accepts_quick_and_rejects_wish() {
    let f = Fixture::new();
    let cfg = f.config(Arc::new(AtomicUsize::new(0)), true);
    let session = modes::runs::create_session(&f.home, Some(&f.project), &brief().scope).unwrap();
    let parent = modes::runs::start_run(&f.home, &session.id, Mode::Chat, None, None).unwrap();
    let mut req = LaunchRequest {
        kind: LaunchKind::Quick,
        session: session.id,
        parent_run: parent.id,
        goal: brief().goal,
        scope: brief().scope,
        verify: Some(brief().verify),
        review: false,
        context: None,
        sources: vec![],
    };
    let launcher = QuickLauncher { cfg };
    let out = launcher.launch(&req).unwrap();
    assert_eq!(out.run.mode, Mode::Quick);
    assert_eq!(out.run.outcome.as_deref(), Some("passed"));
    req.kind = LaunchKind::Wish;
    assert_eq!(launcher.launch(&req).unwrap_err().kind, "invalid_argument");
}
