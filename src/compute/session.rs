use super::{
    compute_dir,
    consent::{build_manifest, ConsentStore, UploadManifest},
    limits::CuMeter,
    CliJob, CliOutput, ColabCli, ComputeRequest,
};
use crate::base::{atomic_write, now_iso, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Assumed google-colab-cli command-line contract; not verified against a real client.
pub mod argv {
    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_owned()).collect()
    }
    /// Assumed: create returns JSON containing session_id.
    pub fn create(gpu: &str) -> Vec<String> {
        words(&[
            "google-colab-cli",
            "session",
            "create",
            "--accelerator",
            gpu,
            "--json",
        ])
    }
    /// Assumed: upload uses project-relative source and destination paths.
    pub fn upload(id: &str, rel: &str) -> Vec<String> {
        words(&[
            "google-colab-cli",
            "files",
            "upload",
            "--session",
            id,
            "--src",
            rel,
            "--dst",
            rel,
        ])
    }
    /// Assumed: exec forwards the job argv after -- and emits JSON.
    pub fn exec(id: &str, args: &[String]) -> Vec<String> {
        let mut v = words(&["google-colab-cli", "exec", "--session", id, "--json", "--"]);
        v.extend_from_slice(args);
        v
    }
    /// Assumed: usage returns JSON containing compute_units.
    pub fn usage(id: &str) -> Vec<String> {
        words(&[
            "google-colab-cli",
            "session",
            "usage",
            "--session",
            id,
            "--json",
        ])
    }
    /// Assumed: download uses project-relative source and destination paths.
    pub fn download(id: &str, rel: &str) -> Vec<String> {
        words(&[
            "google-colab-cli",
            "files",
            "download",
            "--session",
            id,
            "--src",
            rel,
            "--dst",
            rel,
        ])
    }
    /// Assumed: close exits zero when the session is closed.
    pub fn close(id: &str) -> Vec<String> {
        words(&["google-colab-cli", "session", "close", "--session", id])
    }
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("colab", e.to_string())
}
const TIMEOUT: Duration = Duration::from_secs(30);
static PENDING_LOCK: Mutex<()> = Mutex::new(());
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingClose {
    pub session_id: String,
    pub job_id: String,
    pub project: String,
    pub since: String,
}
fn read_pending(home: &Path) -> Res<Vec<PendingClose>> {
    match fs::read(compute_dir(home).join("pending-close.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(err),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(err(e)),
    }
}
fn write_pending(home: &Path, records: &[PendingClose]) -> Res<()> {
    let dir = compute_dir(home);
    fs::create_dir_all(&dir).map_err(err)?;
    atomic_write(
        &dir.join("pending-close.json"),
        &serde_json::to_vec(records).map_err(err)?,
    )
    .map_err(err)
}
pub fn pending(home: &Path) -> Res<Vec<PendingClose>> {
    let _lock = PENDING_LOCK.lock().unwrap();
    read_pending(home)
}
fn add_pending(home: &Path, record: PendingClose) -> Res<()> {
    let _lock = PENDING_LOCK.lock().unwrap();
    let mut records = read_pending(home)?;
    records.push(record);
    write_pending(home, &records)
}
fn remove_pending(home: &Path, id: &str) -> Res<()> {
    let _lock = PENDING_LOCK.lock().unwrap();
    let mut records = read_pending(home)?;
    records.retain(|r| r.session_id != id);
    write_pending(home, &records)
}
#[derive(Deserialize)]
struct Created {
    session_id: String,
}
pub struct ColabSession<'a> {
    cli: &'a dyn ColabCli,
    home: PathBuf,
    cwd: PathBuf,
    project: String,
    id: String,
    closed: bool,
}
impl<'a> ColabSession<'a> {
    pub fn allocate(cli: &'a dyn ColabCli, home: &Path, req: &ComputeRequest) -> Res<Self> {
        let output = cli.run(&argv::create(&req.gpu), &req.project_dir, TIMEOUT)?;
        // An unsuccessful create may nevertheless have allocated a session.
        let created = serde_json::from_str::<Created>(&output.stdout)
            .ok()
            .filter(|c| !c.session_id.is_empty());
        if let Some(created) = created {
            let mut session = Self {
                cli,
                home: home.to_owned(),
                cwd: req.project_dir.clone(),
                project: req.project.clone(),
                id: created.session_id,
                closed: false,
            };
            if let Err(e) = add_pending(
                home,
                PendingClose {
                    session_id: session.id.clone(),
                    job_id: req.job_id.clone(),
                    project: req.project.clone(),
                    since: now_iso(),
                },
            ) {
                let _ = session.close();
                return Err(e);
            }
            if output.code != 0 {
                return Err(err(format!("create failed: {}", output.stderr)));
            }
            Ok(session)
        } else {
            Err(err(format!(
                "create failed or omitted session id: {}",
                output.stderr
            )))
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn upload(&mut self, store: &ConsentStore, manifest: &UploadManifest) -> Res<u64> {
        if !store.is_granted(manifest)? || manifest.project != self.project {
            return Err(Failure::new("consent_missing", "upload consent is missing"));
        }
        let paths: Vec<String> = manifest
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        // Rebuild before any upload: this checks relative paths and symlink confinement,
        // and ensures that consent covers the supplied entries and their current contents.
        let current = build_manifest(&manifest.project, &self.cwd, &paths)?;
        if &current != manifest {
            return Err(Failure::new(
                "consent_missing",
                "upload manifest has changed",
            ));
        }
        let mut uploaded = 0;
        for entry in &manifest.entries {
            let output = self
                .cli
                .run(&argv::upload(&self.id, &entry.path), &self.cwd, TIMEOUT)?;
            if output.code != 0 {
                return Err(err(format!("upload failed: {}", output.stderr)));
            }
            uploaded += 1;
        }
        Ok(uploaded)
    }
    pub fn close(&mut self) -> bool {
        if self.closed {
            return true;
        }
        if !matches!(
            self.cli.run(&argv::close(&self.id), &self.cwd, TIMEOUT),
            Ok(CliOutput { code: 0, .. })
        ) {
            return false;
        }
        if remove_pending(&self.home, &self.id).is_err() {
            return false;
        }
        self.closed = true;
        true
    }
}
impl Drop for ColabSession<'_> {
    fn drop(&mut self) {
        if !self.closed {
            self.close();
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoverReport {
    pub closed: Vec<String>,
    pub still_pending: Vec<String>,
}
pub fn recover(home: &Path, cli: &dyn ColabCli) -> Res<RecoverReport> {
    let records = pending(home)?;
    let mut report = RecoverReport::default();
    for record in records {
        if matches!(
            cli.run(&argv::close(&record.session_id), home, TIMEOUT),
            Ok(CliOutput { code: 0, .. })
        ) {
            remove_pending(home, &record.session_id)?;
            report.closed.push(record.session_id);
        } else {
            report.still_pending.push(record.session_id);
        }
    }
    Ok(report)
}
pub struct ColabMeter<'a> {
    cli: &'a dyn ColabCli,
    id: String,
    cwd: PathBuf,
}
impl<'a> ColabMeter<'a> {
    pub fn new(cli: &'a dyn ColabCli, id: &str, cwd: &Path) -> Self {
        Self {
            cli,
            id: id.into(),
            cwd: cwd.into(),
        }
    }
}
impl CuMeter for ColabMeter<'_> {
    fn used(&self) -> Res<f64> {
        let output = self.cli.run(&argv::usage(&self.id), &self.cwd, TIMEOUT)?;
        if output.code != 0 {
            return Err(err(output.stderr));
        }
        #[derive(Deserialize)]
        struct Usage {
            compute_units: f64,
        }
        let value: Usage = serde_json::from_str(&output.stdout).map_err(err)?;
        if !value.compute_units.is_finite() || value.compute_units < 0.0 {
            return Err(err("invalid compute units"));
        }
        Ok(value.compute_units)
    }
}

#[derive(Default)]
struct FakeState {
    id: String,
    failures: HashSet<String>,
    calls: Vec<Vec<String>>,
    readings: VecDeque<f64>,
    last_cu: f64,
    duration: Duration,
}
#[derive(Clone)]
pub struct FakeColab {
    state: Arc<Mutex<FakeState>>,
}
impl Default for FakeColab {
    fn default() -> Self {
        Self::new("fake-session")
    }
}
impl FakeColab {
    pub fn new(id: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                id: id.into(),
                ..FakeState::default()
            })),
        }
    }
    pub fn fail(&self, step: &str, fail: bool) {
        let mut s = self.state.lock().unwrap();
        if fail {
            s.failures.insert(step.into());
        } else {
            s.failures.remove(step);
        }
    }
    pub fn fail_create_after_id(&self, fail: bool) {
        self.fail("create_after_id", fail);
    }
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.state.lock().unwrap().calls.clone()
    }
    pub fn uploads(&self) -> u64 {
        self.calls()
            .iter()
            .filter(|v| {
                v.get(1).map(String::as_str) == Some("files")
                    && v.get(2).map(String::as_str) == Some("upload")
            })
            .count() as u64
    }
    pub fn set_cu_readings(&self, values: Vec<f64>) {
        self.state.lock().unwrap().readings = values.into();
    }
    pub fn set_exec_duration(&self, duration: Duration) {
        self.state.lock().unwrap().duration = duration;
    }
}
fn step(args: &[String]) -> &str {
    if args.get(1).map(String::as_str) == Some("exec") {
        "exec"
    } else {
        args.get(2).map(String::as_str).unwrap_or("")
    }
}
fn output(code: i32, stdout: String) -> CliOutput {
    CliOutput {
        code,
        stdout,
        stderr: if code == 0 {
            String::new()
        } else {
            "injected failure".into()
        },
    }
}
impl ColabCli for FakeColab {
    fn run(&self, args: &[String], _cwd: &Path, _timeout: Duration) -> Res<CliOutput> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(args.to_vec());
        let name = step(args);
        if name == "create" && s.failures.contains("create_after_id") {
            return Ok(output(
                1,
                serde_json::json!({"session_id": s.id}).to_string(),
            ));
        }
        if s.failures.contains(name) {
            return Ok(output(1, String::new()));
        }
        let stdout = match name {
            "create" => serde_json::json!({"session_id": s.id}).to_string(),
            "usage" => {
                if let Some(v) = s.readings.pop_front() {
                    s.last_cu = v;
                }
                serde_json::json!({"compute_units": s.last_cu}).to_string()
            }
            _ => String::new(),
        };
        Ok(output(0, stdout))
    }
    fn start(&self, args: &[String], _cwd: &Path) -> Res<Box<dyn CliJob>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(args.to_vec());
        Ok(Box::new(FakeJob {
            started: Instant::now(),
            duration: s.duration,
            failed: s.failures.contains(step(args)),
            killed: false,
        }))
    }
}
struct FakeJob {
    started: Instant,
    duration: Duration,
    failed: bool,
    killed: bool,
}
impl CliJob for FakeJob {
    fn poll(&mut self) -> Res<Option<CliOutput>> {
        if self.killed {
            Ok(Some(output(1, String::new())))
        } else if self.started.elapsed() >= self.duration {
            Ok(Some(output(if self.failed { 1 } else { 0 }, String::new())))
        } else {
            Ok(None)
        }
    }
    fn kill(&mut self) {
        self.killed = true;
    }
}
