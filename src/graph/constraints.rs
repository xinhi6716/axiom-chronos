use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::base::{Failure, Res};

pub const DEFAULT_BUDGET: usize = 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ConstraintBlock {
    pub text: String,
    pub bytes: usize,
    pub budget: usize,
    pub included: usize,
    pub omitted: usize,
    pub overflow: bool,
    pub sources: Vec<(String, String)>,
}

// Blank lines delimit paragraphs; bullets only take indented continuations.
fn entries(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut bullet = false;
    for line in input.lines() {
        let is_bullet = line.starts_with("- ");
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if line.trim().is_empty() || is_bullet || (bullet && !indented) {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            bullet = false;
        }
        if line.trim().is_empty() {
            continue;
        }
        if current.is_empty() {
            bullet = is_bullet;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub fn load(
    project_dir: &Path,
    project_store: Option<&Path>,
    budget: usize,
) -> Res<ConstraintBlock> {
    let paths = [
        Some(project_dir.join("CONSTRAINTS.md")),
        project_store.map(|store| store.join("constraints.md")),
    ];
    let mut sources = Vec::new();
    let mut unique = HashSet::new();
    let mut all = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let Some(path) = path else { continue };
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(Failure::new("io", format!("{}: {error}", path.display()))),
        };
        if data.is_empty() {
            continue;
        }
        let input = std::str::from_utf8(&data)
            .map_err(|error| Failure::new("io", format!("{}: {error}", path.display())))?;
        sources.push((
            path.to_string_lossy().into_owned(),
            format!("{:x}", Sha256::digest(&data)),
        ));
        for entry in entries(input) {
            if unique.insert(entry.clone()) {
                all.push((entry, index));
            }
        }
    }

    let header = "CONSTRAINTS\n";
    let mut text = String::new();
    let mut included = 0;
    if !all.is_empty() {
        let mut prefix_len = header.len();
        // Select the longest fitting prefix, including the final notice.
        let mut best = None;
        for count in 0..=all.len() {
            if count > 0 {
                prefix_len = prefix_len.saturating_add(all[count - 1].0.len());
            }
            let omitted = all.len() - count;
            let notice = if omitted == 0 {
                String::new()
            } else {
                let mut names = Vec::new();
                if all[count..].iter().any(|entry| entry.1 == 0) {
                    names.push("CONSTRAINTS.md");
                }
                if all[count..].iter().any(|entry| entry.1 == 1) {
                    names.push("constraints.md");
                }
                format!("OMITTED {omitted} constraints; see {}\n", names.join(", "))
            };
            if prefix_len.saturating_add(notice.len()) <= budget {
                best = Some((count, notice));
            }
        }
        if let Some((count, notice)) = best {
            text.push_str(header);
            for (entry, _) in &all[..count] {
                text.push_str(entry);
            }
            text.push_str(&notice);
            included = count;
        }
    }
    let omitted = all.len() - included;
    Ok(ConstraintBlock {
        bytes: text.len(),
        text,
        budget,
        included,
        omitted,
        overflow: omitted > 0,
        sources,
    })
}

pub fn prepend(brief: &str, block: &ConstraintBlock) -> String {
    if block.text.is_empty() {
        brief.to_owned()
    } else {
        format!("{}\n{}", block.text, brief)
    }
}
