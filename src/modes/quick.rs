//! A single-unit quick task.
use super::{
    context,
    launch::{LaunchKind, LaunchRequest, Launched, Launcher},
    receipts, runs, Mode, Run, Snapshot,
};
use crate::{
    base::{self, Failure, Res},
    codex::{self, AppServer, RunRequest},
    orch,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex},
    thread::ThreadId,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brief {
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: String,
    pub review: bool,
}
impl Brief {
    pub fn validate(&self) -> Res<()> {
        let invalid = || {
            Failure::new(
                "invalid_argument",
                "goal, verify and representable relative scope are required",
            )
        };
        if self.goal.trim().is_empty()
            || self.verify.trim().is_empty()
            || self.scope.is_empty()
            || self
                .scope
                .iter()
                .any(|s| s.trim().is_empty() || !runs::relative(s))
        {
            return Err(invalid());
        }
        let parsed = orch::parse_brief(&self.to_text(None)).map_err(|_| invalid())?;
        if parsed.goal != self.goal || parsed.scope != self.scope || parsed.verify != self.verify {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn to_text(&self, context: Option<&str>) -> String {
        fn field(name: &str, value: &str) -> String {
            let mut lines = value.split('\n');
            let mut out = format!("{name} {}\n", lines.next().unwrap_or(""));
            for line in lines {
                out.push_str(&format!(" {line}\n"));
            }
            out
        }
        let mut text = field("GOAL", &self.goal);
        text.push_str(&format!("SCOPE {}\n", self.scope.join(" ")));
        text.push_str(&field("VERIFY", &self.verify));
        if let Some(c) = context {
            text.push_str(&field("CONTEXT", c));
        }
        text
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewOutcome {
    pub passed: bool,
    pub summary: String,
    pub part: Option<receipts::Part>,
}
pub trait Reviewer {
    fn prepare(&self, _project: &Path) -> Res<()> {
        Ok(())
    }
    fn review(&self, project: &Path, brief: &Brief, changed: &[String]) -> Res<ReviewOutcome>;
}
/// Records the pre-unit HEAD for the review's git diff, including subsequent tracked edits.
#[derive(Default)]
pub struct CodeReviewer {
    bases: Mutex<HashMap<ThreadId, String>>,
}
impl Reviewer for CodeReviewer {
    fn prepare(&self, project: &Path) -> Res<()> {
        let output = base::proc::cmd("git")
            .current_dir(project)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|e| Failure::new("io", e.to_string()))?;
        if !output.status.success() {
            return Err(Failure::new(
                "invalid_argument",
                "review requires a repository with HEAD",
            ));
        }
        self.bases.lock().unwrap().insert(
            std::thread::current().id(),
            String::from_utf8_lossy(&output.stdout).trim().into(),
        );
        Ok(())
    }
    fn review(&self, project: &Path, _brief: &Brief, changed: &[String]) -> Res<ReviewOutcome> {
        let base = self
            .bases
            .lock()
            .unwrap()
            .remove(&std::thread::current().id())
            .ok_or_else(|| Failure::new("invalid_argument", "review base missing"))?;
        // git diff cannot include untracked files: never claim to have reviewed them.
        for path in changed {
            let tracked = base::proc::cmd("git")
                .current_dir(project)
                .args(["ls-files", "--error-unmatch", "--", path])
                .output()
                .map_err(|e| Failure::new("io", e.to_string()))?;
            if !tracked.status.success() {
                return Ok(ReviewOutcome {
                    passed: false,
                    summary: format!("untracked file cannot be reviewed: {path}"),
                    part: None,
                });
            }
        }
        let result = crate::review::run(
            &crate::review::Config::from_env(),
            project,
            &base,
            &AtomicBool::new(false),
        )?;
        let passed = matches!(&result, crate::review::Outcome::Reviewed(r) if r.pass());
        Ok(ReviewOutcome {
            passed,
            summary: result.render(),
            part: None,
        })
    }
}

pub struct QuickConfig {
    pub home: PathBuf,
    pub project: PathBuf,
    pub codex: Vec<String>,
    pub model: String,
    pub slots_home: PathBuf,
    pub timeout_s: u64,
    pub now_ms: Box<dyn Fn() -> u64 + Send + Sync>,
    pub reviewer: Option<Box<dyn Reviewer + Send + Sync>>,
}
impl QuickConfig {
    pub fn from_env(project: &Path) -> Res<Self> {
        let codex = base::resolve_command("AXIOM_CODEX_CMD", "codex")?;
        let slots_home = codex::slots::home();
        let server = AppServer::new(codex.clone()).with_slots_home(slots_home.clone());
        let model = server.resolve_model("terra")?;
        server.close();
        Ok(Self {
            home: base::axiom_home(),
            project: project.to_path_buf(),
            codex,
            model,
            slots_home,
            timeout_s: 1800,
            now_ms: Box::new(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(u64::MAX as u128) as u64
            }),
            reviewer: Some(Box::new(CodeReviewer::default())),
        })
    }
}
#[derive(Debug)]
pub struct QuickOutcome {
    pub run: Run,
    pub store: PathBuf,
    pub state: String,
    pub passed: bool,
    pub verify_exit: Option<i32>,
    pub outside_scope: Vec<String>,
    pub changed: Vec<String>,
    pub review: Option<ReviewOutcome>,
    pub receipt: receipts::Receipt,
}
fn orch_command(store: &Path, args: &[String]) -> Res<()> {
    let out = orch::run(store, args, false, false);
    if out.exit_code == 0 {
        Ok(())
    } else {
        Err(Failure::new("orch", out.stderr))
    }
}
pub fn run_quick(
    cfg: &QuickConfig,
    session: Option<&str>,
    parent: Option<&str>,
    brief: &Brief,
    snapshot: Option<&Snapshot>,
) -> Res<QuickOutcome> {
    brief.validate()?;
    let started_ms = (cfg.now_ms)();
    let session_id = match session {
        Some(id) => id.to_owned(),
        None => runs::create_session(&cfg.home, Some(&cfg.project), &brief.scope)?.id,
    };
    let run = runs::start_run(&cfg.home, &session_id, Mode::Quick, parent, None)?;
    let store = cfg.home.join("modes").join("quick").join(&run.id);
    let context_text =
        snapshot.map(|s| context::bounded(&s.messages, context::DEFAULT_CONTEXT_BYTES));
    let text = brief.to_text(context_text.as_deref());
    let execute = || -> Res<QuickOutcome> {
        orch_command(&store, &["init".into()])?;
        let brief_path = store.join("quick.brief");
        std::fs::write(&brief_path, &text).map_err(|e| Failure::new("io", e.to_string()))?;
        orch_command(
            &store,
            &[
                "unit".into(),
                "add".into(),
                "quick".into(),
                "--track".into(),
                "quick".into(),
                "--brief".into(),
                brief_path.to_string_lossy().into_owned(),
            ],
        )?;
        let prepare_error = if brief.review {
            cfg.reviewer
                .as_ref()
                .and_then(|r| r.prepare(&cfg.project).err())
        } else {
            None
        };
        let attempt = match orch::begin_unit(&store, "quick", "terra", &cfg.project, false)? {
            orch::Begin::Run { attempt_id, .. } => attempt_id,
            orch::Begin::Reused => return Err(Failure::new("orch", "unexpected reused unit")),
        };
        let server = AppServer::new(cfg.codex.clone()).with_slots_home(cfg.slots_home.clone());
        let unit_start = (cfg.now_ms)();
        let request = RunRequest {
            brief: text,
            cwd: cfg.project.to_string_lossy().into_owned(),
            model: cfg.model.clone(),
            sandbox: "workspace-write".into(),
            effort: None,
            output_schema: None,
            developer_instructions: None,
            timeout_s: cfg.timeout_s,
            lean: true,
            keep_plugins: vec![],
            web_search: false,
        };
        let report = match server.run(&request) {
            Ok(r) => r,
            Err(e) => serde_json::json!({"status":"failed", "error":e.to_json()}),
        };
        server.close();
        let unit_end = (cfg.now_ms)();
        let finished = orch::finish_unit(
            &store,
            "quick",
            &attempt,
            &cfg.project,
            "terra",
            &cfg.model,
            &report,
        )?;
        let mut parts = vec![receipts::codex_part(
            "unit",
            &cfg.model,
            &report["usage"],
            unit_end.saturating_sub(unit_start),
        )];
        let review =
            if brief.review && finished.verify_exit == Some(0) && finished.outside_scope.is_empty()
            {
                let review_start = (cfg.now_ms)();
                let result = if let Some(e) = prepare_error {
                    Err(e)
                } else if let Some(r) = &cfg.reviewer {
                    r.review(&cfg.project, brief, &finished.changed)
                } else {
                    Err(Failure::new("invalid_argument", "reviewer unavailable"))
                };
                let review_end = (cfg.now_ms)();
                let result = result.unwrap_or_else(|e| ReviewOutcome {
                    passed: false,
                    summary: e.to_string(),
                    part: None,
                });
                parts.push(result.part.clone().unwrap_or(receipts::Part {
                    label: "review".into(),
                    model: String::new(),
                    usage: None,
                    cost: receipts::Cost::Unknown,
                    wall_ms: review_end.saturating_sub(review_start),
                }));
                Some(result)
            } else {
                None
            };
        let passed = finished.verify_exit == Some(0)
            && finished.outside_scope.is_empty()
            && (!brief.review || review.as_ref().is_some_and(|r| r.passed));
        let outcome = if !finished.outside_scope.is_empty() {
            "rejected"
        } else if passed {
            "passed"
        } else {
            "failed"
        };
        let run = runs::finish_run(&cfg.home, &run.id, outcome)?;
        let receipt = receipts::Receipt::new(
            &run.id,
            &run.session,
            Mode::Quick,
            outcome,
            &run.started,
            run.finished.as_deref().unwrap_or(""),
            (cfg.now_ms)().saturating_sub(started_ms),
            parts,
        );
        receipts::write(&cfg.home, &receipt)?;
        Ok(QuickOutcome {
            run,
            store: store.clone(),
            state: finished.state,
            passed,
            verify_exit: finished.verify_exit,
            outside_scope: finished.outside_scope,
            changed: finished.changed,
            review,
            receipt,
        })
    };
    let result = execute();
    if result.is_err() {
        let _ = runs::finish_run(&cfg.home, &run.id, "failed");
    }
    result
}
pub struct QuickLauncher {
    pub cfg: QuickConfig,
}
impl Launcher for QuickLauncher {
    fn launch(&self, req: &LaunchRequest) -> Res<Launched> {
        if req.kind != LaunchKind::Quick {
            return Err(Failure::new(
                "invalid_argument",
                "QuickLauncher only supports quick",
            ));
        }
        let brief = Brief {
            goal: req.goal.clone(),
            scope: req.scope.clone(),
            verify: req.verify.clone().unwrap_or_default(),
            review: req.review,
        };
        let outcome = run_quick(
            &self.cfg,
            Some(&req.session),
            Some(&req.parent_run),
            &brief,
            req.context.as_ref(),
        )?;
        Ok(Launched {
            run: outcome.run,
            detail: serde_json::json!({"store":outcome.store,"state":outcome.state,"passed":outcome.passed,"verify_exit":outcome.verify_exit,"outside_scope":outcome.outside_scope,"changed":outcome.changed,"receipt":outcome.receipt}),
        })
    }
}
