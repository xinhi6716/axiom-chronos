//! Resume compatibility contract (fixed before any run): existing units may move only
//! among pending, claimed, running, verified, failed, superseded, paused and resumed;
//! new events, receipts and units may be appended. All other existing data, including
//! unknown JSON fields and completed work, must remain field-equal or byte-identical.
//! Verified work must never be executed or charged again.
mod common;

use axiom::{
    app::Poller,
    bridge::{call, Bridge, Config},
    goal, orch, session,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
};

const STATES: &[&str] = &[
    "pending",
    "claimed",
    "running",
    "verified",
    "failed",
    "superseded",
    "paused",
    "resumed",
];

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        assert!(
            !ty.is_symlink(),
            "refuse symlink in source: {:?}",
            entry.path()
        );
        if ty.is_dir() {
            copy_tree(&entry.path(), &dst.join(entry.file_name()));
        } else if ty.is_file() {
            fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        } else {
            panic!("unsupported source entry: {:?}", entry.path());
        }
    }
}
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let ty = e.file_type().unwrap();
            assert!(!ty.is_symlink(), "symlink {:?}", p);
            if ty.is_dir() {
                walk(root, &p, out);
            } else if ty.is_file() {
                out.insert(p.strip_prefix(root).unwrap().into(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}
fn safe_path(home: &Path, path: &str) -> bool {
    let p = Path::new(path);
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    if p.is_absolute() {
        p.starts_with(home)
    } else {
        !path.contains(':') && !path.starts_with(['/', '\\'])
    }
}
fn goal_fields(bytes: &[u8]) -> Value {
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    let obj = v.as_object_mut().unwrap();
    for k in ["status", "reason", "note"] {
        obj.remove(k);
    }
    v
}
fn unit_rows(bytes: &[u8]) -> Result<BTreeMap<String, Vec<String>>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some("id\ttrack\tstate\tbranch\tpr\tsha\tbrief") {
        return Err("units header".into());
    }
    let mut out = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let fields: Vec<String> = line.split('\t').map(str::to_owned).collect();
        if fields.len() != 7 || out.insert(fields[0].clone(), fields.clone()).is_some() {
            return Err("malformed or duplicate unit".into());
        }
    }
    Ok(out)
}
fn json_lines(bytes: &[u8]) -> Result<Vec<Value>, String> {
    std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| serde_json::from_str(s).map_err(|e| e.to_string()))
        .collect()
}
fn compare(
    home: &Path,
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), String> {
    let units_path = Path::new("store/units.tsv");
    for snapshot in [before, after] {
        let goal: Value =
            serde_json::from_slice(snapshot.get(Path::new("goal.json")).ok_or("missing goal")?)
                .map_err(|e| e.to_string())?;
        let cwd = goal["cwd"].as_str().ok_or("missing goal cwd")?;
        if !Path::new(cwd).is_absolute() || !safe_path(home, cwd) {
            return Err(format!("escaping goal cwd: {cwd}"));
        }
    }
    let old_units = unit_rows(before.get(units_path).ok_or("missing original units")?)?;
    let units = unit_rows(after.get(units_path).ok_or("missing units")?)?;
    for row in units.values() {
        if !row[6].is_empty() && !safe_path(home, &row[6]) {
            return Err(format!("escaping brief: {}", row[6]));
        }
    }
    for (id, old) in &old_units {
        let new = units.get(id).ok_or_else(|| format!("deleted unit {id}"))?;
        for i in [0, 1, 3, 4, 5, 6] {
            if old[i] != new[i] {
                return Err(format!("changed unit {id}"));
            }
        }
        if old[2] != new[2]
            && (old[2] == "verified"
                || !STATES.contains(&old[2].as_str())
                || !STATES.contains(&new[2].as_str()))
        {
            return Err(format!("illegal transition {id}"));
        }
    }
    for (path, old) in before {
        let new = after.get(path).ok_or_else(|| format!("deleted {path:?}"))?;
        if path == Path::new("goal.json") {
            if goal_fields(old) != goal_fields(new) {
                return Err("goal fields changed".into());
            }
        } else if path == units_path {
            continue;
        } else if path == Path::new("store/events.jsonl")
            || path.starts_with("store/receipts") && path.extension().is_some_and(|x| x == "jsonl")
        {
            if !new.starts_with(old) {
                return Err(format!("changed history {path:?}"));
            }
        } else if old != new {
            return Err(format!("changed {path:?}"));
        }
    }
    for (path, new) in after {
        if !before.contains_key(path) {
            let append_log = path == Path::new("store/events.jsonl")
                || path == Path::new("store/receipts.json")
                || path.starts_with("store/receipts")
                    && path.extension().is_some_and(|x| x == "jsonl");
            let lock = path.extension().is_some_and(|x| x == "lock");
            let new_unit_artifact = path
                .file_name()
                .and_then(|x| x.to_str())
                .is_some_and(|name| {
                    units
                        .keys()
                        .filter(|id| !old_units.contains_key(*id))
                        .any(|id| name.starts_with(&format!("{id}.")))
                });
            if !(append_log || lock || new_unit_artifact) {
                return Err(format!("unexpected new file {path:?}"));
            }
        }
        if path == Path::new("store/events.jsonl") {
            let mut seq = HashSet::new();
            let mut charges = HashSet::new();
            let n = before
                .get(path)
                .map(|b| json_lines(b).map(|v| v.len()))
                .transpose()?
                .unwrap_or(0);
            for (i, event) in json_lines(new)?.iter().enumerate() {
                if !seq.insert(event["seq"].clone()) {
                    return Err("duplicate event sequence".into());
                }
                if event["data"]["call"] == "codex_run" && event["data"]["attempt_id"].is_string() {
                    let key = (
                        event["unit"].to_string(),
                        event["data"]["attempt_id"].to_string(),
                    );
                    if !charges.insert(key) {
                        return Err("duplicate charge".into());
                    }
                    if i >= n
                        && old_units
                            .get(event["unit"].as_str().unwrap_or(""))
                            .is_some_and(|u| u[2] == "verified")
                    {
                        return Err("completed unit charged again".into());
                    }
                }
            }
        }
        if path == Path::new("store/receipts/runs.jsonl") {
            let mut attempts = HashSet::new();
            for run in json_lines(new)? {
                let key = (run["unit"].to_string(), run["attempt"].to_string());
                if !attempts.insert(key) {
                    return Err("duplicate execution/charge".into());
                }
            }
            let n = before
                .get(path)
                .map(|b| json_lines(b).map(|v| v.len()))
                .transpose()?
                .unwrap_or(0);
            for run in json_lines(new)?.iter().skip(n) {
                if old_units
                    .get(run["unit"].as_str().unwrap_or(""))
                    .is_some_and(|u| u[2] == "verified")
                {
                    return Err("completed unit rerun".into());
                }
            }
        }
    }
    Ok(())
}
fn snapshot(home: &Path, store: &Path, session_id: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    out.insert(
        "goal.json".into(),
        fs::read(home.join("goals").join(format!("{session_id}.json"))).unwrap(),
    );
    for (p, bytes) in files(store) {
        out.insert(Path::new("store").join(p), bytes);
    }
    out
}
fn bridge(home: &Path) -> Bridge {
    let mut c = Config::from_home(Some(home.into()));
    let flag = format!("--fake-log={}", home.join("codex.log").display());
    c.codex = Ok(common::fake_codex(&[&flag]));
    c.wsl = None;
    c.on_windows = false;
    Bridge::new(c)
}
fn cycle(home: &Path, store: &Path, id: &str, project: &str, completed: Option<&str>) {
    let baseline = snapshot(home, store, id);
    let mut previous = baseline.clone();
    // Initial fixtures are paused. First reopen allows an actual interrupt, then resume twice.
    goal::reopen(home, id).unwrap();
    goal::interrupt(home, id, "fixture pause")
        .unwrap()
        .expect("must pause");
    for _ in 0..2 {
        let wrapper = home.join("fake-claude-wrapper.py");
        fs::write(&wrapper, format!("import os,runpy,sys,json,pathlib\nos.environ['FAKE_CLAUDE_SCENARIO']='resume'\nos.environ['FAKE_CLAUDE_SESSION']={}\nos.environ['FAKE_CLAUDE_RECORD']={}\nwrite=pathlib.Path.write_text\ndef pause(path,data,*args,**kwargs):\n if path.parent.name=='goals' and path.name==os.environ['FAKE_CLAUDE_SESSION']+'.json':\n  goal=json.loads(data); goal['status']='blocked'; goal['reason']='interrupted'; data=json.dumps(goal)\n return write(path,data,*args,**kwargs)\npathlib.Path.write_text=pause\nsys.argv=sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n", json!(id), json!(home.join("fake-claude.json").to_string_lossy()))).unwrap();
        let mut cfg = session::Config::from_env();
        cfg.home = home.into();
        cfg.bin = home.join("axiom");
        let fake = common::fake("fake_claude");
        cfg.claude = Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            fake[2].clone(),
        ]);
        let mut output = Vec::new();
        let code = session::resume(
            &cfg,
            id,
            None,
            None,
            Box::new(Cursor::new(Vec::<u8>::new())),
            &mut output,
        )
        .unwrap();
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
        let record: Value =
            serde_json::from_slice(&fs::read(home.join("fake-claude.json")).unwrap()).unwrap();
        let argv = record["argv"].as_array().unwrap();
        assert!(argv.windows(2).any(|a| a[0] == "--resume" && a[1] == id));
        assert_eq!(goal::load(home, id).unwrap().unwrap().status, "blocked");
        let b = bridge(home);
        let repo = home.join("project");
        let result = call(
            &b,
            "orch",
            &json!({"project":project,"args":["resume", "--repo", repo]}),
        );
        assert_eq!(result["isError"], false, "{result}");
        if let Some(unit) = completed {
            b.codex().unwrap().read_rate_limits().unwrap();
            assert!(fs::read_to_string(home.join("codex.log"))
                .unwrap()
                .contains("initialize"));
            let result = call(
                &b,
                "codex_run",
                &json!({"project":project,"unit":unit,"role":"terra","cwd":home.join("project"),"cycle":true}),
            );
            assert_eq!(
                result["structuredContent"]["error"]["kind"], "already_verified",
                "{result}"
            );
            assert!(!fs::read_to_string(home.join("codex.log"))
                .unwrap_or_default()
                .contains("turn/start"));
        }
        let view = Poller::new(home).poll();
        assert!(
            view.wishes
                .iter()
                .any(|w| w.session == id && w.store.as_deref() == Some(store)),
            "{view:?}"
        );
        let current = snapshot(home, store, id);
        compare(home, &baseline, &current).unwrap();
        compare(home, &previous, &current).unwrap();
        previous = current;
        // Everything in b and Poller is discarded; next iteration reloads from disk.
    }
}
fn fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = common::temp();
    let home = temp.path();
    let src = common::repo_root()
        .join("tests/fixtures/resume_201")
        .join(name);
    let store = home.join("orchestrate/demo");
    copy_tree(&src.join("store"), &store);
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::create_dir_all(home.join("project")).unwrap();
    let mut g: Value = serde_json::from_slice(&fs::read(src.join("goal.json")).unwrap()).unwrap();
    g["cwd"] = json!(home.join("project"));
    fs::write(
        home.join("goals/legacy.json"),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    let units = store.join("units.tsv");
    let text = fs::read_to_string(&units).unwrap().replace(
        "/work/project/briefs/",
        &format!(
            "{}/",
            store.join("briefs").to_string_lossy().replace('\\', "/")
        ),
    );
    fs::write(units, text).unwrap();
    (temp, store)
}
#[test]
fn copied_and_synthetic_resume() {
    for name in ["old", "old-crlf", "synthetic"] {
        let (temp, store) = fixture(name);
        cycle(
            temp.path(),
            &store,
            "legacy",
            "demo",
            (name == "synthetic").then_some("done"),
        );
    }
}
#[test]
fn comparison_negative_controls() {
    let (temp, store) = fixture("synthetic");
    let baseline = snapshot(temp.path(), &store, "legacy");
    let mut deleted = baseline.clone();
    deleted.remove(Path::new("store/wish.md"));
    assert!(compare(temp.path(), &baseline, &deleted).is_err());
    let mut duplicate = baseline.clone();
    let runs = duplicate
        .get_mut(Path::new("store/receipts/runs.jsonl"))
        .unwrap();
    runs.extend_from_slice(&runs.clone());
    assert!(compare(temp.path(), &baseline, &duplicate).is_err());
    let mut charged = baseline.clone();
    let events = charged.get_mut(Path::new("store/events.jsonl")).unwrap();
    for seq in [2, 3] {
        events.extend_from_slice(format!("{}\n", json!({"seq":seq,"kind":"role_spend","unit":"other","data":{"call":"codex_run","attempt_id":"same"}})).as_bytes());
    }
    assert!(compare(temp.path(), &baseline, &charged).is_err());
    for path in [
        "../escape",
        if cfg!(windows) {
            "C:/outside/brief"
        } else {
            "/outside/brief"
        },
    ] {
        let mut escape = baseline.clone();
        let p = Path::new("store/units.tsv");
        let text = String::from_utf8(escape[p].clone()).unwrap();
        let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
        let mut row: Vec<_> = lines[1].split('\t').map(str::to_owned).collect();
        row[6] = path.into();
        lines[1] = row.join("\t");
        escape.insert(p.into(), format!("{}\n", lines.join("\n")).into_bytes());
        assert!(compare(temp.path(), &baseline, &escape).is_err());
    }
    let mut dropped = baseline.clone();
    let mut g: Value = serde_json::from_slice(&dropped[Path::new("goal.json")]).unwrap();
    g.as_object_mut().unwrap().remove("future_goal");
    dropped.insert("goal.json".into(), serde_json::to_vec(&g).unwrap());
    assert!(compare(temp.path(), &baseline, &dropped).is_err());
    let mut dropped_store = baseline.clone();
    let p = Path::new("store/frontier.json");
    let mut v: Value = serde_json::from_slice(&dropped_store[p]).unwrap();
    v.as_object_mut().unwrap().remove("future_store");
    dropped_store.insert(p.into(), serde_json::to_vec(&v).unwrap());
    assert!(compare(temp.path(), &baseline, &dropped_store).is_err());
}

