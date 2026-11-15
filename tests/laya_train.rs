mod common;
use axiom::laya::{ort, train::*};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
struct Env(Vec<(&'static str, Option<OsString>)>);
impl Env {
    fn new() -> Self {
        let names = [
            "AXIOM_LAYA_TRAINER",
            "FAKE_LAYA_TRAINER_FAIL",
            "FAKE_LAYA_TRAINER_SLEEP",
            "AXIOM_LAYA_COLAB_CMD",
            "FAKE_COLAB_STATE",
            "FAKE_COLAB_FAIL",
            "FAKE_COLAB_EXEC_SLEEP",
            "FAKE_COLAB_EXEC_UNITS",
        ];
        let previous = names.iter().map(|&n| (n, std::env::var_os(n))).collect();
        for name in names {
            std::env::remove_var(name);
        }
        Self(previous)
    }
    fn set(&self, name: &str, value: impl AsRef<std::ffi::OsStr>) {
        std::env::set_var(name, value);
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            if let Some(v) = value {
                std::env::set_var(name, v);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}
fn spec(root: &Path, backend: Backend) -> TrainSpec {
    let dataset = root.join("dataset");
    fs::create_dir_all(&dataset).unwrap();
    for name in [
        "train.jsonl",
        "calibration.jsonl",
        "dev.jsonl",
        "acceptance.jsonl",
        "datasheet.json",
        "secret.txt",
    ] {
        fs::write(dataset.join(name), "{}\n").unwrap();
    }
    TrainSpec {
        dataset_dir: dataset,
        out_dir: root.join("out"),
        upstream: root.join("upstream"),
        base: "base/model".into(),
        revision: "revision123".into(),
        epochs: 3,
        seed: 42,
        backend,
        caps: Caps {
            wall_clock_s: 30,
            compute_units: 10.0,
        },
    }
}
fn trainer() -> PythonTrainer {
    PythonTrainer {
        python: vec![common::python(), "-I".into()],
        script: common::repo_root().join("tools/laya/train/run.py"),
    }
}
fn fake_env(env: &Env) {
    env.set(
        "AXIOM_LAYA_TRAINER",
        serde_json::to_string(&common::fake("fake_laya_trainer")).unwrap(),
    );
}
fn colab_env(env: &Env, root: &Path) {
    fake_env(env);
    env.set(
        "AXIOM_LAYA_COLAB_CMD",
        serde_json::to_string(&common::fake("fake_colab")).unwrap(),
    );
    env.set("FAKE_COLAB_STATE", root.join("state"));
}
fn calls(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("state/calls.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Vec<String>>(l).unwrap()[0].clone())
        .collect()
}
fn cancel_soon() -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        f.store(true, Ordering::Relaxed);
    });
    (flag, thread)
}

#[test]
fn exact_local_command() {
    let tmp = common::temp();
    let s = spec(tmp.path(), Backend::Local);
    let python = vec!["python".into(), "-I".into()];
    let script = tmp.path().join("run.py");
    let p = |x: &Path| x.to_string_lossy().into_owned();
    assert_eq!(
        local_command(&s, &python, &script),
        vec![
            "python".to_string(),
            "-I".into(),
            p(&script),
            "--dataset".into(),
            p(&s.dataset_dir),
            "--out".into(),
            p(&s.out_dir),
            "--upstream".into(),
            p(&s.upstream),
            "--base".into(),
            "base/model".into(),
            "--revision".into(),
            "revision123".into(),
            "--epochs".into(),
            "3".into(),
            "--seed".into(),
            "42".into(),
            "--device".into(),
            "cuda".into()
        ]
    );
}
#[test]
fn bundle_whitelist_hashes_and_negative_controls() {
    let tmp = common::temp();
    let s = spec(tmp.path(), Backend::Colab);
    let out = tmp.path().join("bundle");
    let m = colab_bundle(&s, &out).unwrap();
    let names: BTreeSet<_> = fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "train.jsonl",
            "calibration.jsonl",
            "dev.jsonl",
            "acceptance.jsonl",
            "datasheet.json",
            "run.py",
            "run_colab.sh",
            "manifest.json"
        ]
        .into_iter()
        .map(String::from)
        .collect()
    );
    assert_eq!(
        serde_json::from_slice::<BundleManifest>(&fs::read(out.join("manifest.json")).unwrap())
            .unwrap(),
        m
    );
    assert_eq!(m.files.len(), 7);
    for (name, hash) in &m.files {
        assert_eq!(&ort::sha256_file(&out.join(name)).unwrap(), hash);
    }
    assert!(colab_bundle(&s, &out).is_err());
    let stale = tmp.path().join("stale");
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("secret.txt"), "secret").unwrap();
    assert!(colab_bundle(&s, &stale).is_err());
    assert!(!stale.join("manifest.json").exists());
    fs::remove_file(s.dataset_dir.join("datasheet.json")).unwrap();
    assert_eq!(
        colab_bundle(&s, &tmp.path().join("missing"))
            .unwrap_err()
            .kind,
        "laya_bundle_no_datasheet"
    );
}
#[test]
fn local_success_failure_cancel_and_cap() {
    let _lock = ENV_LOCK.lock().unwrap();
    let env = Env::new();
    fake_env(&env);
    let tmp = common::temp();
    let mut s = spec(tmp.path(), Backend::Local);
    let t = trainer();
    let output = t.train(&s, &AtomicBool::new(false)).unwrap();
    assert!(
        output.checkpoint.is_dir()
            && output.onnx_dir.is_dir()
            && output.tokenizer.is_file()
            && output.calibration.is_file()
    );
    let manifest = ort::verify_dir(&output.onnx_dir).unwrap();
    assert_eq!(manifest.layout, "fused");
    assert_eq!(output.usage.backend, "local");
    assert!(output.usage.gpu_hours.is_some());
    env.set("FAKE_LAYA_TRAINER_FAIL", "1");
    s.out_dir = tmp.path().join("failed");
    assert_eq!(
        t.train(&s, &AtomicBool::new(false)).unwrap_err().kind,
        "laya_train_failed"
    );
    assert!(!s.out_dir.join("calibration.json").exists());
    std::env::remove_var("FAKE_LAYA_TRAINER_FAIL");
    env.set("FAKE_LAYA_TRAINER_SLEEP", "20");
    s.out_dir = tmp.path().join("cancelled");
    let (flag, thread) = cancel_soon();
    let start = Instant::now();
    assert_eq!(t.train(&s, &flag).unwrap_err().kind, "laya_train_cancelled");
    thread.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    s.out_dir = tmp.path().join("cap");
    s.caps.wall_clock_s = 0;
    assert_eq!(
        t.train(&s, &AtomicBool::new(false)).unwrap_err().kind,
        "laya_train_cap"
    );
}
#[test]
fn colab_success_failure_cancel_and_cap() {
    let _lock = ENV_LOCK.lock().unwrap();
    let env = Env::new();
    let tmp = common::temp();
    colab_env(&env, tmp.path());
    let mut s = spec(tmp.path(), Backend::Colab);
    let t = trainer();
    let output = t.train(&s, &AtomicBool::new(false)).unwrap();
    assert_eq!(output.usage.compute_units, Some(1.5));
    assert_eq!(output.usage.gpu_hours, None);
    assert_eq!(output.usage.backend, "colab");
    assert!(ort::verify_dir(&output.onnx_dir).is_ok());
    assert_eq!(
        calls(tmp.path()),
        ["usage", "new", "upload", "exec", "download", "usage", "close"]
    );
    fs::remove_dir_all(tmp.path().join("state")).unwrap();
    s.out_dir = tmp.path().join("failed");
    env.set("FAKE_COLAB_FAIL", "exec");
    assert_eq!(
        t.train(&s, &AtomicBool::new(false)).unwrap_err().kind,
        "laya_train_failed"
    );
    assert!(tmp.path().join("state/closed").is_file());
    assert_eq!(calls(tmp.path()).last().unwrap(), "close");
    fs::remove_dir_all(tmp.path().join("state")).unwrap();
    std::env::remove_var("FAKE_COLAB_FAIL");
    env.set("FAKE_COLAB_EXEC_SLEEP", "20");
    s.out_dir = tmp.path().join("cancelled");
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&flag);
    let call_log = tmp.path().join("state/calls.jsonl");
    let thread = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if fs::read_to_string(&call_log)
                .unwrap_or_default()
                .lines()
                .any(|line| {
                    serde_json::from_str::<Vec<String>>(line)
                        .is_ok_and(|v| v.first().is_some_and(|s| s == "exec"))
                })
            {
                trigger.store(true, Ordering::Relaxed);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        trigger.store(true, Ordering::Relaxed);
        panic!("colab exec did not start");
    });
    let start = Instant::now();
    assert_eq!(t.train(&s, &flag).unwrap_err().kind, "laya_train_cancelled");
    thread.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(10));
    assert_eq!(calls(tmp.path()).last().unwrap(), "close");
    fs::remove_dir_all(tmp.path().join("state")).unwrap();
    std::env::remove_var("FAKE_COLAB_EXEC_SLEEP");
    s.out_dir = tmp.path().join("cap");
    s.caps.compute_units = 1.0;
    assert_eq!(
        t.train(&s, &AtomicBool::new(false)).unwrap_err().kind,
        "laya_train_cap"
    );
    assert_eq!(calls(tmp.path()).last().unwrap(), "close");
    fs::remove_dir_all(tmp.path().join("state")).unwrap();
    s.out_dir = tmp.path().join("pre-cancel");
    assert_eq!(
        t.train(&s, &AtomicBool::new(true)).unwrap_err().kind,
        "laya_train_cancelled"
    );
    assert!(!tmp.path().join("state").exists());
    env.set("FAKE_COLAB_FAIL", "usage");
    s.out_dir = tmp.path().join("usage-failed");
    assert_eq!(
        t.train(&s, &AtomicBool::new(false)).unwrap_err().kind,
        "laya_train_failed"
    );
    assert_eq!(calls(tmp.path()), ["usage"]);
}
