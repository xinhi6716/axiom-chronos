use super::manifest::*;
use crate::base::{Failure, Res};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, path::Path};
pub fn detect(root: &Path) -> bool {
    root.join("data/literature_inventory.csv").is_file()
        && root.join("research/protocol.yaml").is_file()
}
// A deliberately small flow-style YAML reader. Unsupported syntax invalidates the whole top-level value.
struct Parser<'a> {
    s: &'a str,
    pos: usize,
}
impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.s[self.pos..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            self.pos += self.s[self.pos..].chars().next().unwrap().len_utf8();
        }
    }
    fn take(&mut self, c: char) -> bool {
        self.ws();
        if self.s[self.pos..].starts_with(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn scalar(&mut self, key: bool) -> Option<String> {
        self.ws();
        let start = self.pos;
        let q = self.s[self.pos..].chars().next()?;
        if q == '\'' || q == '"' {
            self.pos += 1;
            let mut out = String::new();
            loop {
                let c = self.s[self.pos..].chars().next()?;
                self.pos += c.len_utf8();
                if c == q {
                    if q == '\'' && self.s[self.pos..].starts_with('\'') {
                        self.pos += 1;
                        out.push('\'');
                        continue;
                    }
                    return Some(out);
                }
                if q == '"' && c == '\\' {
                    let n = self.s[self.pos..].chars().next()?;
                    self.pos += n.len_utf8();
                    out.push(match n {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        _ => return None,
                    });
                } else {
                    out.push(c)
                }
            }
        }
        while let Some(c) = self.s[self.pos..].chars().next() {
            if c == ',' || c == ']' || c == '}' || (key && c == ':') {
                break;
            }
            self.pos += c.len_utf8();
        }
        let t = self.s[start..self.pos].trim();
        if t.is_empty() || t.starts_with(['&', '*', '!', '|', '>']) || t.contains(" #") {
            None
        } else {
            Some(t.to_string())
        }
    }
    fn value(&mut self) -> Option<Value> {
        self.ws();
        if self.take('{') {
            let mut m = Map::new();
            if self.take('}') {
                return Some(Value::Object(m));
            }
            loop {
                let k = self.scalar(true)?;
                if !self.take(':') {
                    return None;
                }
                let v = self.value()?;
                if m.insert(k, v).is_some() {
                    return None;
                }
                if self.take('}') {
                    break;
                }
                if !self.take(',') {
                    return None;
                }
            }
            Some(Value::Object(m))
        } else if self.take('[') {
            let mut a = Vec::new();
            if self.take(']') {
                return Some(Value::Array(a));
            }
            loop {
                a.push(self.value()?);
                if self.take(']') {
                    break;
                }
                if !self.take(',') {
                    return None;
                }
            }
            Some(Value::Array(a))
        } else {
            self.ws();
            let quoted =
                self.s[self.pos..].starts_with('\'') || self.s[self.pos..].starts_with('"');
            let s = self.scalar(false)?;
            if quoted {
                Some(Value::String(s))
            } else {
                match s.as_str() {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    "null" | "~" => Some(Value::Null),
                    _ => serde_json::from_str::<Value>(&s)
                        .ok()
                        .filter(|v| v.is_number())
                        .or(Some(Value::String(s))),
                }
            }
        }
    }
}
fn protocol(text: &str) -> Value {
    let mut map = Map::new();
    let mut unread = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            unread.push(Value::String(t.into()));
            continue;
        };
        let key = k.trim();
        let mut p = Parser { s: v, pos: 0 };
        let val = if !k.starts_with(char::is_whitespace) && !key.is_empty() {
            p.value().filter(|_| {
                p.ws();
                p.pos == p.s.len()
            })
        } else {
            None
        };
        if let Some(v) = val {
            map.insert(key.into(), v);
        } else {
            unread.push(Value::String(key.into()))
        }
    }
    if !unread.is_empty() {
        map.insert("_unread".into(), Value::Array(unread));
    }
    Value::Object(map)
}
fn code(root: &Path) -> Option<String> {
    let mut git = root.join(".git");
    if git.is_file() {
        let s = fs::read_to_string(&git).ok()?;
        let p = s.trim().strip_prefix("gitdir: ")?;
        git = if Path::new(p).is_absolute() {
            p.into()
        } else {
            root.join(p)
        }
    }
    let head = fs::read_to_string(git.join("HEAD")).ok()?;
    let h = head.trim();
    if let Some(r) = h.strip_prefix("ref: ") {
        if r.contains("..") || r.starts_with('/') || r.contains('\\') {
            return None;
        }
        let value = fs::read_to_string(git.join(r)).ok()?;
        Some(value.trim().into())
    } else if !h.is_empty() {
        Some(h.into())
    } else {
        None
    }
}
fn stage(id: &str, progress: Option<&str>, outputs: &[&str], gate: Option<GateDecl>) -> StageDecl {
    StageDecl {
        id: id.into(),
        label: id.into(),
        progress: progress.map(str::to_string),
        outputs: outputs.iter().map(|s| s.to_string()).collect(),
        gate,
    }
}
fn gate(
    artifact: &str,
    pointer: &str,
    op: GateOp,
    value: Option<Value>,
    setting: Option<&str>,
) -> GateDecl {
    GateDecl {
        artifact: artifact.into(),
        pointer: pointer.into(),
        op,
        value,
        setting: setting.map(str::to_string),
    }
}
pub fn manifest(root: &Path, _o: &LocateOptions) -> Res<Manifest> {
    let text = fs::read_to_string(root.join("research/protocol.yaml"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let settings = protocol(&text);
    let project = settings
        .pointer("/meta/project")
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::new("invalid_manifest", "protocol meta.project unreadable"))?
        .to_string();
    crate::base::require_slug("research_id", &project)
        .map_err(|e| Failure::new("invalid_manifest", e.message))?;
    let mut versions = Versions {
        code: code(root),
        ..Default::default()
    };
    if let Some(s) = settings.get("seeds").and_then(Value::as_object) {
        for (k, v) in s {
            if let Some(v) = v.as_i64() {
                versions.seeds.insert(k.clone(), v);
            }
        }
    }
    let prompts = root.join("src/hfo2/prompts");
    if let Ok(files) = fs::read_dir(prompts) {
        for f in files.flatten() {
            if f.path().is_file() {
                let name = f.file_name().to_string_lossy().into_owned();
                versions
                    .prompts
                    .insert(name, super::read::hash_file(&f.path())?);
            }
        }
    }
    if let Ok(py) = fs::read_to_string(root.join("pyproject.toml")) {
        for line in py.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "requires-python" {
                    versions.environment.insert(
                        "requires-python".into(),
                        v.trim().trim_matches('"').trim_matches('\'').into(),
                    );
                    break;
                }
            }
        }
    }
    let research_version = fs::read_dir(root.join("state"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| {
            let n = f.file_name().to_string_lossy().into_owned();
            n.strip_prefix("source_snapshot_")
                .and_then(|s| s.strip_suffix(".json"))
                .map(str::to_string)
        })
        .max()
        .unwrap_or_else(|| "unversioned".into());
    let mut roots = BTreeMap::new();
    if let Some(p) = settings
        .pointer("/paths/source_root")
        .and_then(Value::as_str)
    {
        roots.insert("pdf".into(), p.into());
    }
    let specs = [
        (
            "inventory",
            Role::Inventory,
            "data/literature_inventory.csv",
            Format::Csv,
        ),
        (
            "screening",
            Role::Screening,
            "research/screening_audit.csv",
            Format::Csv,
        ),
        ("aliases", Role::Aliases, "data/aliases.csv", Format::Csv),
        ("evidence", Role::Evidence, "data/evidence.csv", Format::Csv),
        (
            "measurements",
            Role::Measurements,
            "data/measurements.csv",
            Format::Csv,
        ),
        (
            "raw_measurements",
            Role::RawMeasurements,
            "data/raw_measurements.csv",
            Format::Csv,
        ),
        (
            "progress_screen",
            Role::Progress,
            "state/progress/screen.json",
            Format::Json,
        ),
        (
            "driver",
            Role::Driver,
            "state/progress/driver.json",
            Format::Json,
        ),
        (
            "completed",
            Role::Completed,
            "state/completed.jsonl",
            Format::Jsonl,
        ),
        (
            "pilot",
            Role::Pilot,
            "research/pilot_manifest.csv",
            Format::Csv,
        ),
        (
            "acceptance",
            Role::Acceptance,
            "research/acceptance_manifest.csv",
            Format::Csv,
        ),
        (
            "acceptance_sha",
            Role::Other,
            "research/acceptance_manifest.sha256",
            Format::Text,
        ),
        (
            "splits",
            Role::Splits,
            "research/split_manifest.csv",
            Format::Csv,
        ),
        (
            "splits_sha",
            Role::Other,
            "research/split_manifest.sha256",
            Format::Text,
        ),
        (
            "validation",
            Role::Validation,
            "research/validation_report.json",
            Format::Json,
        ),
        (
            "effort_lock",
            Role::Gate,
            "research/effort_lock.json",
            Format::Json,
        ),
        (
            "recall",
            Role::Gate,
            "research/recall_check_final.json",
            Format::Json,
        ),
        (
            "raw_summary",
            Role::RawSummary,
            "research/raw_extract_summary.json",
            Format::Json,
        ),
        (
            "protocol",
            Role::Protocol,
            "research/protocol.yaml",
            Format::Text,
        ),
    ];
    let mut artifacts = Vec::new();
    for (id, role, path, format) in specs {
        if !root.join(path).is_file() {
            continue;
        }
        let mut columns = BTreeMap::new();
        for (k, v) in match id {
            "inventory" => vec![("path", "rel_path"), ("bytes", "size_bytes")],
            "screening" => vec![
                ("sha256", "pdf_sha256"),
                ("path", "rel_path"),
                ("reason", "scope_reason"),
            ],
            "evidence" | "measurements" => vec![("sha256", "pdf_sha256")],
            _ => vec![],
        } {
            columns.insert(k.into(), v.into());
        }
        let pin = match id {
            "acceptance" => Some(("acceptance", "acceptance_sha")),
            "splits" => Some(("splits", "splits_sha")),
            "validation" => Some(("measurements", "validation#/measurements_sha256")),
            _ => None,
        };
        let produced = pin.map(|(a, s)| Produced {
            inputs: vec![InputPin {
                artifact: a.into(),
                sha256: None,
                sha256_from: Some(s.into()),
            }],
            code_version: None,
            prompt_version: None,
            at: None,
        });
        artifacts.push(Artifact {
            id: id.into(),
            role,
            root: None,
            path: path.into(),
            format,
            sha256: None,
            columns,
            documents_root: if id == "inventory" {
                Some("pdf".into())
            } else {
                None
            },
            produced,
        });
    }
    let has = |id: &str| artifacts.iter().any(|a| a.id == id);
    let mut capabilities = Vec::new();
    for (yes, c) in [
        (has("inventory") && has("evidence"), Capability::PdfPages),
        (has("raw_measurements"), Capability::RawMeasurements),
        (has("inventory"), Capability::Literature),
        (has("screening"), Capability::Screening),
        (has("evidence"), Capability::Evidence),
        (
            has("measurements") || has("raw_measurements"),
            Capability::Dataset,
        ),
        (has("pilot"), Capability::Pilot),
        (has("acceptance"), Capability::Acceptance),
        (has("splits"), Capability::Splits),
        (
            has("progress_screen") || has("driver") || has("completed"),
            Capability::Progress,
        ),
    ] {
        if yes {
            capabilities.push(c)
        }
    }
    let stages = vec![
        stage("inventory", None, &["inventory"], None),
        stage(
            "screen",
            Some("progress_screen"),
            &["screening"],
            Some(gate(
                "recall",
                "/passed",
                GateOp::Eq,
                Some(Value::Bool(true)),
                None,
            )),
        ),
        stage(
            "pilot",
            None,
            &["pilot"],
            Some(gate(
                "effort_lock",
                "/pilot/accuracy",
                GateOp::Ge,
                None,
                Some("/pilot/accuracy_threshold"),
            )),
        ),
        stage(
            "acceptance",
            None,
            &["acceptance"],
            Some(gate(
                "effort_lock",
                "/acceptance/accuracy",
                GateOp::Ge,
                None,
                Some("/acceptance/accuracy_threshold"),
            )),
        ),
        stage(
            "extract",
            None,
            &["measurements", "evidence"],
            Some(gate(
                "validation",
                "/n_errors",
                GateOp::Eq,
                Some(0.into()),
                None,
            )),
        ),
        stage("splits", None, &["splits"], None),
    ];
    let links = vec![
        Link {
            from: "evidence".into(),
            to: "measurements".into(),
            key: "evidence_id".into(),
            separator: Some(";".into()),
        },
        Link {
            from: "inventory".into(),
            to: "screening".into(),
            key: "path".into(),
            separator: None,
        },
        Link {
            from: "measurements".into(),
            to: "splits".into(),
            key: "paper_id".into(),
            separator: None,
        },
    ];
    let stall_after_s = settings.pointer("/codex/timeout_s").and_then(Value::as_u64);
    Ok(Manifest {
        manifest_version: MANIFEST_VERSION,
        research_id: project.clone(),
        research_version,
        title: project,
        session: None,
        roots,
        capabilities,
        settings,
        versions,
        artifacts,
        stages,
        links,
        groups: Some(GroupDecl {
            pilot: "pilot".into(),
            acceptance: "acceptance".into(),
            by: GroupBy::StudyGroups,
        }),
        stall_after_s,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_reads_flow_protocol_without_writes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::create_dir_all(dir.path().join("research")).unwrap();
        std::fs::write(
            dir.path().join("data/literature_inventory.csv"),
            "rel_path\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("research/protocol.yaml"), "meta: {project: rram_fixture}\npaths: {source_root: 'pdfs'}\nseeds: {global: 42}\ncodex: {timeout_s: 1800, effort_ladder: [high, max]}\n").unwrap();
        assert!(detect(dir.path()));
        let m = manifest(
            dir.path(),
            &super::super::manifest::LocateOptions::default(),
        )
        .unwrap();
        assert_eq!(m.research_id, "rram_fixture");
        assert_eq!(m.settings["codex"]["effort_ladder"][1], "max");
        assert_eq!(m.versions.seeds["global"], 42);
        assert!(!m
            .capabilities
            .contains(&super::super::manifest::Capability::RawMeasurements));
        assert!(!dir.path().join("research-manifest.json").exists());
    }
}
#[cfg(test)]
mod extra_tests {
    use super::*;
    fn put(root: &Path, path: &str, content: &str) {
        let p = root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
    fn listing(root: &Path) -> Vec<(String, u64)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let m = entry.metadata().unwrap();
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    if m.is_dir() { 0 } else { m.len() },
                ));
                if m.is_dir() {
                    walk(root, &path, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort();
        out
    }
    #[test]
    fn manifest_contract_and_no_writes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        put(
            root,
            "research/protocol.yaml",
            "meta: {project: example}\npaths: {source_root: pdfs}\ncodex: {timeout_s: 123}\n",
        );
        let specs = [
            (
                "inventory",
                "inventory",
                "data/literature_inventory.csv",
                "csv",
            ),
            (
                "screening",
                "screening",
                "research/screening_audit.csv",
                "csv",
            ),
            ("aliases", "aliases", "data/aliases.csv", "csv"),
            ("evidence", "evidence", "data/evidence.csv", "csv"),
            (
                "measurements",
                "measurements",
                "data/measurements.csv",
                "csv",
            ),
            (
                "raw_measurements",
                "raw_measurements",
                "data/raw_measurements.csv",
                "csv",
            ),
            (
                "progress_screen",
                "progress",
                "state/progress/screen.json",
                "json",
            ),
            ("driver", "driver", "state/progress/driver.json", "json"),
            ("completed", "completed", "state/completed.jsonl", "jsonl"),
            ("pilot", "pilot", "research/pilot_manifest.csv", "csv"),
            (
                "acceptance",
                "acceptance",
                "research/acceptance_manifest.csv",
                "csv",
            ),
            (
                "acceptance_sha",
                "other",
                "research/acceptance_manifest.sha256",
                "text",
            ),
            ("splits", "splits", "research/split_manifest.csv", "csv"),
            (
                "splits_sha",
                "other",
                "research/split_manifest.sha256",
                "text",
            ),
            (
                "validation",
                "validation",
                "research/validation_report.json",
                "json",
            ),
            ("effort_lock", "gate", "research/effort_lock.json", "json"),
            ("recall", "gate", "research/recall_check_final.json", "json"),
            (
                "raw_summary",
                "raw_summary",
                "research/raw_extract_summary.json",
                "json",
            ),
            ("protocol", "protocol", "research/protocol.yaml", "text"),
        ];
        for (_, _, path, _) in specs {
            if path != "research/protocol.yaml" {
                put(root, path, "x");
            }
        }
        put(root, "src/hfo2/prompts/screen.md", "prompt");
        put(root, "pyproject.toml", "requires-python = '>=3.11'\n");
        put(root, ".git/HEAD", "ref: refs/heads/main\n");
        put(root, ".git/refs/heads/main", "revision-ref\n");
        put(root, "state/source_snapshot_20240101.json", "{}");
        put(root, "state/source_snapshot_20250101.json", "{}");
        let before = listing(root);
        let m = manifest(root, &Default::default()).unwrap();
        assert_eq!(listing(root), before);
        assert_eq!(m.artifacts.len(), specs.len());
        for (id, role, path, format) in specs {
            let a = m.artifacts.iter().find(|a| a.id == id).unwrap();
            assert_eq!(serde_json::to_value(&a.role).unwrap(), role);
            assert_eq!(a.path, path);
            assert_eq!(serde_json::to_value(&a.format).unwrap(), format);
            let expected = match id {
                "inventory" => serde_json::json!({"path":"rel_path", "bytes":"size_bytes"}),
                "screening" => {
                    serde_json::json!({"sha256":"pdf_sha256", "path":"rel_path", "reason":"scope_reason"})
                }
                "evidence" | "measurements" => serde_json::json!({"sha256":"pdf_sha256"}),
                _ => serde_json::json!({}),
            };
            assert_eq!(serde_json::to_value(&a.columns).unwrap(), expected, "{id}");
            assert_eq!(
                a.documents_root.as_deref(),
                if id == "inventory" { Some("pdf") } else { None }
            );
        }
        for (id, input, pin) in [
            ("acceptance", "acceptance", "acceptance_sha"),
            ("splits", "splits", "splits_sha"),
            (
                "validation",
                "measurements",
                "validation#/measurements_sha256",
            ),
        ] {
            let p = &m
                .artifacts
                .iter()
                .find(|a| a.id == id)
                .unwrap()
                .produced
                .as_ref()
                .unwrap()
                .inputs;
            assert_eq!(p.len(), 1);
            assert_eq!(p[0].artifact, input);
            assert_eq!(p[0].sha256_from.as_deref(), Some(pin));
        }
        let stages = serde_json::to_value(&m.stages).unwrap();
        let expected = serde_json::json!([
            {"id":"inventory","label":"inventory","progress":null,"outputs":["inventory"],"gate":null},
            {"id":"screen","label":"screen","progress":"progress_screen","outputs":["screening"],"gate":{"artifact":"recall","pointer":"/passed","op":"eq","value":true,"setting":null}},
            {"id":"pilot","label":"pilot","progress":null,"outputs":["pilot"],"gate":{"artifact":"effort_lock","pointer":"/pilot/accuracy","op":"ge","value":null,"setting":"/pilot/accuracy_threshold"}},
            {"id":"acceptance","label":"acceptance","progress":null,"outputs":["acceptance"],"gate":{"artifact":"effort_lock","pointer":"/acceptance/accuracy","op":"ge","value":null,"setting":"/acceptance/accuracy_threshold"}},
            {"id":"extract","label":"extract","progress":null,"outputs":["measurements","evidence"],"gate":{"artifact":"validation","pointer":"/n_errors","op":"eq","value":0,"setting":null}},
            {"id":"splits","label":"splits","progress":null,"outputs":["splits"],"gate":null}
        ]);
        assert_eq!(stages, expected);
        assert_eq!(
            serde_json::to_value(&m.links).unwrap(),
            serde_json::json!([
                {"from":"evidence","to":"measurements","key":"evidence_id","separator":";"},
                {"from":"inventory","to":"screening","key":"path","separator":null},
                {"from":"measurements","to":"splits","key":"paper_id","separator":null}
            ])
        );
        let groups = m.groups.unwrap();
        assert_eq!(
            (groups.pilot.as_str(), groups.acceptance.as_str(), groups.by),
            ("pilot", "acceptance", GroupBy::StudyGroups)
        );
        assert_eq!(m.stall_after_s, Some(123));
        assert_eq!(m.roots["pdf"], "pdfs");
        assert_eq!(
            m.versions.prompts["screen.md"],
            crate::seams::sha256_hex(b"prompt")
        );
        assert_eq!(m.versions.code.as_deref(), Some("revision-ref"));
        assert_eq!(m.versions.environment["requires-python"], ">=3.11");
        assert_eq!(m.research_version, "20250101");
        put(root, ".git/HEAD", "detached-revision\n");
        fs::remove_file(root.join("state/source_snapshot_20240101.json")).unwrap();
        fs::remove_file(root.join("state/source_snapshot_20250101.json")).unwrap();
        fs::remove_file(root.join("research/recall_check_final.json")).unwrap();
        let m = manifest(root, &Default::default()).unwrap();
        assert_eq!(m.versions.code.as_deref(), Some("detached-revision"));
        assert_eq!(m.research_version, "unversioned");
        assert!(!m.artifacts.iter().any(|a| a.id == "recall"));
    }
    #[test]
    fn nested_flow_unread_and_artifact_availability() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("data")).unwrap();
        std::fs::create_dir_all(d.path().join("research")).unwrap();
        std::fs::write(d.path().join("data/literature_inventory.csv"), "").unwrap();
        std::fs::write(d.path().join("data/raw_measurements.csv"), "").unwrap();
        std::fs::write(d.path().join("research/protocol.yaml"),"meta: {project: test}\npaths: {source_root: 'pdfs'}\nmodels: {nested: {values: [1, 2]}, label: 'x,y'}\nbad: [unfinished\n").unwrap();
        let m = manifest(d.path(), &Default::default()).unwrap();
        assert_eq!(m.settings["models"]["nested"]["values"][1], 2);
        assert_eq!(m.settings["_unread"][0], "bad");
        assert!(m.capabilities.contains(&Capability::RawMeasurements));
        assert!(!m.capabilities.contains(&Capability::PdfPages));
        assert_eq!(
            m.artifacts
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["inventory", "raw_measurements", "protocol"]
        );
    }
}
