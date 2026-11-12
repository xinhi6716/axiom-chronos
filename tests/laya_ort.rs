use axiom::{base::Res, laya::ort::*};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const ENV_LIB: &str = "AXIOM_LAYA_ORT_LIB";

struct RestoreEnv(Option<OsString>);

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        match &self.0 {
            Some(value) => std::env::set_var(ENV_LIB, value),
            None => std::env::remove_var(ENV_LIB),
        }
    }
}

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    copy_dir(&repo().join("tests/fixtures/laya/model"), temp.path()).unwrap();
    temp
}

fn fixture_runtime() -> Res<PathBuf> {
    let _lock = ENV_LOCK.lock().unwrap();
    find_runtime(None, Some(repo()))
}

#[test]
fn fixture_runtime_loads() {
    let temp = fixture();
    let manifest = verify_dir(temp.path()).unwrap();
    assert_eq!(manifest.model_version, "fixture-1");
    let path = fixture_runtime().unwrap_or_else(|err| panic!("{err}"));
    let runtime = Runtime::load(&path).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(runtime.api_version(), ORT_API_VERSION);
}

#[test]
fn corrupted_model_is_refused() {
    let temp = fixture();
    let path = temp.path().join("model.onnx");
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(&path, bytes).unwrap();
    let err = verify_dir(temp.path()).unwrap_err();
    assert_eq!(err.kind, "laya_sha256_mismatch");
    assert!(err.message.contains("model.onnx"), "{err}");
}

#[test]
fn missing_runtime_is_refused() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _restore = RestoreEnv(std::env::var_os(ENV_LIB));
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-runtime");
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    std::env::set_var(ENV_LIB, &missing);
    let candidates = runtime_candidates(Some(&home), Some(&repo));
    assert_eq!(candidates.first(), Some(&missing));
    assert!(candidates.contains(&repo.join(".laya/runtime").join(runtime_lib_name())));
    if candidates.iter().all(|candidate| !candidate.is_file()) {
        let err = find_runtime(Some(&home), Some(&repo)).unwrap_err();
        assert_eq!(err.kind, "laya_runtime_missing");
        assert!(err.message.contains(ENV_LIB), "{err}");
        assert!(err.message.contains("tools/laya/fetch.py"), "{err}");
    } else {
        assert!(
            cfg!(windows),
            "unexpected existing runtime candidate: {candidates:?}"
        );
        let inbox = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join(runtime_lib_name());
        assert!(inbox.is_file());
        assert!(candidates.contains(&inbox));
    }
    assert!(Runtime::load(&missing).is_err());
}

#[test]
fn laya_runtime_directory_is_git_ignored() {
    let output = Command::new("git")
        .args(["check-ignore", ".laya/x"])
        .current_dir(repo())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cargo_has_no_ort_or_tokenizers_dependency() {
    let manifest = fs::read_to_string(repo().join("Cargo.toml")).unwrap();
    for line in manifest.lines() {
        let line = line.trim_start();
        let key = line
            .split(|c: char| c == '=' || c.is_whitespace())
            .next()
            .unwrap_or("");
        assert!(
            key != "ort" && key != "tokenizers",
            "forbidden dependency: {line}"
        );
    }
}

fn session_fixture() -> (tempfile::TempDir, Session) {
    let temp = fixture();
    verify_dir(temp.path()).unwrap();
    let path = fixture_runtime().unwrap_or_else(|err| panic!("{err}"));
    let runtime = Runtime::load(&path).unwrap();
    let alias = path
        .parent()
        .unwrap()
        .join(".")
        .join(path.file_name().unwrap());
    let cached = Runtime::load(&alias).unwrap();
    assert!(std::sync::Arc::ptr_eq(&runtime, &cached));
    let session = Session::new(&runtime, &temp.path().join("model.onnx"), 1).unwrap();
    (temp, session)
}
fn fixture_inputs() -> Vec<(&'static str, Tensor)> {
    vec![
        (
            "input_ids",
            Tensor::I64 {
                shape: vec![2, 5],
                data: vec![11, 12, 13, 14, 15, 21, 22, 23, 24, 25],
            },
        ),
        (
            "attention_mask",
            Tensor::I64 {
                shape: vec![2, 5],
                data: vec![1; 10],
            },
        ),
        (
            "marker_pos",
            Tensor::I64 {
                shape: vec![2, 2],
                data: vec![0, 2, 1, 3],
            },
        ),
        (
            "marker_mask",
            Tensor::Bool {
                shape: vec![2, 2],
                data: vec![true; 4],
            },
        ),
        (
            "qtype",
            Tensor::I64 {
                shape: vec![2],
                data: vec![3, 7],
            },
        ),
    ]
}
#[test]
fn fixture_session_inference() {
    let (_temp, session) = session_fixture();
    assert_eq!(
        session.inputs(),
        &[
            IoInfo {
                name: "input_ids".into(),
                elem: ElemType::I64,
                dims: vec![-1, -1]
            },
            IoInfo {
                name: "attention_mask".into(),
                elem: ElemType::I64,
                dims: vec![-1, -1]
            },
            IoInfo {
                name: "marker_pos".into(),
                elem: ElemType::I64,
                dims: vec![-1, -1]
            },
            IoInfo {
                name: "marker_mask".into(),
                elem: ElemType::Bool,
                dims: vec![-1, -1]
            },
            IoInfo {
                name: "qtype".into(),
                elem: ElemType::I64,
                dims: vec![-1]
            },
        ]
    );
    assert_eq!(
        session.outputs(),
        &[
            IoInfo {
                name: "logits".into(),
                elem: ElemType::F32,
                dims: vec![-1, -1]
            },
            IoInfo {
                name: "act_logits".into(),
                elem: ElemType::F32,
                dims: vec![-1]
            },
        ]
    );
    let inputs = fixture_inputs();
    let results = session.run(&inputs, &["logits", "act_logits"]).unwrap();
    let Tensor::F32 { shape, data } = &results[0] else {
        panic!("expected logits f32")
    };
    assert_eq!(shape, &[2, 2]);
    let (
        Tensor::I64 { data: ids, .. },
        Tensor::I64 {
            data: positions, ..
        },
    ) = (&inputs[0].1, &inputs[2].1)
    else {
        unreachable!()
    };
    for b in 0..2 {
        for m in 0..2 {
            let expected = 0.1 * ids[b * 5 + positions[b * 2 + m] as usize + 1] as f32;
            assert!((data[b * 2 + m] - expected).abs() <= 1e-6);
        }
    }
    assert_eq!(
        results[1],
        Tensor::F32 {
            shape: vec![2],
            data: vec![3.0, 7.0]
        }
    );
    assert_eq!(
        session.run(&inputs, &["act_logits"]).unwrap(),
        vec![results[1].clone()]
    );
}
#[test]
fn wrong_input_name_is_ort_error() {
    let (_temp, session) = session_fixture();
    let mut inputs = fixture_inputs();
    inputs[0].0 = "wrong_name";
    let err = session.run(&inputs, &["logits"]).unwrap_err();
    assert_eq!(err.kind, "laya_ort");
    assert!(err.message.contains("wrong_name"), "{err}");
}

#[test]
fn malformed_tensor_shape_is_rejected() {
    let (_temp, session) = session_fixture();
    let mut inputs = fixture_inputs();
    inputs[0].1 = Tensor::I64 {
        shape: vec![2, 5],
        data: vec![1],
    };
    assert_eq!(
        session.run(&inputs, &["logits"]).unwrap_err().kind,
        "laya_ort"
    );
}
