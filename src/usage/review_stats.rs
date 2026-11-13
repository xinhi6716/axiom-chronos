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
#[derive(Serialize, Debug, PartialEq)]
pub struct FirstPassRow {
    pub worker_model: String,
    pub worker_effort: String,
    pub risk: String,
    pub units: u64,
    pub first_pass: Rate,
    pub verified: u64,
    pub self_check_fixes: u64,
    pub overruled: u64,
    pub unfinished: u64,
    pub unreviewed_supersedes: u64,
}
fn worker_key(data: &Value) -> Key {
    (string(data, "worker_model").into(), string(data, "worker_effort").into(), string(data, "risk").into())
}
fn first_row(key: &Key) -> FirstPassRow {
    FirstPassRow { worker_model: key.0.clone(), worker_effort: key.1.clone(), risk: key.2.clone(),
        units: 0, first_pass: Rate::new(0, 0), verified: 0, self_check_fixes: 0,
        overruled: 0, unfinished: 0, unreviewed_supersedes: 0 }
}
fn fixed(store: &Path, unit: &str) -> bool {
    // Unit ids in recorded events need not have come through the unit parser.
    if crate::base::require_slug("unit", unit).is_err() { return false; }
    fs::read(store.join("reports").join(format!("{unit}.selfcheck.json"))).ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|v| v["fixed"] == true)
}
fn collect_first_pass_impl(stores: &[PathBuf]) -> Res<(Vec<FirstPassRow>, Option<String>)> {
    let mut rows = BTreeMap::<Key, FirstPassRow>::new();
    let mut seen = HashSet::new();
    let mut cutoff: Option<(i64, String)> = None;
    for store in stores {
        if !seen.insert(fs::canonicalize(store).unwrap_or_else(|_| store.clone())) { continue; }
        let units = if store.join("units.tsv").exists() { orch::read_units(store)? } else { vec![] };
        let states: HashMap<_, _> = units.iter().map(|u| (u.id.as_str(), u.state.as_str())).collect();
        let mut events = orch::events::read_events(store, 0)?;
        events.sort_by_key(|e| e.seq);
        let mut reviews = HashMap::new();
        let mut first = BTreeMap::new();
        let mut verifications = HashMap::new();
        let mut counted = |e: &Event| {
            if cutoff.as_ref().is_none_or(|(ms, _)| e.ts_ms > *ms) {
                cutoff = Some((e.ts_ms, e.ts.clone()));
            }
        };
        for e in &events {
            match e.kind.as_str() {
                "review_recorded" => {
                    if let std::collections::hash_map::Entry::Vacant(entry) = reviews.entry(string(&e.data,"review_id")) {
                        entry.insert(e);
                        first.entry(string(&e.data,"unit")).or_insert(e);
                        counted(e);
                    }
                }
                "finding_verified" => { verifications.insert((string(&e.data,"review_id"),e.data["finding"].clone()),e); }
                _ => {}
            }
        }
        for (unit, e) in &first {
            let key = worker_key(&e.data);
            let row = rows.entry(key.clone()).or_insert_with(|| first_row(&key));
            row.units += 1;
            let verified = states.get(unit) == Some(&"verified");
            row.verified += u64::from(verified);
            let severe = e.data["findings"].as_array().into_iter().flatten().any(|f|
                matches!(string(f,"severity").to_ascii_lowercase().as_str(), "blocker" | "major"));
            row.first_pass.num += u64::from(verified && e.data["pass"] == true && !severe);
            row.self_check_fixes += u64::from(fixed(store, unit));
        }
        for ((id, _), e) in verifications {
            if e.data["status"] != "rejected" || e.data["failed"] != false { continue; }
            if let Some(review) = reviews.get(id) {
                let unit = string(&review.data,"unit");
                let key = worker_key(&first[unit].data);
                rows.get_mut(&key).unwrap().overruled += 1;
                counted(e);
            }
        }
        let mut routing = HashMap::new();
        for receipt in orch::receipts::read_routing(store)? {
            routing.insert(receipt.unit.clone(), receipt);
        }
        for unit in &units {
            let unfinished = unit.state != "verified" && !unit.state.starts_with("superseded:");
            let unreviewed = !first.contains_key(unit.id.as_str());
            let superseded = unit.state.starts_with("superseded:") && unreviewed;
            let fixes = unreviewed && fixed(store, &unit.id);
            if !unfinished && !superseded && !fixes { continue; }
            let key = routing.get(&unit.id).map(|r| (
                r.model_used.as_ref().unwrap_or(&r.model_assigned).clone(),
                r.effort.clone().unwrap_or_default(), r.risk.clone().unwrap_or_default()
            )).unwrap_or_default();
            let row = rows.entry(key.clone()).or_insert_with(|| first_row(&key));
            row.unfinished += u64::from(unfinished);
            row.unreviewed_supersedes += u64::from(superseded);
            row.self_check_fixes += u64::from(fixes);
        }
        for e in &events {
            if e.kind == orch::events::UNIT_STATE && e.unit.as_deref()
                .or_else(|| e.data["unit"].as_str()).is_some_and(|u| states.contains_key(u)) {
                counted(e);
            }
        }
    }
    let rows = rows.into_values().map(|mut r| {
        r.first_pass = Rate::new(r.first_pass.num, r.units); r
    }).collect();
    Ok((rows, cutoff.map(|(_, ts)| ts)))
}
pub fn collect_first_pass(stores: &[PathBuf]) -> Res<Vec<FirstPassRow>> {
    collect_first_pass_impl(stores).map(|(rows, _)| rows)
}
fn display_key(value: &str) -> String {
    let value = regex::Regex::new("(?i)unknown").unwrap().replace_all(value, "-").into_owned();
    if value.is_empty() { "-".into() } else { value.replace(['\r','\n','\t'], " ") }
}
pub fn astra_summary(home: &Path, project: &str) -> String {
    let Ok(root) = orch::wish_store(home, project, None) else { return String::new(); };
    let mut stores = orch::wish_stores(&root);
    if root.join("events.jsonl").exists() || root.join("units.tsv").exists() {
        stores.push(root);
    }
    let Ok((rows, cutoff)) = collect_first_pass_impl(&stores) else { return String::new(); };
    if rows.is_empty() { return String::new(); }
    let mut text = format!("FIRST-PASS STATS (project {project}, cutoff {})\n", cutoff.as_deref().unwrap_or("1970-01-01T00:00:00Z"));
    let mut shown = 0;
    for row in rows.iter().take(40) {
        let rate = if row.units < 3 { "rate=n/a (n<3)".into() }
            else { format!("first_pass={}", format_rate(&row.first_pass)) };
        let line = format!("model={} effort={} risk={} n={} {} verified={} selfcheck_fixes={} overruled={} unfinished={} unreviewed_supersedes={}\n",
            display_key(&row.worker_model), display_key(&row.worker_effort), display_key(&row.risk), row.units, rate,
            row.verified, row.self_check_fixes, row.overruled, row.unfinished, row.unreviewed_supersedes);
        let note = format!("TRUNCATED: showing {} of {} rows\n", shown + 1, rows.len());
        let reserve = if shown + 1 < rows.len() { note.len() } else { 0 };
        // Keep complete UTF-8 row lines; an oversized row is omitted in full.
        if text.len() + line.len() + reserve > 4000 { break; }
        text.push_str(&line); shown += 1;
    }
    if shown < rows.len() { text.push_str(&format!("TRUNCATED: showing {shown} of {} rows\n", rows.len())); }
    text
}
pub fn to_json_with_first_pass(rows: &[Row], first: &[FirstPassRow]) -> Value {
    json!({"rows":rows,"first_pass":first})
}
pub fn render_text_with_first_pass(rows: &[Row], first: &[FirstPassRow]) -> String {
    let mut text = render_text(rows);
    if !first.is_empty() {
        text.push_str("worker_model\tworker_effort\trisk\tunits\tfirst_pass\tverified\tself_check_fixes\toverruled\tunfinished\tunreviewed_supersedes\n");
        for r in first {
            text.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                display_key(&r.worker_model), display_key(&r.worker_effort), display_key(&r.risk), r.units,
                format_rate(&r.first_pass), r.verified, r.self_check_fixes, r.overruled, r.unfinished, r.unreviewed_supersedes));
        }
    }
    text
}
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
    to_json_with_first_pass(rows, &[])
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
    let first = collect_first_pass(&stores)?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&to_json_with_first_pass(&rows, &first)).expect("serializable rows")
        );
    } else {
        print!("{}", render_text_with_first_pass(&rows, &first));
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
