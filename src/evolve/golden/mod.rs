//! Golden evaluation data.
use crate::base::{atomic_write, Failure, Res};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateKind {
    HarnessEdit,
    Prompt,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub kind: CandidateKind,
    pub target: String,
    pub content: String,
    pub rationale: String,
}
pub trait Proposer {
    fn propose(&self, target: &str, current: &str) -> Res<Vec<Candidate>>;
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Fixture {
    pub id: String,
    pub gates: Vec<String>,
    pub data: serde_json::Value,
}
pub struct FixtureSet {
    pub dev: PathBuf,
    pub heldout: PathBuf,
    pub seeds: Vec<u64>,
}
pub trait Replayer {
    fn replay(
        &self,
        repo: &Path,
        overrides: &BTreeMap<String, String>,
        f: &Fixture,
        seed: u64,
    ) -> Res<BTreeMap<String, bool>>;
}
pub struct CommandReplayer;
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Score {
    pub per_gate: BTreeMap<String, (u64, u64)>,
    pub missing: u64,
    pub target: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub baseline_dev: Score,
    pub candidate_dev: Score,
    pub baseline_heldout: Score,
    pub candidate_heldout: Score,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Adopt,
    Reject(String),
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Version {
    pub target: String,
    pub n: u64,
    pub sha256: String,
    pub previous_sha256: String,
    pub candidate: String,
    pub kind: CandidateKind,
    pub score: Score,
    pub ts_ms: i64,
}

fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("golden", e.to_string())
}
fn valid_target(target: &str) -> Res<()> {
    // Reject Windows path syntax on every platform, including drive-relative paths.
    if target.is_empty()
        || target.starts_with(['/', '\\'])
        || target.contains(':')
        || target.split(['/', '\\']).any(|c| c == ".." || c.is_empty())
        || Path::new(target)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(err(
            "target must be a repo-relative path without parent components",
        ));
    }
    Ok(())
}
// Canonicalize the existing prefix even when the leaf does not exist.
fn resolved(path: &Path) -> Res<PathBuf> {
    let mut p = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        match fs::canonicalize(&p) {
            Ok(mut base) => {
                for part in tail.iter().rev() {
                    base.push(part);
                }
                return Ok(base);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = p
                    .file_name()
                    .ok_or_else(|| err(format!("cannot resolve {}", path.display())))?
                    .to_os_string();
                tail.push(name);
                p = p
                    .parent()
                    .ok_or_else(|| err("cannot resolve path"))?
                    .to_path_buf();
            }
            Err(e) => return Err(err(e)),
        }
    }
}
fn installed_cache(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    parts
        .windows(2)
        .any(|w| w[0] == "plugins" && w[1] == "cache")
}
fn target_path(repo: &Path, target: &str, home: Option<&Path>) -> Res<PathBuf> {
    valid_target(target)?;
    let root = resolved(repo)?;
    let path = resolved(&repo.join(target))?;
    if !path.starts_with(&root) || installed_cache(&root) || installed_cache(&path) {
        return Err(err("target is outside repo or in an installed cache"));
    }
    if let Some(home) = home {
        let engine = resolved(&home.join("engine"))?;
        if root.starts_with(&engine) || path.starts_with(&engine) {
            return Err(err("plugin directory is protected"));
        }
    }
    Ok(path)
}
fn ensure_sets(fx: &FixtureSet) -> Res<()> {
    if resolved(&fx.dev)? == resolved(&fx.heldout)? {
        return Err(err("dev and heldout directories must differ"));
    }
    Ok(())
}
fn fixtures(dir: &Path) -> Res<Vec<Fixture>> {
    let mut paths = fs::read_dir(dir)
        .map_err(err)?
        .map(|e| e.map(|v| v.path()).map_err(err))
        .collect::<Res<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "json"));
    paths.sort();
    paths
        .into_iter()
        .map(|p| serde_json::from_slice(&fs::read(p).map_err(err)?).map_err(err))
        .collect()
}
fn score(
    repo: &Path,
    path: &Path,
    seeds: &[u64],
    target_gate: &str,
    overrides: &BTreeMap<String, String>,
    r: &dyn Replayer,
) -> Res<Score> {
    let mut out = Score {
        per_gate: BTreeMap::new(),
        missing: 0,
        target: 0.0,
    };
    let mut target_seen = false;
    for f in fixtures(path)? {
        target_seen |= f.gates.iter().any(|g| g == target_gate);
        for &seed in seeds {
            match r.replay(repo, overrides, &f, seed) {
                Err(_) => out.missing += 1,
                Ok(gates) => {
                    for gate in &f.gates {
                        if let Some(passed) = gates.get(gate) {
                            let counts = out.per_gate.entry(gate.clone()).or_insert((0, 0));
                            counts.1 += 1;
                            counts.0 += u64::from(*passed);
                        } else {
                            out.missing += 1;
                        }
                    }
                }
            }
        }
    }
    if !target_seen {
        out.missing += 1;
    }
    if let Some(&(p, t)) = out.per_gate.get(target_gate) {
        if t > 0 {
            out.target = p as f64 / t as f64;
        }
    }
    Ok(out)
}
pub fn evaluate(
    repo: &Path,
    c: &Candidate,
    fx: &FixtureSet,
    target_gate: &str,
    r: &dyn Replayer,
) -> Res<Comparison> {
    target_path(repo, &c.target, None)?;
    ensure_sets(fx)?;
    let baseline = BTreeMap::new();
    let candidate = BTreeMap::from([(c.target.clone(), c.content.clone())]);
    Ok(Comparison {
        baseline_dev: score(repo, &fx.dev, &fx.seeds, target_gate, &baseline, r)?,
        candidate_dev: score(repo, &fx.dev, &fx.seeds, target_gate, &candidate, r)?,
        baseline_heldout: score(repo, &fx.heldout, &fx.seeds, target_gate, &baseline, r)?,
        candidate_heldout: score(repo, &fx.heldout, &fx.seeds, target_gate, &candidate, r)?,
    })
}
fn drops(base: &Score, candidate: &Score) -> bool {
    base.per_gate
        .iter()
        .any(|(g, &(passed, _))| candidate.per_gate.get(g).is_none_or(|&(p, _)| p < passed))
}
pub fn decide(cmp: &Comparison) -> Verdict {
    if [
        &cmp.baseline_dev,
        &cmp.candidate_dev,
        &cmp.baseline_heldout,
        &cmp.candidate_heldout,
    ]
    .iter()
    .any(|s| s.missing > 0)
    {
        return Verdict::Reject("missing evidence".into());
    }
    if !(cmp.candidate_heldout.target > cmp.baseline_heldout.target) {
        return Verdict::Reject("heldout target did not improve".into());
    }
    if drops(&cmp.baseline_dev, &cmp.candidate_dev)
        || drops(&cmp.baseline_heldout, &cmp.candidate_heldout)
    {
        return Verdict::Reject("gate regression".into());
    }
    Verdict::Adopt
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn slug(target: &str) -> String {
    target
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}
fn version_dir(home: &Path, target: &str) -> Res<PathBuf> {
    valid_target(target)?;
    let dir = resolved(&home.join("evolve").join("versions").join(slug(target)))?;
    if installed_cache(&dir) || dir.starts_with(resolved(&home.join("engine"))?) {
        return Err(err("version directory is protected"));
    }
    Ok(dir)
}
fn write(path: &Path, bytes: &[u8]) -> Res<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| err("missing parent"))?).map_err(err)?;
    atomic_write(path, bytes).map_err(err)
}
pub fn versions(home: &Path, target: &str) -> Res<Vec<Version>> {
    let dir = version_dir(home, target)?;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(err(e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(err)?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name
            .strip_prefix('v')
            .and_then(|s| s.strip_suffix(".json"))
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            let v: Version = serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)?;
            if v.target != target {
                return Err(err("version target slug collision"));
            }
            out.push(v);
        }
    }
    out.sort_by_key(|v| v.n);
    Ok(out)
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}
fn read_current(path: &Path) -> Res<Vec<u8>> {
    match fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(err(e)),
    }
}
fn record_contents(dir: &Path, v: &Version, before: &[u8], after: &[u8]) -> Res<()> {
    write(&dir.join(format!("v{}.before", v.n)), before)?;
    write(&dir.join(format!("v{}.after", v.n)), after)
}
fn record_version(dir: &Path, v: &Version) -> Res<()> {
    write(
        &dir.join(format!("v{}.json", v.n)),
        &serde_json::to_vec(v).map_err(err)?,
    )
}
pub fn adopt(home: &Path, repo: &Path, c: &Candidate, cmp: &Comparison) -> Res<Version> {
    if let Verdict::Reject(reason) = decide(cmp) {
        return Err(err(reason));
    }
    let path = target_path(repo, &c.target, Some(home))?;
    let dir = version_dir(home, &c.target)?;
    let n = match versions(home, &c.target)?.last() {
        Some(v) => v.n.checked_add(1).ok_or_else(|| err("version overflow"))?,
        None => 1,
    };
    let before = read_current(&path)?;
    let v = Version {
        target: c.target.clone(),
        n,
        sha256: hash(c.content.as_bytes()),
        previous_sha256: hash(&before),
        candidate: c.id.clone(),
        kind: c.kind,
        score: cmp.candidate_heldout.clone(),
        ts_ms: now_ms(),
    };
    record_contents(&dir, &v, &before, c.content.as_bytes())?;
    write(&path, c.content.as_bytes())?;
    record_version(&dir, &v)?;
    Ok(v)
}
pub fn check_regression(
    home: &Path,
    repo: &Path,
    target: &str,
    fx: &FixtureSet,
    target_gate: &str,
    r: &dyn Replayer,
) -> Res<Option<Version>> {
    let path = target_path(repo, target, Some(home))?;
    ensure_sets(fx)?;
    let Some(last) = versions(home, target)?.pop() else {
        return Ok(None);
    };
    if last.candidate.starts_with("revert:") {
        return Ok(None);
    }
    let current = read_current(&path)?;
    let content = String::from_utf8(current.clone()).map_err(err)?;
    let result = score(
        repo,
        &fx.heldout,
        &fx.seeds,
        target_gate,
        &BTreeMap::from([(target.to_owned(), content)]),
        r,
    )?;
    if result.missing == 0 && !drops(&last.score, &result) {
        return Ok(None);
    }
    let dir = version_dir(home, target)?;
    let before = fs::read(dir.join(format!("v{}.before", last.n))).map_err(err)?;
    if hash(&before) != last.previous_sha256 {
        return Err(err("version before content hash mismatch"));
    }
    let v = Version {
        target: target.into(),
        n: last
            .n
            .checked_add(1)
            .ok_or_else(|| err("version overflow"))?,
        sha256: hash(&before),
        previous_sha256: hash(&current),
        candidate: format!("revert:{}", last.candidate),
        kind: last.kind,
        score: result,
        ts_ms: now_ms(),
    };
    record_contents(&dir, &v, &current, &before)?;
    if last.previous_sha256 == hash(b"") {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
    } else {
        write(&path, &before)?;
    }
    record_version(&dir, &v)?;
    Ok(Some(v))
}
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl CommandReplayer {
    fn temp() -> Res<TempDir> {
        loop {
            let path = std::env::temp_dir().join(format!(
                "axiom-golden-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(TempDir(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err(e)),
            }
        }
    }
}
impl Replayer for CommandReplayer {
    fn replay(
        &self,
        repo: &Path,
        overrides: &BTreeMap<String, String>,
        f: &Fixture,
        seed: u64,
    ) -> Res<BTreeMap<String, bool>> {
        let command = f
            .data
            .get("command")
            .and_then(|v| v.as_array())
            .ok_or_else(|| err("command must be an argv array"))?;
        let args = command
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| err("command arguments must be strings"))
            })
            .collect::<Res<Vec<_>>>()?;
        let (exe, args) = args.split_first().ok_or_else(|| err("empty command"))?;
        let temp = Self::temp()?;
        for (target, content) in overrides {
            valid_target(target)?;
            write(&temp.0.join(target), content.as_bytes())?;
        }
        let output = crate::base::proc::cmd(exe)
            .args(args)
            .current_dir(&temp.0)
            .env("AXIOM_GOLDEN_SEED", seed.to_string())
            .env("AXIOM_GOLDEN_REPO", repo)
            .output()
            .map_err(err)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = BTreeMap::new();
        for gate in &f.gates {
            let expected = f.data.get("expect").and_then(|v| v.get(gate));
            let passed = if let Some(value) = expected {
                let lines = value
                    .as_array()
                    .ok_or_else(|| err("expected lines must be an array"))?;
                let lines = lines
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| err("expected lines must be strings"))
                    })
                    .collect::<Res<Vec<_>>>()?;
                lines
                    .iter()
                    .all(|line| stdout.lines().any(|actual| actual == *line))
            } else {
                true
            };
            result.insert(gate.clone(), output.status.success() && passed);
        }
        Ok(result)
    }
}
