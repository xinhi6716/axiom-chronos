use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: String,
    pub context: Option<String>,
    pub effort: Option<String>,
    pub tier: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    pub text: String,
    #[serde(default)]
    pub contract: Vec<String>,
    #[serde(default)]
    pub test_matrix: Vec<MatrixRow>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixRow {
    pub id: String,
    pub kind: String,
    pub file: String,
    pub case: String,
    pub assertion: String,
    pub command: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    #[serde(default)]
    pub routing: Vec<RouteLine>,
    pub id: String,
    pub purpose: String,
    pub paths: Vec<String>,
    pub depends: Vec<String>,
    pub contract: String,
    pub accept_file: String,
    pub accept_command: String,
    /// Plan item ids this module delivers (`SERVES`); empty in maps written before the field existed.
    #[serde(default)]
    pub serves: Vec<String>,
    #[serde(default)]
    pub new_paths: Vec<String>,
    #[serde(default)]
    pub shared_owner: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteLine {
    pub key: String,
    pub tier: String,
    pub effort: String,
    pub risk: String,
    pub reason: String,
}

/// A plan item the map records as not built: `OUT OF SCOPE <id> <reason>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutOfScope {
    pub id: String,
    pub reason: String,
}
/// One item of `astra/plan-final.md`, a heading `### <id> <title>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Map {
    pub preamble: String,
    pub modules: Vec<Module>,
    pub full_test: Option<String>,
    #[serde(default)]
    pub out_of_scope: Vec<OutOfScope>,
}
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const TIERS: [&str; 3] = ["terra", "sol", "astra"];
const RISKS: [&str; 11] = [
    "trivial", "standard", "state_machine", "concurrency", "migration", "parser", "security",
    "privacy", "cross_module", "prior_failure", "novel",
];
fn is_field(line: &str, name: &str) -> bool {
    line.strip_prefix(name)
        .is_some_and(|s| s.is_empty() || s.starts_with(char::is_whitespace))
}
fn fields(text: &str, names: &[&str]) -> HashMap<String, String> {
    let mut result = HashMap::<String, String>::new();
    let mut active: Option<&str> = None;
    for line in text.lines() {
        let field = names.iter().find(|name| is_field(line, name));
        if let Some(name) = field {
            active = Some(name);
            result
                .entry(name.to_string())
                .or_default()
                .push_str(line[name.len()..].trim());
        } else if line.starts_with(char::is_whitespace) {
            if let Some(name) = active {
                let value = result.get_mut(name).unwrap();
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line.trim());
            }
        } else {
            active = None;
        }
    }
    result
}
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}
fn setting(
    line: &str,
    name: &str,
    allowed: &[&str],
    slot: &mut Option<String>,
) -> Result<(), String> {
    let value = line[name.len()..].trim();
    if slot.is_some() || !allowed.contains(&value) {
        return Err(format!(
            "has invalid {name} {value:?}: give it once, as one of {}",
            allowed.join(", ")
        ));
    }
    *slot = Some(value.into());
    Ok(())
}
fn is_valid_setting(line: &str) -> bool {
    let valid = |name: &str, allowed: &[&str]| {
        is_field(line, name) && allowed.contains(&line[name.len()..].trim())
    };
    valid("EFFORT", &EFFORTS)
        || valid("TIER", &TIERS)
        || valid("TIER", &["luna"])
        || valid("RISK", &RISKS)
}
const SECTION_FIELDS: [&str; 9] = [
    "GOAL",
    "SCOPE",
    "VERIFY",
    "CONTEXT",
    "EFFORT",
    "TIER",
    "RISK",
    "CONTRACT",
    "TEST_MATRIX",
];
/// The complete SCOPE section: its own line and every following line up to the next brief field,
/// bullets and blank lines allowed. Line numbers in errors are 1-based in the whole brief.
fn scope_section(text: &str, from: usize) -> Result<Vec<String>, String> {
    let mut scope = Vec::new();
    let mut first: Option<usize> = None;
    let mut inside = false;
    for (index, line) in text.lines().enumerate().skip(from) {
        let number = index + 1;
        let field = SECTION_FIELDS.iter().find(|name| is_field(line, name));
        if let Some(name) = field {
            inside = false;
            if *name == "SCOPE" {
                if let Some(m) = first {
                    return Err(format!(
                        "has SCOPE twice: line {number} repeats line {m}"
                    ));
                }
                first = Some(number);
                inside = true;
            }
        }
        if !inside {
            continue;
        }
        let mut rest = if field.is_some() {
            &line["SCOPE".len()..]
        } else {
            line
        };
        rest = rest.trim();
        for bullet in ["- ", "* ", "+ "] {
            if let Some(r) = rest.strip_prefix(bullet) {
                rest = r;
                break;
            }
        }
        for token in words(rest) {
            if token.contains(['(', ')', '"', '`']) || token.ends_with(':') {
                return Err(format!(
                    "has SCOPE entry {token:?} on line {number} that is not a path; put notes in CONTEXT"
                ));
            }
            scope.push(token);
        }
    }
    Ok(scope)
}
fn brief_sections(body: &str, scope: &[String]) -> Result<(Vec<String>, Vec<MatrixRow>), String> {
    let (mut contract_seen, mut matrix_seen) = (false, false);
    let mut active = "";
    let mut contract = Vec::new();
    let mut raw_rows = Vec::new();
    for line in body.lines() {
        if is_field(line, "CONTEXT") {
            break;
        }
        let field = SECTION_FIELDS.iter().find(|name| is_field(line, name));
        let value = if let Some(name) = field {
            active = name;
            if *name == "CONTRACT" {
                if contract_seen {
                    return Err("has CONTRACT twice".into());
                }
                contract_seen = true;
            } else if *name == "TEST_MATRIX" {
                if matrix_seen {
                    return Err("has TEST_MATRIX twice".into());
                }
                matrix_seen = true;
            }
            &line[name.len()..]
        } else {
            line
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if active == "CONTRACT" {
            contract.push(
                value
                    .strip_prefix("- ")
                    .or_else(|| value.strip_prefix("* "))
                    .unwrap_or(value)
                    .trim()
                    .to_string(),
            );
        } else if active == "TEST_MATRIX" {
            raw_rows.push(value.to_string());
        }
    }
    if contract_seen && !matrix_seen {
        return Err("has CONTRACT without TEST_MATRIX".into());
    }
    if matrix_seen && !contract_seen {
        return Err("has TEST_MATRIX without CONTRACT".into());
    }
    let mut rows: Vec<MatrixRow> = Vec::new();
    for raw in raw_rows {
        let parts: Vec<_> = raw.splitn(5, '|').map(str::trim).collect();
        let target = parts.get(2).and_then(|s| s.split_once("::"));
        if parts.len() != 5
            || parts.iter().any(|s| s.is_empty())
            || target.is_none_or(|(f, c)| f.trim().is_empty() || c.trim().is_empty())
        {
            return Err(format!("has an incomplete TEST_MATRIX row: {raw}"));
        }
        let id = parts[0];
        let mut chars = id.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("has an invalid TEST_MATRIX id {id}"));
        }
        if rows.iter().any(|r| r.id == id) {
            return Err(format!("has a duplicate TEST_MATRIX id {id}"));
        }
        let kind = parts[1];
        if !["behaviour", "edge", "failure"].contains(&kind) {
            return Err(format!("has an unknown TEST_MATRIX kind {kind}"));
        }
        let (file, case) = target.unwrap();
        let (file, case) = (file.trim(), case.trim());
        if !covers(scope, file) {
            return Err(format!("has a TEST_MATRIX test outside SCOPE: {file}"));
        }
        rows.push(MatrixRow {
            id: id.into(),
            kind: kind.into(),
            file: file.into(),
            case: case.into(),
            assertion: parts[3].into(),
            command: parts[4].into(),
        });
    }
    Ok((contract, rows))
}
pub fn parse_brief(text: &str) -> Result<Brief, String> {
    let start = text.lines().position(|line| is_field(line, "GOAL"));
    let body = start
        .map(|n| text.lines().skip(n).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    let f = fields(&body, &["GOAL", "SCOPE", "VERIFY", "CONTEXT"]);
    if f.get("GOAL").is_none_or(|s| s.trim().is_empty()) {
        return Err("is missing GOAL".into());
    }
    let scope = scope_section(text, start.unwrap_or(0))?;
    if scope.is_empty() {
        return Err("is missing SCOPE".into());
    }
    if f.get("VERIFY").is_none_or(|s| s.trim().is_empty()) {
        return Err("is missing VERIFY".into());
    }
    let (contract, test_matrix) = brief_sections(&body, &scope)?;
    let (mut effort, mut tier, mut risk) = (None, None, None);
    let mut worker_text = String::with_capacity(text.len());
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next_if(|l| !is_field(l, "GOAL")) {
        worker_text.push_str(line);
    }
    // Pasted CONTEXT can start a line with anything, so only lines before it are settings.
    while let Some(line) = lines.next_if(|l| !is_field(l, "CONTEXT")) {
        if is_field(line, "EFFORT") {
            setting(line, "EFFORT", &EFFORTS, &mut effort)?;
        } else if is_field(line, "TIER") {
            if line["TIER".len()..].trim() == "luna" {
                return Err("TIER luna is below the terra worker floor".into());
            }
            setting(line, "TIER", &TIERS, &mut tier)?;
        } else if is_field(line, "RISK") {
            setting(line, "RISK", &RISKS, &mut risk)?;
        } else {
            worker_text.push_str(line);
        }
    }
    worker_text.extend(lines);
    if let Some(stray) = worker_text.lines().find(|l| is_valid_setting(l)) {
        return Err(format!(
            "has {stray:?} outside the lines between GOAL and CONTEXT"
        ));
    }
    Ok(Brief {
        goal: f["GOAL"].clone(),
        scope,
        verify: f["VERIFY"].clone(),
        context: f.get("CONTEXT").cloned(),
        effort,
        tier,
        risk,
        text: worker_text,
        contract,
        test_matrix,
    })
}
fn normalized(s: &str) -> String {
    let s = s.replace('\\', "/");
    s.trim_start_matches("./").to_string()
}
pub(super) fn repo_path_exists(repo: &std::path::Path, path: &str) -> bool {
    let path = normalized(path);
    let bytes = path.as_bytes();
    if path.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || path.split('/').any(|part| part == "..")
    {
        return false;
    }
    repo.join(path).exists()
}
pub fn covers(patterns: &[String], path: &str) -> bool {
    let p = normalized(path);
    patterns.iter().any(|x| {
        let x = normalized(x);
        x == p || (x.ends_with('/') && p.starts_with(&x))
    })
}
pub fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| covers(b, x)) || b.iter().any(|x| covers(a, x))
}
fn backtick(s: &str) -> Option<String> {
    let (_, tail) = s.split_once('`')?;
    Some(tail.split_once('`')?.0.to_string())
}
/// A plan item id is the `id` of a change in `astra/plan.json`, which the schema leaves free:
/// any one word, without the spaces and commas that separate ids in SERVES.
fn is_plan_id(s: &str) -> bool {
    !s.is_empty() && !s.contains(|c: char| c.is_whitespace() || c == ',')
}
/// Plan items in order: each `### <id> <title>` heading outside the `## Coverage` section, whose
/// read and not-read lists may have `###` headings of their own.
pub fn parse_plan(text: &str) -> Vec<PlanItem> {
    let mut items: Vec<PlanItem> = Vec::new();
    let mut coverage = false;
    for line in text.lines() {
        if let Some(section) = line.strip_prefix("## ") {
            coverage = section.trim_start().starts_with("Coverage");
            continue;
        }
        let Some(rest) = line.strip_prefix("### ").filter(|_| !coverage) else {
            continue;
        };
        let rest = rest.trim();
        let (id, title) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let id = id.trim_end_matches(':');
        let title = title.trim().trim_start_matches([':', '-', ' ']).trim();
        if !id.is_empty() && !items.iter().any(|i| i.id == id) {
            items.push(PlanItem {
                id: id.into(),
                title: title.into(),
            });
        }
    }
    items
}
/// Every plan item must be served by a module or recorded out of scope, and the map may name no other item.
pub fn check_goals(map: &Map, plan: &[PlanItem]) -> Result<(), String> {
    if plan.is_empty() {
        return Ok(());
    }
    let known = |id: &str| plan.iter().any(|i| i.id == id);
    for m in &map.modules {
        if let Some(id) = m.serves.iter().find(|id| !known(id)) {
            return Err(format!("module {}: SERVES unknown plan item {id}", m.id));
        }
    }
    if let Some(o) = map.out_of_scope.iter().find(|o| !known(&o.id)) {
        return Err(format!("OUT OF SCOPE names unknown plan item {}", o.id));
    }
    for item in plan {
        let served = map.modules.iter().any(|m| m.serves.contains(&item.id));
        let out = map.out_of_scope.iter().any(|o| o.id == item.id);
        if !served && !out {
            return Err(format!(
                "plan item {} ({}) is not served by any module and not listed as OUT OF SCOPE",
                item.id, item.title
            ));
        }
    }
    Ok(())
}
fn routing_lines(value: Option<&String>) -> Result<Vec<RouteLine>, (&'static str, String)> {
    use crate::codex::routing::{self, Routing, Tier};
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut routes = Vec::new();
    for line in value.lines().filter(|l| !l.trim().is_empty()) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() < 3 {
            return Err(("routing_value", format!("invalid routing line: {line}")));
        }
        if parts.len() == 3 {
            return Err(("routing_reason", format!("missing reason: {line}")));
        }
        let (tier, effort) = parts[1]
            .split_once('/')
            .ok_or(("routing_value", format!("invalid tier/effort: {line}")))?;
        let t = routing::parse_tier(tier).map_err(|e| ("routing_value", e))?;
        let e = routing::parse_effort(effort).map_err(|e| ("routing_value", e))?;
        let risk: routing::Risk =
            serde_json::from_value(serde_json::Value::String(parts[2].into()))
                .map_err(|e| ("routing_value", e.to_string()))?;
        if t == Tier::Luna {
            return Err(("routing_luna", "luna is not valid in maps".into()));
        }
        let reason = parts[3..].join(" ");
        routing::check_rubric(&Routing {
            tier: t,
            effort: e,
            risk: Some(risk),
            reason: reason.clone(),
        })
        .map_err(|e| ("routing_rubric", e))?;
        routes.push(RouteLine {
            key: parts[0].into(),
            tier: tier.into(),
            effort: effort.into(),
            risk: parts[2].into(),
            reason,
        });
    }
    if !routes.iter().any(|r| r.key == "default") {
        return Err(("routing_default", "ROUTING needs a default".into()));
    }
    Ok(routes)
}
pub fn parse_map(text: &str) -> Result<Map, String> {
    let mut preamble = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut full_test = None;
    let mut out_of_scope: Vec<OutOfScope> = Vec::new();
    for line in text.lines() {
        if let Some(v) = line
            .strip_prefix("OUT OF SCOPE")
            .filter(|v| v.starts_with(char::is_whitespace))
        {
            let (id, reason) = v
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((v.trim(), ""));
            let id = id.trim_end_matches(':');
            if !is_plan_id(id) || reason.trim().is_empty() {
                return Err(format!(
                    "OUT OF SCOPE needs a plan item id and a reason, got {:?}",
                    line.trim()
                ));
            }
            if out_of_scope.iter().any(|o| o.id == id) {
                return Err(format!("OUT OF SCOPE {id}: duplicate"));
            }
            out_of_scope.push(OutOfScope {
                id: id.into(),
                reason: reason.trim().into(),
            });
            continue;
        }
        if let Some(v) = line
            .strip_prefix("FULL TEST")
            .filter(|v| v.is_empty() || v.starts_with(char::is_whitespace))
        {
            full_test = Some(backtick(v).unwrap_or_else(|| v.trim().into()));
            break;
        }
        if let Some(id) = line.strip_prefix("## ") {
            sections.push((id.trim().into(), String::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push('\n');
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    let mut modules: Vec<Module> = Vec::new();
    for (id, body) in sections {
        let f = fields(
            &body,
            &[
                "PURPOSE",
                "PATHS",
                "DEPENDS",
                "CONTRACT",
                "ACCEPT",
                "SERVES",
                "NEW_PATHS",
                "SHARED_OWNER",
                "ROUTING",
            ],
        );
        if modules.iter().any(|m| m.id == id) {
            return Err(format!("module {id}: duplicate id"));
        }
        for n in ["PURPOSE", "PATHS", "DEPENDS", "CONTRACT", "ACCEPT"] {
            if f.get(n).is_none_or(|s| s.trim().is_empty()) {
                return Err(format!("module {id}: missing {n}"));
            }
        }
        let depends = if f["DEPENDS"].trim() == "none" {
            vec![]
        } else {
            words(&f["DEPENDS"])
        };
        if f["DEPENDS"].trim() != "none" && depends.is_empty() {
            return Err(format!("module {id}: missing DEPENDS"));
        }
        for dep in &depends {
            if !modules.iter().any(|m| m.id == *dep) {
                return Err(format!("module {id}: unknown or later dependency {dep}"));
            }
        }
        let paths = words(&f["PATHS"]);
        if paths.is_empty() {
            return Err(format!("module {id}: missing PATHS"));
        }
        let owners = words(f.get("SHARED_OWNER").map_or("", String::as_str));
        if f.contains_key("SHARED_OWNER") && owners.len() != 1 {
            return Err(format!("module {id}: SHARED_OWNER needs one module id"));
        }
        let shared_owner = owners.first().cloned();
        for m in &modules {
            if overlaps(&paths, &m.paths)
                && ![shared_owner.as_deref(), m.shared_owner.as_deref()]
                    .iter()
                    .flatten()
                    .any(|owner| *owner == id || *owner == m.id)
            {
                return Err(format!("module {id}: PATHS overlap module {}", m.id));
            }
        }
        let accept = &f["ACCEPT"];
        let first = accept
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(',');
        let command = backtick(accept).unwrap_or_else(|| {
            accept
                .split_once(char::is_whitespace)
                .map(|(_, rest)| rest.trim().to_string())
                .unwrap_or_default()
        });
        if first.is_empty() || command.is_empty() {
            return Err(format!("module {id}: missing ACCEPT"));
        }
        let serves = words(f.get("SERVES").map_or("", String::as_str));
        if let Some(bad) = serves.iter().find(|s| !is_plan_id(s)) {
            return Err(format!("module {id}: invalid SERVES item {bad:?}"));
        }
        if f.contains_key("SERVES") && serves.is_empty() {
            return Err(format!("module {id}: empty SERVES; omit the field instead"));
        }
        if let Some(s) = serves
            .iter()
            .find(|s| out_of_scope.iter().any(|o| &o.id == *s))
        {
            return Err(format!(
                "module {id}: SERVES {s}, which is also OUT OF SCOPE"
            ));
        }
        let routing = routing_lines(f.get("ROUTING"))
            .map_err(|(kind, msg)| format!("module {id}: {kind}: {msg}"))?;
        modules.push(Module {
            routing,
            id,
            serves,
            new_paths: words(f.get("NEW_PATHS").map_or("", String::as_str)),
            shared_owner,
            purpose: f["PURPOSE"].clone(),
            paths,
            depends,
            contract: f["CONTRACT"].clone(),
            accept_file: first.into(),
            accept_command: command,
        });
    }
    let routed_count = modules.iter().filter(|m| !m.routing.is_empty()).count();
    if routed_count > 0 && routed_count != modules.len() {
        return Err("routing_partial: not every module has ROUTING".into());
    }
    Ok(Map {
        preamble,
        modules,
        full_test,
        out_of_scope,
    })
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct MapProblem {
    pub module: Option<String>,
    pub kind: String,
    pub message: String,
}

/// Collect independent map errors without letting one malformed module hide the others.
pub fn lint_map(text: &str, plan: Option<&str>, repo: Option<&Path>) -> Vec<MapProblem> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut outs: Vec<OutOfScope> = Vec::new();
    let mut global = Vec::new();
    let mut seen_out = Vec::new();
    for line in text.lines() {
        if let Some(v) = line
            .strip_prefix("OUT OF SCOPE")
            .filter(|v| v.starts_with(char::is_whitespace))
        {
            let (id, reason) = v
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((v.trim(), ""));
            let id = id.trim_end_matches(':');
            if !is_plan_id(id) || reason.trim().is_empty() {
                global.push(format!(
                    "OUT OF SCOPE needs a plan item id and a reason, got {:?}",
                    line.trim()
                ));
            } else if seen_out.contains(&id.to_string()) {
                global.push(format!("OUT OF SCOPE {id}: duplicate"));
            } else {
                seen_out.push(id.to_string());
                outs.push(OutOfScope {
                    id: id.into(),
                    reason: reason.trim().into(),
                });
            }
        } else if line
            .strip_prefix("FULL TEST")
            .is_some_and(|v| v.is_empty() || v.starts_with(char::is_whitespace))
        {
            break;
        } else if let Some(id) = line.strip_prefix("## ") {
            sections.push((id.trim().into(), String::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    let names = [
        "PURPOSE",
        "PATHS",
        "DEPENDS",
        "CONTRACT",
        "ACCEPT",
        "SERVES",
        "NEW_PATHS",
        "SHARED_OWNER",
        "ROUTING",
    ];
    let fs: Vec<_> = sections
        .iter()
        .map(|(_, body)| fields(body, &names))
        .collect();
    let ids: Vec<_> = sections.iter().map(|(id, _)| id.as_str()).collect();
    let paths: Vec<_> = fs
        .iter()
        .map(|f| words(f.get("PATHS").map_or("", String::as_str)))
        .collect();
    let deps: Vec<_> = fs
        .iter()
        .map(|f| {
            let d = f.get("DEPENDS").map_or("", String::as_str);
            if d.trim() == "none" {
                vec![]
            } else {
                words(d)
            }
        })
        .collect();
    let owners: Vec<_> = fs
        .iter()
        .map(|f| words(f.get("SHARED_OWNER").map_or("", String::as_str)))
        .collect();
    let mut cycles: Vec<Vec<String>> = vec![vec![]; ids.len()];
    fn walk(
        start: usize,
        at: usize,
        ids: &[&str],
        deps: &[Vec<String>],
        stack: &mut Vec<usize>,
        cycles: &mut [Vec<String>],
    ) {
        for dep in &deps[at] {
            let Some(next) = ids.iter().position(|id| *id == dep) else {
                continue;
            };
            if next == start {
                let mut c: Vec<_> = stack.iter().map(|i| ids[*i]).collect();
                c.push(ids[start]);
                let c = c.join(" -> ");
                if !cycles[start].contains(&c) {
                    cycles[start].push(c);
                }
            } else if next > start && !stack.contains(&next) {
                stack.push(next);
                walk(start, next, ids, deps, stack, cycles);
                stack.pop();
            }
        }
    }
    for i in 0..ids.len() {
        walk(i, i, &ids, &deps, &mut vec![i], &mut cycles);
    }
    let env = regex::Regex::new(
        r"(?i)(?:^|&&|\|\||[;|&(])\s*(?:export|set)(?:\s|$|[;|&(])|%[A-Za-z_][A-Za-z0-9_]*%",
    )
    .unwrap();
    let mut problems = Vec::new();
    for (i, (id, _)) in sections.iter().enumerate() {
        let mut add = |kind: &str, message: String| {
            problems.push(MapProblem {
                module: Some(id.clone()),
                kind: kind.into(),
                message,
            })
        };
        let f = &fs[i];
        if fs.iter().any(|f| f.contains_key("ROUTING"))
            && fs.iter().any(|f| !f.contains_key("ROUTING"))
            && !f.contains_key("ROUTING")
        {
            add("routing_partial", "not every module has ROUTING".into());
        }
        if let Err((kind, message)) = routing_lines(f.get("ROUTING")) {
            add(kind, message);
        }
        if ids[..i].contains(&id.as_str()) {
            add("parse", format!("module {id}: duplicate id"));
        }
        for n in ["PURPOSE", "PATHS", "DEPENDS", "CONTRACT", "ACCEPT"] {
            if f.get(n).is_none_or(|s| s.trim().is_empty())
                || (n == "PATHS" && paths[i].is_empty())
                || (n == "DEPENDS"
                    && f.get(n)
                        .is_some_and(|s| s.trim() != "none" && deps[i].is_empty()))
            {
                add("parse", format!("module {id}: missing {n}"));
            }
        }
        if let Some(a) = f.get("ACCEPT") {
            let first = a
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(',');
            let command = backtick(a).unwrap_or_else(|| {
                a.split_once(char::is_whitespace)
                    .map(|(_, r)| r.trim().to_string())
                    .unwrap_or_default()
            });
            if !a.trim().is_empty() && (first.is_empty() || command.is_empty()) {
                add("parse", format!("module {id}: missing ACCEPT"));
            }
            if env.is_match(&command) {
                add(
                    "accept_env",
                    format!("ACCEPT command contains environment setup: {command}"),
                );
            }
        }
        let serves = words(f.get("SERVES").map_or("", String::as_str));
        if f.contains_key("SERVES") && serves.is_empty() {
            add(
                "parse",
                format!("module {id}: empty SERVES; omit the field instead"),
            );
        }
        for s in &serves {
            if !is_plan_id(s) {
                add("parse", format!("module {id}: invalid SERVES item {s:?}"));
            }
            if outs.iter().any(|o| o.id == *s) {
                add(
                    "parse",
                    format!("module {id}: SERVES {s}, which is also OUT OF SCOPE"),
                );
            }
        }
        let new = words(f.get("NEW_PATHS").map_or("", String::as_str));
        for p in &paths[i] {
            if repo.is_some_and(|r| !repo_path_exists(r, p))
                && !new.iter().any(|n| covers(std::slice::from_ref(n), p))
            {
                add("paths_unmatched", format!("PATHS {p} does not exist"));
            }
        }
        for n in &new {
            if !covers(&paths[i], n) {
                add(
                    "new_paths",
                    format!("NEW_PATHS {n} is not covered by PATHS"),
                );
            }
        }
        for d in &deps[i] {
            match ids.iter().position(|x| *x == d) {
                None => add("depends_unknown", format!("unknown dependency {d}")),
                Some(j) if j >= i => add("depends_order", format!("later dependency {d}")),
                _ => (),
            }
        }
        for c in &cycles[i] {
            add("depends_cycle", c.clone());
        }
        for j in 0..i {
            if overlaps(&paths[i], &paths[j])
                && !owners[i]
                    .iter()
                    .chain(owners[j].iter())
                    .any(|o| o == id || o == ids[j])
            {
                add("paths_overlap", format!("PATHS overlap module {}", ids[j]));
            }
        }
        if f.contains_key("SHARED_OWNER") && owners[i].len() != 1 {
            add(
                "parse",
                format!("module {id}: SHARED_OWNER needs one module id"),
            );
        }
        for o in &owners[i] {
            if !ids.contains(&o.as_str()) {
                add("shared_owner", format!("SHARED_OWNER {o} names no module"));
            }
        }
    }
    for message in global {
        problems.push(MapProblem {
            module: None,
            kind: "parse".into(),
            message,
        });
    }
    if let Some(plan) = plan {
        let items = parse_plan(plan);
        if !items.is_empty() {
            for (i, (id, _)) in sections.iter().enumerate() {
                for s in words(fs[i].get("SERVES").map_or("", String::as_str)) {
                    if !items.iter().any(|p| p.id == s) {
                        problems.push(MapProblem {
                            module: Some(id.clone()),
                            kind: "plan".into(),
                            message: format!("SERVES unknown plan item {s}"),
                        });
                    }
                }
            }
            for o in &outs {
                if !items.iter().any(|p| p.id == o.id) {
                    problems.push(MapProblem {
                        module: None,
                        kind: "plan".into(),
                        message: format!("OUT OF SCOPE names unknown plan item {}", o.id),
                    });
                }
            }
            for item in items {
                if !fs
                    .iter()
                    .any(|f| words(f.get("SERVES").map_or("", String::as_str)).contains(&item.id))
                    && !outs.iter().any(|o| o.id == item.id)
                {
                    problems.push(MapProblem { module: None, kind: "plan".into(), message: format!("plan item {} ({}) is not served by any module and not listed as OUT OF SCOPE", item.id, item.title) });
                }
            }
        }
    }
    // Keep the documented invariant even for an unusual syntax case not covered above.
    if problems.is_empty() {
        if let Err(message) = parse_map(text) {
            problems.push(MapProblem {
                module: None,
                kind: "parse".into(),
                message,
            });
        }
    }
    problems
}
