#![allow(dead_code)]

use axiom::desktop::{
    model::Screen,
    targets::{Opening, Target},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

pub fn source_id(repo: &Path) -> Result<String, String> {
    const PATHS: [&str; 5] = ["src", "assets", "Cargo.lock", "Cargo.toml", "build.rs"];
    let mut status_args = vec!["status", "--porcelain", "--"];
    status_args.extend(PATHS);
    if !git(repo, &status_args)?.is_empty() {
        return Err(format!("uncommitted changes in {}", PATHS.join(", ")));
    }
    let output = git(
        repo,
        &[
            "rev-parse",
            "HEAD:src",
            "HEAD:assets",
            "HEAD:Cargo.lock",
            "HEAD:Cargo.toml",
            "HEAD:build.rs",
        ],
    )?;
    Ok(format!("{:x}", Sha256::digest(output)))
}

pub fn commit(repo: &Path) -> String {
    String::from_utf8(git(repo, &["rev-parse", "HEAD"]).expect("git rev-parse HEAD failed"))
        .expect("commit is not UTF-8")
        .trim()
        .to_owned()
}

pub fn release_exe(name: &str) -> Result<PathBuf, String> {
    let path = repo()
        .join("target")
        .join("release")
        .join(format!("{name}.exe"));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("release executable missing: {}", path.display()))
    }
}

pub fn isolated_home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("axiom-release-")
        .tempdir_in(std::env::temp_dir())
        .expect("create isolated home in system temporary directory")
}

#[derive(Clone, Debug)]
pub struct View {
    pub id: String,
    pub group: String,
    pub opening: String,
    pub panels: Value,
    pub actions: Value,
    pub states: Value,
    pub animated: bool,
    pub live: bool,
    pub gates: Value,
}

#[derive(Clone, Debug)]
pub struct Matrix {
    pub version: Value,
    pub budgets: Value,
    pub capture: Value,
    pub layout: Value,
    pub views: Vec<View>,
    pub gates: Value,
    pub global_gates: Value,
}

impl Matrix {
    pub fn from_value(value: Value) -> Result<Self, String> {
        fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
            v.get(key).ok_or_else(|| format!("missing {key}"))
        }
        fn string(v: &Value, key: &str) -> Result<String, String> {
            field(v, key)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must be a string"))
        }
        fn boolean(v: &Value, key: &str) -> Result<bool, String> {
            field(v, key)?
                .as_bool()
                .ok_or_else(|| format!("{key} must be a boolean"))
        }
        let views = field(&value, "views")?
            .as_array()
            .ok_or("views must be an array")?
            .iter()
            .map(|v| {
                Ok(View {
                    id: string(v, "id")?,
                    group: string(v, "group")?,
                    opening: string(v, "opening")?,
                    panels: field(v, "panels")?.clone(),
                    actions: field(v, "actions")?.clone(),
                    states: field(v, "states")?.clone(),
                    animated: boolean(v, "animated")?,
                    live: boolean(v, "live")?,
                    gates: field(v, "gates")?.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            version: value.get("version").cloned().unwrap_or(Value::Null),
            budgets: field(&value, "budgets")?.clone(),
            capture: field(&value, "capture")?.clone(),
            layout: field(&value, "layout")?.clone(),
            views,
            gates: field(&value, "gates")?.clone(),
            global_gates: value.get("global_gates").cloned().unwrap_or(Value::Null),
        })
    }
}

pub fn load_matrix() -> Matrix {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("release-gates.json");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("invalid JSON in {}: {e}", path.display()));
    Matrix::from_value(value)
        .unwrap_or_else(|e| panic!("invalid matrix in {}: {e}", path.display()))
}

#[derive(Debug, PartialEq)]
pub enum GateResult {
    Pass,
    Fail(String),
    Unknown,
}
pub fn counts_as_pass(result: &GateResult) -> bool {
    matches!(result, GateResult::Pass)
}
pub fn fresh(evidence_source_id: &str, current: &str) -> bool {
    !current.is_empty() && evidence_source_id == current
}

pub fn opening_name(opening: &Opening) -> String {
    match opening {
        Opening::Screen(screen) => format!(
            "screen:{}",
            match screen {
                Screen::Run => "run",
                Screen::Wishes => "wishes",
                Screen::Memory => "memory",
                Screen::Usage => "usage",
                Screen::Doctor => "doctor",
            }
        ),
        Opening::Page(name) => format!("page:{name}"),
        Opening::ResearchTab(name) => format!("research_tab:{name}"),
        Opening::PaperStep(name) => format!("paper_step:{name}"),
        Opening::Window(name) => format!("window:{name}"),
        Opening::Overlay(name) => format!("overlay:{name}"),
        Opening::Dialog(name) => format!("dialog:{name}"),
    }
}

