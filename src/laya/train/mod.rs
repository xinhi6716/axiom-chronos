use crate::{
    base::{proc, Failure, Res},
    laya::ort,
    usage::laya::UsageEvent,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Local,
    Colab,
}
pub struct Caps {
    pub wall_clock_s: u64,
    pub compute_units: f64,
}
pub struct TrainSpec {
    pub dataset_dir: PathBuf,
    pub out_dir: PathBuf,
    pub upstream: PathBuf,
    pub base: String,
    pub revision: String,
    pub epochs: u32,
    pub seed: u64,
    pub backend: Backend,
    pub caps: Caps,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrainOutput {
    pub checkpoint: PathBuf,
    pub onnx_dir: PathBuf,
    pub tokenizer: PathBuf,
    pub calibration: PathBuf,
    pub usage: UsageEvent,
}
pub trait Trainer {
    fn train(&self, spec: &TrainSpec, cancel: &AtomicBool) -> Res<TrainOutput>;
}
pub struct PythonTrainer {
    pub python: Vec<String>,
    pub script: PathBuf,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BundleManifest {
    pub files: BTreeMap<String, String>,
    pub base: String,
    pub revision: String,
    pub caps_wall_clock_s: u64,
    pub caps_compute_units: f64,
}

fn failed(e: impl std::fmt::Display) -> Failure {
    Failure::new("laya_train_failed", e.to_string())
}
fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
pub fn local_command(spec: &TrainSpec, python: &[String], script: &Path) -> Vec<String> {
    let mut argv = python.to_vec();
    argv.extend([
        path(script),
        "--dataset".into(),
        path(&spec.dataset_dir),
        "--out".into(),
        path(&spec.out_dir),
        "--upstream".into(),
        path(&spec.upstream),
        "--base".into(),
        spec.base.clone(),
        "--revision".into(),
        spec.revision.clone(),
        "--epochs".into(),
        spec.epochs.to_string(),
        "--seed".into(),
        spec.seed.to_string(),
        "--device".into(),
        "cuda".into(),
    ]);
    argv
}

pub fn colab_bundle(spec: &TrainSpec, out: &Path) -> Res<BundleManifest> {
    if !spec.dataset_dir.join("datasheet.json").is_file() {
        return Err(Failure::new(
            "laya_bundle_no_datasheet",
            "dataset has no datasheet.json",
        ));
    }
    fs::create_dir_all(out).map_err(failed)?;
    if fs::read_dir(out).map_err(failed)?.next().is_some() {
        return Err(failed("bundle directory must be empty"));
    }
    let mut files = BTreeMap::new();
    for name in [
        "train.jsonl",
        "calibration.jsonl",
        "dev.jsonl",
        "acceptance.jsonl",
        "datasheet.json",
    ] {
        let source = spec.dataset_dir.join(name);
        if source.is_file() {
            fs::copy(source, out.join(name)).map_err(failed)?;
            files.insert(name.into(), ort::sha256_file(&out.join(name))?);
        }
    }
    for (name, contents) in [
        ("run.py", include_str!("../../../tools/laya/train/run.py")),
        (
            "run_colab.sh",
            include_str!("../../../tools/laya/train/run_colab.sh"),
        ),
    ] {
        fs::write(out.join(name), contents).map_err(failed)?;
        files.insert(name.into(), ort::sha256_file(&out.join(name))?);
    }
    let manifest = BundleManifest {
        files,
        base: spec.base.clone(),
        revision: spec.revision.clone(),
        caps_wall_clock_s: spec.caps.wall_clock_s,
        caps_compute_units: spec.caps.compute_units,
    };
    fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(failed)?,
    )
    .map_err(failed)?;
    Ok(manifest)
}

fn wait(
    child: &mut Child,
    cancel: &AtomicBool,
    start: Instant,
    cap: u64,
) -> Res<std::process::ExitStatus> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            proc::kill_tree(child);
            return Err(Failure::new("laya_train_cancelled", "training cancelled"));
        }
        if start.elapsed() > Duration::from_secs(cap) {
            proc::kill_tree(child);
            return Err(Failure::new("laya_train_cap", "wall-clock cap exceeded"));
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                proc::kill_tree(child);
                return Err(failed(e));
            }
        }
    }
}

