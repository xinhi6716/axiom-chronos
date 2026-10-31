use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brief {
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: String,
    /// Absent for historical briefs written before delivery kinds existed.
    #[serde(default)]
    pub delivery: Option<String>,
    #[serde(default)]
    pub outputs: Option<Vec<String>>,
    pub context: Option<String>,
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
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Map {
    pub preamble: String,
    pub modules: Vec<Module>,
    pub full_test: Option<String>,
}
fn fields(text: &str, names: &[&str]) -> HashMap<String, String> {
    let mut result = HashMap::<String, String>::new();
    let mut active: Option<&str> = None;
    for line in text.lines() {
        let field = names.iter().find(|name| {
            line.strip_prefix(**name)
                .is_some_and(|s| s.is_empty() || s.starts_with(char::is_whitespace))
        });
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
pub fn parse_brief(text: &str) -> Result<Brief, String> {
    let start = text.lines().position(|line| {
        line.strip_prefix("GOAL")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    });
    let body = start
        .map(|n| text.lines().skip(n).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    // Delivery declarations are authority, not decorative preamble. Never
    // silently downgrade a malformed modern brief into a legacy brief.
    if let Some(n)=start {
        for line in text.lines().take(n) {
            for key in ["DELIVERY","OUTPUTS"] {
                if line.strip_prefix(key).is_some_and(|s| s.is_empty() || s.starts_with(char::is_whitespace)) {
                    return Err(key.into());
                }
            }
        }
    }
    let f = fields(&body, &["GOAL", "SCOPE", "VERIFY", "DELIVERY", "OUTPUTS", "CONTEXT"]);
    for n in ["GOAL", "SCOPE", "VERIFY"] {
        if f.get(n).is_none_or(|s| s.trim().is_empty()) {
            return Err(n.into());
        }
    }
    if words(&f["SCOPE"]).is_empty() || words(&f["SCOPE"]).iter().any(|p| !valid_scope(p)) {
        return Err("SCOPE".into());
    }
    let declarations = body.lines().filter(|line| {
        line.strip_prefix("DELIVERY")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    }).count();
    let delivery = f.get("DELIVERY").cloned();
    if declarations > 1 || delivery.as_deref().is_some_and(|v| !matches!(v, "implementation" | "verification" | "measurement")) {
        return Err("DELIVERY".into());
    }
    let output_declarations = body.lines().filter(|line| {
        line.strip_prefix("OUTPUTS")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    }).count();
    if output_declarations > 1 { return Err("OUTPUTS".into()); }
    let outputs = if let Some(value) = f.get("OUTPUTS") {
        let paths = words(value);
        if paths == ["none"] {
            if !matches!(delivery.as_deref(), Some("verification" | "measurement")) {
                return Err("OUTPUTS".into());
            }
            Some(vec![])
        } else {
            let mut seen = std::collections::HashSet::new();
            let mut result = Vec::new();
            for path in paths {
                if path == "none" || path == "." || !valid_scope(&path) || path.contains(['*', '?', '[', ']']) {
                    return Err("OUTPUTS".into());
                }
                let path = normalized(&path);
                let key = path.trim_end_matches('/').to_string();
                #[cfg(windows)]
                let key = key.to_ascii_lowercase();
                if !seen.insert(key) || !covers(&words(&f["SCOPE"]), &path) {
                    return Err("OUTPUTS".into());
                }
                result.push(path);
            }
            if result.is_empty() { return Err("OUTPUTS".into()); }
            Some(result)
        }
    } else { None };
    Ok(Brief {
        goal: f["GOAL"].clone(),
        scope: words(&f["SCOPE"]),
        verify: f["VERIFY"].clone(),
        delivery,
        outputs,
        context: f.get("CONTEXT").cloned(),
        text: text.into(),
    })
}
fn valid_scope(path: &str) -> bool {
    if path == "." {
        return true;
    }
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    !p.starts_with('/')
        && !p.contains(':')
        && !p.contains('\0')
        && p.trim_end_matches('/')
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
fn normalized(s: &str) -> String {
    let s = s.replace('\\', "/");
    s.trim_start_matches("./").to_string()
}
pub fn covers(patterns: &[String], path: &str) -> bool {
    if !valid_scope(path) {
        return false;
    }
    let p = normalized(path);
    #[cfg(windows)]
    let p = p.to_ascii_lowercase();
    patterns.iter().any(|x| {
        if !valid_scope(x) {
            return false;
        }
        let x = normalized(x);
        #[cfg(windows)]
        let x = x.to_ascii_lowercase();
        x == "." || x == p || (x.ends_with('/') && p.starts_with(&x))
    })
}
pub fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| covers(b, x)) || b.iter().any(|x| covers(a, x))
}
fn backtick(s: &str) -> Option<String> {
    let (_, tail) = s.split_once('`')?;
    Some(tail.split_once('`')?.0.to_string())
}
pub fn parse_map(text: &str) -> Result<Map, String> {
    let mut preamble = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut full_test = None;
    for line in text.lines() {
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
            &["PURPOSE", "PATHS", "DEPENDS", "CONTRACT", "ACCEPT"],
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
        if paths.is_empty() || paths.iter().any(|p| !valid_scope(p)) {
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
        modules.push(Module {
            id,
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
    })
}
