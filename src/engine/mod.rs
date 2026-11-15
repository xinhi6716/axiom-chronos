use crate::base;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub mod adjudicator;
pub mod approvals;
pub mod budgets;
pub mod calls;
pub mod dual_core;
pub mod finding_verification;
pub mod interjections;
pub mod kinds;
pub mod policy;
pub mod self_check;

pub const FILES: &[(&str, &str)] = &[
    (
        "agents/pm-qa.md",
        include_str!("../../engine/agents/pm-qa.md"),
    ),
    (
        "roles/adjudicator.md",
        include_str!("../../engine/roles/adjudicator.md"),
    ),
    (
        "roles/verifier.md",
        include_str!("../../engine/roles/verifier.md"),
    ),
    (
        "roles/astra-reviewer.md",
        include_str!("../../engine/roles/astra-reviewer.md"),
    ),
    (
        "roles/self-check.md",
        include_str!("../../engine/roles/self-check.md"),
    ),
    (
        "roles/co-core.md",
        include_str!("../../engine/roles/co-core.md"),
    ),
    (
        "roles/arbiter.md",
        include_str!("../../engine/roles/arbiter.md"),
    ),
    (
        ".claude-plugin/plugin.json",
        include_str!("../../engine/.claude-plugin/plugin.json"),
    ),
    ("DISPATCH.md", include_str!("../../engine/DISPATCH.md")),
    (
        "agents/architect.md",
        include_str!("../../engine/agents/architect.md"),
    ),
    (
        "agents/astra.md",
        include_str!("../../engine/agents/astra.md"),
    ),
    (
        "agents/critic.md",
        include_str!("../../engine/agents/critic.md"),
    ),
    (
        "agents/dispatcher.md",
        include_str!("../../engine/agents/dispatcher.md"),
    ),
    (
        "agents/memory-keeper.md",
        include_str!("../../engine/agents/memory-keeper.md"),
    ),
    (
        "agents/narrator.md",
        include_str!("../../engine/agents/narrator.md"),
    ),
    (
        "agents/reviewer.md",
        include_str!("../../engine/agents/reviewer.md"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../engine/hooks/hooks.json"),
    ),
    (
        "research/audit.schema.json",
        include_str!("../../engine/research/audit.schema.json"),
    ),
    (
        "research/COUNCIL.md",
        include_str!("../../engine/research/COUNCIL.md"),
    ),
    (
        "research/council.schema.json",
        include_str!("../../engine/research/council.schema.json"),
    ),
    (
        "research/dossier.schema.json",
        include_str!("../../engine/research/dossier.schema.json"),
    ),
    (
        "research/plan.schema.json",
        include_str!("../../engine/research/plan.schema.json"),
    ),
    (
        "research/PROGRAM.md",
        include_str!("../../engine/research/PROGRAM.md"),
    ),
    (
        "research/review.schema.json",
        include_str!("../../engine/research/review.schema.json"),
    ),
    (
        "skills/research/SKILL.md",
        include_str!("../../engine/skills/research/SKILL.md"),
    ),
    (
        "skills/wish/BATCH.md",
        include_str!("../../engine/skills/wish/BATCH.md"),
    ),
    (
        "skills/wish/SKILL.md",
        include_str!("../../engine/skills/wish/SKILL.md"),
    ),
    // BEGIN paper assets
    ("paper/pass.schema.json", include_str!("../../engine/paper/pass.schema.json")),
    ("paper/reviewer.schema.json", include_str!("../../engine/paper/reviewer.schema.json")),
    ("paper/templates/registry.json", include_str!("../../engine/paper/templates/registry.json")),
    ("paper/presets/journals.json", include_str!("../../engine/paper/presets/journals.json")),
    ("paper/policies/ai-use.json", include_str!("../../engine/paper/policies/ai-use.json")),
    ("paper/templates/main.tex", include_str!("../../engine/paper/templates/main.tex")),
    ("paper/prompts/draft.md", include_str!("../../engine/paper/prompts/draft.md")),
    ("paper/prompts/critique.md", include_str!("../../engine/paper/prompts/critique.md")),
    ("paper/prompts/revise.md", include_str!("../../engine/paper/prompts/revise.md")),
    ("paper/prompts/style.md", include_str!("../../engine/paper/prompts/style.md")),
    ("paper/prompts/review.md", include_str!("../../engine/paper/prompts/review.md")),
    ("paper/academic-humanizer/SKILL.md", include_str!("../../engine/paper/academic-humanizer/SKILL.md")),
    ("paper/academic-humanizer/LICENSE", include_str!("../../engine/paper/academic-humanizer/LICENSE")),
    ("paper/academic-humanizer/SOURCE.json", include_str!("../../engine/paper/academic-humanizer/SOURCE.json")),
    // END paper assets
];

pub fn asset(path: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, contents)| *contents)
}

