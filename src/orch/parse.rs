use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: String,
    pub context: Option<String>,
    pub effort: Option<String>,
    pub tier: Option<String>,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
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
const EFFORTS: [&str; 3] = ["low", "medium", "high"];
const TIERS: [&str; 1] = ["luna"];
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
    valid("EFFORT", &EFFORTS) || valid("TIER", &TIERS)
}
pub fn parse_brief(text: &str) -> Result<Brief, String> {
    let start = text.lines().position(|line| is_field(line, "GOAL"));
    let body = start
        .map(|n| text.lines().skip(n).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    let f = fields(&body, &["GOAL", "SCOPE", "VERIFY", "CONTEXT"]);
    for n in ["GOAL", "SCOPE", "VERIFY"] {
        if f.get(n).is_none_or(|s| s.trim().is_empty()) {
            return Err(format!("is missing {n}"));
        }
    }
    if words(&f["SCOPE"]).is_empty() {
        return Err("is missing SCOPE".into());
    }
    let (mut effort, mut tier) = (None, None);
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
            setting(line, "TIER", &TIERS, &mut tier)?;
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
        scope: words(&f["SCOPE"]),
        verify: f["VERIFY"].clone(),
        context: f.get("CONTEXT").cloned(),
        effort,
        tier,
        text: worker_text,
    })
}
fn normalized(s: &str) -> String {
    let s = s.replace('\\', "/");
    s.trim_start_matches("./").to_string()
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
            let (id, reason) = v.trim().split_once(char::is_whitespace).unwrap_or((v.trim(), ""));
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
            &["PURPOSE", "PATHS", "DEPENDS", "CONTRACT", "ACCEPT", "SERVES"],
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
        for m in &modules {
            if overlaps(&paths, &m.paths) {
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
        if let Some(s) = serves.iter().find(|s| out_of_scope.iter().any(|o| &o.id == *s)) {
            return Err(format!("module {id}: SERVES {s}, which is also OUT OF SCOPE"));
        }
        modules.push(Module {
            id,
            serves,
            purpose: f["PURPOSE"].clone(),
            paths,
            depends,
            contract: f["CONTRACT"].clone(),
            accept_file: first.into(),
            accept_command: command,
        });
    }
    Ok(Map {
        preamble,
        modules,
        full_test,
        out_of_scope,
    })
}
