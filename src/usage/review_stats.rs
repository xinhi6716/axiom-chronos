use crate::base::{Failure, Res};
use crate::orch::{self, events::Event};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Debug, PartialEq)]
pub struct Rate {
    pub num: u64,
    pub den: u64,
    pub rate: Option<f64>,
}
#[derive(Serialize, Debug, PartialEq)]
pub struct Row {
    pub role: String,
    pub reviewer: String,
    pub model: String,
    pub reviews: u64,
    pub findings: u64,
    pub finding_rate: Rate,
    pub rejected: u64,
    pub rejection_rate: Rate,
    pub escapes: u64,
    pub escape_rate: Rate,
}
impl Rate {
    fn new(num: u64, den: u64) -> Self {
        Self {
            num,
            den,
            rate: (den > 0).then(|| num as f64 / den as f64),
        }
    }
}
type Key = (String, String, String);
#[derive(Default)]
struct Counts {
    reviews: u64,
    findings: u64,
    passing: u64,
    verified: u64,
    rejected: u64,
    escapes: u64,
}
fn string<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or("")
}
fn group(review: &Event) -> Key {
    (
        string(&review.data, "role").to_owned(),
        string(&review.data, "reviewer").to_owned(),
        string(&review.data, "model").to_owned(),
    )
}
fn paths(review: &Event) -> impl Iterator<Item = &str> {
    review.data["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}
fn normalized(path: &str) -> String {
    path.replace('\\', "/")
}
fn covered(a: &str, b: &str) -> bool {
    let (a, b) = (normalized(a), normalized(b));
    !a.is_empty()
        && !b.is_empty()
        && (a == b
            || (a.ends_with('/') && b.starts_with(&a))
            || (b.ends_with('/') && a.starts_with(&b)))
}
fn location_path(location: &str) -> &str {
    if let Some((path, suffix)) = location.rsplit_once(':') {
        let mut parts = suffix.split('-');
        let valid_number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if parts.next().is_some_and(valid_number)
            && parts.next().is_none_or(valid_number)
            && parts.next().is_none()
        {
            return path;
        }
    }
    location
}
fn latest<'a>(
    reviews: &'a HashMap<String, Event>,
    predicate: impl Fn(&Event) -> bool,
) -> Option<&'a Event> {
    reviews
        .values()
        .filter(|r| r.data["pass"].as_bool() == Some(true) && predicate(r))
        .max_by_key(|r| r.seq)
}

