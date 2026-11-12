//! Frozen graph A/B measurement harness.
use crate::{
    base::{Failure, Res},
    graph::{
        constraints, feedback,
        ingest::{import_source, SourceLink},
        query::{explore, ExploreRequest, View},
        Direction, GraphStore,
    },
    memory::{Memory, Record},
    orch,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub version: u32,
    pub frozen: String,
    pub design: Design,
    pub metric: Metric,
    pub thresholds: Thresholds,
    pub scenarios: Vec<Scenario>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Design {
    pub arms: Vec<String>,
    pub runs_per_arm: u8,
    #[serde(rename = "A")]
    pub a: String,
    #[serde(rename = "B")]
    pub b: String,
    pub fixed: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Metric {
    pub claude_processed_input: Vec<String>,
    pub aggregate: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Thresholds {
    pub claude_processed_input_median_saving_min: f64,
    pub required_evidence_regressions_max: usize,
    pub condition_regressions_max: usize,
    pub major_finding_regressions_max: usize,
    pub wrong_certain_edges_max: u64,
    pub budget_overruns_max: usize,
    pub resolution_recall_min: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scenario {
    pub id: String,
    pub family: String,
    pub language: String,
    pub repo: String,
    pub seed: String,
    pub task: String,
    pub query: String,
    pub a_brief: String,
    pub required_evidence: Vec<String>,
    pub conditions: Vec<String>,
    pub major_findings: Vec<String>,
    pub budget: usize,
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
pub fn load_manifest(dir: &Path) -> Res<Manifest> {
    serde_json::from_slice(&fs::read(dir.join("manifest.json")).map_err(err)?).map_err(err)
}
#[derive(Deserialize)]
struct Seed {
    project: String,
    entries: Vec<Entry>,
    research: Vec<Research>,
    constraints: String,
    aliases: Vec<(String, String)>,
    amendments: Vec<String>,
}
#[derive(Deserialize)]
struct Entry {
    unit: String,
    title: String,
    summary: String,
    body: String,
    symbols: Vec<String>,
    files: Vec<String>,
    wish: String,
}
#[derive(Deserialize)]
struct Research {
    path: String,
    text: String,
    links: Vec<SourceLink>,
}
fn put(path: &Path, text: &str) -> Res<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(err)?;
    }
    fs::write(path, text).map_err(err)
}
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}
pub fn seed_home(home: &Path, project_dir: &Path, scenario_dir: &Path, seed: &Path) -> Res<()> {
    let path = if seed.is_absolute() {
        seed.to_path_buf()
    } else {
        scenario_dir.join(seed)
    };
    let seed: Seed = serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)?;
    let project_store = orch::wish_store(home, &seed.project, None)?;
    let wish_id = "w-1700000000001-abcdef";
    let wish_store = orch::wish_store(home, &seed.project, Some(wish_id))?;
    let goal = json!({"wish": "graph evaluation", "wish_id": wish_id, "project": seed.project, "cwd": project_dir, "status":"active", "amendments": seed.amendments});
    put(&home.join("goals/graph-eval.json"), &goal.to_string())?;
    let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for entry in &seed.entries {
        let brief = wish_store.join("briefs").join(format!("{}.md", entry.unit));
        put(
            &brief,
            &format!(
                "GOAL {}\nSCOPE {}\nVERIFY fixture\n",
                entry.title,
                entry.files.join(" ")
            ),
        )?;
        rows.push_str(&format!(
            "{}\tbuild\tverified\t\t\t\t{}\n",
            entry.unit,
            brief.display()
        ));
    }
    put(&wish_store.join("units.tsv"), &rows)?;
    put(&project_dir.join("CONSTRAINTS.md"), &seed.constraints)?;
    let mut aliases = String::from("canonical,alias\n");
    for (canonical, alias) in &seed.aliases {
        aliases.push_str(&format!("{},{}\n", csv_field(canonical), csv_field(alias)));
    }
    put(&project_store.join("aliases.csv"), &aliases)?;
    let memory = Memory::open(home)?;
    for entry in seed.entries {
        memory.record(&Record {
            project: seed.project.clone(),
            project_dir: project_dir.to_string_lossy().into_owned(),
            unit: entry.unit,
            title: entry.title,
            summary: entry.summary,
            body: entry.body,
            symbols: entry.symbols,
            files: entry.files,
            wish: entry.wish,
        })?;
    }
    let store = GraphStore::open(home)?;
    if !seed.research.is_empty() {
        crate::graph::code::sync(&store, project_dir, None, &Default::default())?;
    }
    feedback::ingest(&store, home, &seed.project, project_dir)?;
    for research in seed.research {
        if Path::new(&research.path)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(Failure::new(
                "invalid_argument",
                "research path must be relative to the project",
            ));
        }
        put(&project_dir.join(&research.path), &research.text)?;
        import_source(&store, project_dir, &research.path, &research.links)?;
    }
    Ok(())
}
fn project_for(home: &Path, project_dir: &Path) -> Res<Option<String>> {
    let goals = match fs::read_dir(home.join("goals")) {
        Ok(g) => g,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(e)),
    };
    for g in goals {
        let p = g.map_err(err)?.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let v: Value = serde_json::from_slice(&fs::read(p).map_err(err)?).map_err(err)?;
        if v.get("cwd").and_then(Value::as_str).map(Path::new) == Some(project_dir) {
            return Ok(v.get("project").and_then(Value::as_str).map(str::to_owned));
        }
    }
    Ok(None)
}
pub fn graph_brief(home: &Path, project_dir: &Path, s: &Scenario) -> Res<String> {
    let project = project_for(home, project_dir)?;
    let store = project
        .as_ref()
        .map(|p| orch::wish_store(home, p, None))
        .transpose()?;
    let block = constraints::load(project_dir, store.as_deref(), constraints::DEFAULT_BUDGET)?;
    let request = ExploreRequest {
        project_dir: project_dir.to_path_buf(),
        worktree: None,
        project,
        query: Some(s.query.clone()),
        node: None,
        view: View::Mixed,
        direction: Direction::Both,
        hops: 2,
        budget: s.budget,
        refresh: false,
    };
    let payload = serde_json::to_string_pretty(&explore(home, &request)?).map_err(err)?;
    Ok(format!("{}\n\n{}", block.text, payload))
}
#[derive(Serialize)]
pub struct PayloadCheck {
    pub scenario: String,
    pub a_bytes: usize,
    pub b_bytes: usize,
    pub budget_overrun: bool,
    pub evidence_regressions: Vec<String>,
    pub condition_regressions: Vec<String>,
    pub finding_regressions: Vec<String>,
}
pub fn payload_check(s: &Scenario, a: &str, b: &str, explore_pretty_len: usize) -> PayloadCheck {
    let missing = |items: &[String]| {
        items
            .iter()
            .filter(|x| a.contains(x.as_str()) && !b.contains(x.as_str()))
            .cloned()
            .collect()
    };
    PayloadCheck {
        scenario: s.id.clone(),
        a_bytes: a.len(),
        b_bytes: b.len(),
        budget_overrun: explore_pretty_len > s.budget,
        evidence_regressions: missing(&s.required_evidence),
        condition_regressions: missing(&s.conditions),
        finding_regressions: missing(&s.major_findings),
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ClaudeUsage {
    pub input: Option<u64>,
    pub cache_write: Option<u64>,
    pub cache_read: Option<u64>,
    pub output: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RunUsage {
    pub claude: ClaudeUsage,
    pub codex: Option<ClaudeUsage>,
}
pub fn processed_input(u: &ClaudeUsage) -> Option<u64> {
    u.input?
        .checked_add(u.cache_write?)?
        .checked_add(u.cache_read?)
}
#[derive(Serialize)]
pub struct AbReport {
    pub status: String,
    pub reason: Option<String>,
    pub scenarios: Vec<Value>,
    pub median_saving: Option<f64>,
    pub thresholds_met: Option<bool>,
    pub classes: Value,
    pub payload: Vec<PayloadCheck>,
    pub byte_delta_note: String,
    pub wrong_certain_edges: Option<u64>,
    pub budget_overruns: usize,
}
fn median(mut x: Vec<f64>) -> Option<f64> {
    if x.is_empty() {
        return None;
    }
    x.sort_by(f64::total_cmp);
    let n = x.len();
    Some(if n % 2 == 0 {
        (x[n / 2 - 1] + x[n / 2]) / 2.0
    } else {
        x[n / 2]
    })
}
fn class_totals<'a>(items: impl Iterator<Item = Option<&'a ClaudeUsage>>) -> Value {
    let mut sums = [0u64; 4];
    let mut unknown = [0usize; 4];
    for item in items {
        let values = item
            .map(|u| [u.input, u.cache_write, u.cache_read, u.output])
            .unwrap_or([None; 4]);
        for (i, v) in values.into_iter().enumerate() {
            if let Some(v) = v {
                sums[i] = sums[i].saturating_add(v);
            } else {
                unknown[i] += 1;
            }
        }
    }
    json!({"input": sums[0], "cache_write": sums[1], "cache_read": sums[2], "output": sums[3], "unknown": {"input":unknown[0], "cache_write":unknown[1], "cache_read":unknown[2], "output":unknown[3]}})
}
pub fn evaluate(
    m: &Manifest,
    payload: Vec<PayloadCheck>,
    runs: &BTreeMap<(String, String, u8), RunUsage>,
    wrong_certain_edges: Option<u64>,
) -> AbReport {
    let mut scenarios = Vec::new();
    let mut savings = Vec::new();
    for s in &m.scenarios {
        let arm = |name: &str| -> Option<f64> {
            let vals = (0..3)
                .map(|i| {
                    runs.get(&(s.id.clone(), name.to_owned(), i))
                        .and_then(|r| processed_input(&r.claude))
                        .map(|x| x as f64)
                })
                .collect::<Option<Vec<_>>>()?;
            median(vals)
        };
        let a = arm("A");
        let b = arm("B");
        let saving = match (a, b) {
            (Some(a), Some(b)) if a > 0.0 => Some(1.0 - b / a),
            _ => None,
        };
        if let Some(v) = saving {
            savings.push(v);
        }
        scenarios.push(json!({"scenario":s.id,"a_median":a,"b_median":b,"saving":saving}));
    }
    let status = if savings.is_empty() {
        "not_measured"
    } else if savings.len() == m.scenarios.len() {
        "measured"
    } else {
        "incomplete"
    };
    let median_saving = median(savings);
    let budget_overruns = payload.iter().filter(|p| p.budget_overrun).count();
    let thresholds_met = if status == "measured" {
        wrong_certain_edges.map(|wrong| {
            median_saving.unwrap() >= m.thresholds.claude_processed_input_median_saving_min
                && payload
                    .iter()
                    .map(|p| p.evidence_regressions.len())
                    .sum::<usize>()
                    <= m.thresholds.required_evidence_regressions_max
                && payload
                    .iter()
                    .map(|p| p.condition_regressions.len())
                    .sum::<usize>()
                    <= m.thresholds.condition_regressions_max
                && payload
                    .iter()
                    .map(|p| p.finding_regressions.len())
                    .sum::<usize>()
                    <= m.thresholds.major_finding_regressions_max
                && budget_overruns <= m.thresholds.budget_overruns_max
                && wrong <= m.thresholds.wrong_certain_edges_max
        })
    } else {
        None
    };
    let classes = json!({"A": {"claude": class_totals(runs.iter().filter(|((_,arm,_),_)|arm=="A").map(|(_,r)|Some(&r.claude))), "codex":class_totals(runs.iter().filter(|((_,arm,_),_)|arm=="A").map(|(_,r)|r.codex.as_ref()))}, "B": {"claude": class_totals(runs.iter().filter(|((_,arm,_),_)|arm=="B").map(|(_,r)|Some(&r.claude))), "codex":class_totals(runs.iter().filter(|((_,arm,_),_)|arm=="B").map(|(_,r)|r.codex.as_ref()))}});
    AbReport {
        status: status.into(),
        reason: if status == "not_measured" {
            Some("No scenario has complete, usable Claude input measurements for both arms.".into())
        } else {
            None
        },
        scenarios,
        median_saving,
        thresholds_met,
        classes,
        payload,
        byte_delta_note: "bytes, not tokens".into(),
        wrong_certain_edges,
        budget_overruns,
    }
}
pub fn report_markdown(r: &AbReport) -> String {
    let mut out = format!("# Graph A/B report\n\nStatus: {}\n\n", r.status);
    if let Some(v) = r.median_saving {
        out.push_str(&format!("Median measured saving: {:.2}%\n\n", v * 100.0));
    } else {
        out.push_str(&format!(
            "Not measured: {}\n\n",
            r.reason.as_deref().unwrap_or("No usable measurements")
        ));
    }
    for arm in ["A", "B"] {
        for provider in ["claude", "codex"] {
            let c = &r.classes[arm][provider];
            out.push_str(&format!("{arm} {provider}:\n"));
            for name in ["input", "cache_write", "cache_read", "output"] {
                out.push_str(&format!(
                    "- {name}: {} (unknown: {})\n",
                    c[name], c["unknown"][name]
                ));
            }
            out.push('\n');
        }
    }
    out.push_str("Payload regressions:\n");
    for p in &r.payload {
        out.push_str(&format!(
            "- {}: evidence {:?}; conditions {:?}; findings {:?}; budget overrun {}\n",
            p.scenario,
            p.evidence_regressions,
            p.condition_regressions,
            p.finding_regressions,
            p.budget_overrun
        ));
    }
    out.push_str(&format!("\n{}\n", r.byte_delta_note));
    out
}
