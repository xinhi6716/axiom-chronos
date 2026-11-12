//! Build scrubbed, wish-separated training datasets from verified decisions.
use crate::base::{Failure, Res};
use crate::laya::router::{DecisionKind, DecisionRecord};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Outcome {
    pub record_id: String,
    pub label: String,
    pub verified_by: String,
    pub ts_ms: i64,
}
fn allowed(s: &str) -> bool {
    matches!(
        s,
        "unit_verified" | "unit_failed" | "review_upheld" | "review_rejected" | "adjudicated"
    )
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn json(e: impl std::fmt::Display) -> Failure {
    Failure::new("json", e.to_string())
}
pub fn record_outcome(home: &Path, o: &Outcome) -> Res<()> {
    if !allowed(&o.verified_by) {
        return Err(Failure::new("invalid_outcome", "unknown verified_by"));
    }
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    let _lock = crate::orch::store::lock_store(&dir)?;
    let mut data = serde_json::to_vec(o).map_err(json)?;
    data.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("outcomes.jsonl"))
        .map_err(io)?
        .write_all(&data)
        .map_err(io)
}
pub trait OutcomeSource {
    fn name(&self) -> String;
    fn outcomes(&self) -> Res<Vec<Outcome>>;
}
pub struct VerifiedOutcomeSource {
    pub home: PathBuf,
}
impl OutcomeSource for VerifiedOutcomeSource {
    fn name(&self) -> String {
        "verified_outcomes".into()
    }
    fn outcomes(&self) -> Res<Vec<Outcome>> {
        let file = match File::open(crate::laya::laya_dir(&self.home).join("outcomes.jsonl")) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(io(e)),
        };
        let mut result = Vec::new();
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line).map_err(io)? == 0 {
                break;
            }
            if let Ok(o) = serde_json::from_slice::<Outcome>(&line) {
                if allowed(&o.verified_by)
                    && DecisionKind::ALL
                        .iter()
                        .any(|k| k.labels().contains(&o.label.as_str()))
                {
                    result.push(o);
                }
            }
        }
        Ok(result)
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Calibration,
    Dev,
    Acceptance,
}
impl Split {
    const ALL: [Self; 4] = [Self::Train, Self::Calibration, Self::Dev, Self::Acceptance];
    fn name(self) -> &'static str {
        match self {
            Self::Train => "train",
            Self::Calibration => "calibration",
            Self::Dev => "dev",
            Self::Acceptance => "acceptance",
        }
    }
}
#[derive(Default, Clone, Debug)]
pub struct ScrubRules {
    pub user_names: Vec<String>,
}
fn patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| [
        r"(?s)-----BEGIN [^-\r\n]+-----.*?-----END [^-\r\n]+-----",
        r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b",
        r"\b(?:sk-|ghp_|github_pat_|AKIA|xox[a-zA-Z]-)[A-Za-z0-9_-]+",
        r#"(?i)(\b[\w.-]*(?:token|secret|password|api_key|apikey)[\w.-]*\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;]+)"#,
        r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b",
        r#"(?i)\b[A-Z]:[\\/][^\s<>"']+|\\\\[^\s\\/<>"']+[\\/][^\s<>"']+"#,
        r"(^|[^\w:/\\])(/[A-Za-z0-9._~-]+(?:/[A-Za-z0-9._~+-]+)*)",
        r"\b[A-Za-z0-9+/=_-]{32,}",
    ].into_iter().map(|p| Regex::new(p).expect("valid scrub pattern")).collect())
}
pub fn scrub(text: &str, rules: &ScrubRules) -> String {
    let p = patterns();
    let mut s = text.to_owned();
    for re in &p[..3] {
        s = re.replace_all(&s, "<secret>").into_owned();
    }
    s = p[3]
        .replace_all(&s, |c: &Captures<'_>| format!("{}<secret>", &c[1]))
        .into_owned();
    s = p[4].replace_all(&s, "<email>").into_owned();
    s = p[5].replace_all(&s, "<path>").into_owned();
    s = p[6]
        .replace_all(&s, |c: &Captures<'_>| format!("{}<path>", &c[1]))
        .into_owned();
    let mut names = rules.user_names.clone();
    names.extend(
        ["USERNAME", "USER"]
            .iter()
            .filter_map(|key| std::env::var(key).ok()),
    );
    for name in names.into_iter().filter(|n| n.chars().count() >= 2) {
        let re =
            Regex::new(&format!(r"(?i)\b{}\b", regex::escape(&name))).expect("escaped username");
        s = re.replace_all(&s, "<user>").into_owned();
    }
    p[7].replace_all(&s, "<secret>").into_owned()
}
pub struct CurateSpec {
    pub seed: u64,
    pub fractions: [f64; 4],
    pub scrub: ScrubRules,
}
impl CurateSpec {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            fractions: [0.6, 0.15, 0.15, 0.10],
            scrub: ScrubRules::default(),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Row {
    pub state: String,
    pub questions: serde_json::Value,
    pub expected: serde_json::Value,
    pub meta: RowMeta,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RowMeta {
    pub kind: DecisionKind,
    pub wish: String,
    pub record: String,
    pub source: String,
    pub claude: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Datasheet {
    pub seed: u64,
    pub per_source: BTreeMap<String, u64>,
    pub per_class: BTreeMap<String, BTreeMap<String, u64>>,
    pub per_split: BTreeMap<String, u64>,
    pub excluded: BTreeMap<String, u64>,
}
pub struct Curated {
    pub splits: BTreeMap<Split, Vec<Row>>,
    pub datasheet: Datasheet,
}
fn hash(seed: u64, id: &str) -> [u8; 32] {
    Sha256::digest(format!("{seed}:{id}").as_bytes()).into()
}
fn wish_hash(h: &[u8; 32]) -> String {
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}
struct Entry {
    ts: i64,
    label: String,
    split: Split,
    row: Row,
}
pub fn curate(
    records: &[DecisionRecord],
    sources: &[&dyn OutcomeSource],
    spec: &CurateSpec,
) -> Res<Curated> {
    let mut outcomes: BTreeMap<String, (Outcome, String, usize)> = BTreeMap::new();
    for (index, source) in sources.iter().enumerate() {
        let name = source.name();
        for o in source.outcomes()? {
            let replace = outcomes
                .get(&o.record_id)
                .map_or(true, |(old, _, old_index)| {
                    o.ts_ms > old.ts_ms || (o.ts_ms == old.ts_ms && index > *old_index)
                });
            if replace {
                outcomes.insert(o.record_id.clone(), (o, name.clone(), index));
            }
        }
    }
    let mut excluded: BTreeMap<String, u64> = [
        "unverified",
        "no_wish",
        "duplicate",
        "conflict",
        "unknown_label",
    ]
    .into_iter()
    .map(|s| (s.into(), 0))
    .collect();
    let mut groups: BTreeMap<(DecisionKind, String), Vec<Entry>> = BTreeMap::new();
    let mut ordered: Vec<_> = records.iter().collect();
    ordered.sort_by(|a, b| (a.ts_ms, &a.id).cmp(&(b.ts_ms, &b.id)));
    for r in ordered {
        let Some((o, source, _)) = outcomes.get(&r.id) else {
            *excluded.get_mut("unverified").unwrap() += 1;
            continue;
        };
        let Some(wish) = r.wish_id.as_deref().filter(|w| !w.is_empty()) else {
            *excluded.get_mut("no_wish").unwrap() += 1;
            continue;
        };
        if !r.kind.labels().contains(&o.label.as_str()) {
            *excluded.get_mut("unknown_label").unwrap() += 1;
            continue;
        }
        let h = hash(spec.seed, wish);
        let x = u64::from_be_bytes(h[..8].try_into().unwrap()) as f64 / 18446744073709551616.0;
        let mut cumulative = 0.0;
        let mut split = Split::Acceptance;
        for (i, fraction) in spec.fractions.iter().enumerate().take(3) {
            cumulative += fraction;
            if x < cumulative {
                split = Split::ALL[i];
                break;
            }
        }
        let state = scrub(&r.context, &spec.scrub);
        let key = r.kind.as_str();
        let row = Row {
            state: state.clone(),
            questions: serde_json::json!({key: {"type": "choice", "instructions": r.kind.instructions(), "criteria": r.kind.labels()}}),
            expected: serde_json::json!({key: o.label}),
            meta: RowMeta {
                kind: r.kind,
                wish: wish_hash(&h),
                record: r.id.clone(),
                source: source.clone(),
                claude: scrub(&r.claude, &spec.scrub),
            },
        };
        groups.entry((r.kind, state)).or_default().push(Entry {
            ts: r.ts_ms,
            label: o.label.clone(),
            split,
            row,
        });
    }
    let mut entries = Vec::new();
    for (_, mut group) in groups {
        if group.iter().any(|e| e.label != group[0].label) {
            *excluded.get_mut("conflict").unwrap() += group.len() as u64;
        } else {
            *excluded.get_mut("duplicate").unwrap() += (group.len() - 1) as u64;
            entries.push(group.remove(0));
        }
    }
    let mut classes: BTreeMap<DecisionKind, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
    for (i, e) in entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.split == Split::Train)
    {
        classes
            .entry(e.row.meta.kind)
            .or_default()
            .entry(e.label.clone())
            .or_default()
            .push(i);
    }
    let mut keep = vec![true; entries.len()];
    for class in classes.values_mut() {
        let n = class.values().map(Vec::len).min().unwrap_or(0);
        for indices in class.values_mut() {
            indices.sort_by(|&a, &b| {
                hash(spec.seed, &entries[a].row.meta.record)
                    .cmp(&hash(spec.seed, &entries[b].row.meta.record))
                    .then_with(|| entries[a].row.meta.record.cmp(&entries[b].row.meta.record))
            });
            for &i in indices.iter().skip(n) {
                keep[i] = false;
            }
        }
    }
    let mut entries: Vec<_> = entries
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| keep[i].then_some(e))
        .collect();
    entries.sort_by(|a, b| (a.ts, &a.row.meta.record).cmp(&(b.ts, &b.row.meta.record)));
    let mut splits: BTreeMap<Split, Vec<Row>> =
        Split::ALL.into_iter().map(|s| (s, vec![])).collect();
    let mut ds = Datasheet {
        seed: spec.seed,
        per_source: BTreeMap::new(),
        per_class: BTreeMap::new(),
        per_split: Split::ALL
            .into_iter()
            .map(|s| (s.name().into(), 0))
            .collect(),
        excluded,
    };
    for e in entries {
        *ds.per_source.entry(e.row.meta.source.clone()).or_default() += 1;
        *ds.per_split.get_mut(e.split.name()).unwrap() += 1;
        *ds.per_class
            .entry(format!("{}:{}", e.row.meta.kind.as_str(), e.label))
            .or_default()
            .entry(e.split.name().into())
            .or_default() += 1;
        splits.get_mut(&e.split).unwrap().push(e.row);
    }
    Ok(Curated {
        splits,
        datasheet: ds,
    })
}
pub fn write(c: &Curated, out: &Path) -> Res<Vec<PathBuf>> {
    fs::create_dir_all(out).map_err(io)?;
    let mut paths = Vec::new();
    for split in Split::ALL {
        let path = out.join(format!("{}.jsonl", split.name()));
        let mut bytes = Vec::new();
        if let Some(rows) = c.splits.get(&split) {
            for row in rows {
                serde_json::to_writer(&mut bytes, row).map_err(json)?;
                bytes.push(b'\n');
            }
        }
        crate::base::atomic_write(&path, &bytes).map_err(io)?;
        paths.push(path);
    }
    let path = out.join("datasheet.json");
    crate::base::atomic_write(&path, &serde_json::to_vec(&c.datasheet).map_err(json)?)
        .map_err(io)?;
    paths.push(path);
    let mut md = format!("# Datasheet\n\nSeed: {}\n", c.datasheet.seed);
    for (title, counts) in [
        ("Source", &c.datasheet.per_source),
        ("Split", &c.datasheet.per_split),
        ("Excluded", &c.datasheet.excluded),
    ] {
        md.push_str(&format!("\n| {title} | Count |\n| --- | ---: |\n"));
        for (key, value) in counts {
            md.push_str(&format!("| {} | {} |\n", key.replace('|', "\\|"), value));
        }
    }
    md.push_str("\n| Class | Split | Count |\n| --- | --- | ---: |\n");
    for (class, splits) in &c.datasheet.per_class {
        for (split, count) in splits {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                class.replace('|', "\\|"),
                split,
                count
            ));
        }
    }
    let path = out.join("datasheet.md");
    crate::base::atomic_write(&path, md.as_bytes()).map_err(io)?;
    paths.push(path);
    Ok(paths)
}
