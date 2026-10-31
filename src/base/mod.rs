use serde::Serialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
pub mod proc;
pub mod schema;
#[derive(Serialize, Debug, Clone)]
pub struct Failure {
    pub kind: String,
    pub message: String,
}
impl Failure {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"kind": self.kind, "message": self.message})
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}
impl std::error::Error for Failure {}
pub type Res<T> = Result<T, Failure>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainOutcome {
    AwaitingCore, Verified, Blocked, Failed, Timeout, Cancelled, Unconfirmed, ProtocolIncomplete,
}
impl DomainOutcome {
    pub fn successful(self) -> bool { matches!(self, Self::AwaitingCore | Self::Verified) }
}
/// Transport completion alone cannot credit a failed/blocked domain outcome.
pub fn tool_succeeded(v: &serde_json::Value) -> bool {
    v["status"].as_str().is_none_or(|s| s == "completed")
        && v.get("outcome").is_none_or(|o| serde_json::from_value::<DomainOutcome>(o.clone()).is_ok_and(DomainOutcome::successful))
}
pub fn resolve_command(env_name: &str, executable: &str) -> Res<Vec<String>> {
    if let Some(raw) = std::env::var_os(env_name).filter(|v| !v.is_empty()) {
        let parsed = raw
            .to_str()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok());
        return parsed
            .filter(|v| !v.is_empty() && v.iter().all(|s| !s.is_empty()))
            .ok_or_else(|| {
                Failure::new(
                    "config",
                    format!("{env_name} must be a non-empty JSON array of non-empty strings"),
                )
            });
    }
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&paths) {
        #[cfg(unix)]
        let candidates = vec![dir.join(executable)];
        #[cfg(windows)]
        let candidates = {
            let extensions =
                std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
            let extensions = extensions.to_string_lossy();
            let extensions: Vec<_> = extensions.split(';').filter(|x| !x.is_empty()).collect();
            if extensions.iter().any(|ext| {
                executable
                    .to_ascii_lowercase()
                    .ends_with(&ext.to_ascii_lowercase())
            }) {
                vec![dir.join(executable)]
            } else {
                extensions
                    .iter()
                    .map(|ext| dir.join(format!("{executable}{ext}")))
                    .collect()
            }
        };
        for path in candidates {
            if path.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if path
                        .metadata()
                        .map(|m| m.permissions().mode() & 0o111 == 0)
                        .unwrap_or(true)
                    {
                        continue;
                    }
                }
                return Ok(vec![path.to_string_lossy().into_owned()]);
            }
        }
    }
    Err(Failure::new(
        "not_installed",
        format!("{executable} is not on PATH; install it or set {env_name}"),
    ))
}
pub fn axiom_home() -> PathBuf {
    std::env::var_os("AXIOM_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .unwrap_or_default(),
            )
            .join(".axiom")
        })
}
pub fn data_home() -> Res<PathBuf> {
    std::env::var_os("AXIOM_DATA_HOME")
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("CLAUDE_PLUGIN_DATA").filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| {
            Failure::new(
                "config",
                "AXIOM_DATA_HOME or CLAUDE_PLUGIN_DATA must be set",
            )
        })
}
pub fn require_slug(name: &str, value: &str) -> Res<()> {
    let b = value.as_bytes();
    if !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
    {
        Ok(())
    } else {
        Err(Failure::new(
            "invalid_argument",
            format!("invalid {name}: {value}"),
        ))
    }
}
pub fn slug_for_dir(dir: &Path) -> String {
    let name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut s = String::new();
    for c in name.chars() {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if c != '-' || !s.ends_with('-') {
            s.push(c);
        }
    }
    let s = s.trim_start_matches(['-', '_']).trim_end_matches('-');
    let s = &s[..s.len().min(64)];
    let s = s.trim_end_matches('-');
    if s.is_empty() {
        "project".into()
    } else {
        s.into()
    }
}
pub fn require_directory(path: &str) -> Res<PathBuf> {
    let p = PathBuf::from(path);
    if p.is_absolute() && p.is_dir() {
        Ok(p)
    } else {
        Err(Failure::new(
            "invalid_argument",
            format!("not an absolute directory: {path}"),
        ))
    }
}
static COUNTER: AtomicU64 = AtomicU64::new(0);
pub fn unique() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    h.update(std::process::id().to_le_bytes());
    h.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Allocate a fresh, short Cargo target for one source and execution. The atomic
/// directory creation is the collision arbiter even across processes.
pub fn cargo_target(source: &Path, execution: &str) -> Res<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    #[cfg(windows)]
    if let Some(drive) = std::env::var_os("SystemDrive") {
        roots.push(PathBuf::from(format!("{}\\", drive.to_string_lossy())));
    }
    #[cfg(not(windows))]
    roots.push(PathBuf::from("/tmp"));
    cargo_target_in(source, execution, &roots)
}

