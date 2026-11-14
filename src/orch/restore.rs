use std::path::Path;

pub const RULES_PATH: &str = "agents/dispatcher.md";
pub const CONTRACT_BUDGET: usize = 4096;
pub const WISH_BUDGET: usize = 4096;
pub const RESTORE_CAP: usize = 24576;

pub fn dispatcher_rules() -> String {
    let contents = crate::engine::FILES
        .iter()
        .find(|(path, _)| *path == RULES_PATH)
        .expect("embedded dispatcher rules")
        .1
        .replace("\r\n", "\n");
    let mut delimiters = 0;
    let mut start = 0;
    for line in contents.split_inclusive('\n') {
        start += line.len();
        if line.trim_end_matches('\n') == "---" {
            delimiters += 1;
            if delimiters == 2 {
                return contents[start..].trim_start_matches('\n').to_owned();
            }
        }
    }
    contents.trim_start_matches('\n').to_owned()
}

pub fn contract_block(map_text: &str, module: &str) -> Option<String> {
    let normalized = map_text.replace("\r\n", "\n");
    let mut lines = normalized.split('\n');
    lines.find(|line| {
        line.strip_prefix("## ")
            .is_some_and(|id| id.trim() == module)
    })?;
    let mut block = Vec::new();
    for line in lines {
        if line.starts_with("## ")
            || line.starts_with("OUT OF SCOPE")
            || line.starts_with("FULL TEST")
        {
            break;
        }
        if block.is_empty() {
            if line == "CONTRACT"
                || line
                    .strip_prefix("CONTRACT")
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            {
                block.push(line);
            }
        } else if line.trim().is_empty() || line.starts_with(char::is_whitespace) {
            block.push(line);
        } else {
            break;
        }
    }
    while block.last().is_some_and(|line| line.trim().is_empty()) {
        block.pop();
    }
    (!block.is_empty()).then(|| block.join("\n"))
}

pub struct Anchor<'a> {
    pub rules: bool,
    pub wish: Option<&'a str>,
    pub wish_file: Option<&'a Path>,
    pub module: Option<&'a str>,
}

fn prefix(text: &str, budget: usize) -> &str {
    let mut end = text.len().min(budget);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn section(header: &str, content: &str, budget: usize, kind: &str, location: &str) -> String {
    let kept = prefix(content, budget);
    let mut result = format!("{header}{kept}\n");
    if kept.len() < content.len() {
        result.push_str(&format!(
            "INCOMPLETE: {kind} is cut at {} of {} bytes; read the rest in {location}.\n",
            kept.len(),
            content.len()
        ));
    }
    result
}

pub fn anchor_text(store: &Path, anchor: &Anchor<'_>) -> String {
    let map = store.join("modules.md");
    let map_path = map.display().to_string();
    let mut rest = String::new();
    if let Some(wish) = anchor.wish {
        let location = anchor.wish_file.map_or_else(
            || "the goal file".to_owned(),
            |path| path.display().to_string(),
        );
        rest.push_str(&section(
            "WISH, verbatim:\n",
            wish,
            WISH_BUDGET,
            "wish",
            &location,
        ));
    }
    if let Some(module) = anchor.module {
        let block = std::fs::read_to_string(&map)
            .ok()
            .and_then(|text| contract_block(&text, module));
        if let Some(block) = block {
            rest.push_str(&section(
                &format!("CONTRACT of module {module}, verbatim from {map_path}:\n"),
                &block,
                CONTRACT_BUDGET,
                "contract",
                &map_path,
            ));
        } else {
            rest.push_str(&format!(
                "CONTRACT of module {module}: not found in {map_path}.\n"
            ));
        }
    }
    let mut result = String::new();
    if anchor.rules {
        let rules = dispatcher_rules();
        let header = "BINDING RULES, verbatim from agents/dispatcher.md:\n";
        let location = "${CLAUDE_PLUGIN_ROOT}/agents/dispatcher.md";
        let full = section(header, &rules, rules.len(), "rules", location);
        if full.len().saturating_add(rest.len()) <= RESTORE_CAP {
            result = full;
        } else {
            // The marker's byte count changes at decimal boundaries, so test each char boundary.
            for end in rules
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(rules.len()))
            {
                let candidate = section(header, &rules, end, "rules", location);
                if candidate.len().saturating_add(rest.len()) <= RESTORE_CAP {
                    result = candidate;
                }
            }
        }
    }
    result.push_str(&rest);
    // Unbounded caller-supplied paths or module names can alone exceed the cap.
    if result.len() <= RESTORE_CAP {
        return result;
    }

    let mut before_markers = result.len();
    let mut offset = 0;
    let mut markers = String::new();
    for line in result.split_inclusive('\n') {
        if line.starts_with("INCOMPLETE:") {
            before_markers = before_markers.min(offset);
            // A caller-supplied line or path can itself exceed the entire cap.
            if line.len() <= RESTORE_CAP - markers.len() {
                markers.push_str(line);
            }
        }
        offset += line.len();
    }
    let budget = RESTORE_CAP - markers.len();
    let text = &result[..before_markers];
    let mut kept = prefix(text, budget);
    if !kept.is_empty() && !kept.ends_with('\n') {
        kept = prefix(text, budget - 1);
    }
    let mut output = kept.to_owned();
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&markers);
    output
}

