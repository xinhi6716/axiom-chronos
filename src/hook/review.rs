//! PostToolUse on a dispatcher's `Agent` call to `chronos:reviewer`: records the bound unit's verdict
//! through the unchanged `orch review` command and returns the bounded outcome as added context.
use crate::{
    base, goal,
    orch::{
        self,
        summary::{self, Next, Outcome},
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const REVIEWER: &str = "chronos:reviewer";

fn reply(text: String) -> Option<Value> {
    Some(json!({"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":text}}))
}

fn key_of(id: &str) -> Option<String> {
    let key: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    (!key.is_empty()).then_some(key)
}

fn bound_unit(store: &Path, key: &str) -> Option<String> {
    let unit =
        std::fs::read_to_string(store.join("agents").join(format!("{key}.reviewer"))).ok()?;
    let unit = unit.trim();
    (!unit.is_empty()).then(|| unit.to_string())
}

/// The single `<unit>.review.md` path named in the prompt, when some reviewer marker is bound to that unit.
fn prompt_unit(store: &Path, prompt: &str) -> Option<String> {
    let prefix = store.join("reports").join("");
    let pattern = format!(
        r"{}([a-z0-9][a-z0-9_-]*)\.review\.md",
        regex::escape(&prefix.to_string_lossy())
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let mut units: Vec<String> = Vec::new();
    let delimiter = |c: char| c.is_whitespace() || "\"'`()[]<>,;!?:".contains(c);
    for capture in re.captures_iter(prompt) {
        let path = capture.get(0)?;
        let rest = &prompt[path.end()..];
        let trailing_period = rest
            .strip_prefix('.')
            .is_some_and(|r| r.chars().next().map_or(true, delimiter));
        if prompt[..path.start()]
            .chars()
            .next_back()
            .is_some_and(|c| !delimiter(c))
            || (!trailing_period && rest.chars().next().is_some_and(|c| !delimiter(c)))
        {
            continue;
        }
        let unit = capture.get(1)?.as_str();
        if base::require_slug("unit", unit).is_ok() && !units.iter().any(|u| u == unit) {
            units.push(unit.to_string());
        }
    }
    if units.len() != 1 {
        return None;
    }
    let bound = std::fs::read_dir(store.join("agents"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".reviewer"))
        .any(|e| std::fs::read_to_string(e.path()).is_ok_and(|s| s.trim() == units[0]));
    bound.then(|| units.remove(0))
}

fn failure_text(unit: Option<&str>, error: &str) -> String {
    let line = error
        .lines()
        .map(|l| l.trim().trim_start_matches("error:").trim())
        .find(|l| !l.is_empty())
        .unwrap_or("unknown error");
    let line = summary::one_line(line);
    let what = unit.map_or("the review".to_string(), |u| format!("the review of {u}"));
    format!("Chronos did not record {what}: {line}. Call orch review yourself after fixing it.")
}

fn record(store: &Path, unit: &str, repo: &Path) -> Result<String, String> {
    let args: Vec<String> = [
        "review",
        unit,
        "--repo",
        &repo.to_string_lossy(),
        "--reviewer",
        REVIEWER,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let out = orch::run(store, &args, true, false);
    if out.exit_code != 0 {
        return Err(failure_text(Some(unit), &out.stderr));
    }
    let state = serde_json::from_str::<Value>(&out.stdout)
        .ok()
        .and_then(|v| {
            v["states"]
                .as_array()?
                .iter()
                .find(|r| r[0] == unit)
                .and_then(|r| r[1].as_str().map(str::to_string))
        })
        .unwrap_or_default();
    outcome_text(store, unit, state)
}

fn outcome_text(store: &Path, unit: &str, state: String) -> Result<String, String> {
    let verdict = orch::read_verdict(store, unit)
        .map_err(|e| failure_text(Some(unit), &e.message))?
        .ok_or_else(|| failure_text(Some(unit), "verdict file is missing"))?;
    let units = orch::read_units(store).map_err(|e| failure_text(Some(unit), &e.message))?;
    let this = units.iter().find(|u| u.id == unit);
    let state = if state.is_empty() {
        this.map(|u| u.state.clone()).unwrap_or_default()
    } else {
        state
    };
    let standing: Vec<_> = this
        .map(|t| {
            units
                .iter()
                .filter(|u| u.track == t.track && u.superseded_by().is_none())
                .collect()
        })
        .unwrap_or_default();
    let done = !standing.is_empty() && standing.iter().all(|u| u.state == "verified");
    let reports = store.join("reports");
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    paths.insert("pack".into(), orch::pack_path(store, unit));
    paths.insert("verdict".into(), orch::verdict_path(store, unit));
    paths.insert("report".into(), reports.join(format!("{unit}.json")));
    let outcome = Outcome {
        unit: unit.to_string(),
        state,
        next: summary::next_for(store, unit).unwrap_or(Next::Wait),
        reason: format!("review recorded: {}", verdict.verdict),
        verdict: Some(summary::verdict_counts(&verdict)),
        review: Vec::new(),
        paths,
        module_done: Some(done),
    };
    let p = |k: &str| outcome.paths[k].display().to_string();
    let mut text = format!(
        "Chronos recorded the review.\n{}\npaths: pack={} verdict={} report={}",
        outcome.line(),
        p("pack"),
        p("verdict"),
        p("report")
    );
    if done {
        let module = this.map(|u| u.track.as_str()).unwrap_or("");
        let list: Vec<_> = standing
            .iter()
            .map(|u| format!("{}={}", u.id, u.state))
            .collect();
        text.push_str(&format!("\nmodule {module} done: {}", list.join(", ")));
    }
    let serious: Vec<_> = verdict.findings.iter().filter(|f| f.is_serious()).collect();
    if !serious.is_empty() {
        text.push_str("\nblocker and major findings:");
        for f in serious {
            let severity = match f.severity {
                orch::Severity::Blocker => "blocker",
                _ => "major",
            };
            let finding = format!(
                "#{} {severity} {} {}: {}",
                f.number, f.check, f.location, f.evidence
            );
            let mut chars = finding.lines().next().unwrap_or("").chars();
            let mut cut: String = chars.by_ref().take(240).collect();
            if chars.next().is_some() {
                cut.push('…');
            }
            text.push_str(&format!("\n  {cut}"));
        }
    }
    Ok(text)
}

pub fn after_agent(event: &Value, home: Option<&Path>) -> Option<Value> {
    if !matches!(
        event.get("tool_name").and_then(Value::as_str),
        Some("Agent" | "Task")
    ) {
        return None;
    }
    let input = event.get("tool_input")?;
    if input.get("subagent_type").and_then(Value::as_str) != Some(REVIEWER) {
        return None;
    }
    let home = home?;
    let session = event
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let g = goal::load(home, session).ok().flatten()?;
    if g.status != "active" {
        return None;
    }
    let store = orch::store_for(home, g.project.as_deref()?).ok()?;
    let unit = match event.pointer("/tool_response/agentId") {
        Some(id) => id
            .as_str()
            .and_then(key_of)
            .and_then(|key| bound_unit(&store, &key)),
        None => prompt_unit(
            &store,
            input.get("prompt").and_then(Value::as_str).unwrap_or(""),
        ),
    };
    let Some(unit) = unit else {
        return reply(failure_text(
            None,
            "no unit is bound to this reviewer; it never read a review pack",
        ));
    };
    let repo = g
        .cwd
        .clone()
        .or_else(|| event.get("cwd").and_then(Value::as_str).map(str::to_string));
    let Some(repo) = repo else {
        return reply(failure_text(Some(&unit), "no project directory is known"));
    };
    reply(match record(&store, &unit, Path::new(&repo)) {
        Ok(text) | Err(text) => text,
    })
}