#[cfg(test)]
mod asset_tests {
    use super::*;
    #[test]
    fn lookup_exposes_new_assets_and_rejects_unknown_paths() {
        for path in [
            "agents/pm-qa.md",
            "roles/adjudicator.md",
            "roles/verifier.md",
            "roles/astra-reviewer.md",
            "roles/co-core.md",
            "roles/arbiter.md",
        ] {
            assert!(asset(path).is_some_and(|body| !body.is_empty()), "{path}");
        }
        assert_eq!(asset("roles/not-present.md"), None);
    }
}

fn short_hash(digest: &[u8]) -> String {
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Lengths are hashed too, so bytes cannot move between a path and its contents unnoticed.
#[doc(hidden)]
pub fn content_id(files: &[(&str, &str)]) -> String {
    let mut hash = Sha256::new();
    for (path, contents) in files {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    short_hash(&hash.finalize())
}

pub fn build_id() -> String {
    content_id(FILES)
}

pub fn binary_id(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| short_hash(&Sha256::digest(bytes)))
}

/// Each build gets its own directory; installation never removes a copy, so it cannot change or delete
/// the rules another running build is reading.
#[doc(hidden)]
pub fn install(home: &Path, version: &str, files: &[(&str, &str)]) -> std::io::Result<PathBuf> {
    let dir = home
        .join("engine")
        .join(format!("{version}-{}", content_id(files)));
    for &(path, contents) in files {
        let target = dir.join(path);
        let in_place = || std::fs::read(&target).ok().as_deref() == Some(contents.as_bytes());
        if !in_place() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // A concurrent launch of this build can hold the file open once it has written it.
            if let Err(e) = base::atomic_write(&target, contents.as_bytes()) {
                if !in_place() {
                    return Err(e);
                }
            }
        }
    }
    Ok(dir)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn system_owner() -> crate::storage::Owner {
    crate::storage::Owner {
        project: crate::storage::SYSTEM_PROJECT.into(),
        wish: None,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn log_storage_error(home: &Path, error: &impl std::fmt::Display) {
    use std::io::Write;
    let dir = crate::storage::dir(home);
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("errors.log"))
        {
            let _ = writeln!(file, "{error}");
        }
    }
}

fn register_and_lease(home: &Path, dir: &Path, now: i64) -> base::Res<()> {
    use crate::storage::{Class, Lease, Ledger};
    use std::sync::{Mutex, OnceLock};
    static LEASES: OnceLock<Mutex<Vec<(PathBuf, Lease)>>> = OnceLock::new();
    let ledger = Ledger::open(home)?;
    ledger.register(
        dir,
        Class::EngineCopy,
        &system_owner(),
        serde_json::json!({}),
    )?;
    ledger.touch(dir, now)?;
    let path = crate::storage::ledger::normalize(dir);
    let mut leases = LEASES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|e| base::Failure::new("storage", e.to_string()))?;
    if !leases.iter().any(|(p, _)| same_path(p, &path)) {
        leases.push((path, ledger.lease(dir)?));
    }
    Ok(())
}