pub const MAX_CONTEXT_BYTES: usize = 16384;

#[derive(serde::Serialize, Clone, Debug)]
pub struct ContextPath {
    pub path: String,
    pub exists: bool,
}
#[derive(serde::Serialize, Clone, Debug)]
pub struct ContextPlan {
    pub id: String,
    pub title: String,
    pub text: String,
}
#[derive(serde::Serialize, Clone, Debug)]
pub struct ContextUnit {
    pub unit: String,
    pub state: String,
    pub next: String,
}
#[derive(serde::Serialize, Clone, Debug)]
pub struct ContextIncomplete {
    pub section: String,
    pub source: String,
}
#[derive(serde::Serialize, Clone, Debug)]
pub struct ModuleContext {
    pub module: String,
    pub purpose: String,
    pub paths: Vec<ContextPath>,
    pub depends: Vec<String>,
    pub contract: String,
    pub accept_file: String,
    pub accept_command: String,
    pub new_paths: Vec<String>,
    pub shared_owner: Option<String>,
    pub serves: Vec<String>,
    pub plan: Vec<ContextPlan>,
    pub decisions: Vec<String>,
    pub units: Vec<ContextUnit>,
    pub missing: Vec<String>,
    pub incomplete: Vec<ContextIncomplete>,
}

