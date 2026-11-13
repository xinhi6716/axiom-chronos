use super::{
    ledger,
    retention::{self, AuditRecord},
    Class, Entry, EntryState, Ledger, Owner, Policy,
};
use crate::{
    base::{self, Failure, Res},
    goal::Goal,
    orch::{events, store},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Active,
    Dirty,
    Merged,
    Abandoned,
    Gone,
}
impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Dirty => "dirty",
            Self::Merged => "merged",
            Self::Abandoned => "abandoned",
            Self::Gone => "gone",
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub repo: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    pub state: State,
    pub owner: Owner,
    pub build_caches: Vec<PathBuf>,
    pub last_used_ms: i64,
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn git<I, S>(cwd: &Path, args: I) -> Res<(bool, String, String)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let child = base::proc::cmd("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(io)?;
    let out = base::proc::capture(child, Duration::from_secs(20))?;
    if out.timed_out {
        return Err(Failure::new(
            "git_timeout",
            "git timed out after 20 seconds",
        ));
    }
    Ok((
        out.exit_code == 0,
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    ))
}
fn required<I, S>(cwd: &Path, args: I) -> Res<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (ok, out, err) = git(cwd, args)?;
    if ok {
        Ok(out)
    } else {
        Err(Failure::new("git", if err.is_empty() { out } else { err }))
    }
}
fn path_string(path: &Path) -> String {
    ledger::normalize(path).to_string_lossy().into_owned()
}
fn same(a: &Path, b: &Path) -> bool {
    let (a, b) = (path_string(a), path_string(b));
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}
fn resolve(cwd: &Path, output: &str) -> PathBuf {
    let p = Path::new(output);
    ledger::normalize(&if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    })
}
pub fn common_dir(path: &Path) -> Res<Option<PathBuf>> {
    if !path.is_dir() {
        return Ok(None);
    }
    let (ok, inside, _) = git(path, ["rev-parse", "--is-inside-work-tree"])?;
    if !ok || inside != "true" {
        return Ok(None);
    }
    Ok(Some(resolve(
        path,
        &required(path, ["rev-parse", "--git-common-dir"])?,
    )))
}
pub fn is_linked_worktree(path: &Path) -> Res<bool> {
    let Some(common) = common_dir(path)? else {
        return Ok(false);
    };
    let dir = resolve(path, &required(path, ["rev-parse", "--git-dir"])?);
    Ok(!same(&dir, &common))
}
fn root(path: &Path) -> Res<PathBuf> {
    Ok(resolve(
        path,
        &required(path, ["rev-parse", "--show-toplevel"])?,
    ))
}
fn branch_head(path: &Path) -> Res<(Option<String>, String)> {
    let (ok, branch, _) = git(path, ["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    Ok((ok.then_some(branch), required(path, ["rev-parse", "HEAD"])?))
}
fn register_inner(
    home: &Path,
    path: &Path,
    owner: &Owner,
    record_head: bool,
) -> Res<Option<Entry>> {
    if !is_linked_worktree(path)? || !same(path, &root(path)?) {
        return Ok(None);
    }
    let repo = common_dir(path)?.unwrap();
    let (branch, head) = branch_head(path)?;
    let ledger = Ledger::open(home)?;
    let existing = ledger.get(path)?;
    let mut meta = json!({"repo": path_string(&repo), "branch": branch, "head": head});
    if existing.is_none() {
        meta["worktree_state"] = json!("active");
        if record_head {
            meta["registered_head"] = json!(head);
        }
    }
    let entry = ledger.register(path, Class::Worktree, owner, meta)?;
    register_caches(&ledger, path, owner, true)?;
    Ok(Some(entry))
}
fn register_caches(ledger: &Ledger, path: &Path, owner: &Owner, touch: bool) -> Res<()> {
    for target in [path.join("target"), path.join("tests/desktop-smoke/target")] {
        if !target.is_dir() {
            continue;
        }
        let worktree = path_string(path);
        if touch
            || ledger.get(&target)?.is_none_or(|e| {
                e.state == EntryState::Deleted
                    || e.class != Class::BuildCache
                    || e.owner != *owner
                    || !e.meta["worktree"]
                        .as_str()
                        .is_some_and(|p| same(Path::new(p), path))
            })
        {
            ledger.register(
                &target,
                Class::BuildCache,
                owner,
                json!({"worktree": worktree}),
            )?;
        }
    }
    Ok(())
}
pub fn register(home: &Path, path: &Path, owner: &Owner) -> Res<Option<Entry>> {
    register_inner(home, path, owner, true)
}
fn main_dir(common: &Path, repo: &Path) -> Res<PathBuf> {
    if common.file_name().is_some_and(|n| n == ".git") {
        return Ok(common.parent().unwrap_or(common).to_path_buf());
    }
    let list = listed(repo)?;
    Ok(list.first().cloned().unwrap_or_else(|| repo.to_path_buf()))
}
pub fn shared_target(home: &Path, repo_path: &Path, owner: &Owner) -> Res<Option<PathBuf>> {
    if !repo_path.join("Cargo.toml").is_file() {
        return Ok(None);
    }
    let Some(common) = common_dir(repo_path)? else {
        return Ok(None);
    };
    if !same(repo_path, &root(repo_path)?) {
        return Ok(None);
    }
    let main = main_dir(&common, repo_path)?;
    let normalized = path_string(&common);
    let hash_input = if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized.clone()
    };
    let hash = format!("{:x}", Sha256::digest(hash_input.as_bytes()));
    let target = ledger::normalize(&home.join("build-cache").join(format!(
        "{}-{}",
        base::slug_for_dir(&main),
        &hash[..12]
    )));
    fs::create_dir_all(&target).map_err(io)?;
    let ledger = Ledger::open(home)?;
    let existing = ledger.get(&target)?;
    let mut users = existing
        .as_ref()
        .and_then(|e| e.meta["users"].as_array())
        .cloned()
        .unwrap_or_default();
    if !users
        .iter()
        .any(|u| u.as_str().is_some_and(|s| same(Path::new(s), repo_path)))
    {
        users.push(json!(path_string(repo_path)));
    }
    let cache_owner = existing.as_ref().map(|e| &e.owner).unwrap_or(owner);
    ledger.register(
        &target,
        Class::BuildCache,
        cache_owner,
        json!({"shared": true, "repo": normalized, "users": users}),
    )?;
    Ok(Some(target))
}
pub fn worker_env(home: &Path, repo_path: &Path, owner: &Owner) -> Res<Vec<(String, String)>> {
    Ok(shared_target(home, repo_path, owner)?
        .map(|p| vec![("CARGO_TARGET_DIR".into(), p.to_string_lossy().into_owned())])
        .unwrap_or_default())
}
fn listed(repo: &Path) -> Res<Vec<PathBuf>> {
    let output = required(repo, ["worktree", "list", "--porcelain"])?;
    Ok(output
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(|p| ledger::normalize(Path::new(p)))
        .collect())
}
fn goal_done(home: &Path, owner: &Owner) -> Res<bool> {
    let Some(wish) = owner.wish.as_ref() else {
        return Ok(false);
    };
    let files = match fs::read_dir(home.join("goals")) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io(e)),
    };
    for f in files {
        let path = f.map_err(io)?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let bytes = fs::read(path).map_err(io)?;
        if let Ok(g) = serde_json::from_slice::<Goal>(&bytes) {
            if g.project.as_deref() == Some(&owner.project)
                && g.wish_id.as_ref() == Some(wish)
                && matches!(g.status.as_str(), "done" | "blocked")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn own_cache(e: &Entry, path: &Path) -> bool {
    e.class == Class::BuildCache
        && e.meta["shared"].as_bool() != Some(true)
        && e.meta["worktree"]
            .as_str()
            .is_some_and(|p| same(Path::new(p), path))
}
fn scan_inner(home: &Path, now_ms: i64, persist: bool) -> Res<Vec<Worktree>> {
    let ledger = Ledger::open(home)?;
    let mut entries = ledger.entries()?;
    if persist {
        for e in entries.iter().filter(|e| e.class == Class::Worktree) {
            register_caches(&ledger, &e.path, &e.owner, false)?;
        }
        entries = ledger.entries()?;
    }
    let policy = Policy::load(home)?;
    let mut result = Vec::new();
    for e in entries.iter().filter(|e| e.class == Class::Worktree) {
        let repo = PathBuf::from(e.meta["repo"].as_str().unwrap_or(""));
        let caches: Vec<_> = entries
            .iter()
            .filter(|c| own_cache(c, &e.path))
            .map(|c| c.path.clone())
            .collect();
        let mut branch = e.meta["branch"].as_str().map(str::to_owned);
        let mut head = e.meta["head"].as_str().unwrap_or("").to_owned();
        let present =
            e.path.is_dir() && repo.is_dir() && listed(&repo)?.iter().any(|p| same(p, &e.path));
        let state = if !present {
            State::Gone
        } else {
            (branch, head) = branch_head(&e.path)?;
            if !required(&e.path, ["status", "--porcelain"])?.is_empty() {
                State::Dirty
            } else {
                let mut tip = None;
                for name in [
                    "refs/remotes/origin/HEAD",
                    "refs/heads/main",
                    "refs/heads/master",
                ] {
                    let (ok, output, _) = git(&e.path, ["rev-parse", "--verify", name])?;
                    if ok {
                        tip = Some(output);
                        break;
                    }
                }
                let merged = if let Some(tip) = tip {
                    let changed = e.meta["registered_head"]
                        .as_str()
                        .map_or(head != tip, |initial| head != initial);
                    changed && git(&e.path, ["merge-base", "--is-ancestor", &head, &tip])?.0
                } else {
                    false
                };
                let age = policy
                    .build_cache_days
                    .saturating_mul(86_400_000)
                    .min(i64::MAX as u64) as i64;
                if merged {
                    State::Merged
                } else if now_ms.saturating_sub(e.last_used_ms) > age && goal_done(home, &e.owner)?
                {
                    State::Abandoned
                } else {
                    State::Active
                }
            }
        };
        if persist {
            let mut meta =
                json!({"worktree_state": state.as_str(), "branch": branch, "head": head});
            let since = if e.meta["worktree_state"].as_str() != Some(state.as_str()) {
                Some(now_ms)
            } else {
                e.meta["state_since_ms"].as_i64()
            };
            if let Some(since) = since {
                meta["state_since_ms"] = json!(since);
            }
            ledger.set_meta(&e.path, meta)?;
            for cache in &caches {
                let mut meta = json!({"worktree_state": state.as_str()});
                if let Some(since) = since {
                    meta["state_since_ms"] = json!(since);
                }
                ledger.set_meta(cache, meta)?;
            }
        }
        result.push(Worktree {
            path: e.path.clone(),
            repo,
            branch,
            head,
            state,
            owner: e.owner.clone(),
            build_caches: caches,
            last_used_ms: e.last_used_ms,
        });
    }
    if persist {
        for cache in entries
            .iter()
            .filter(|e| e.class == Class::BuildCache && e.meta["shared"].as_bool() == Some(true))
        {
            if let Some(users) = cache.meta["users"].as_array() {
                let kept: Vec<_> = users
                    .iter()
                    .filter(|u| {
                        !u.as_str().is_some_and(|p| {
                            result.iter().any(|w| {
                                matches!(w.state, State::Gone | State::Merged)
                                    && same(Path::new(p), &w.path)
                            })
                        })
                    })
                    .cloned()
                    .collect();
                if kept.len() != users.len() {
                    ledger.set_meta(&cache.path, json!({"users": kept}))?;
                }
            }
        }
    }
    Ok(result)
}
pub fn scan(home: &Path, now_ms: i64) -> Res<Vec<Worktree>> {
    scan_inner(home, now_ms, true)
}
pub fn adopt(home: &Path, repo: &Path) -> Res<Vec<Worktree>> {
    let Some(common) = common_dir(repo)? else {
        return Ok(Vec::new());
    };
    let main = main_dir(&common, repo)?;
    let owner = Owner {
        project: base::slug_for_dir(&main),
        wish: None,
    };
    let mut paths = Vec::new();
    for path in listed(repo)? {
        if !same(&path, &main) && is_linked_worktree(&path)? {
            let ledger = Ledger::open(home)?;
            if ledger.get(&path)?.is_none() {
                register_inner(home, &path, &owner, false)?;
            }
            paths.push(path);
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    Ok(scan(home, now)?
        .into_iter()
        .filter(|w| paths.iter().any(|p| same(p, &w.path)))
        .collect())
}
pub const EVENT_CLEANUP_AVAILABLE: &str = "worktree_cleanup_available";
pub fn announce_merged(home: &Path, now_ms: i64) -> Res<Vec<Worktree>> {
    let ledger = Ledger::open(home)?;
    let mut announced = Vec::new();
    for w in scan(home, now_ms)?
        .into_iter()
        .filter(|w| w.state == State::Merged)
    {
        let entry = ledger.get(&w.path)?.unwrap();
        if entry.meta["announced_head"].as_str() == Some(&w.head) {
            continue;
        }
        let data = json!({"path": w.path, "branch": w.branch, "build_caches": w.build_caches});
        if let Some(wish) = &w.owner.wish {
            let store = store::wish_store(home, &w.owner.project, Some(wish))?;
            if store.is_dir() {
                events::append_event(&store, EVENT_CLEANUP_AVAILABLE, None, None, data.clone())?;
            }
        }
        fs::create_dir_all(super::dir(home)).map_err(io)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(super::dir(home).join("events.jsonl"))
            .map_err(io)?;
        writeln!(file, "{}", json!({"ts": base::now_iso(), "kind": EVENT_CLEANUP_AVAILABLE, "path": w.path, "branch": w.branch, "build_caches": w.build_caches})).map_err(io)?;
        ledger.set_meta(&w.path, json!({"announced_head": w.head}))?;
        announced.push(w);
    }
    Ok(announced)
}
#[derive(Debug)]
pub enum Removal {
    Planned {
        worktree: Worktree,
        caches: Vec<PathBuf>,
    },
    Removed {
        records: Vec<AuditRecord>,
    },
}
pub fn remove(home: &Path, path: &Path, confirmed: bool, now_ms: i64) -> Res<Removal> {
    let ledger = Ledger::open(home)?;
    let worktree = scan_inner(home, now_ms, false)?
        .into_iter()
        .find(|w| same(&w.path, path))
        .ok_or_else(|| Failure::new("not_found", "worktree not found"))?;
    if matches!(worktree.state, State::Dirty | State::Active) || ledger.in_use(&worktree.path)? {
        return Err(Failure::new(
            "worktree_busy",
            "worktree is active, dirty or leased",
        ));
    }
    let caches = worktree.build_caches.clone();
    if !confirmed {
        return Ok(Removal::Planned { worktree, caches });
    }
    if worktree.state != State::Gone {
        let args: [&OsStr; 3] = [
            OsStr::new("worktree"),
            OsStr::new("remove"),
            worktree.path.as_os_str(),
        ];
        let (ok, out, err) = git(&worktree.repo, args)?;
        if !ok {
            return Err(Failure::new(
                "worktree_busy",
                if err.is_empty() { out } else { err },
            ));
        }
    }
    let mut records = Vec::new();
    for cache in &caches {
        let record = retention::remove_entry(home, cache, "user_confirmed", now_ms);
        if !cache.exists() && record.outcome.starts_with("failed:") {
            ledger.mark_deleted(cache, now_ms)?;
        }
        records.push(record);
    }
    for cache in ledger
        .entries()?
        .into_iter()
        .filter(|e| e.class == Class::BuildCache && e.meta["shared"].as_bool() == Some(true))
    {
        if let Some(users) = cache.meta["users"].as_array() {
            let kept: Vec<_> = users
                .iter()
                .filter(|u| {
                    !u.as_str()
                        .is_some_and(|p| same(Path::new(p), &worktree.path))
                })
                .cloned()
                .collect();
            if kept.len() != users.len() {
                ledger.set_meta(&cache.path, json!({"users": kept}))?;
            }
        }
    }
    ledger.mark_deleted(&worktree.path, now_ms)?;
    Ok(Removal::Removed { records })
}