pub fn budgets() -> Value {
    json!({"frame_p95_ms":8.3,"frame_p99_ms":16.7,"frame_max_ms":33,"event_to_pixel_ms":250,
        "idle_cpu_pct_one_core":0.5,"idle_window_s":60,"cold_start_p95_s":2.0,"idle_private_mib":512,
        "large_extra_peak_mib":512,"exe_mib":150,"large_rows":60000})
}

pub fn animated(id: &str) -> bool {
    matches!(
        id,
        "run"
            | "wishes"
            | "memory"
            | "usage"
            | "settings"
            | "research.map"
            | "research.sources"
            | "research.evidence"
            | "research.dataset"
            | "plaza"
            | "chat"
            | "session"
            | "paper.progress"
            | "paper.changelog"
            | "mini"
            | "focus.graph"
    )
}
pub fn live(id: &str) -> bool {
    matches!(
        id,
        "run"
            | "wishes"
            | "usage"
            | "session"
            | "chat"
            | "plaza"
            | "paper.progress"
            | "mini"
            | "focus.graph"
            | "focus.checklist"
    )
}

pub fn validate(matrix: &Matrix, registry: &[Target]) -> Vec<String> {
    let mut problems = Vec::new();
    if matrix.version != json!(1) {
        problems.push("missing or incorrect version".into());
    }
    for (key, expected) in budgets().as_object().unwrap() {
        if matrix.budgets.get(key) != Some(expected) {
            problems.push(format!("budget {key} differs"));
        }
    }
    let capture = json!({"langs":["zh-TW","en"],"themes":["dark","light"],"sizes":["1100x700","1920x1200"],"states":["live","empty","error","action"]});
    let layout =
        json!({"sizes":["800x600","1100x700","1920x1200","3440x1440"],"dpi":[100,150,200]});
    if matrix.capture != capture {
        problems.push("capture configuration differs".into());
    }
    if matrix.layout != layout {
        problems.push("layout configuration differs".into());
    }
    let methods = [
        ("png", "captures/png/manifest.json"),
        ("diff", "captures/png/manifest.json"),
        ("layout", "layout_matrix <group>::"),
        ("fault", "fault_matrix <group>::"),
        ("tokens", "desktop_architecture tokens::<group>"),
        ("deps", "desktop_architecture deps::<group>"),
        ("frame", "perf/results.json"),
        ("event_to_pixel", "perf/results.json"),
        ("idle_cpu", "perf/results.json"),
        ("cold_start", "perf/results.json"),
        ("memory", "perf/results.json"),
        ("exe_size", "perf/results.json"),
    ];
    for (gate, method) in methods {
        if matrix
            .gates
            .get(gate)
            .and_then(|v| v.get("method"))
            .and_then(Value::as_str)
            != Some(method)
        {
            problems.push(format!("gate {gate} missing or incorrect method"));
        }
    }
    for gate in ["idle_cpu", "cold_start", "memory", "exe_size"] {
        if !contains(&matrix.global_gates, gate) {
            problems.push(format!("missing global gate {gate}"));
        }
    }
    let mut seen = HashSet::new();
    for view in &matrix.views {
        if !seen.insert(view.id.as_str()) {
            problems.push(format!("duplicate view {}", view.id));
        }
        let Some(target) = registry.iter().find(|t| t.id == view.id) else {
            problems.push(format!("unregistered view {}", view.id));
            continue;
        };
        if view.group != target.group {
            problems.push(format!("{} group differs", view.id));
        }
        if view.opening != opening_name(&target.opening) {
            problems.push(format!("{} opening differs", view.id));
        }
        if view.panels != json!(target.panels) {
            problems.push(format!("{} panels differ", view.id));
        }
        if view.actions != json!(target.actions) {
            problems.push(format!("{} actions differ", view.id));
        }
        if view.states != json!(target.states.iter().map(|s| s.key()).collect::<Vec<_>>()) {
            problems.push(format!("{} states differ", view.id));
        }
        if view.animated != animated(&view.id) {
            problems.push(format!("{} animated differs", view.id));
        }
        if view.live != live(&view.id) {
            problems.push(format!("{} live differs", view.id));
        }
        for gate in ["png", "diff", "layout", "fault", "tokens", "deps"]
            .into_iter()
            .chain(view.animated.then_some("frame"))
            .chain(view.live.then_some("event_to_pixel"))
        {
            if !contains(&view.gates, gate) {
                problems.push(format!("{} missing gate {gate}", view.id));
            }
        }
        if let Some(gates) = view.gates.as_array() {
            for gate in gates {
                if gate
                    .as_str()
                    .and_then(|name| matrix.gates.get(name))
                    .and_then(|v| v.get("method"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .is_none()
                {
                    problems.push(format!("{} gate {gate} has no method", view.id));
                }
            }
        } else {
            problems.push(format!("{} gates must be an array", view.id));
        }
    }
    for target in registry {
        if !seen.contains(target.id) {
            problems.push(format!("registry target {} missing from matrix", target.id));
        }
    }
    problems
}
fn contains(value: &Value, item: &str) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(item)))
}