pub fn render_module_context(c: &ModuleContext) -> String {
    let mut s = format!("MODULE: {}\nPURPOSE: {}\nPATHS:\n", c.module, c.purpose);
    for p in &c.paths {
        s.push_str(&format!(
            "- {} ({})\n",
            p.path,
            if p.exists { "exists" } else { "missing" }
        ));
    }
    s.push_str(&format!(
        "DEPENDS: {}\nACCEPT: {}, {}\nNEW_PATHS: {}\nSHARED_OWNER: {}\nSERVES: {}\n",
        c.depends.join(", "),
        c.accept_file,
        c.accept_command,
        c.new_paths.join(", "),
        c.shared_owner.as_deref().unwrap_or(""),
        c.serves.join(", ")
    ));
    for u in &c.units {
        s.push_str(&format!("UNIT: {} {} {}\n", u.unit, u.state, u.next));
    }
    if !c.contract.is_empty() {
        s.push_str(&c.contract);
        s.push('\n');
    }
    for p in &c.plan {
        s.push_str(&p.text);
        if !p.text.ends_with('\n') {
            s.push('\n');
        }
    }
    for d in &c.decisions {
        s.push_str(d);
        if !d.ends_with('\n') {
            s.push('\n');
        }
    }
    for m in &c.missing {
        s.push_str(&format!("MISSING: {m}\n"));
    }
    for i in &c.incomplete {
        s.push_str(&format!("INCOMPLETE: {}: {}\n", i.section, i.source));
    }
    s
}
fn context_fits(c: &ModuleContext) -> bool {
    render_module_context(c).len() <= MAX_CONTEXT_BYTES
        && serde_json::to_string_pretty(c).unwrap().len() + 1 <= MAX_CONTEXT_BYTES
}
fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}
fn names_id(text: &str, id: &str) -> bool {
    !id.is_empty()
        && text.match_indices(id).any(|(i, _)| {
            !text[..i].chars().next_back().is_some_and(word_char)
                && !text[i + id.len()..].chars().next().is_some_and(word_char)
        })
}
fn names_path(text: &str, paths: &[String], plan: bool) -> bool {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
            )
    })
    .map(|s| s.trim_end_matches(['.', ':']))
    .any(|t| {
        !t.is_empty()
            && (crate::orch::covers(paths, t)
                || plan
                    && paths.iter().any(|p| {
                        p.ends_with('/')
                            && crate::orch::covers(&[p.trim_end_matches('/').to_string()], t)
                    }))
    })
}
fn plan_sections(text: &str) -> Vec<ContextPlan> {
    let normalized = text.replace("\r\n", "\n");
    let mut result: Vec<ContextPlan> = Vec::new();
    let mut current: Option<ContextPlan> = None;
    let mut coverage = false;
    for line in normalized.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if let Some(section) = bare.strip_prefix("## ") {
            if let Some(p) = current.take() {
                result.push(p);
            }
            coverage = section.trim_start().starts_with("Coverage");
        } else if bare.starts_with("### ") {
            if let Some(p) = current.take() {
                result.push(p);
            }
            if !coverage {
                if let Some(item) = crate::orch::parse_plan(bare).first() {
                    current = Some(ContextPlan {
                        id: item.id.clone(),
                        title: item.title.clone(),
                        text: line.into(),
                    });
                }
            }
        } else if let Some(p) = &mut current {
            p.text.push_str(line);
        }
    }
    if let Some(p) = current {
        result.push(p);
    }
    result
}
fn decision_entries(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n");
    let list = regex::Regex::new(r"^(?:[-*] |[0-9]+\. )").unwrap();
    let mut result: Vec<String> = Vec::new();
    let mut continuation = false;
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continuation = false;
            continue;
        }
        if list.is_match(line) {
            result.push(line.to_string());
            continuation = true;
        } else if continuation && line.starts_with(char::is_whitespace) {
            let last = result.last_mut().unwrap();
            last.push('\n');
            last.push_str(line);
        } else {
            result.push(line.to_string());
            continuation = false;
        }
    }
    result
}

