use crate::base::{self, proc, Failure, Res};
use crate::orch::scratch_dir;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const MANIFEST: &str = ".axiom/setup.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub timeout_s: u64,
    pub plan_independent: bool,
    pub cache_dirs: Vec<String>,
    pub builtin: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StepStatus {
    pub id: String,
    pub state: String,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub exit_code: Option<i32>,
    pub scratch: PathBuf,
    pub reason: Option<String>,
}

fn failure(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> Failure {
    Failure::new("invalid_argument", e.to_string())
}

pub fn builtin_steps(repo: &Path) -> Vec<Step> {
    let make = |id: &str, argv: &[&str]| Step {
        id: id.into(),
        argv: argv.iter().map(|x| (*x).into()).collect(),
        cwd: ".".into(),
        timeout_s: 600,
        plan_independent: true,
        cache_dirs: vec![],
        builtin: true,
    };
    let mut steps = vec![make("index", &["git", "ls-files", "-z"])];
    if repo.join("Cargo.toml").exists() {
        steps.push(make("cargo-fetch", &["cargo", "fetch", "--locked"]));
    }
    steps
}

#[derive(Deserialize)]
struct Manifest {
    version: u64,
    steps: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct RawStep {
    id: String,
    argv: Vec<String>,
    #[serde(default = "default_cwd")]
    cwd: String,
    #[serde(default = "default_timeout")]
    timeout_s: u64,
    #[serde(default)]
    plan_independent: bool,
    #[serde(default)]
    cache_dirs: Vec<String>,
}
fn default_cwd() -> String {
    ".".into()
}
fn default_timeout() -> u64 {
    600
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn resolved_path(path: &Path) -> Res<PathBuf> {
    let path = std::path::absolute(path).map_err(failure)?;
    let mut ancestor = path.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| invalid("invalid store path"))?;
        suffix.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| invalid("invalid store path"))?;
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(failure)?;
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

pub fn load_steps(repo: &Path) -> Res<(Vec<Step>, Vec<String>)> {
    let mut steps = builtin_steps(repo);
    let bytes = match fs::read(repo.join(MANIFEST)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((steps, vec![])),
        Err(e) => return Err(invalid(e)),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(invalid)?;
    if manifest.version != 1 {
        return Err(invalid("unsupported setup manifest version"));
    }
    let root = fs::canonicalize(repo).map_err(invalid)?;
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    for value in manifest.steps {
        let raw: RawStep = match serde_json::from_value(value) {
            Ok(raw) => raw,
            Err(e) => {
                warnings.push(format!("invalid step: {e}"));
                continue;
            }
        };
        let problem = if base::require_slug("id", &raw.id).is_err() {
            Some("invalid id")
        } else if !seen.insert(raw.id.clone()) {
            Some("duplicate id")
        } else if raw.argv.is_empty() || raw.argv[0].is_empty() {
            Some("empty argv")
        } else if !relative(&raw.cwd)
            || !fs::canonicalize(repo.join(&raw.cwd))
                .is_ok_and(|p| p.is_dir() && p.starts_with(&root))
        {
            Some("invalid cwd")
        } else if !(1..=7200).contains(&raw.timeout_s) {
            Some("invalid timeout")
        } else if raw.cache_dirs.iter().any(|p| !relative(p)) {
            Some("invalid cache dir")
        } else {
            None
        };
        if let Some(problem) = problem {
            warnings.push(format!("{}: {problem}", raw.id));
            continue;
        }
        let step = Step {
            id: raw.id,
            argv: raw.argv,
            cwd: raw.cwd,
            timeout_s: raw.timeout_s,
            plan_independent: raw.plan_independent,
            cache_dirs: raw.cache_dirs,
            builtin: false,
        };
        if let Some(pos) = steps.iter().position(|s| s.id == step.id) {
            steps[pos] = step;
        } else {
            steps.push(step);
        }
    }
    Ok((steps, warnings))
}

fn state_path(store: &Path, id: &str) -> PathBuf {
    store.join("setup").join(format!("{id}.json"))
}
fn marker(store: &Path, id: &str) -> PathBuf {
    store.join("setup").join(format!("{id}.ok"))
}
fn read_state(store: &Path, id: &str) -> Res<Option<StepStatus>> {
    match fs::read(state_path(store, id)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(failure),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(e)),
    }
}
fn write_state(store: &Path, state: &StepStatus) -> Res<()> {
    base::atomic_write(
        &state_path(store, &state.id),
        &serde_json::to_vec(state).map_err(failure)?,
    )
    .map_err(failure)
}
fn remove_file(path: &Path) -> Res<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(failure(e)),
    }
}
fn remove_scratch(path: &Path) -> Res<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e)
                if cfg!(windows)
                    && e.kind() == io::ErrorKind::PermissionDenied
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(failure(e)),
        }
    }
}
// A per-step filesystem lock serializes start, completion and cancellation across processes.
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
fn lock(store: &Path, id: &str) -> Res<Lock> {
    let path = store.join("setup").join(format!(".{id}.lock"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match fs::create_dir(&path) {
            Ok(()) => return Ok(Lock(path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(failure(e)),
        }
    }
}

pub fn start(store: &Path, repo: &Path) -> Res<Vec<StepStatus>> {
    let (steps, _) = load_steps(repo)?;
    let repo = fs::canonicalize(repo).map_err(failure)?;
    let store = resolved_path(store)?;
    if store.starts_with(&repo) {
        return Err(invalid("setup store must be outside the repository"));
    }
    fs::create_dir_all(store.join("setup")).map_err(failure)?;
    let mut result = Vec::new();
    for step in steps {
        let _guard = lock(&store, &step.id)?;
        if let Some(state) = read_state(&store, &step.id)? {
            result.push(state);
            continue;
        }
        let scratch = scratch_dir(&store, &["setup", &step.id]);
        let mut state = StepStatus {
            id: step.id.clone(),
            state: "skipped".into(),
            pid: None,
            started_at: None,
            ended_at: None,
            exit_code: None,
            scratch: scratch.clone(),
            reason: Some("not plan_independent".into()),
        };
        if !step.plan_independent {
            write_state(&store, &state)?;
            result.push(state);
            continue;
        }
        let env = proc::scratch_env(&scratch).map_err(failure)?;
        let mut command = proc::cmd(&step.argv[0]);
        command
            .args(&step.argv[1..])
            .current_dir(repo.join(&step.cwd))
            .envs(env)
            .env("AXIOM_SETUP_OUT", &scratch)
            .stdin(Stdio::null());
        let index = step.builtin && step.id == "index";
        let index_file = if index {
            Some(fs::File::create(scratch.join("files.txt")).map_err(failure)?)
        } else {
            None
        };
        if index {
            command.stdout(Stdio::piped()).stderr(Stdio::null());
        } else {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        let mut child = match proc::spawn_group(&mut command) {
            Ok(child) => child,
            Err(e) => {
                state.state = "failed".into();
                state.started_at = Some(base::now_iso());
                state.ended_at = Some(base::now_iso());
                state.reason = Some(e.to_string());
                write_state(&store, &state)?;
                result.push(state);
                continue;
            }
        };
        state.state = "running".into();
        state.pid = Some(child.id());
        state.started_at = Some(base::now_iso());
        state.reason = None;
        if let Err(e) = write_state(&store, &state) {
            if proc::terminate_tree(&mut child).is_ok() || child.kill().is_ok() {
                let _ = child.wait();
            }
            return Err(e);
        }
        let worker_store = store.clone();
        let worker_state = state.clone();
        thread::spawn(move || {
            supervise(
                worker_store,
                worker_state,
                child,
                step.timeout_s,
                index_file,
            )
        });
        result.push(state);
    }
    Ok(result)
}

fn supervise(
    store: PathBuf,
    state: StepStatus,
    mut child: Child,
    timeout: u64,
    index_file: Option<fs::File>,
) {
    let drain = if let Some(mut file) = index_file {
        let mut out = child.stdout.take().unwrap();
        Some(thread::spawn(move || -> io::Result<()> {
            io::copy(&mut out, &mut file)?;
            Ok(())
        }))
    } else {
        None
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let (exit, timed_out, error, can_drain) = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break (Some(exit), false, None, true),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let error = proc::terminate_tree(&mut child)
                    .err()
                    .map(|e| e.to_string());
                if error.is_some() && child.kill().is_err() {
                    break (None, true, error, false);
                }
                break (child.wait().ok(), true, error, true);
            }
            Err(e) => {
                let killed = proc::terminate_tree(&mut child).is_ok() || child.kill().is_ok();
                if killed {
                    let _ = child.wait();
                }
                break (None, false, Some(e.to_string()), killed);
            }
        }
    };
    let output_error = drain
        .filter(|_| can_drain)
        .and_then(|d| d.join().ok())
        .and_then(Result::err)
        .map(|e| e.to_string());
    if let Ok(_guard) = lock(&store, &state.id) {
        if let Ok(Some(mut current)) = read_state(&store, &state.id) {
            if current.state != "running" || current.pid != state.pid {
                return;
            }
            current.ended_at = Some(base::now_iso());
            current.exit_code = exit.and_then(|s| s.code());
            current.reason = error.or(output_error);
            current.state = if current.reason.is_some() {
                "failed"
            } else if timed_out {
                "timeout"
            } else if exit.is_some_and(|s| s.success()) {
                "succeeded"
            } else {
                "failed"
            }
            .into();
            if current.state == "succeeded" {
                if let Err(e) = base::atomic_write(&marker(&store, &state.id), b"") {
                    current.state = "failed".into();
                    current.reason = Some(e.to_string());
                }
            } else {
                let _ = remove_file(&marker(&store, &state.id));
            }
            let _ = write_state(&store, &current);
        }
    }
}

pub fn status(store: &Path) -> Res<Vec<StepStatus>> {
    let entries = match fs::read_dir(store.join("setup")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(failure(e)),
    };
    let mut states = Vec::new();
    for entry in entries {
        let path = entry.map_err(failure)?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let mut state: StepStatus =
                serde_json::from_slice(&fs::read(path).map_err(failure)?).map_err(failure)?;
            if state.state == "running" && state.pid.is_some_and(|pid| !proc::pid_alive(pid)) {
                state.state = "failed".into();
                state.reason = Some("supervisor lost".into());
            }
            states.push(state);
        }
    }
    states.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(states)
}

pub fn cancel(store: &Path, ids: Option<&[String]>, reason: &str) -> Res<Vec<StepStatus>> {
    let all = status(store)?;
    for item in all {
        if ids.is_some_and(|ids| !ids.contains(&item.id)) {
            continue;
        }
        let _guard = lock(store, &item.id)?;
        let Some(mut state) = read_state(store, &item.id)? else {
            continue;
        };
        if state.state != "running" {
            continue;
        }
        // Publish cancellation before the process is killed so its supervisor cannot record success.
        state.state = "cancelled".into();
        state.reason = Some(reason.into());
        state.ended_at = Some(base::now_iso());
        write_state(store, &state)?;
        let live_pid = state.pid.filter(|&pid| proc::pid_alive(pid));
        if let Some(pid) = live_pid {
            if let Err(error) = proc::kill_pid_tree(pid) {
                state.state = "running".into();
                state.reason = None;
                state.ended_at = None;
                write_state(store, &state)?;
                return Err(error);
            }
        }
        drop(_guard);
        if let Some(pid) = live_pid {
            if !proc::wait_pid_exit(pid, Duration::from_secs(10)) {
                return Err(Failure::new(
                    "kill_failed",
                    format!("pid {pid} did not exit"),
                ));
            }
        }
        remove_file(&marker(store, &state.id))?;
        remove_scratch(&state.scratch)?;
    }
    status(store)
}
pub fn keep(store: &Path, ids: &[String]) -> Res<Vec<StepStatus>> {
    let others: Vec<String> = status(store)?
        .into_iter()
        .filter(|s| !ids.contains(&s.id))
        .map(|s| s.id)
        .collect();
    cancel(store, Some(&others), "left out by plan")
}
pub fn cancel_all(store: &Path, reason: &str) -> Res<()> {
    cancel(store, None, reason).map(|_| ())
}