/// Retain this binary's copy, the most recently used other copy, and leased copies.
pub fn prune_engine_copies(
    home: &Path,
    now_ms: i64,
) -> base::Res<Vec<crate::storage::retention::AuditRecord>> {
    use crate::storage::{self, Class, EntryState, Ledger};
    let ledger = Ledger::open(home)?;
    let engine_dir = home.join("engine");
    let root = storage::ledger::normalize(&engine_dir);
    let current = storage::ledger::normalize(&engine_dir.join(format!(
        "{}-{}",
        env!("CARGO_PKG_VERSION"),
        build_id()
    )));
    let children = match std::fs::read_dir(&engine_dir) {
        Ok(children) => children,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(base::Failure::new("storage", e.to_string())),
    };
    for child in children {
        let child = child.map_err(|e| base::Failure::new("storage", e.to_string()))?;
        if !child
            .file_type()
            .map_err(|e| base::Failure::new("storage", e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let path = child.path();
        let normalized = storage::ledger::normalize(&path);
        if !normalized
            .parent()
            .is_some_and(|parent| same_path(parent, &root))
        {
            continue;
        }
        if ledger
            .get(&path)?
            .is_none_or(|entry| entry.state == EntryState::Deleted)
        {
            ledger.register(
                &path,
                Class::EngineCopy,
                &system_owner(),
                serde_json::json!({}),
            )?;
            // Adoption must preserve the filesystem's age rather than making it look newly used.
            ledger.touch(&path, 0)?;
        }
    }
    let entries: Vec<_> = ledger
        .entries()?
        .into_iter()
        .filter(|entry| {
            entry.class == Class::EngineCopy
                && entry
                    .path
                    .parent()
                    .is_some_and(|parent| same_path(parent, &root))
                && std::fs::symlink_metadata(&entry.path).is_ok_and(|m| m.file_type().is_dir())
        })
        .collect();
    let previous = entries
        .iter()
        .filter(|e| !same_path(&e.path, &current))
        .max_by_key(|e| {
            if e.last_used_ms != 0 {
                e.last_used_ms
            } else {
                std::fs::metadata(&e.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
                    .unwrap_or(0)
            }
        })
        .map(|e| e.id);
    let mut records = Vec::new();
    for entry in entries {
        if same_path(&entry.path, &current)
            || Some(entry.id) == previous
            || ledger.in_use(&entry.path)?
        {
            continue;
        }
        records.push(storage::retention::remove_entry(
            home,
            &entry.path,
            "engine_retention",
            now_ms,
        ));
    }
    Ok(records)
}

pub fn plugin_dir(home: &Path) -> std::io::Result<PathBuf> {
    let dir = install(home, env!("CARGO_PKG_VERSION"), FILES)?;
    let now = now_ms();
    match register_and_lease(home, &dir, now) {
        Ok(()) => match prune_engine_copies(home, now) {
            Ok(records) => {
                for record in records {
                    if record.outcome.starts_with("failed:") {
                        log_storage_error(home, &format!("{}: {}", record.path, record.outcome));
                    }
                }
            }
            Err(error) => log_storage_error(home, &error),
        },
        Err(error) => log_storage_error(home, &error),
    }
    Ok(dir)
}

pub fn plugin_version(root: &Path) -> Result<String, String> {
    let manifest = root.join(".claude-plugin/plugin.json");
    let bytes =
        std::fs::read(&manifest).map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} is not valid JSON: {e}", manifest.display()))?;
    value["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} declares no version", manifest.display()))
}

fn normalized(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Embedded files the plugin at `root` lacks or holds with other contents. Line endings do not
/// count: a Windows checkout of the same commit differs from the build only in those.
pub fn differing_files(root: &Path) -> Vec<&'static str> {
    FILES
        .iter()
        .filter(|(path, contents)| {
            std::fs::read(root.join(path)).map_or(true, |bytes| {
                normalized(&bytes) != normalized(contents.as_bytes())
            })
        })
        .map(|(path, _)| *path)
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct PluginCheck {
    pub version: String,
    pub differing: Vec<&'static str>,
}

/// Err is the explanation to stop on: the plugin at `root` cannot be shown to be this binary's.
pub fn check_plugin(root: &Path) -> Result<PluginCheck, String> {
    let binary = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().map_or_else(|_| "axiom".into(), |p| p.display().to_string());
    let version = plugin_version(root).map_err(|why| {
        format!(
            "the version of the plugin at {} is unavailable ({why}) and the axiom binary at {exe} is version {binary}. Reinstall the plugin, for example `claude plugin update chronos@axiom-chronos`, then restart Claude Code and run `axiom doctor`.",
            root.display()
        )
    })?;
    if version != binary {
        return Err(format!(
            "the plugin at {} is version {version} but the axiom binary at {exe} is version {binary}, and the plugin's rules and the binary's gates must come from the same release. Update the plugin with `claude plugin update chronos@axiom-chronos`, or install axiom {version} and put it on PATH or in AXIOM_BIN, then restart Claude Code and run `axiom doctor`.",
            root.display()
        ));
    }
    Ok(PluginCheck {
        version,
        differing: differing_files(root),
    })
}
