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