/// Explicit candidates allow callers to test failure without changing process environment.
pub fn cargo_target_in(source: &Path, execution: &str, roots: &[PathBuf]) -> Res<PathBuf> {
    use sha2::{Digest, Sha256};
    let source = std::fs::canonicalize(source)
        .map_err(|e| Failure::new("cargo_target", format!("source identity unavailable: {e}")))?;
    let mut errors = Vec::new();
    for root in roots {
        // Reserve space for Cargo's own deeply nested build artifacts.
        if !root.is_absolute() || root.to_string_lossy().encode_utf16().count() > 50 {
            errors.push(format!("{}: not an absolute short root", root.display()));
            continue;
        }
        for _ in 0..8 {
            let nonce = unique();
            let mut hash = Sha256::new();
            hash.update(source.as_os_str().to_string_lossy().as_bytes());
            hash.update([0]);
            hash.update(execution.as_bytes());
            hash.update([0]);
            hash.update(nonce.as_bytes());
            let name = format!("ct-{:x}", hash.finalize());
            let path = root.join(&name[..27]);
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    let binding = serde_json::json!({"source":source,"execution":execution,"nonce":nonce});
                    match std::fs::write(path.join(".chronos-binding.json"), binding.to_string()) {
                        Ok(()) => return Ok(path),
                        Err(e) => {
                            let _ = std::fs::remove_dir_all(&path);
                            errors.push(format!("{}: {e}", root.display()));
                            break;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => { errors.push(format!("{}: {e}", root.display())); break; }
            }
        }
    }
    Err(Failure::new("cargo_target", format!("no writable short Cargo target root: {}", errors.join("; "))))
}
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    atomic_write_steps(path, contents, |_| Ok(()))
}
fn atomic_write_steps(
    path: &Path,
    contents: &[u8],
    mut step: impl FnMut(&str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing filename"))?
        .to_string_lossy();
    let temp = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), unique()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        step("created")?;
        f.write_all(contents)?;
        step("written")?;
        f.sync_all()?;
        step("synced")?;
        drop(f);
        replace_file(&temp, path)?;
        step("replaced")?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
/// Sync directory entries after rename on Unix. Windows uses write-through
/// MoveFileExW instead (directory FlushFileBuffers is not supported there).
pub fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}
#[cfg(unix)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}
#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}
pub fn now_iso() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sec = (ms / 1000) as i64;
    let (y, m, d) = civil(sec / 86400);
    let t = sec % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        t / 3600,
        t % 3600 / 60,
        t % 60,
        ms % 1000
    )
}
pub fn today() -> String {
    now_iso()[..10].to_string()
}
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Family {
    OpenAi,
    Google,
    Anthropic,
    Unknown,
}
pub fn family_of(model: &str) -> Family {
    if model.starts_with("gpt-")
        || model.starts_with("codex-")
        || (model.as_bytes().first() == Some(&b'o')
            && model.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
    {
        Family::OpenAi
    } else if model.starts_with("gemini-") {
        Family::Google
    } else if model.starts_with("claude-")
        || model.starts_with("chronos:")
        || ["opus", "sonnet", "haiku"].contains(&model)
    {
        Family::Anthropic
    } else {
        Family::Unknown
    }
}

#[cfg(test)]
mod atomic_fault_tests {
    use super::*;
    #[test]
    fn atomic_child() {
        let Some(dir) = std::env::var_os("CHRONOS_ATOMIC_CHILD") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let at = std::env::var("CHRONOS_ATOMIC_PHASE").unwrap();
        atomic_write_steps(&dir.join("state"), br#"{"version":2}"#, |phase| {
            if phase == at {
                std::fs::write(dir.join("ready"), phase)?;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn killed_writer_preserves_whole_old_or_new_state_at_each_boundary() {
        for phase in ["created", "written", "synced", "replaced"] {
            let dir = tempfile::tempdir().unwrap();
            atomic_write(&dir.path().join("state"), br#"{"version":1}"#).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "base::atomic_fault_tests::atomic_child"])
                .env("CHRONOS_ATOMIC_CHILD", dir.path())
                .env("CHRONOS_ATOMIC_PHASE", phase)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !dir.path().join("ready").exists() {
                if std::time::Instant::now() > until {
                    let _ = child.kill();
                    panic!("child did not reach {phase}");
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            child.kill().unwrap();
            assert!(!child.wait().unwrap().success());
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(dir.path().join("state")).unwrap()).unwrap();
            assert_eq!(value["version"], if phase == "replaced" { 2 } else { 1 });
            // Orphan temp files are ignored, not promoted during restart.
            atomic_write(&dir.path().join("state"), br#"{"version":3}"#).unwrap();
        }
    }
    #[test]
    fn write_sync_and_replace_failure_do_not_damage_committed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        atomic_write(&path, b"old").unwrap();
        for fail_at in ["created", "written", "synced"] {
            assert!(atomic_write_steps(&path, b"new", |phase| {
                if phase == fail_at {
                    Err(std::io::Error::other("injected write/sync failure"))
                } else {
                    Ok(())
                }
            })
            .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        let target_dir = dir.path().join("directory");
        std::fs::create_dir(&target_dir).unwrap();
        assert!(atomic_write(&target_dir, b"new").is_err()); // Actual OS rename failure.
        assert!(target_dir.is_dir());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
