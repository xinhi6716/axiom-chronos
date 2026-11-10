use crate::{
    base,
    orch::{self, Evidence, Map},
};
use serde::Serialize;
use std::path::{Path, PathBuf};

const UNITS: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief";
const CLAIMS: &str = "unit\tattempt\tscope\tstarted";
const EVIDENCE: &str = "kind\tcommand\texit_code\tcommit\tts";

#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    pub attempt: String,
    pub started: String,
    pub scope: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct UnitView {
    pub id: String,
    pub track: String,
    pub state: String,
    pub derived: String,
    pub attempts: usize,
    pub claim: Option<Claim>,
    pub brief: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub report_json: Option<PathBuf>,
    pub verify_output: Option<PathBuf>,
    pub review: Option<Evidence>,
    pub evidence: Vec<Evidence>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: usize,
    pub verified: usize,
    pub failed: usize,
    pub in_progress: usize,
    pub unreviewed: usize,
    pub pending: usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModuleView {
    pub id: String,
    pub purpose: String,
    pub paths: Vec<String>,
    pub depends: Vec<String>,
    pub accept_command: String,
    pub state: String,
    pub accepted: Option<bool>,
    pub counts: Counts,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AstraFiles {
    pub plan: bool,
    pub review: bool,
    pub ruling: bool,
    pub plan_final: bool,
}
/// One item of the final plan and the modules whose map section serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GoalView {
    pub id: String,
    pub title: String,
    pub modules: Vec<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct View {
    pub goals: Vec<GoalView>,
    pub map: Option<Map>,
    pub map_error: Option<String>,
    pub units: Vec<UnitView>,
    pub modules: Vec<ModuleView>,
    pub counts: Counts,
    pub stages: Vec<Evidence>,
    pub full: Vec<Evidence>,
    pub decisions: Vec<String>,
    pub astra: AstraFiles,
    pub verdict: Option<String>,
}

fn read(path: &Path, label: &str, warnings: &mut Vec<String>) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).replace('\r', "")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warnings.push(format!("{label}: {e}"));
            None
        }
    }
}
fn tsv(
    path: &Path,
    label: &str,
    header: &str,
    cols: usize,
    warnings: &mut Vec<String>,
) -> Vec<Vec<String>> {
    let Some(text) = read(path, label, warnings) else {
        return vec![];
    };
    let mut lines = text.split('\n');
    if lines.next() != Some(header) {
        warnings.push(format!("{label}: unexpected header"));
    }
    let mut rows = Vec::new();
    let mut bad = 0;
    for line in lines.filter(|l| !l.is_empty()) {
        let c: Vec<String> = line.split('\t').map(str::to_owned).collect();
        if c.len() == cols {
            rows.push(c);
        } else {
            bad += 1;
        }
    }
    if bad > 0 {
        warnings.push(format!("{label}: {bad} malformed row(s) skipped"));
    }
    rows
}
fn evidence(path: &Path, label: &str, warnings: &mut Vec<String>) -> Vec<Evidence> {
    let mut bad = 0;
    let rows = tsv(path, label, EVIDENCE, 5, warnings)
        .into_iter()
        .filter_map(|c| {
            let Ok(exit_code) = c[2].parse() else {
                bad += 1;
                return None;
            };
            Some(Evidence {
                kind: c[0].clone(),
                command: c[1].clone(),
                exit_code,
                commit: c[3].clone(),
                ts: c[4].clone(),
            })
        })
        .collect();
    if bad > 0 {
        warnings.push(format!("{label}: {bad} row(s) with an invalid exit code skipped"));
    }
    rows
}
fn nonempty(path: PathBuf) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() > 0)
}
fn count<'a>(units: impl Iterator<Item = &'a UnitView>) -> Counts {
    let mut c = Counts::default();
    for u in units {
        c.total += 1;
        if u.claim.is_some() {
            c.in_progress += 1;
            continue;
        }
        match u.state.as_str() {
            "verified" => c.verified += 1,
            "failed" | "accept-failed" | "review-failed" => c.failed += 1,
            "built" => c.unreviewed += 1,
            _ => c.pending += 1,
        }
    }
    c
}
pub(crate) fn rollup(c: &Counts) -> &'static str {
    if c.total == 0 {
        "empty"
    } else if c.verified == c.total {
        "verified"
    } else if c.in_progress > 0 {
        "running"
    } else if c.failed > 0 {
        "failed"
    } else if c.pending == 0 {
        "built"
    } else {
        "pending"
    }
}
pub(crate) fn view(store: &Path, warnings: &mut Vec<String>) -> Option<View> {
    if !store.is_dir() {
        return None;
    }
    let name = store
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = |file: &str| format!("{name}/{file}");
    let (map, map_error) = match read(&store.join("modules.md"), &label("modules.md"), warnings) {
        Some(text) => match orch::parse_map(&text) {
            Ok(map) => (Some(map), None),
            Err(e) => {
                warnings.push(format!("{}: {e}", label("modules.md")));
                (None, Some(e))
            }
        },
        None => (None, None),
    };
    let claims: Vec<(String, Claim)> = tsv(
        &store.join("claims.tsv"),
        &label("claims.tsv"),
        CLAIMS,
        4,
        warnings,
    )
    .into_iter()
    .map(|c| {
        (
            c[0].clone(),
            Claim {
                attempt: c[1].clone(),
                started: c[3].clone(),
                scope: c[2].split_whitespace().map(str::to_owned).collect(),
            },
        )
    })
    .collect();
    let reports = store.join("reports");
    let units: Vec<UnitView> = tsv(
        &store.join("units.tsv"),
        &label("units.tsv"),
        UNITS,
        7,
        warnings,
    )
    .into_iter()
    .map(|c| {
        let id = c[0].clone();
        let valid = base::require_slug("unit", &id).is_ok();
        let rows = if valid {
            evidence(
                &reports.join(format!("{id}.evidence.tsv")),
                &label(&format!("reports/{id}.evidence.tsv")),
                warnings,
            )
        } else {
            warnings.push(format!("{}: invalid unit id {id:?}", label("units.tsv")));
            vec![]
        };
        let needs_accept = map
            .as_ref()
            .is_some_and(|m| m.modules.iter().any(|x| x.id == c[1]));
        let file = |p: PathBuf| (valid && p.is_file()).then_some(p);
        UnitView {
            derived: orch::derive_state(&rows, needs_accept),
            attempts: rows.iter().filter(|r| r.kind == "run").count(),
            claim: claims
                .iter()
                .find(|(u, _)| *u == id)
                .map(|(_, claim)| claim.clone()),
            brief: if c[6].is_empty() {
                file(store.join("briefs").join(format!("{id}.md")))
            } else {
                Some(PathBuf::from(&c[6]))
            },
            report: file(reports.join(format!("{id}.md"))),
            report_json: file(reports.join(format!("{id}.json"))),
            verify_output: file(reports.join(format!("{id}.verify.txt"))),
            review: rows.iter().rev().find(|r| r.kind == "review").cloned(),
            evidence: rows,
            track: c[1].clone(),
            state: c[2].clone(),
            id,
        }
    })
    .collect();
    let counts = count(units.iter());
    let modules = map
        .as_ref()
        .map(|m| {
            m.modules
                .iter()
                .map(|module| {
                    let mine: Vec<&UnitView> =
                        units.iter().filter(|u| u.track == module.id).collect();
                    let counts = count(mine.iter().copied());
                    let accepted = mine
                        .iter()
                        .flat_map(|u| u.evidence.iter())
                        .filter(|r| r.kind == "accept")
                        .max_by(|a, b| a.ts.cmp(&b.ts))
                        .map(|r| r.exit_code == 0);
                    ModuleView {
                        id: module.id.clone(),
                        purpose: module.purpose.clone(),
                        paths: module.paths.clone(),
                        depends: module.depends.clone(),
                        accept_command: module.accept_command.clone(),
                        state: rollup(&counts).into(),
                        accepted,
                        counts,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let stages = evidence(
        &reports.join("_stages.evidence.tsv"),
        &label("reports/_stages.evidence.tsv"),
        warnings,
    );
    let full = evidence(
        &reports.join("_full.evidence.tsv"),
        &label("reports/_full.evidence.tsv"),
        warnings,
    );
    let decisions = read(&store.join("decisions.md"), &label("decisions.md"), warnings)
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(200)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let astra_dir = store.join("astra");
    let review = read(
        &astra_dir.join("review.json"),
        &label("astra/review.json"),
        warnings,
    )
    .filter(|s| !s.trim().is_empty());
    let verdict = review
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v["verdict"].as_str().map(str::to_owned));
    let astra = AstraFiles {
        plan: astra_dir.join("plan.json").is_file(),
        review: review.is_some(),
        ruling: nonempty(astra_dir.join("ruling.md")),
        plan_final: nonempty(astra_dir.join("plan-final.md")),
    };
    let goals = read(
        &astra_dir.join("plan-final.md"),
        &label("astra/plan-final.md"),
        warnings,
    )
    .map(|t| orch::parse_plan(&t))
    .unwrap_or_default()
    .into_iter()
    .map(|item| GoalView {
        modules: map
            .as_ref()
            .map(|m| {
                m.modules
                    .iter()
                    .filter(|x| x.serves.contains(&item.id))
                    .map(|x| x.id.clone())
                    .collect()
            })
            .unwrap_or_default(),
        id: item.id,
        title: item.title,
    })
    .collect();
    Some(View {
        goals,
        map,
        map_error,
        units,
        modules,
        counts,
        stages,
        full,
        decisions,
        astra,
        verdict,
    })
}
