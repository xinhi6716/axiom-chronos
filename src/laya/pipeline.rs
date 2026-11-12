use crate::base::{Failure, Res};
use crate::laya::{
    curate::{self, CurateSpec, OutcomeSource, Split, VerifiedOutcomeSource},
    ort,
    router::{self, DecisionKind, Mode},
    shadow::{self, Confirmation, EvalReport},
    train::{self, Backend, Caps, TrainOutput, TrainSpec, Trainer},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct PipelineSpec {
    pub home: PathBuf,
    pub repo: PathBuf,
    pub work_dir: PathBuf,
    pub curate: CurateSpec,
    pub train: TrainSpec,
    pub date: String,
    pub dry_run: bool,
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StepResult {
    pub step: String,
    pub status: String,
    pub detail: String,
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PipelineReport {
    pub steps: Vec<StepResult>,
    pub promoted: Option<DecisionKind>,
}

const STEPS: [&str; 7] = [
    "curate",
    "train",
    "export",
    "shadow_eval",
    "await_confirmation",
    "promote",
    "rollback",
];
fn step(name: &str, status: &str, detail: impl Into<String>) -> StepResult {
    StepResult {
        step: name.into(),
        status: status.into(),
        detail: detail.into(),
    }
}
fn curated(home: &Path, spec: &CurateSpec) -> Res<curate::Curated> {
    let (records, _) = router::read_records(home)?;
    let source = VerifiedOutcomeSource { home: home.into() };
    curate::curate(&records, &[&source], spec)
}
fn summary(report: &EvalReport) -> String {
    report
        .metrics
        .iter()
        .map(|(k, m)| {
            format!(
                "{}: samples={}, agreement={}",
                k.as_str(),
                m.samples,
                m.agreement.map_or_else(|| "n/a".into(), |v| v.to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn run(
    spec: &PipelineSpec,
    trainer: &dyn Trainer,
    eval: &dyn Fn(&TrainOutput) -> Res<EvalReport>,
    confirm: Option<&Confirmation>,
) -> Res<PipelineReport> {
    let mut result = PipelineReport {
        steps: Vec::new(),
        promoted: None,
    };
    if spec.dry_run {
        result.steps = STEPS
            .iter()
            .map(|s| step(s, "preview", "dry run"))
            .collect();
        return Ok(result);
    }
    macro_rules! perform {
        ($name:expr, $body:expr) => {{
            match $body {
                Ok((value, detail)) => {
                    result.steps.push(step($name, "ok", detail));
                    value
                }
                Err(e) => {
                    result.steps.push(step($name, "failed", format!("{}", e)));
                    while result.steps.len() < STEPS.len() {
                        result.steps.push(step(
                            STEPS[result.steps.len()],
                            "skipped",
                            "previous step failed",
                        ));
                    }
                    return Ok(result);
                }
            }
        }};
    }
    perform!(
        "curate",
        (|| -> Res<_> {
            let data = curated(&spec.home, &spec.curate)?;
            curate::write(&data, &spec.train.dataset_dir)?;
            let detail = [
                Split::Train,
                Split::Calibration,
                Split::Dev,
                Split::Acceptance,
            ]
            .iter()
            .zip(["train", "calibration", "dev", "acceptance"])
            .map(|(s, n)| format!("{}={}", n, data.splits.get(s).map_or(0, Vec::len)))
            .collect::<Vec<_>>()
            .join(", ");
            Ok(((), detail))
        })()
    );
    let output = perform!(
        "train",
        (|| -> Res<_> {
            let output = trainer.train(&spec.train, &AtomicBool::new(false))?;
            crate::usage::laya::record(&spec.home, &output.usage)?;
            Ok((output, "training complete".to_string()))
        })()
    );
    perform!(
        "export",
        (|| -> Res<_> { Ok(((), ort::verify_dir(&output.onnx_dir)?.model_version)) })()
    );
    let report = perform!(
        "shadow_eval",
        (|| -> Res<_> {
            let report = eval(&output)?;
            let detail = summary(&report);
            Ok((report, detail))
        })()
    );
    let Some(confirm) = confirm else {
        result.steps.push(step(
            "await_confirmation",
            "waiting",
            "confirmation required",
        ));
        result
            .steps
            .push(step("promote", "skipped", "confirmation required"));
        result
            .steps
            .push(step("rollback", "skipped", "confirmation required"));
        return Ok(result);
    };
    result
        .steps
        .push(step("await_confirmation", "ok", "confirmation supplied"));
    let thresholds = perform!(
        "promote",
        (|| -> Res<_> {
            let (t, _) = shadow::load_thresholds(None)?;
            shadow::promote(
                &spec.home,
                &spec.repo,
                &[confirm.kind],
                &report,
                &t,
                Some(confirm),
                &spec.date,
            )?;
            Ok((t, confirm.kind.as_str().to_string()))
        })()
    );
    result.promoted = Some(confirm.kind);
    let rollback = (|| -> Res<(Vec<&'static str>, Vec<&'static str>)> {
        let state = shadow::load_state(&spec.home)?;
        let mut rolled = Vec::new();
        let mut missing = Vec::new();
        for (kind, state) in state.types {
            if state.decider == "laya" {
                let Some(m) = report.metrics.get(&kind).filter(|m| m.kind == kind) else {
                    missing.push(kind.as_str());
                    continue;
                };
                if shadow::check_drift(&spec.home, kind, m, &thresholds)?.is_some() {
                    rolled.push(kind.as_str());
                }
            }
        }
        Ok((rolled, missing))
    })();
    match rollback {
        Ok((rolled, missing)) => {
            let status = if rolled.is_empty() { "skipped" } else { "ok" };
            let mut detail = if rolled.is_empty() {
                "no drift".into()
            } else {
                rolled.join(", ")
            };
            if !missing.is_empty() {
                detail.push_str(&format!("; missing metrics: {}", missing.join(", ")));
            }
            result.steps.push(step("rollback", status, detail));
        }
        Err(e) => result.steps.push(step("rollback", "failed", e.to_string())),
    }
    Ok(result)
}

fn default_train(dataset: PathBuf, out: PathBuf) -> TrainSpec {
    TrainSpec {
        dataset_dir: dataset,
        out_dir: out,
        upstream: std::env::var_os("AXIOM_LAYA_UPSTREAM")
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("."), PathBuf::from),
        base: "convaiinnovations/laya-multilingual".into(),
        revision: "main".into(),
        epochs: 3,
        seed: 0,
        backend: Backend::Colab,
        caps: Caps {
            wall_clock_s: 7200,
            compute_units: 10.0,
        },
    }
}
enum CliError {
    Usage(String),
    Failure(Failure),
}
impl From<Failure> for CliError {
    fn from(e: Failure) -> Self {
        Self::Failure(e)
    }
}
fn usage(message: impl Into<String>) -> CliError {
    CliError::Usage(message.into())
}
fn flags(args: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, CliError> {
    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        if !allowed.contains(&key.as_str())
            || i + 1 == args.len()
            || args[i + 1].starts_with("--")
            || map.contains_key(key)
        {
            return Err(usage(format!("invalid option {key}")));
        }
        map.insert(key.clone(), args[i + 1].clone());
        i += 2;
    }
    Ok(map)
}
fn required<'a>(m: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, CliError> {
    m.get(key)
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .ok_or_else(|| usage(format!("{key} required")))
}
fn io(e: impl std::fmt::Display) -> CliError {
    CliError::Failure(Failure::new("io", e.to_string()))
}
fn model_hash(home: &Path) -> Result<String, CliError> {
    let cfg = router::load_config(home)?;
    let dir = cfg.model_dir.ok_or_else(|| {
        CliError::Failure(Failure::new("model_required", "router model_dir required"))
    })?;
    Ok(ort::sha256_file(&dir.join("axiom-laya.json"))?)
}
pub fn status_text(home: &Path) -> Res<String> {
    let cfg = router::load_config(home)?;
    let runtime = cfg
        .runtime_lib
        .filter(|path| path.is_file())
        .or_else(|| ort::find_runtime(Some(home), None).ok());
    let mut text = format!(
        "mode: {}\nkill: {}\nruntime: {}\n",
        if cfg.mode == Mode::Off {
            "off"
        } else {
            "shadow"
        },
        if router::killed(home) { "yes" } else { "no" },
        runtime.map_or_else(|| "not found".into(), |p| format!("found {}", p.display())),
    );
    let state = shadow::load_state(home)?;
    if state.types.is_empty() {
        text.push_str("promotion: none\n");
    } else {
        for (kind, state) in state.types {
            text.push_str(&format!("promotion {}: {}\n", kind.as_str(), state.decider));
        }
    }
    text.push_str(&crate::usage::laya::cli_text(home)?);
    Ok(text)
}

pub fn dry_run_json(home: &Path) -> Res<String> {
    let work = crate::laya::laya_dir(home).join("work");
    let spec = PipelineSpec {
        home: home.into(),
        repo: PathBuf::from("."),
        work_dir: work.clone(),
        curate: CurateSpec::new(0),
        train: default_train(work.join("dataset"), work.join("out")),
        date: crate::base::today(),
        dry_run: true,
    };
    struct NoTrainer;
    impl Trainer for NoTrainer {
        fn train(&self, _: &TrainSpec, _: &AtomicBool) -> Res<TrainOutput> {
            unreachable!()
        }
    }
    let report = run(&spec, &NoTrainer, &|_| unreachable!(), None)?;
    serde_json::to_string_pretty(&report).map_err(|e| Failure::new("json", e.to_string()))
}

fn cli_inner(home: &Path, sub: &str, args: &[String]) -> Result<(), CliError> {
    match sub {
        "status" => {
            flags(args, &[])?;
            print!("{}", status_text(home)?);
        }
        "shadow" => {
            let mode = match args.first().map(String::as_str) { Some("on") => Mode::Shadow, Some("off") => Mode::Off, _ => return Err(usage("shadow requires on or off")) };
            let m = flags(&args[1..], &["--model", "--calibration", "--runtime"])?;
            let mut cfg = router::load_config(home)?; cfg.mode = mode;
            if let Some(v) = m.get("--model") { cfg.model_dir = Some(v.into()); }
            if let Some(v) = m.get("--calibration") { cfg.calibration = Some(v.into()); }
            if let Some(v) = m.get("--runtime") { cfg.runtime_lib = Some(v.into()); }
            router::save_config(home, &cfg)?;
        }
        "kill" | "unkill" => { flags(args, &[])?; if sub == "kill" { router::kill(home)? } else { router::unkill(home)? } }
        "curate" => {
            let m = flags(args, &["--out", "--seed"])?;
            let out = PathBuf::from(required(&m, "--out")?);
            let seed = m.get("--seed").map_or(Ok(0), |v| v.parse::<u64>().map_err(|_| usage("invalid seed")))?;
            curate::write(&curated(home, &CurateSpec::new(seed))?, &out)?;
            println!("{}", out.join("datasheet.json").display());
        }
        "bundle" => {
            let m = flags(args, &["--dataset", "--out"])?;
            let dataset = PathBuf::from(required(&m, "--dataset")?); let out = PathBuf::from(required(&m, "--out")?);
            train::colab_bundle(&default_train(dataset, out.clone()), &out)?;
            println!("{}", out.join("manifest.json").display());
        }
        "eval" => {
            let m = flags(args, &["--split"])?;
            let split_name = required(&m, "--split")?;
            let split = match split_name { "dev" => Split::Dev, "acceptance" => Split::Acceptance, _ => return Err(usage("split must be dev or acceptance")) };
            let hash = model_hash(home)?; let (thresholds, digest) = shadow::load_thresholds(None)?;
            let data = curated(home, &CurateSpec::new(0))?;
            let ids: HashSet<_> = data.splits.get(&split).into_iter().flatten().map(|r| r.meta.record.as_str()).collect();
            let (records, _) = router::read_records(home)?;
            let records: Vec<_> = records.into_iter().filter(|r| ids.contains(r.id.as_str())).collect();
            let outcomes = VerifiedOutcomeSource { home: home.into() }.outcomes()?;
            let report = EvalReport { model_hash: hash, split: split_name.into(), thresholds_sha256: digest, metrics: shadow::metrics(&records, &outcomes, thresholds.ece_bins), ts_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64 };
            shadow::save_report(home, &report)?;
            println!("type\tsamples\tagreement");
            for (kind, m) in &report.metrics { println!("{}\t{}\t{}", kind.as_str(), m.samples, m.agreement.map_or_else(|| "n/a".into(), |v| v.to_string())); }
        }
        "promote" => {
            let kind = args.first().and_then(|s| DecisionKind::parse(s)).ok_or_else(|| usage("unknown promotion type"))?;
            let m = flags(&args[1..], &["--repo", "--confirm", "--date"])?;
            if required(&m, "--confirm")? != kind.as_str() { return Err(usage("confirmation type must match")); }
            let repo = PathBuf::from(required(&m, "--repo")?); let date = m.get("--date").cloned().unwrap_or_else(crate::base::today);
            let hash = model_hash(home)?;
            let path = crate::laya::laya_dir(home).join("eval").join(format!("{hash}-acceptance.json"));
            let report: EvalReport = serde_json::from_slice(&fs::read(path).map_err(io)?).map_err(io)?;
            if report.model_hash != hash { return Err(CliError::Failure(Failure::new("model_mismatch", "acceptance report model hash differs"))); }
            let (t, _) = shadow::load_thresholds(None)?;
            let confirmation = Confirmation { kind, model_hash: hash };
            let state = shadow::promote(home, &repo, &[kind], &report, &t, Some(&confirmation), &date)?;
            println!("branch: laya/{}-{date}\n{}", kind.as_str(), serde_json::to_string_pretty(&state).map_err(io)?);
        }
        "run" => {
            if args != ["--dry-run"] { return Err(usage("real runs need curate, bundle, eval and promote separately")); }
            println!("{}", dry_run_json(home)?);
        }
        _ => return Err(usage("unknown subcommand\nusage: axiom laya status|shadow|kill|unkill|curate|bundle|eval|promote|run")),
    }
    Ok(())
}
pub fn cli(args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str).unwrap_or("");
    if ![
        "status", "shadow", "kill", "unkill", "curate", "bundle", "eval", "promote", "run",
    ]
    .contains(&sub)
    {
        eprintln!("axiom: laya: unknown subcommand\nusage: axiom laya status|shadow|kill|unkill|curate|bundle|eval|promote|run");
        return 2;
    }
    let Some(home) = std::env::var_os("AXIOM_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        eprintln!("axiom: laya: AXIOM_DATA_HOME required");
        return 2;
    };
    match cli_inner(&home, sub, &args[1..]) {
        Ok(()) => 0,
        Err(CliError::Usage(e)) => {
            eprintln!("axiom: laya: {e}");
            2
        }
        Err(CliError::Failure(e)) => {
            eprintln!("axiom: laya: {e}");
            1
        }
    }
}
