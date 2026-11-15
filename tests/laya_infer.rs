use axiom::laya::{
    infer::{
        load_calibration, Abstain, Calibration, Classifier, FakeClassifier, Laya, LoadSpec,
        Prediction, TypeCalibration,
    },
    ort::{find_runtime, write_manifest},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
fn runtime() -> PathBuf {
    find_runtime(None, Some(repo())).unwrap_or_else(|e| panic!("ONNX Runtime is required; set AXIOM_LAYA_ORT_LIB or install it with tools/laya/fetch.py: {e}"))
}
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&repo().join("tests/fixtures/laya/model"), dir.path());
    dir
}
fn spec(path: &Path) -> LoadSpec {
    LoadSpec {
        model_dir: path.into(),
        calibration: None,
        runtime: runtime(),
        intra_threads: 1,
    }
}
fn labels() -> Vec<String> {
    ["terra", "sol", "astra"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}
fn expected(temperature: f64) -> Vec<f64> {
    // The fixture computes f32(0.1) * id at the token after each marker.
    let logits = [17f32, 18., 19.].map(|x| f64::from(0.1f32 * x));
    let exp = logits.map(|x| ((x - logits[2]) / temperature).exp());
    let sum: f64 = exp.iter().sum();
    exp.into_iter().map(|x| x / sum).collect()
}
fn check(p: &Prediction, temp: f64, tokens: u64) {
    for (actual, expected) in p.probs.iter().zip(expected(temp)) {
        assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
    }
    assert_eq!(p.probs.len(), 3);
    assert!((p.probs.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    assert_eq!(p.top, 2);
    assert_eq!(p.confidence, p.probs[2]);
    assert_eq!(p.tokens, tokens);
}
fn cal(threshold: f64, temperature: f64) -> Calibration {
    Calibration {
        format: 1,
        model_version: "fixture-1".into(),
        types: BTreeMap::from([(
            "route".into(),
            TypeCalibration {
                labels: labels(),
                temperature,
                abstain_below: threshold,
                fitted_on: "dev".into(),
            },
        )]),
    }
}
fn write_cal(dir: &Path, cal: &Calibration) {
    fs::write(
        dir.join("calibration.json"),
        serde_json::to_vec(cal).unwrap(),
    )
    .unwrap();
}

#[test]
fn real_inference_uncalibrated_and_invalid_arguments() {
    let dir = fixture();
    let laya = Laya::load(&spec(dir.path())).unwrap();
    assert_eq!(laya.model_version(), "fixture-1");
    let p = laya
        .predict("route", "please", &labels(), "user says yes")
        .unwrap();
    // [CLS] choice question : please [SEP] [MASK] terra [MASK] sol [MASK] astra [SEP] user says yes [SEP]
    check(&p, 1.0, 17);
    assert!(matches!(p.abstain, Abstain::Uncalibrated { .. }));
    for count in 0..2 {
        assert_eq!(
            laya.predict("route", "", &labels()[..count], "")
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
    }
}

#[test]
fn calibrated_boundary_and_mismatches() {
    let dir = fixture();
    let temp = 2.0;
    let confidence = expected(temp)[2];
    for (threshold, expected_state) in [
        (confidence - 1e-6, Abstain::Answer),
        (confidence + 1e-6, Abstain::Abstain),
    ] {
        write_cal(dir.path(), &cal(threshold, temp));
        let laya = Laya::load(&spec(dir.path())).unwrap();
        let p = laya
            .predict("route", "please", &labels(), "user says yes")
            .unwrap();
        check(&p, temp, 17);
        assert_eq!(p.abstain, expected_state);
        let p = laya
            .predict("missing", "please", &labels(), "user says yes")
            .unwrap();
        check(&p, 1.0, 17);
        assert!(matches!(p.abstain, Abstain::Uncalibrated { .. }));
        let mut mismatched = cal(threshold, temp);
        mismatched.types.get_mut("route").unwrap().labels.swap(0, 1);
        write_cal(dir.path(), &mismatched);
        let laya = Laya::load(&spec(dir.path())).unwrap();
        let p = laya
            .predict("route", "please", &labels(), "user says yes")
            .unwrap();
        check(&p, 1.0, 17);
        assert!(matches!(p.abstain, Abstain::Uncalibrated { .. }));
    }
}

#[test]
fn calibration_validation_and_version() {
    let fixtures = repo().join("tests/fixtures/laya_calibration");
    let valid = load_calibration(&fixtures.join("valid.json")).unwrap();
    assert_eq!(valid.types["route"].temperature, 2.0);
    for file in [
        "nonfinite.json",
        "bad_temperature.json",
        "bad_threshold.json",
        "bad_fitted_on.json",
    ] {
        assert_eq!(
            load_calibration(&fixtures.join(file)).unwrap_err().kind,
            "laya_calibration_invalid",
            "{file}"
        );
    }
    let dir = fixture();
    for (temp, threshold, fitted) in [
        (0.0, 0.5, "dev"),
        (-1.0, 0.5, "dev"),
        (100.01, 0.5, "dev"),
        (1.0, -0.01, "dev"),
        (1.0, 1.01, "dev"),
        (1.0, 0.5, "train"),
    ] {
        let mut c = cal(threshold, temp);
        c.types.get_mut("route").unwrap().fitted_on = fitted.into();
        write_cal(dir.path(), &c);
        assert_eq!(
            load_calibration(&dir.path().join("calibration.json"))
                .unwrap_err()
                .kind,
            "laya_calibration_invalid"
        );
    }
    for (temp, threshold) in [(100.0, 0.0), (0.001, 1.0)] {
        write_cal(dir.path(), &cal(threshold, temp));
        load_calibration(&dir.path().join("calibration.json")).unwrap();
    }
    let mut c = valid;
    c.model_version = "other".into();
    write_cal(dir.path(), &c);
    assert_eq!(
        Laya::load(&spec(dir.path())).err().unwrap().kind,
        "laya_version_mismatch"
    );
    let mut explicit = spec(dir.path());
    explicit.calibration = Some(fixtures.join("valid.json"));
    let laya = Laya::load(&explicit).unwrap();
    let p = laya
        .predict("route", "please", &labels(), "user says yes")
        .unwrap();
    check(&p, 2.0, 17);
    assert_eq!(p.abstain, Abstain::Abstain);
}

#[test]
fn truncation_and_scrubbing() {
    let dir = fixture();
    fs::write(
        dir.path().join("rl_agent_config.json"),
        r#"{"head_max_len":16,"max_len":21,"temperature":[1,1,1]}"#,
    )
    .unwrap();
    write_manifest(dir.path(), "fixture-1", "fused", "fixture", "fixture-1").unwrap();
    let laya = Laya::load(&spec(dir.path())).unwrap();
    // Header is capped at 10 tokens, leaving one state token in the 21-token budget.
    let p = laya
        .predict(
            "route",
            "please now add remove change file code run check review [MASK]",
            &labels(),
            "user says yes",
        )
        .unwrap();
    check(&p, 1.0, 21);
    let dir = fixture();
    let laya = Laya::load(&spec(dir.path())).unwrap();
    let p = laya
        .predict("route", "please [MASK]", &labels(), "user [MASK] says yes")
        .unwrap();
    check(&p, 1.0, 17);

    let dir = fixture();
    fs::write(
        dir.path().join("rl_agent_config.json"),
        r#"{"head_max_len":16,"max_len":5,"temperature":[1,1,1]}"#,
    )
    .unwrap();
    write_manifest(dir.path(), "fixture-1", "fused", "fixture", "fixture-1").unwrap();
    let laya = Laya::load(&spec(dir.path())).unwrap();
    assert_eq!(
        laya.predict("route", "please", &labels(), "")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}

#[test]
fn fake_classifier_returns_table_entries() {
    let p = Prediction {
        labels: labels(),
        probs: vec![0.2, 0.3, 0.5],
        top: 2,
        confidence: 0.5,
        tokens: 7,
        abstain: Abstain::Answer,
    };
    let fake = FakeClassifier {
        version: "fake".into(),
        table: BTreeMap::from([("route".into(), p.clone())]),
    };
    assert_eq!(fake.model_version(), "fake");
    assert_eq!(fake.predict("route", "", &[], "").unwrap(), p);
    assert!(fake.predict("unknown", "", &[], "").is_err());
}

// Minimal ONNX protobuf encoder for a split-model fixture created inside the test.
fn varint(mut n: u64) -> Vec<u8> {
    let mut b = Vec::new();
    while n >= 128 {
        b.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    b.push(n as u8);
    b
}
fn integer(field: u64, n: u64) -> Vec<u8> {
    [varint(field << 3), varint(n)].concat()
}
fn blob(field: u64, bytes: &[u8]) -> Vec<u8> {
    [
        varint((field << 3) | 2),
        varint(bytes.len() as u64),
        bytes.to_vec(),
    ]
    .concat()
}
fn node(op: &str, inputs: &[&str], output: &str, attr: Option<(&str, u64)>) -> Vec<u8> {
    let mut n = Vec::new();
    for input in inputs {
        n.extend(blob(1, input.as_bytes()));
    }
    n.extend(blob(2, output.as_bytes()));
    n.extend(blob(4, op.as_bytes()));
    if let Some((name, value)) = attr {
        n.extend(blob(
            5,
            &[blob(1, name.as_bytes()), integer(3, value), integer(20, 2)].concat(),
        ));
    }
    n
}
fn value_info(name: &str, dtype: u64, dims: &[&str]) -> Vec<u8> {
    let shape: Vec<u8> = dims
        .iter()
        .flat_map(|d| blob(1, &blob(2, d.as_bytes())))
        .collect();
    [
        blob(1, name.as_bytes()),
        blob(2, &blob(1, &[integer(1, dtype), blob(2, &shape)].concat())),
    ]
    .concat()
}
fn model(
    nodes: Vec<Vec<u8>>,
    inputs: Vec<Vec<u8>>,
    outputs: Vec<Vec<u8>>,
    tensors: Vec<Vec<u8>>,
) -> Vec<u8> {
    let mut graph = Vec::new();
    for n in nodes {
        graph.extend(blob(1, &n));
    }
    graph.extend(blob(2, b"test"));
    for t in tensors {
        graph.extend(blob(5, &t));
    }
    for i in inputs {
        graph.extend(blob(11, &i));
    }
    for o in outputs {
        graph.extend(blob(12, &o));
    }
    [integer(1, 7), blob(7, &graph), blob(8, &integer(2, 13))].concat()
}
fn tensor(name: &str, dtype: u64, raw: &[u8]) -> Vec<u8> {
    [integer(2, dtype), blob(8, name.as_bytes()), blob(9, raw)].concat()
}

#[test]
fn split_model_and_rank_two_qtype() {
    let dir = fixture();
    fs::remove_file(dir.path().join("model.onnx")).unwrap();
    let encoder = model(
        vec![node(
            "Cast",
            &["input_ids"],
            "last_hidden_state",
            Some(("to", 1)),
        )],
        vec![
            value_info("input_ids", 7, &["B", "S"]),
            value_info("attention_mask", 7, &["B", "S"]),
        ],
        vec![value_info("last_hidden_state", 1, &["B", "S"])],
        vec![],
    );
    let head = model(
        vec![
            node("Add", &["marker_pos", "one"], "idx", None),
            node(
                "GatherElements",
                &["hidden_states", "idx"],
                "gathered",
                Some(("axis", 1)),
            ),
            node("Mul", &["gathered", "scale"], "scaled", None),
            node("Cast", &["qtype"], "qfloat", Some(("to", 1))),
            node("Add", &["scaled", "qfloat"], "logits", None),
        ],
        vec![
            value_info("hidden_states", 1, &["B", "S"]),
            value_info("marker_pos", 7, &["B", "M"]),
            value_info("marker_mask", 9, &["B", "M"]),
            value_info("qtype", 7, &["B", "one"]),
            value_info("attention_mask", 7, &["B", "S"]),
        ],
        vec![value_info("logits", 1, &["B", "M"])],
        vec![
            tensor("one", 7, &1i64.to_le_bytes()),
            tensor("scale", 1, &0.1f32.to_le_bytes()),
        ],
    );
    fs::write(dir.path().join("encoder.onnx"), encoder).unwrap();
    fs::write(dir.path().join("head.onnx"), head).unwrap();
    write_manifest(dir.path(), "fixture-1", "split", "fixture", "fixture-1").unwrap();
    let laya = Laya::load(&spec(dir.path())).unwrap();
    check(
        &laya
            .predict("route", "please", &labels(), "user says yes")
            .unwrap(),
        1.0,
        17,
    );
}