// The held-out source is never locked; all path rewrites and all resume work are on the copy.
#[test]
#[ignore]
fn heldout_rram_copy() {
    let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .expect("user home is unavailable");
    let source_home = PathBuf::from(user_home).join(".axiom");
    let source_store = source_home.join("orchestrate/ml_hfo2_v2");
    assert!(
        source_store.is_dir(),
        "missing held-out store: {}",
        source_store.display()
    );
    let mut goals = Vec::new();
    for e in fs::read_dir(source_home.join("goals")).expect("missing held-out goals") {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() && e.path().extension().is_some_and(|x| x == "json") {
            let bytes = fs::read(e.path()).unwrap();
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if v["project"] == "ml_hfo2_v2" {
                    goals.push((e.file_name(), v));
                }
            }
        }
    }
    assert_eq!(goals.len(), 1, "expected exactly one held-out goal");
    let (filename, mut g) = goals.pop().unwrap();
    let id = Path::new(&filename)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let old_cwd = g["cwd"]
        .as_str()
        .expect("held-out goal must have cwd")
        .to_owned();
    let temp = common::temp();
    let home = temp.path();
    let copied = home.join("orchestrate/ml_hfo2_v2");
    copy_tree(&source_store, &copied);
    let project = home.join("project");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(home.join("goals")).unwrap();
    // Replace known roots in every UTF-8 record, including JSON-escaped variants.
    // Then remove any remaining Windows user paths, including those unrelated to cwd.
    let roots = [
        (old_cwd, project.clone()),
        (
            source_home.to_string_lossy().into_owned(),
            home.to_path_buf(),
        ),
    ];
    let win_user = regex::Regex::new(r#"(?i)[a-z]:[/\\]+users[/\\]+[^\r\n\t"']+"#).unwrap();
    for (rel, bytes) in files(&copied) {
        let Ok(mut text) = String::from_utf8(bytes) else {
            continue;
        };
        for (old, new) in &roots {
            let new = new.to_string_lossy().replace('\\', "/");
            for old in [
                old.clone(),
                old.replace('\\', "/"),
                old.replace('\\', "\\\\"),
            ] {
                if !old.is_empty() {
                    text = text.replace(&old, &new);
                }
            }
        }
        text = win_user
            .replace_all(&text, project.to_string_lossy().replace('\\', "/").as_str())
            .into_owned();
        fs::write(copied.join(rel), text).unwrap();
    }
    fn remap_goal(v: &mut Value, roots: &[(String, PathBuf)], re: &regex::Regex, fallback: &Path) {
        match v {
            Value::String(s) => {
                for (old, new) in roots {
                    let replacement = new.to_string_lossy().replace('\\', "/");
                    for root in [old.clone(), old.replace('\\', "/")] {
                        if !root.is_empty() {
                            *s = s.replace(&root, &replacement);
                        }
                    }
                }
                *s = re
                    .replace_all(s, fallback.to_string_lossy().replace('\\', "/").as_str())
                    .into_owned();
            }
            Value::Array(a) => {
                for value in a {
                    remap_goal(value, roots, re, fallback);
                }
            }
            Value::Object(m) => {
                for value in m.values_mut() {
                    remap_goal(value, roots, re, fallback);
                }
            }
            _ => {}
        }
    }
    remap_goal(&mut g, &roots, &win_user, &project);
    g["cwd"] = json!(project);
    fs::write(
        home.join("goals").join(filename),
        serde_json::to_vec(&g).unwrap(),
    )
    .unwrap();
    let goal = goal::load(home, &id).unwrap().unwrap();
    let store = orch::store_for_goal(home, &goal).unwrap();
    assert!(
        store.is_dir(),
        "held-out goal store absent from copied project: {}",
        store.display()
    );
    // Ensure every unit brief is contained even if the original used a different absolute root.
    let units_path = store.join("units.tsv");
    let data = fs::read_to_string(&units_path).unwrap();
    let mut lines = Vec::new();
    for (i, line) in data.lines().enumerate() {
        if i == 0 {
            lines.push(line.to_owned());
            continue;
        }
        let mut cols: Vec<_> = line.split('\t').map(str::to_owned).collect();
        assert_eq!(cols.len(), 7);
        if !cols[6].is_empty() && !safe_path(home, &cols[6]) {
            let name = cols[6].rsplit(['/', '\\']).next().unwrap();
            cols[6] = store
                .join("briefs")
                .join(name)
                .to_string_lossy()
                .into_owned();
        }
        lines.push(cols.join("\t"));
    }
    fs::write(units_path, format!("{}\n", lines.join("\n"))).unwrap();
    cycle(home, &store, &id, "ml_hfo2_v2", None);
}