pub fn collect(stores: &[PathBuf]) -> Res<Vec<Row>> {
    let mut counts = BTreeMap::<Key, Counts>::new();
    let mut seen = HashSet::new();
    for store in stores {
        let identity = fs::canonicalize(store).unwrap_or_else(|_| store.clone());
        if !seen.insert(identity) {
            continue;
        }
        let mut reviews = HashMap::<String, Event>::new();
        let mut verifications = HashMap::<(String, Value), Event>::new();
        for event in orch::events::read_events(store, 0)? {
            match event.kind.as_str() {
                "review_recorded" => {
                    reviews
                        .entry(string(&event.data, "review_id").to_owned())
                        .or_insert(event);
                }
                "finding_verified" => {
                    let key = (
                        string(&event.data, "review_id").to_owned(),
                        event.data["finding"].clone(),
                    );
                    match verifications.entry(key) {
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(event);
                        }
                        std::collections::hash_map::Entry::Occupied(mut e) => {
                            if event.seq > e.get().seq {
                                e.insert(event);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for review in reviews.values() {
            let row = counts.entry(group(review)).or_default();
            row.reviews += 1;
            row.findings += review.data["findings"]
                .as_array()
                .map_or(0, |a| a.len() as u64);
            if review.data["pass"].as_bool() == Some(true) {
                row.passing += 1;
            }
        }
        for ((id, _), verification) in &verifications {
            let data = &verification.data;
            let key = reviews
                .get(id)
                .map(group)
                .unwrap_or_else(|| ("-".into(), string(data, "reviewer").into(), "".into()));
            let row = counts.entry(key).or_default();
            if data["failed"].as_bool() == Some(false) {
                row.verified += 1;
                if string(data, "status") == "rejected" {
                    row.rejected += 1;
                }
            }
            if matches!(
                string(data, "severity").to_ascii_lowercase().as_str(),
                "blocker" | "major"
            ) && matches!(string(data, "status"), "confirmed" | "inconclusive")
            {
                let location = location_path(string(data, "location"));
                if let Some(review) = latest(&reviews, |r| {
                    r.seq < verification.seq
                        && string(&r.data, "review_id") != id
                        && paths(r).any(|p| covered(p, location))
                }) {
                    counts.get_mut(&group(review)).unwrap().escapes += 1;
                }
            }
        }
        if !reviews.is_empty() && store.join("units.tsv").exists() {
            for unit in orch::read_units(store)? {
                let fixes = orch::evidence(store, &unit.id)?;
                let Some(brief) = fs::read_to_string(&unit.brief)
                    .ok()
                    .and_then(|s| orch::parse_brief(&s).ok())
                else {
                    continue;
                };
                let candidate = fixes
                    .iter()
                    .filter(|e| e.kind == orch::FIXES_KIND)
                    .filter_map(|e| orch::receipts::parse_timestamp(&e.ts))
                    .filter_map(|ts| {
                        latest(&reviews, |r| {
                            r.ts_ms < ts
                                && string(&r.data, "unit") != unit.id
                                && paths(r)
                                    .any(|p| brief.scope.iter().any(|scope| covered(p, scope)))
                        })
                    })
                    .max_by_key(|r| r.seq);
                if let Some(review) = candidate {
                    counts.get_mut(&group(review)).unwrap().escapes += 1;
                }
            }
        }
    }
    Ok(counts
        .into_iter()
        .map(|((role, reviewer, model), c)| Row {
            role,
            reviewer,
            model,
            reviews: c.reviews,
            findings: c.findings,
            finding_rate: Rate::new(c.findings, c.reviews),
            rejected: c.rejected,
            rejection_rate: Rate::new(c.rejected, c.verified),
            escapes: c.escapes,
            escape_rate: Rate::new(c.escapes, c.passing),
        })
        .collect())
}
fn format_rate(rate: &Rate) -> String {
    let mut text = format!("{}/{}", rate.num, rate.den);
    if let Some(value) = rate.rate {
        text.push_str(&format!(" ({:.1}%)", value * 100.0));
    }
    text
}
pub fn render_text(rows: &[Row]) -> String {
    let mut text =
        String::from("role\treviewer\tmodel\treviews\tfinding rate\trejection rate\tescape rate\n");
    if rows.is_empty() {
        text.push_str("No review events.\n");
    }
    let sanitize = regex::Regex::new("(?i)unknown").unwrap();
    for row in rows {
        let model = if row.model.is_empty() {
            "-"
        } else {
            &row.model
        };
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            sanitize.replace_all(&row.role, "-"),
            sanitize.replace_all(&row.reviewer, "-"),
            sanitize.replace_all(model, "-"),
            row.reviews,
            format_rate(&row.finding_rate),
            format_rate(&row.rejection_rate),
            format_rate(&row.escape_rate)
        ));
    }
    text
}
pub fn to_json(rows: &[Row]) -> Value {
    json!({"rows": rows})
}
fn command_impl(
    home: &Path,
    project: Option<&str>,
    wish: Option<&str>,
    json_output: bool,
) -> Res<()> {
    let stores = if let Some(project) = project {
        let project_store = orch::wish_store(home, project, wish)?;
        if wish.is_some() {
            vec![project_store]
        } else {
            orch::wish_stores(&project_store)
        }
    } else {
        let dir = home.join("orchestrate");
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Failure::new("io", format!("{}: {e}", dir.display()))),
        };
        let mut stores = Vec::new();
        if let Some(entries) = entries {
            for entry in entries {
                let path = entry.map_err(|e| Failure::new("io", e.to_string()))?.path();
                if path.is_dir() {
                    stores.extend(orch::wish_stores(&path));
                }
            }
        }
        stores
    };
    let rows = collect(&stores)?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&to_json(&rows)).expect("serializable rows")
        );
    } else {
        print!("{}", render_text(&rows));
    }
    Ok(())
}
pub fn command(home: &Path, args: &[String]) -> i32 {
    let (mut project, mut wish, mut json_output) = (None, None, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json_output = true,
            "--project" | "--wish" => {
                let option = args[i].as_str();
                i += 1;
                if i == args.len() || args[i].starts_with('-') || args[i].is_empty() {
                    eprintln!("axiom: {option} requires a value");
                    return 2;
                }
                if option == "--project" {
                    project = Some(args[i].as_str());
                } else {
                    wish = Some(args[i].as_str());
                }
            }
            option => {
                eprintln!("axiom: unknown option {option}");
                return 2;
            }
        }
        i += 1;
    }
    if wish.is_some() && project.is_none() {
        eprintln!("axiom: --wish requires --project");
        return 2;
    }
    if wish.is_some_and(|id| !orch::is_wish_id(id)) {
        eprintln!("axiom: invalid wish id");
        return 2;
    }
    match command_impl(home, project, wish, json_output) {
        Ok(()) => 0,
        Err(failure) => {
            eprintln!("axiom: {}", failure.message);
            1
        }
    }
}