// Drain both pipes while waiting so a verbose CLI cannot block the child.
fn colab_call(
    argv: &[String],
    args: &[&str],
    cancel: &AtomicBool,
    start: Instant,
    cap: u64,
) -> Res<String> {
    let mut cmd = proc::command(argv);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = proc::spawn_group(&mut cmd).map_err(failed)?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = wait(&mut child, cancel, start, cap);
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if !status?.success() {
        return Err(failed(format!(
            "colab {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&err)
        )));
    }
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}
struct Session {
    argv: Vec<String>,
    id: String,
}
impl Drop for Session {
    fn drop(&mut self) {
        let mut cmd = proc::command(&self.argv);
        cmd.args(["close", &self.id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut child) = proc::spawn_group(&mut cmd) {
            let _ = child.wait();
        }
    }
}
fn units(s: &str) -> Res<f64> {
    let v: serde_json::Value = serde_json::from_str(s).map_err(failed)?;
    v.get("compute_units")
        .and_then(|v| v.as_f64())
        .filter(|n| n.is_finite())
        .ok_or_else(|| failed("invalid colab usage"))
}
fn finish(spec: &TrainSpec, wall_s: f64, used: Option<f64>) -> Res<TrainOutput> {
    let checkpoint = spec.out_dir.join("checkpoint");
    let onnx_dir = spec.out_dir.join("onnx");
    let tokenizer = spec.out_dir.join("tokenizer.json");
    let calibration = spec.out_dir.join("calibration.json");
    if !checkpoint.is_dir() || !onnx_dir.is_dir() || !tokenizer.is_file() || !calibration.is_file()
    {
        return Err(Failure::new(
            "laya_train_incomplete",
            "missing training artifacts",
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&calibration).map_err(failed)?).map_err(failed)?;
    let version = value
        .get("model_version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| failed("invalid calibration model_version"))?;
    ort::write_manifest(&onnx_dir, version, "fused", &spec.base, &spec.revision)?;
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let usage = UsageEvent {
        id: format!(
            "train-{}-{}-{}",
            now.as_nanos(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
        ts_ms: now.as_millis().min(i64::MAX as u128) as i64,
        model_version: version.into(),
        kind: "train".into(),
        backend: if used.is_some() { "colab" } else { "local" }.into(),
        tokens: None,
        gpu_hours: if used.is_none() {
            Some(wall_s / 3600.0)
        } else {
            None
        },
        compute_units: used,
    };
    Ok(TrainOutput {
        checkpoint,
        onnx_dir,
        tokenizer,
        calibration,
        usage,
    })
}
impl Trainer for PythonTrainer {
    fn train(&self, spec: &TrainSpec, cancel: &AtomicBool) -> Res<TrainOutput> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Failure::new("laya_train_cancelled", "training cancelled"));
        }
        let start = Instant::now();
        fs::create_dir_all(&spec.out_dir).map_err(failed)?;
        match spec.backend {
            Backend::Local => {
                let argv = local_command(spec, &self.python, &self.script);
                let mut cmd = proc::command(&argv);
                cmd.stdin(Stdio::null())
                    .stdout(File::create(spec.out_dir.join("train.stdout.log")).map_err(failed)?)
                    .stderr(File::create(spec.out_dir.join("train.stderr.log")).map_err(failed)?);
                let mut child = proc::spawn_group(&mut cmd).map_err(failed)?;
                if !wait(&mut child, cancel, start, spec.caps.wall_clock_s)?.success() {
                    let bytes = fs::read(spec.out_dir.join("train.stderr.log")).unwrap_or_default();
                    return Err(failed(format!(
                        "trainer failed: {}",
                        String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4096)..])
                    )));
                }
                finish(spec, start.elapsed().as_secs_f64(), None)
            }
            Backend::Colab => {
                let argv: Vec<String> = match std::env::var("AXIOM_LAYA_COLAB_CMD") {
                    Ok(v) => serde_json::from_str(&v).map_err(failed)?,
                    Err(std::env::VarError::NotPresent) => {
                        vec!["wsl.exe".into(), "-e".into(), "colab".into()]
                    }
                    Err(e) => return Err(failed(e)),
                };
                if argv.is_empty() || argv[0].is_empty() {
                    return Err(failed("empty colab command"));
                }
                let bundle = spec.out_dir.join("bundle");
                colab_bundle(spec, &bundle)?;
                let call =
                    |args: &[&str]| colab_call(&argv, args, cancel, start, spec.caps.wall_clock_s);
                let before = units(&call(&["usage"])?)?;
                let id = call(&["new"])?;
                if id.is_empty() {
                    return Err(failed("colab returned empty session id"));
                }
                let session = Session {
                    argv: argv.clone(),
                    id,
                };
                call(&["upload", &session.id, &path(&bundle)])?;
                call(&["exec", &session.id, "sh", "run_colab.sh"])?;
                call(&["download", &session.id, "out", &path(&spec.out_dir)])?;
                let used = before - units(&call(&["usage"])?)?;
                if used > spec.caps.compute_units {
                    return Err(Failure::new("laya_train_cap", "compute unit cap exceeded"));
                }
                finish(spec, start.elapsed().as_secs_f64(), Some(used))
            }
        }
    }
}
