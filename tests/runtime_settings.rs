mod common;
use axiom::{base::Failure, bridge::{Bridge, Config}, codex::slots::{self, Runtime}};
use std::{collections::BTreeMap, fs::{self, OpenOptions, TryLockError}, path::{Path, PathBuf}, sync::Mutex, time::{Duration, Instant}};

static ENV: Mutex<()> = Mutex::new(());
struct Env(&'static str, Option<std::ffi::OsString>);
impl Env {
    fn set(key: &'static str, value: Option<&str>) -> Self {
        let old = std::env::var_os(key);
        unsafe { match value { Some(v) => std::env::set_var(key, v), None => std::env::remove_var(key) } }
        Self(key, old)
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        unsafe { match &self.1 { Some(v) => std::env::set_var(self.0, v), None => std::env::remove_var(self.0) } }
    }
}
fn runtime(cap: u64, command: &[&str]) -> Runtime {
    Runtime { codex_max_parallel: cap, codex_command: command.iter().map(|s| s.to_string()).collect() }
}
fn held(home: &Path, index: u64) {
    let file = OpenOptions::new().read(true).write(true).open(home.join(format!("codex-slots/{index}.lock"))).unwrap();
    assert!(matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
}
fn snapshot(home: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path == root.join("settings") { continue; }
            if path.is_dir() { visit(root, &path, files); }
            else {
                // Windows denies reads of locked files; a zero-length file has exactly no bytes.
                let bytes = if fs::metadata(&path).unwrap().len() == 0 { Vec::new() } else { fs::read(&path).unwrap() };
                files.insert(path.strip_prefix(root).unwrap().into(), bytes);
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(home, home, &mut files);
    files
}

#[test]
fn cap_applies_to_next_acquire() {
    let t = common::temp(); let home = t.path();
    assert_eq!(slots::save_runtime(home, &runtime(1, &[])).unwrap(), 1);
    let first = slots::acquire(home, slots::cap(home).unwrap(), &mut Default::default(), || Ok(())).unwrap();
    assert_eq!(slots::save_runtime(home, &runtime(2, &[])).unwrap(), 2);
    assert_eq!(slots::cap(home).unwrap(), 2);
    let start = Instant::now(); let mut wait = slots::Wait::default();
    let second = slots::acquire(home, slots::cap(home).unwrap(), &mut wait, || {
        if start.elapsed() > Duration::from_secs(2) { Err(Failure::new("cancel", "deadline")) } else { Ok(()) }
    }).unwrap();
    assert!(!wait.full);
    assert_eq!(slots::occupancy(home).unwrap().total, 2);
    held(home, 0); held(home, 1);
    drop((first, second));
}

#[test]
fn cap_precedence() {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _env = Env::set(slots::ENV, None);
    let t = common::temp(); let home = t.path();
    assert_eq!(slots::cap(home).unwrap(), slots::DEFAULT);
    let _value = Env::set(slots::ENV, Some("3"));
    assert_eq!(slots::cap(home).unwrap(), 3);
    slots::save_runtime(home, &runtime(2, &[])).unwrap();
    assert_eq!(slots::cap(home).unwrap(), 2);
}

#[test]
fn lowering_cap_never_preempts() {
    let t = common::temp(); let home = t.path();
    slots::save_runtime(home, &runtime(2, &[])).unwrap();
    let a = slots::acquire(home, slots::cap(home).unwrap(), &mut Default::default(), || Ok(())).unwrap();
    let b = slots::acquire(home, slots::cap(home).unwrap(), &mut Default::default(), || Ok(())).unwrap();
    slots::save_runtime(home, &runtime(1, &[])).unwrap();
    assert_eq!(slots::cap(home).unwrap(), 1);
    assert_eq!(slots::occupancy(home).unwrap().total, 2);
    held(home, 0); held(home, 1);
    let start = Instant::now(); let mut wait = slots::Wait::default();
    let result = slots::acquire(home, slots::cap(home).unwrap(), &mut wait, || {
        if start.elapsed() >= Duration::from_millis(100) { Err(Failure::new("cancel", "stopped")) } else { Ok(()) }
    });
    let error = result.err().expect("full pool must cancel");
    assert_eq!(error.kind, "cancel"); assert_eq!(error.message, "stopped");
    assert!(wait.full);
    assert_eq!(slots::occupancy(home).unwrap().total, 2);
    held(home, 0); held(home, 1); drop((a, b));
}

#[test]
fn command_applies_to_new_bridge_only() {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _env = Env::set("AXIOM_CODEX_CMD", None);
    let t = common::temp(); let home = t.path();
    slots::save_runtime(home, &runtime(1, &["a", "b"])).unwrap();
    let a = Bridge::new(Config::from_home(Some(home.into())));
    assert_eq!(a.config.codex.as_ref().unwrap(), &["a", "b"]);
    slots::save_runtime(home, &runtime(1, &["c"])).unwrap();
    assert_eq!(a.config.codex.as_ref().unwrap(), &["a", "b"]);
    let b = Bridge::new(Config::from_home(Some(home.into())));
    assert_eq!(b.config.codex.as_ref().unwrap(), &["c"]);
}

#[test]
fn command_env_beats_runtime() {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _env = Env::set("AXIOM_CODEX_CMD", Some(r#"["env-cmd"]"#));
    let t = common::temp();
    slots::save_runtime(t.path(), &runtime(1, &["runtime-cmd"])).unwrap();
    assert_eq!(Config::from_home(Some(t.path().into())).codex.unwrap(), vec!["env-cmd"]);
}

#[test]
fn invalid_rejected_atomically() {
    let t = common::temp(); let home = t.path();
    let old = runtime(2, &["old"]);
    for prior in [false, true] {
        if prior { assert_eq!(slots::save_runtime(home, &old).unwrap(), 1); }
        let before = prior.then(|| fs::read(home.join("settings/runtime.json")).unwrap());
        for invalid in [runtime(0, &[]), runtime(1, &["cmd", ""])] {
            assert_eq!(invalid.validate().unwrap_err().kind, "invalid_argument");
            let error = slots::save_runtime(home, &invalid).unwrap_err();
            assert_eq!(error.kind, "invalid_argument");
            if invalid.codex_max_parallel == 0 { assert_eq!(error.message, slots::POSITIVE_INTEGER); }
            if let Some(bytes) = &before {
                assert_eq!(&fs::read(home.join("settings/runtime.json")).unwrap(), bytes);
                assert_eq!(slots::load_runtime(home).unwrap(), Some((old.clone(), 1)));
            } else { assert!(!home.join("settings").exists()); }
        }
    }
}

#[test]
fn version_increments_and_no_temp_files() {
    let t = common::temp(); let home = t.path(); let r = runtime(3, &[]);
    assert_eq!(slots::load_runtime(home).unwrap(), None);
    assert_eq!(slots::save_runtime(home, &r).unwrap(), 1);
    assert_eq!(slots::load_runtime(home).unwrap(), Some((r.clone(), 1)));
    assert_eq!(slots::save_runtime(home, &r).unwrap(), 2);
    assert_eq!(slots::load_runtime(home).unwrap(), Some((r.clone(), 2)));
    assert_eq!(fs::read_dir(home.join("settings")).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("runtime.json")]);
    fs::write(home.join("settings/runtime.json"), b"damaged").unwrap();
    assert!(slots::load_runtime(home).is_err()); assert!(slots::cap(home).is_err());
    assert_eq!(slots::save_runtime(home, &r).unwrap(), 1);
    fs::write(home.join("settings/runtime.json"), br#"{"codex_max_parallel":4,"version":8}"#).unwrap();
    assert_eq!(slots::load_runtime(home).unwrap(), Some((runtime(4, &[]), 8)));
}

#[test]
fn save_has_no_side_effects() {
    let t = common::temp(); let home = t.path();
    for (name, bytes) in [("goals/g.json", "{\"status\":\"done\"}"), ("briefs/u.md", "brief"), ("evidence/u.json", "{\"valid\":true}")] {
        let path = home.join(name); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(path, bytes).unwrap();
    }
    let slot = slots::acquire(home, 1, &mut Default::default(), || Ok(())).unwrap();
    let before = snapshot(home);
    assert_eq!(slots::save_runtime(home, &runtime(2, &["cmd"])).unwrap(), 1);
    assert_eq!(snapshot(home), before);
    held(home, 0); assert_eq!(slots::occupancy(home).unwrap().total, 1);
    assert_eq!(fs::read_dir(home.join("settings")).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), vec![std::ffi::OsString::from("runtime.json")]);
    drop(slot);
}

#[test]
fn descriptor_describes_routing() {
    let tools = axiom::bridge::server::tools();
    let description = tools.as_array().unwrap().iter().find(|t| t["name"] == "codex_run").unwrap()["description"].as_str().unwrap();
    for text in ["A registered brief can set EFFORT and TIER; unit mode takes no argument for them.", "xhigh", "astra", "routing", "map"] {
        assert!(description.contains(text), "missing {text}: {description}");
    }
}
