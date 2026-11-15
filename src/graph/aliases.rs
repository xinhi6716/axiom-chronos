use crate::base::{Failure, Res};
use serde::Serialize;
use std::{collections::HashMap, fs, io::ErrorKind, path::Path};

#[derive(Clone, Debug, Default)]
pub struct Aliases {
    mappings: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Resolved {
    pub canonical: String,
    pub via: Option<String>,
}

fn key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn invalid(line: usize, reason: &str) -> Failure {
    Failure::new("invalid_aliases", format!("line {line}: {reason}"))
}

pub fn load(project_store: &Path) -> Res<Aliases> {
    let path = project_store.join("aliases.csv");
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Aliases::default()),
        Err(error) => return Err(Failure::new("io", format!("{}: {error}", path.display()))),
    };

    let mut aliases = Aliases::default();
    let mut header_seen = false;
    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split(',').map(str::trim);
        let canonical = fields.next().unwrap_or_default();
        let alias = fields.next().unwrap_or_default();
        if canonical.is_empty() || alias.is_empty() || fields.next().is_some() {
            return Err(invalid(line_number, "expected canonical,alias"));
        }
        if !header_seen {
            if canonical != "canonical" || alias != "alias" {
                return Err(invalid(line_number, "expected canonical,alias header"));
            }
            header_seen = true;
            continue;
        }

        let alias_key = key(alias);
        if let Some(existing) = aliases.mappings.get(&alias_key) {
            if key(existing) != key(canonical) {
                return Err(invalid(line_number, "conflicting alias mapping"));
            }
            continue;
        }
        // A new edge forms a cycle exactly when its destination already reaches its source.
        let mut next = key(canonical);
        loop {
            if next == alias_key {
                return Err(invalid(line_number, "alias cycle"));
            }
            match aliases.mappings.get(&next) {
                Some(destination) => next = key(destination),
                None => break,
            }
        }
        aliases.mappings.insert(alias_key, canonical.to_owned());
    }
    if !header_seen {
        return Err(invalid(1, "missing canonical,alias header"));
    }
    Ok(aliases)
}

impl Aliases {
    pub fn resolve(&self, name: &str) -> Resolved {
        let mut canonical = name.to_owned();
        let mut matched = false;
        for _ in 0..8 {
            match self.mappings.get(&key(&canonical)) {
                Some(next) => {
                    canonical = next.clone();
                    matched = true;
                }
                None => break,
            }
        }
        Resolved {
            canonical,
            via: matched.then(|| name.to_owned()),
        }
    }

    pub fn len(&self) -> usize {
        self.mappings.len()
    }
}