#[test]
fn legacy_verified_unit_is_not_reexecuted() {
    let (temp, store) = fixture("synthetic");
    // Before the fix this assertion fails: a verified legacy unit with old evidence
    // but no modern run row is treated as pending and can be claimed again.
    let result = orch::begin_unit(&store, "done", "terra", &temp.path().join("project"), false);
    assert_eq!(
        result.err().map(|e| e.kind),
        Some("already_verified".into())
    );
}

#[test]
fn verified_unit_with_run_evidence_is_not_reexecuted_after_tree_change() {
    let (temp, store) = fixture("synthetic");
    fs::write(store.join("reports/done.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nrun\tcodex\t0\told-tree\t2026-01-01\nverify\techo fixture\t0\told-tree\t2026-01-01\naccept\techo fixture\t0\told-tree\t2026-01-01\nreview\treviewer\t0\told-tree\t2026-01-01\n").unwrap();
    // Before the fix, a changed tree causes the completed unit to be claimed again.
    let result = orch::begin_unit(&store, "done", "terra", &temp.path().join("project"), false);
    assert_eq!(
        result.err().map(|e| e.kind),
        Some("already_verified".into())
    );
}

#[test]
// The earlier kill_after_worker refusal was at the live-owner check, before
// the verified-unit checks. Its log has neither the claim PID nor the Windows
// OpenProcess error, so it cannot distinguish PID reuse/access denial from a
// parent-side held entry. The aborting child has its own RUNNING map; its
// entries cannot by themselves survive in the parent. Do not infer a root
// cause from passing reruns without those missing diagnostics.
fn verified_unit_checkpoint_resumes_only_after_worker_done() {
    let (temp, store) = fixture("synthetic");
    let units = store.join("units.tsv");
    let original = fs::read_to_string(&units).unwrap();
    fs::write(&units, original.replace("\tverified\t", "\tpending\t")).unwrap();
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&store, "done", "terra", &temp.path().join("project"), false).unwrap()
    else {
        panic!("expected claim")
    };
    let checkpoint_path = store.join("checkpoints/done.json");
    let mut checkpoint: Value = serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
    orch::release_claim(&store, "done", &attempt_id).unwrap();
    fs::write(&units, original).unwrap();
    let mut child = std::process::Command::new(common::python())
        .args(["-c", "pass"]).spawn().unwrap();
    let dead_pid = child.id();
    assert!(child.wait().unwrap().success());
    assert!(!axiom::base::proc::pid_alive(dead_pid), "child PID was reused");
    checkpoint["pid"] = json!(dead_pid);
    fs::write(store.join("claims.tsv"), format!("unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\ndone\t{attempt_id}\tsrc/done\t2026-01-01\tcore\tdemo\t{dead_pid}\t\n")).unwrap();
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    // Before the fix, the stale claimed checkpoint launches a worker for completed work.
    let result = orch::begin_unit(&store, "done", "terra", &temp.path().join("project"), false);
    assert_eq!(
        result.err().map(|e| e.kind),
        Some("already_verified".into())
    );

    checkpoint["stage"] = json!("worker_done");
    fs::write(&checkpoint_path, serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    let orch::Begin::Run { attempt_id: resumed, .. } =
        orch::begin_unit(&store, "done", "terra", &temp.path().join("project"), false)
            .expect("worker_done must resume the existing attempt")
    else {
        panic!("expected resumed attempt")
    };
    assert_eq!(resumed, attempt_id);
    orch::release_claim(&store, "done", &resumed).unwrap();
}