pub fn module_context(store: &Path, module: &str, repo: &Path) -> crate::base::Res<ModuleContext> {
    use crate::base::Failure;
    if !repo.is_dir() {
        return Err(Failure::new(
            "invalid_argument",
            format!("--repo must be an existing directory: {}", repo.display()),
        ));
    }
    let map_path = store.join("modules.md");
    #[cfg(test)]
    super::evidence::read_count::map();
    let text = std::fs::read_to_string(&map_path)
        .map_err(|e| Failure::new("io", format!("cannot read {}: {e}", map_path.display())))?;
    let map = crate::orch::parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))?;
    let m = map.modules.iter().find(|m| m.id == module).ok_or_else(|| {
        Failure::new(
            "not_found",
            format!("module {module} not found in {}", map_path.display()),
        )
    })?;
    let mut c = ModuleContext {
        module: m.id.clone(),
        purpose: m.purpose.clone(),
        paths: m
            .paths
            .iter()
            .map(|p| ContextPath {
                path: p.clone(),
                exists: super::parse::repo_path_exists(repo, p),
            })
            .collect(),
        depends: m.depends.clone(),
        contract: String::new(),
        accept_file: m.accept_file.clone(),
        accept_command: m.accept_command.clone(),
        new_paths: m.new_paths.clone(),
        shared_owner: m.shared_owner.clone(),
        serves: m.serves.clone(),
        plan: vec![],
        decisions: vec![],
        units: vec![],
        missing: vec![],
        incomplete: vec![],
    };
    let plan_path = store.join("astra").join("plan-final.md");
    let decisions_path = store.join("decisions.md");
    let mut read_optional = |path: &Path| -> crate::base::Res<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                c.missing.push(path.display().to_string());
                Ok(None)
            }
            Err(e) => Err(Failure::new(
                "io",
                format!("cannot read {}: {e}", path.display()),
            )),
        }
    };
    let plan_text = read_optional(&plan_path)?;
    let decisions_text = read_optional(&decisions_path)?;
    let plans: Vec<_> = plan_text
        .as_deref()
        .map(plan_sections)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            m.serves.contains(&p.id)
                || names_id(&p.text, &m.id)
                || names_path(&p.text, &m.paths, true)
        })
        .collect();
    let decisions: Vec<_> = decisions_text
        .as_deref()
        .map(decision_entries)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| {
            let tagged = map
                .modules
                .iter()
                .any(|x| names_id(d, &x.id) || names_path(d, &x.paths, false));
            !tagged || names_id(d, &m.id) || names_path(d, &m.paths, false)
        })
        .collect();
    // One scan serves every unit, so the store is read once instead of once per unit.
    let scan = super::lifecycle::Scan::new(store)?;
    let mut units = Vec::new();
    for u in scan.units().iter().filter(|u| u.track == module) {
        let state = if u.superseded_by().is_some() {
            u.state.clone()
        } else {
            scan.state(u)?
        };
        units.push(ContextUnit {
            next: scan.next_step(&u.id)?,
            unit: u.id.clone(),
            state,
        });
    }
    let contract = contract_block(&text, module).unwrap_or_default();
    let mut full = c.clone();
    full.units = units.clone();
    full.contract = contract.clone();
    full.plan = plans.clone();
    full.decisions = decisions.clone();
    if context_fits(&full) {
        return Ok(full);
    }
    let unit_count = units.len();
    let sources = [
        ("units", store.join("units.tsv")),
        ("contract", map_path),
        ("plan", plan_path),
        ("decisions", decisions_path),
    ];
    for (section, source) in &sources {
        c.incomplete.push(ContextIncomplete {
            section: (*section).into(),
            source: source.display().to_string(),
        });
    }
    if !context_fits(&c) {
        return Err(Failure::new(
            "invalid_argument",
            "module context header exceeds byte limit",
        ));
    }
    for u in units {
        c.units.push(u);
        if !context_fits(&c) {
            c.units.pop();
            break;
        }
    }
    let units_full = c.units.len() == unit_count;
    if units_full {
        c.incomplete.retain(|i| i.section != "units");
    }
    let mut lo = 0;
    let mut hi = contract.chars().count();
    let boundaries: Vec<_> = contract
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(contract.len()))
        .collect();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        c.contract = contract[..boundaries[mid]].into();
        if context_fits(&c) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    c.contract = contract[..boundaries[lo]].into();
    if lo == boundaries.len() - 1 {
        c.incomplete.retain(|i| i.section != "contract");
    }
    let plan_count = plans.len();
    for p in plans {
        c.plan.push(p);
        if !context_fits(&c) {
            c.plan.pop();
            break;
        }
    }
    if c.plan.len() == plan_count {
        c.incomplete.retain(|i| i.section != "plan");
    }
    let decision_count = decisions.len();
    for d in decisions {
        c.decisions.push(d);
        if !context_fits(&c) {
            c.decisions.pop();
            break;
        }
    }
    if c.decisions.len() == decision_count {
        c.incomplete.retain(|i| i.section != "decisions");
    }
    Ok(c)
}
