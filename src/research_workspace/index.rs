use super::{
    manifest::{self, Artifact, Format, Located, Role},
    read::{self, Limits, SourceVersion},
};
use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

// `read_rows` returns maps, so retain the order of JSON object keys separately.
struct OrderedKeys(Vec<String>);
impl<'de> Deserialize<'de> for OrderedKeys {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OrderedKeys;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut keys = Vec::new();
                while let Some(k) = map.next_key::<String>()? {
                    map.next_value::<serde::de::IgnoredAny>()?;
                    keys.push(k);
                }
                Ok(OrderedKeys(keys))
            }
        }
        d.deserialize_map(Visitor)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Op {
    Eq(String),
    Contains(String),
    In(Vec<String>),
    Empty,
    NotEmpty,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Filter {
    pub column: String,
    pub op: Op,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}
fn default_limit() -> usize {
    50
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Query {
    pub artifact: String,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub sort: Option<Sort>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            artifact: String::new(),
            filters: Vec::new(),
            sort: None,
            offset: 0,
            limit: 50,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QueryPage {
    pub rows: Vec<u32>,
    pub total: usize,
    pub offset: usize,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactStatus {
    Ready(SourceVersion),
    Missing,
    Corrupt(String),
    NotTabular,
}
struct TableRow {
    number: u64,
    fields: Vec<String>,
    key: String,
}
struct Table {
    columns: HashMap<String, usize>,
    rows: Vec<TableRow>,
    keys: HashMap<String, u32>,
}
#[derive(Clone)]
struct Entry {
    status: ArtifactStatus,
    version: Option<SourceVersion>,
    table: Option<Arc<Table>>,
}
pub struct RowRef<'a> {
    artifact: &'a str,
    table: &'a Table,
    row: &'a TableRow,
}
impl RowRef<'_> {
    pub fn get(&self, canonical: &str) -> &str {
        self.table
            .columns
            .get(canonical)
            .and_then(|&i| self.row.fields.get(i))
            .map(String::as_str)
            .unwrap_or("")
    }
    pub fn number(&self) -> u64 {
        self.row.number
    }
    pub fn key(&self) -> &str {
        &self.row.key
    }
    pub fn record_id(&self) -> String {
        manifest::record_id(self.artifact, self.key())
    }
}
pub struct Index {
    located: Arc<Located>,
    limits: Limits,
    generation: u64,
    entries: HashMap<String, Entry>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub reparsed: Vec<String>,
    pub reused: Vec<String>,
}
fn failure_entry(e: Failure, version: Option<SourceVersion>) -> Entry {
    Entry {
        status: if e.kind == "missing_artifact" {
            ArtifactStatus::Missing
        } else {
            ArtifactStatus::Corrupt(e.message)
        },
        version,
        table: None,
    }
}
fn key(role: &Role, table: &Table, fields: &[String], number: u64) -> String {
    let get = |name: &str| {
        table
            .columns
            .get(name)
            .and_then(|&i| fields.get(i))
            .map(String::as_str)
            .unwrap_or("")
    };
    match role {
        Role::Inventory => get("path").into(),
        Role::Screening => get("sha256").into(),
        Role::Evidence => get("evidence_id").into(),
        Role::Measurements | Role::RawMeasurements => get("row_id").into(),
        Role::Aliases => format!("{}|{}", get("category"), get("alias")),
        _ => number.to_string(),
    }
}
fn push_row(table: &mut Table, role: &Role, number: u64, fields: Vec<String>) -> Res<()> {
    let k = key(role, table, &fields, number);
    let index = u32::try_from(table.rows.len())
        .map_err(|_| Failure::new("oversized_record", "too many rows"))?;
    if table.keys.contains_key(&k)
        && matches!(
            role,
            Role::Inventory | Role::Evidence | Role::Measurements | Role::RawMeasurements
        )
    {
        return Err(Failure::new(
            "corrupt_artifact",
            format!("duplicate key: {k}"),
        ));
    }
    table.keys.entry(k.clone()).or_insert(index);
    table.rows.push(TableRow {
        number,
        fields,
        key: k,
    });
    Ok(())
}
fn parse(l: &Located, a: &Artifact, lim: &Limits, version: SourceVersion) -> Entry {
    let result = (|| -> Res<Table> {
        if lim.max_page_rows == 0 {
            return Err(Failure::new(
                "invalid_argument",
                "max_page_rows must be positive",
            ));
        }
        let path = l.artifact_path(a)?;
        let file = std::fs::File::open(&path).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut table = Table {
            columns: HashMap::new(),
            rows: Vec::new(),
            keys: HashMap::new(),
        };
        if a.format == Format::Csv {
            let mut csv = read::CsvReader::new(BufReader::new(file), lim.clone());
            let header = csv.next_record()?.unwrap_or_default();
            // Reproduce the map construction and ordered remapping in read_rows once,
            // including duplicate headers and mappings that overwrite or chain.
            let mut mapping: BTreeMap<String, usize> = header
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name, i))
                .collect();
            for (canonical, actual) in &a.columns {
                if let Some(i) = mapping.remove(actual) {
                    mapping.insert(canonical.clone(), i);
                }
            }
            let mut positions = Vec::with_capacity(mapping.len());
            for (name, position) in mapping {
                table.columns.insert(name, positions.len());
                positions.push(position);
            }
            let identity = positions.iter().enumerate().all(|(i, &p)| i == p);
            let mut number = 0;
            while let Some(mut record) = csv.next_record()? {
                number += 1;
                let fields = if identity {
                    record.truncate(positions.len());
                    record.resize_with(positions.len(), String::new);
                    record
                } else {
                    positions
                        .iter()
                        .map(|&i| record.get_mut(i).map(std::mem::take).unwrap_or_default())
                        .collect()
                };
                push_row(&mut table, &a.role, number, fields)?;
            }
            if table.rows.is_empty() {
                table.columns.clear();
            }
        } else {
            let mut reader = BufReader::new(file);
            let mut line = Vec::new();
            let mut number = 0u64;
            loop {
                line.clear();
                let count = (&mut reader)
                    .take(lim.max_record_bytes as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .map_err(|e| Failure::new("io", e.to_string()))?;
                if count == 0 {
                    break;
                }
                number += 1;
                if count > lim.max_record_bytes {
                    return Err(Failure::new(
                        "oversized_record",
                        format!("JSONL line {number}"),
                    ));
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let mut fields: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_slice(&line).map_err(|e| {
                        Failure::new("corrupt_artifact", format!("JSONL line {number}: {e}"))
                    })?;
                for (canonical, actual) in &a.columns {
                    if let Some(v) = fields.remove(actual) {
                        fields.insert(canonical.clone(), v);
                    }
                }
                let keys: OrderedKeys = serde_json::from_slice(&line)
                    .map_err(|e| Failure::new("corrupt_artifact", e.to_string()))?;
                for name in keys.0 {
                    let canonical = a
                        .columns
                        .iter()
                        .find_map(|(c, actual)| (actual == &name).then_some(c))
                        .unwrap_or(&name);
                    if fields.contains_key(canonical) && !table.columns.contains_key(canonical) {
                        table.columns.insert(canonical.clone(), table.columns.len());
                    }
                }
                for name in fields.keys() {
                    if !table.columns.contains_key(name) {
                        table.columns.insert(name.clone(), table.columns.len());
                    }
                }
                let mut values = vec![String::new(); table.columns.len()];
                for (name, value) in fields {
                    values[table.columns[&name]] = match value {
                        serde_json::Value::String(s) => s,
                        serde_json::Value::Null => String::new(),
                        value => value.to_string(),
                    };
                }
                push_row(&mut table, &a.role, number, values)?;
            }
        }
        let current = read::version(&path)?;
        if current.bytes != version.bytes || current.modified_ns != version.modified_ns {
            return Err(Failure::new(
                "stale_cursor",
                "source changed during indexing",
            ));
        }
        Ok(table)
    })();
    match result {
        Ok(table) => Entry {
            status: ArtifactStatus::Ready(version.clone()),
            version: Some(version),
            table: Some(Arc::new(table)),
        },
        Err(e) => failure_entry(e, Some(version)),
    }
}

fn inspect(l: &Located, a: &Artifact) -> Res<SourceVersion> {
    let path = l.artifact_path(a)?;
    let mut v = read::version(&path)?;
    v.sha256 = Some(read::hash_file(&path)?);
    Ok(v)
}
fn parsed(l: &Located, a: &Artifact, lim: &Limits, version: SourceVersion) -> Entry {
    if a.sha256
        .as_ref()
        .is_some_and(|pin| !pin.eq_ignore_ascii_case(version.sha256.as_deref().unwrap_or("")))
    {
        return failure_entry(
            Failure::new("corrupt_artifact", format!("sha256 mismatch: {}", a.id)),
            Some(version),
        );
    }
    parse(l, a, lim, version)
}
impl Index {
    pub fn build(l: Arc<Located>, lim: Limits, progress: &dyn Fn(usize, usize)) -> Res<Self> {
        let total = l.manifest.artifacts.len();
        let mut entries = HashMap::new();
        progress(0, total);
        for (n, a) in l.manifest.artifacts.iter().enumerate() {
            let entry = if matches!(a.format, Format::Json | Format::Text) {
                Entry {
                    status: ArtifactStatus::NotTabular,
                    version: None,
                    table: None,
                }
            } else {
                match inspect(&l, a) {
                    Ok(v) => parsed(&l, a, &lim, v),
                    Err(e) => failure_entry(e, None),
                }
            };
            entries.insert(a.id.clone(), entry);
            progress(n + 1, total);
        }
        Ok(Self {
            located: l,
            limits: lim,
            generation: 1,
            entries,
        })
    }
    pub fn refresh(&self) -> Res<(Self, RefreshReport)> {
        let mut entries = HashMap::new();
        let mut report = RefreshReport::default();
        let mut changed = false;
        for a in &self.located.manifest.artifacts {
            let old = &self.entries[&a.id];
            let entry = if matches!(old.status, ArtifactStatus::NotTabular) {
                old.clone()
            } else {
                let path = self.located.artifact_path(a);
                let v = path.and_then(|p| read::version(&p).map(|v| (p, v)));
                match v {
                    Err(e) => {
                        let new = failure_entry(e, None);
                        if new.status != old.status {
                            changed = true;
                        }
                        new
                    }
                    Ok((path, mut v)) => {
                        let same_meta = old
                            .version
                            .as_ref()
                            .is_some_and(|x| x.bytes == v.bytes && x.modified_ns == v.modified_ns);
                        if same_meta && matches!(old.status, ArtifactStatus::Ready(_)) {
                            report.reused.push(a.id.clone());
                            old.clone()
                        } else {
                            match read::hash_file(&path) {
                                Err(e) => {
                                    let new = failure_entry(e, Some(v));
                                    if new.status != old.status {
                                        changed = true;
                                    }
                                    new
                                }
                                Ok(hash) => {
                                    v.sha256 = Some(hash);
                                    let same_hash =
                                        old.version.as_ref().and_then(|x| x.sha256.as_ref())
                                            == v.sha256.as_ref();
                                    if same_hash && matches!(old.status, ArtifactStatus::Ready(_)) {
                                        report.reused.push(a.id.clone());
                                        Entry {
                                            status: ArtifactStatus::Ready(v.clone()),
                                            version: Some(v),
                                            table: old.table.clone(),
                                        }
                                    } else if same_hash
                                        && same_meta
                                        && matches!(old.status, ArtifactStatus::Corrupt(_))
                                    {
                                        let new = parsed(&self.located, a, &self.limits, v);
                                        if new.status != old.status {
                                            changed = true;
                                        }
                                        new
                                    } else {
                                        report.reparsed.push(a.id.clone());
                                        changed = true;
                                        parsed(&self.located, a, &self.limits, v)
                                    }
                                }
                            }
                        }
                    }
                }
            };
            entries.insert(a.id.clone(), entry);
        }
        Ok((
            Self {
                located: self.located.clone(),
                limits: self.limits.clone(),
                generation: self.generation + u64::from(changed),
                entries,
            },
            report,
        ))
    }
    pub fn located(&self) -> &Arc<Located> {
        &self.located
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn status(&self, id: &str) -> ArtifactStatus {
        self.entries
            .get(id)
            .map(|e| e.status.clone())
            .unwrap_or(ArtifactStatus::Missing)
    }
    pub fn len(&self, id: &str) -> usize {
        self.entries
            .get(id)
            .and_then(|e| e.table.as_ref())
            .map_or(0, |t| t.rows.len())
    }
    pub fn row(&self, id: &str, i: u32) -> Option<RowRef<'_>> {
        let (id, e) = self.entries.get_key_value(id)?;
        let table = e.table.as_deref()?;
        Some(RowRef {
            artifact: id,
            table,
            row: table.rows.get(i as usize)?,
        })
    }
    pub fn find(&self, id: &str, key: &str) -> Option<u32> {
        self.entries.get(id)?.table.as_ref()?.keys.get(key).copied()
    }
    pub fn query(&self, q: &Query) -> Res<QueryPage> {
        let entry = self.entries.get(&q.artifact).ok_or_else(|| {
            Failure::new(
                "missing_artifact",
                format!("artifact {} missing", q.artifact),
            )
        })?;
        let table = entry
            .table
            .as_ref()
            .ok_or_else(|| Failure::new("unavailable", format!("{:?}", entry.status)))?;
        let filters: Vec<_> = q
            .filters
            .iter()
            .map(|f| {
                (
                    table.columns.get(&f.column).copied(),
                    &f.op,
                    match &f.op {
                        Op::Contains(s) => Some(s.to_lowercase()),
                        _ => None,
                    },
                )
            })
            .collect();
        fn val(row: &TableRow, col: Option<usize>) -> &str {
            col.and_then(|i| row.fields.get(i))
                .map(String::as_str)
                .unwrap_or("")
        }
        let mut rows: Vec<u32> = table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                filters.iter().all(|(col, op, needle)| {
                    let v = val(row, *col);
                    match op {
                        Op::Eq(s) => col.is_some() && v == s,
                        Op::In(xs) => col.is_some() && xs.iter().any(|x| v == x),
                        Op::Contains(_) => {
                            col.is_some() && v.to_lowercase().contains(needle.as_deref().unwrap())
                        }
                        Op::Empty => v.trim().is_empty(),
                        Op::NotEmpty => !v.trim().is_empty(),
                    }
                })
            })
            .map(|(i, _)| i as u32)
            .collect();
        if let Some(sort) = &q.sort {
            enum SortKey {
                Number(f64),
                Text(String),
                Empty,
            }
            let col = table.columns.get(&sort.column).copied();
            let mut keyed: Vec<_> = rows
                .into_iter()
                .map(|row| {
                    let value = val(&table.rows[row as usize], col);
                    let key = if value.trim().is_empty() {
                        SortKey::Empty
                    } else if let Ok(n) = value.parse::<f64>() {
                        if n.is_finite() {
                            SortKey::Number(n)
                        } else {
                            SortKey::Text(value.to_lowercase())
                        }
                    } else {
                        SortKey::Text(value.to_lowercase())
                    };
                    (row, key)
                })
                .collect();
            keyed.sort_by(|(_, a), (_, b)| {
                let order = match (a, b) {
                    (SortKey::Empty, SortKey::Empty) => return Ordering::Equal,
                    (SortKey::Empty, _) => return Ordering::Greater,
                    (_, SortKey::Empty) => return Ordering::Less,
                    (SortKey::Number(a), SortKey::Number(b)) => a.total_cmp(b),
                    (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
                    (SortKey::Number(_), SortKey::Text(_)) => Ordering::Less,
                    (SortKey::Text(_), SortKey::Number(_)) => Ordering::Greater,
                };
                // Descending reverses the non-empty classes too: text precedes numbers.
                if sort.descending {
                    order.reverse()
                } else {
                    order
                }
            });
            rows = keyed.into_iter().map(|(row, _)| row).collect();
        }
        let total = rows.len();
        let rows = rows
            .into_iter()
            .skip(q.offset)
            .take(q.limit.min(500))
            .collect();
        Ok(QueryPage {
            rows,
            total,
            offset: q.offset,
            generation: self.generation,
        })
    }
    pub fn column_values(&self, id: &str, col: &str, limit: usize) -> Vec<(String, usize)> {
        let Some(table) = self.entries.get(id).and_then(|e| e.table.as_ref()) else {
            return Vec::new();
        };
        let Some(&i) = table.columns.get(col) else {
            return Vec::new();
        };
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for row in &table.rows {
            *counts
                .entry(row.fields.get(i).map(String::as_str).unwrap_or(""))
                .or_default() += 1;
        }
        let mut values: Vec<_> = counts.into_iter().collect();
        values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        values
            .into_iter()
            .take(limit)
            .map(|(v, n)| (v.to_owned(), n))
            .collect()
    }
}
#[derive(Debug)]
pub enum IndexEvent {
    Progress { done: usize, total: usize },
    Ready { generation: u64 },
    Failed(Failure),
}
enum Request {
    Refresh,
    Stop,
}
pub struct Indexer {
    tx: mpsc::Sender<Request>,
    latest: Arc<Mutex<Option<Arc<Index>>>>,
    thread: Option<JoinHandle<()>>,
}
impl Indexer {
    pub fn start(l: Arc<Located>, lim: Limits, on_event: Box<dyn Fn(IndexEvent) + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let shared = latest.clone();
        let thread = thread::spawn(move || {
            let mut current = match Index::build(l, lim, &|done, total| {
                on_event(IndexEvent::Progress { done, total })
            }) {
                Ok(i) => i,
                Err(e) => {
                    on_event(IndexEvent::Failed(e));
                    return;
                }
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(current));
            let generation = shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .unwrap()
                .generation();
            on_event(IndexEvent::Ready { generation });
            while let Ok(request) = rx.recv() {
                if matches!(request, Request::Stop) {
                    break;
                }
                let mut stop = false;
                while let Ok(r) = rx.try_recv() {
                    if matches!(r, Request::Stop) {
                        stop = true;
                        break;
                    }
                }
                if stop {
                    break;
                }
                let old = shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .unwrap()
                    .clone();
                match old.refresh() {
                    Ok((i, _)) => {
                        current = i;
                        let generation = current.generation();
                        *shared.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(current));
                        on_event(IndexEvent::Ready { generation });
                    }
                    Err(e) => on_event(IndexEvent::Failed(e)),
                }
            }
        });
        Self {
            tx,
            latest,
            thread: Some(thread),
        }
    }
    pub fn latest(&self) -> Option<Arc<Index>> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn refresh(&self) {
        let _ = self.tx.send(Request::Refresh);
    }
    pub fn stop(&mut self) {
        let _ = self.tx.send(Request::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
impl Drop for Indexer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_workspace::manifest::{self, LocateOptions};
    use std::{fs, sync::mpsc, time::Duration};

    fn setup() -> (tempfile::TempDir, Arc<Located>) {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("research-manifest.json"), r#"{"manifest_version":1,"research_id":"test","research_version":"1","title":"T","settings":{},"versions":{},"artifacts":[{"id":"inventory","role":"inventory","path":"a.csv","format":"csv","columns":{"path":"name"}},{"id":"aliases","role":"aliases","path":"aliases.csv","format":"csv"},{"id":"other","role":"other","path":"other.jsonl","format":"jsonl"},{"id":"text","role":"other","path":"x.txt","format":"text"},{"id":"missing","role":"other","path":"missing.csv","format":"csv"},{"id":"pinned","role":"other","path":"pin.csv","format":"csv","sha256":"wrong"}]}"#).unwrap();
        fs::write(
            d.path().join("a.csv"),
            "name,value\na,10\nb,2\nc,\nd,Apple\ne,apple\n",
        )
        .unwrap();
        fs::write(d.path().join("pin.csv"), "x\n1\n").unwrap();
        fs::write(d.path().join("aliases.csv"), "category,alias\nu,x\nu,x\n").unwrap();
        fs::write(d.path().join("other.jsonl"), "{\"x\":1}\n{\"y\":2}\n").unwrap();
        let l = Arc::new(manifest::locate(d.path(), &LocateOptions::default()).unwrap());
        (d, l)
    }
    fn build(l: Arc<Located>) -> Index {
        Index::build(
            l,
            Limits {
                max_page_rows: 2,
                ..Default::default()
            },
            &|_, _| {},
        )
        .unwrap()
    }
    fn q(op: Op, col: &str) -> Query {
        Query {
            artifact: "inventory".into(),
            filters: vec![Filter {
                column: col.into(),
                op,
            }],
            ..Default::default()
        }
    }
    #[test]
    fn fixture_keys_and_statuses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/research/rram/project");
        let l = Arc::new(manifest::locate(&root, &LocateOptions::default()).unwrap());
        let i = build(l.clone());
        for artifact in &l.manifest.artifacts {
            if matches!(artifact.format, Format::Csv | Format::Jsonl) {
                assert!(
                    matches!(i.status(&artifact.id), ArtifactStatus::Ready(_)),
                    "{}: {:?}",
                    artifact.id,
                    i.status(&artifact.id)
                );
            }
        }
        let row = i.row("inventory", 0).unwrap();
        assert_eq!(row.key(), row.get("path"));
        assert_eq!(i.find("inventory", row.get("path")), Some(0));
        let a = i.row("aliases", 0).unwrap();
        assert_eq!(a.key(), format!("{}|{}", a.get("category"), a.get("alias")));
        assert_eq!(a.record_id(), format!("aliases:{}", a.key()));
        assert_eq!(i.find("aliases", a.key()), Some(0));
        assert_eq!(a.number(), 1);
        let (_d, l) = setup();
        let i = build(l);
        assert_eq!(i.status("missing"), ArtifactStatus::Missing);
        assert!(matches!(i.status("pinned"), ArtifactStatus::Corrupt(_)));
        assert_eq!(i.status("text"), ArtifactStatus::NotTabular);
        assert_eq!(i.find("aliases", "u|x"), Some(0));
        assert_eq!(i.row("other", 1).unwrap().get("x"), "");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/research/rawonly");
        let raw = build(Arc::new(
            manifest::locate(&root, &LocateOptions::default()).unwrap(),
        ));
        assert!(matches!(raw.status("run_b"), ArtifactStatus::Ready(_)));
    }
    #[test]
    fn filters_sort_and_paging() {
        let (_d, l) = setup();
        let i = build(l);
        for (op, col, n) in [
            (Op::Eq("2".into()), "value", 1),
            (Op::Contains("APP".into()), "value", 2),
            (Op::In(vec!["2".into(), "10".into()]), "value", 2),
            (Op::Empty, "value", 1),
            (Op::NotEmpty, "value", 4),
            (Op::Empty, "unknown", 5),
            (Op::NotEmpty, "unknown", 0),
            (Op::Eq("".into()), "unknown", 0),
            (Op::Contains("".into()), "unknown", 0),
            (Op::In(vec!["".into()]), "unknown", 0),
        ] {
            assert_eq!(i.query(&q(op, col)).unwrap().total, n);
        }
        let mut both = q(Op::Contains("app".into()), "value");
        both.filters.push(Filter {
            column: "path".into(),
            op: Op::Eq("d".into()),
        });
        assert_eq!(i.query(&both).unwrap().rows, vec![3]);
        let mut query = Query {
            artifact: "inventory".into(),
            sort: Some(Sort {
                column: "value".into(),
                descending: false,
            }),
            ..Default::default()
        };
        let page = i.query(&query).unwrap();
        assert_eq!(page.rows, vec![1, 0, 3, 4, 2]);
        query.sort.as_mut().unwrap().descending = true;
        assert_eq!(i.query(&query).unwrap().rows, vec![3, 4, 0, 1, 2]);
        query.sort = None;
        query.offset = 2;
        query.limit = 2;
        assert_eq!(i.query(&query).unwrap().rows, vec![2, 3]);
        query.limit = 0;
        assert_eq!(i.query(&query).unwrap().total, 5);
        assert!(i.query(&query).unwrap().rows.is_empty());
        assert_eq!(i.column_values("inventory", "value", 2)[0], ("".into(), 1));
        assert_eq!(
            i.query(&Query {
                artifact: "missing".into(),
                ..Default::default()
            })
            .unwrap_err()
            .kind,
            "unavailable"
        );
        assert_eq!(
            i.query(&Query {
                artifact: "none".into(),
                ..Default::default()
            })
            .unwrap_err()
            .kind,
            "missing_artifact"
        );
    }
    #[test]
    fn duplicate_and_refresh() {
        let (d, l) = setup();
        let i = build(l);
        fs::write(d.path().join("a.csv"), "name,value\ndupkey,1\ndupkey,2\n").unwrap();
        let (j, r) = i.refresh().unwrap();
        assert!(r.reparsed.contains(&"inventory".into()));
        assert!(matches!(j.status("inventory"),ArtifactStatus::Corrupt(s) if s.contains("dupkey")));
        assert!(matches!(j.status("aliases"), ArtifactStatus::Ready(_)));
        assert!(Arc::ptr_eq(
            &i.entries["aliases"].table.as_ref().unwrap(),
            &j.entries["aliases"].table.as_ref().unwrap()
        ));
        fs::write(d.path().join("a.csv"), "name,value\nz,3\n").unwrap();
        let (k, r) = j.refresh().unwrap();
        assert!(r.reparsed.contains(&"inventory".into()));
        assert_eq!(k.find("inventory", "z"), Some(0));
        assert_eq!(
            k.query(&q(Op::Eq("z".into()), "path")).unwrap().rows,
            vec![0]
        );
        let (m, r) = k.refresh().unwrap();
        assert_eq!(m.generation(), k.generation());
        assert!(r.reused.contains(&"inventory".into()));
        fs::write(d.path().join("a.csv"), "name,value\nz,3\n").unwrap();
        let f = fs::OpenOptions::new()
            .write(true)
            .open(d.path().join("a.csv"))
            .unwrap();
        let old = match m.status("inventory") {
            ArtifactStatus::Ready(v) => v,
            _ => panic!(),
        };
        let t =
            std::time::UNIX_EPOCH + Duration::from_nanos((old.modified_ns as u64) + 2_000_000_000);
        f.set_times(fs::FileTimes::new().set_modified(t)).unwrap();
        let new_mtime = read::version(&d.path().join("a.csv")).unwrap().modified_ns;
        let (n, r) = m.refresh().unwrap();
        assert!(r.reused.contains(&"inventory".into()));
        let new = match n.status("inventory") {
            ArtifactStatus::Ready(v) => v,
            _ => panic!(),
        };
        assert_ne!(old.modified_ns, new_mtime);
        assert_eq!(new.modified_ns, new_mtime);
        assert_eq!(new.sha256, old.sha256);
        assert_eq!(n.generation(), m.generation());
        assert!(Arc::ptr_eq(
            m.entries["inventory"].table.as_ref().unwrap(),
            n.entries["inventory"].table.as_ref().unwrap()
        ));
    }
    #[test]
    fn sort_mixed_column_is_total() {
        let (d, l) = setup();
        let values = [
            "10", "2", "apple", "NaN", "inf", "", "Apple", "-1", " ", "-0", "0",
        ];
        let write_rows = |count: usize| {
            let mut csv = String::from("name,value\n");
            for n in 0..count {
                csv.push_str(&format!("row{n},{}\n", values[n % values.len()]));
            }
            fs::write(d.path().join("a.csv"), csv).unwrap();
        };
        write_rows(values.len());
        let i = build(l.clone());
        let mut query = Query {
            artifact: "inventory".into(),
            sort: Some(Sort {
                column: "value".into(),
                descending: false,
            }),
            ..Default::default()
        };
        assert_eq!(
            i.query(&query).unwrap().rows,
            vec![7, 9, 10, 1, 0, 2, 6, 4, 3, 5, 8]
        );
        query.sort.as_mut().unwrap().descending = true;
        assert_eq!(
            i.query(&query).unwrap().rows,
            vec![3, 4, 2, 6, 0, 1, 10, 9, 7, 5, 8]
        );

        write_rows(2000);
        let i = build(l);
        for (descending, groups) in [
            (false, vec![7, 9, 10, 1, 0, 2, 6, 4, 3, 5, 8]),
            (true, vec![3, 4, 2, 6, 0, 1, 10, 9, 7, 5, 8]),
        ] {
            query.sort.as_mut().unwrap().descending = descending;
            query.limit = 500;
            let mut actual = Vec::new();
            for offset in (0..2000).step_by(500) {
                query.offset = offset;
                let page = i.query(&query).unwrap();
                assert_eq!(page.total, 2000);
                actual.extend(page.rows);
            }
            // Case-insensitive text ties and all empty ties preserve source order.
            let mut expected = Vec::new();
            for group in [
                groups[0..5].to_vec(),
                groups[5..9].to_vec(),
                groups[9..11].to_vec(),
            ] {
                for (pos, &value) in group.iter().enumerate() {
                    if pos > 0
                        && ((value == 6 && group[pos - 1] == 2)
                            || (value == 8 && group[pos - 1] == 5))
                    {
                        continue;
                    }
                    for n in 0..2000 {
                        let rem = n % values.len();
                        if rem == value || (value == 2 && rem == 6) || (value == 5 && rem == 8) {
                            expected.push(n as u32);
                        }
                    }
                }
            }
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn column_values_count_then_value() {
        let (d, l) = setup();
        fs::write(
            d.path().join("a.csv"),
            "name,value\na,b\nb,a\nc,b\nd,c\ne,a\nf,b\ng,d\n",
        )
        .unwrap();
        let i = build(l);
        assert_eq!(
            i.column_values("inventory", "value", 10),
            vec![
                ("b".into(), 3),
                ("a".into(), 2),
                ("c".into(), 1),
                ("d".into(), 1)
            ]
        );
    }
    #[test]
    fn consecutive_refreshes_publish_latest_generation() {
        let (d, l) = setup();
        let (tx, rx) = mpsc::channel();
        let mut worker = Indexer::start(
            l,
            Limits::default(),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
        );
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                IndexEvent::Ready { .. } => break,
                IndexEvent::Failed(e) => panic!("{e:?}"),
                _ => {}
            }
        }
        fs::write(d.path().join("a.csv"), "name,value\nnew,3\n").unwrap();
        for _ in 0..8 {
            worker.refresh();
        }
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(first, IndexEvent::Ready { generation: 2 }));
        worker.stop();
        let mut last = match first {
            IndexEvent::Ready { generation } => generation,
            _ => unreachable!(),
        };
        for event in rx.try_iter() {
            match event {
                IndexEvent::Ready { generation } => last = generation,
                IndexEvent::Failed(e) => panic!("{e:?}"),
                _ => {}
            }
        }
        assert_eq!(worker.latest().unwrap().generation(), last);
    }
    #[test]
    fn cap_and_indexer() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Index>();
        let (d, l) = setup();
        fs::write(
            d.path().join("a.csv"),
            format!(
                "name,value\n{}",
                (0..600).map(|n| format!("{n},{n}\n")).collect::<String>()
            ),
        )
        .unwrap();
        let i = build(l.clone());
        let p = i
            .query(&Query {
                artifact: "inventory".into(),
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((p.rows.len(), p.total), (500, 600));
        let (tx, rx) = mpsc::channel();
        let mut worker = Indexer::start(
            l,
            Limits::default(),
            Box::new(move |e| {
                tx.send(e).unwrap();
            }),
        );
        for done in 0..=6 {
            assert!(
                matches!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), IndexEvent::Progress { done: n, total: 6 } if n == done)
            );
        }
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            IndexEvent::Ready { generation: 1 }
        ));
        assert!(worker.latest().is_some());
        worker.refresh();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            IndexEvent::Ready { .. }
        ));
        worker.stop();
    }
    #[test]
    fn serde_order_and_missing_recovery() {
        let (d, l) = setup();
        fs::write(
            d.path().join("other.jsonl"),
            "{\"z\":1,\"a\":2}\n{\"b\":3}\n",
        )
        .unwrap();
        let i = build(l);
        let t = i.entries["other"].table.as_ref().unwrap();
        assert_eq!((t.columns["z"], t.columns["a"], t.columns["b"]), (0, 1, 2));
        let q: Query = serde_json::from_str(r#"{"artifact":"inventory"}"#).unwrap();
        assert_eq!(q.limit, 50);
        assert_eq!(
            serde_json::from_str::<Query>(&serde_json::to_string(&q).unwrap()).unwrap(),
            q
        );
        fs::write(d.path().join("missing.csv"), "v\none\n").unwrap();
        let (j, report) = i.refresh().unwrap();
        assert!(report.reparsed.contains(&"missing".into()));
        assert_eq!(j.generation(), i.generation() + 1);
        assert!(matches!(j.status("missing"), ArtifactStatus::Ready(_)));
        fs::remove_file(d.path().join("missing.csv")).unwrap();
        let (k, _) = j.refresh().unwrap();
        assert_eq!(k.status("missing"), ArtifactStatus::Missing);
        assert_eq!(k.generation(), j.generation() + 1);
    }
}
