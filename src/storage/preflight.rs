//! Disk-space checks and cross-process reservations.
use crate::{
    base::{self, Failure, Res},
    orch::events,
    storage::{archive, Ledger, Policy},
};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const REASON: &str = "disk_space";
pub const EVENT_PREFLIGHT_PAUSED: &str = "disk_preflight_paused";
pub const EVENT_DISK_PAUSE: &str = "disk_pause";
pub const EVENT_DISK_RESUMED: &str = "disk_resumed";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Wish,
    Batch,
    Unpack,
    CacheBuild,
    SetupStep,
}
impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Batch => "batch",
            Self::Unpack => "unpack",
            Self::CacheBuild => "cache_build",
            Self::SetupStep => "setup_step",
        }
    }
    fn default_bytes(self, p: &Policy) -> u64 {
        let e = &p.estimates;
        let value = match self {
            Self::Wish => e.wish,
            Self::Batch => e.batch,
            Self::Unpack => e.unpack,
            Self::CacheBuild => e.cache_build,
            Self::SetupStep => e.setup_step,
        };
        if value == 0 {
            let defaults = Policy::default();
            self.default_bytes(&defaults)
        } else {
            value
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum NeedSource {
    Override,
    ProjectStep,
    Project,
    Default,
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Need {
    pub bytes: u64,
    pub source: NeedSource,
}
fn invalid(e: impl std::fmt::Display) -> Failure {
    Failure::new("invalid_argument", e.to_string())
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn storage(e: impl std::fmt::Display) -> Failure {
    Failure::new("storage", e.to_string())
}

pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let number = &s[..end];
    let suffix = s[end..].to_ascii_lowercase();
    let shift = match suffix.as_str() {
        "" | "b" => 0,
        "k" | "kb" | "kib" => 10,
        "m" | "mb" | "mib" => 20,
        "g" | "gb" | "gib" => 30,
        "t" | "tb" | "tib" => 40,
        "p" | "pb" | "pib" => 50,
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (number.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let unit = 1u128 << shift;
    let base = whole.parse::<u128>().ok()?.checked_mul(unit)?;
    let extra = if fraction.is_empty() {
        0
    } else {
        let numerator = fraction.parse::<u128>().ok()?;
        let denominator = 10u128.checked_pow(fraction.len().try_into().ok()?)?;
        numerator.checked_mul(unit)? / denominator
    };
    u64::try_from(base.checked_add(extra)?).ok()
}
fn size(value: &Value) -> Res<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(parse_size))
        .ok_or_else(|| invalid("invalid estimate"))
}
fn estimates(repo: &Path, kind: Kind, step: Option<&str>) -> Res<(Option<u64>, Option<u64>)> {
    let data = match fs::read(repo.join(".axiom/setup.json")) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(e) => return Err(io(e)),
    };
    let root: Value = serde_json::from_slice(&data).map_err(invalid)?;
    let object = root
        .as_object()
        .ok_or_else(|| invalid("setup manifest must be an object"))?;
    let mut project = None;
    if let Some(value) = object.get("estimates") {
        let map = value
            .as_object()
            .ok_or_else(|| invalid("estimates must be an object"))?;
        for key in ["wish", "batch", "unpack", "cache_build"] {
            if let Some(v) = map.get(key) {
                let n = size(v)?;
                if key == kind.key() {
                    project = Some(n);
                }
            }
        }
    }
    let mut step_value = None;
    if let Some(value) = object.get("steps") {
        let steps = value
            .as_array()
            .ok_or_else(|| invalid("steps must be an array"))?;
        for item in steps {
            let obj = item
                .as_object()
                .ok_or_else(|| invalid("step must be an object"))?;
            let id = obj
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("step requires an id"))?;
            if let Some(v) = obj.get("estimate") {
                let n = size(v)?;
                if Some(id) == step {
                    step_value = Some(n);
                }
            }
        }
    }
    Ok((step_value, project))
}
pub fn project_estimate(repo: &Path, kind: Kind, step: Option<&str>) -> Res<Option<u64>> {
    let (s, p) = estimates(repo, kind, step)?;
    Ok(s.or(p))
}
pub fn need(
    policy: &Policy,
    kind: Kind,
    override_bytes: Option<u64>,
    repo: Option<&Path>,
    step: Option<&str>,
) -> Res<Need> {
    if let Some(bytes) = override_bytes {
        if bytes == 0 {
            return Err(invalid("zero override"));
        }
        return Ok(Need {
            bytes,
            source: NeedSource::Override,
        });
    }
    let (s, p) = match repo {
        Some(repo) => estimates(repo, kind, step)?,
        None => (None, None),
    };
    let (bytes, source) = if let Some(n) = s {
        (n, NeedSource::ProjectStep)
    } else if let Some(n) = p {
        (n, NeedSource::Project)
    } else {
        (kind.default_bytes(policy), NeedSource::Default)
    };
    Ok(Need {
        bytes: if bytes == 0 {
            kind.default_bytes(policy)
        } else {
            bytes
        },
        source,
    })
}
pub trait Probe: Send + Sync {
    fn volume(&self, path: &Path) -> Res<String>;
    fn free_bytes(&self, path: &Path) -> Res<u64>;
}
struct OsProbe;
pub fn os_probe() -> Box<dyn Probe> {
    Box::new(OsProbe)
}
fn fake() -> Option<std::ffi::OsString> {
    std::env::var_os("AXIOM_FAKE_DISK")
}
fn existing(path: &Path) -> Res<PathBuf> {
    let absolute = std::path::absolute(path).map_err(io)?;
    let mut p = absolute.as_path();
    loop {
        match fs::metadata(p) {
            Ok(_) => return fs::canonicalize(p).map_err(io),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                p = p.parent().ok_or_else(|| io(e))?
            }
            Err(e) => return Err(io(e)),
        }
    }
}
impl Probe for OsProbe {
    fn volume(&self, path: &Path) -> Res<String> {
        if fake().is_some() {
            return Ok("fake".into());
        }
        let p = existing(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(format!("dev:{}", fs::metadata(p).map_err(io)?.dev()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
            let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut buf = vec![0u16; 32768];
            if unsafe { GetVolumePathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0
            {
                return Err(io(std::io::Error::last_os_error()));
            }
            let len = buf.iter().position(|&x| x == 0).unwrap_or(buf.len());
            Ok(String::from_utf16_lossy(&buf[..len]).to_lowercase())
        }
    }
    fn free_bytes(&self, path: &Path) -> Res<u64> {
        if let Some(file) = fake() {
            let v: Value = serde_json::from_slice(&fs::read(file).map_err(io)?).map_err(invalid)?;
            let default = v
                .get("free_bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("invalid fake disk free_bytes"))?;
            let mut selected = None;
            if let Some(paths) = v.get("paths") {
                let paths = paths
                    .as_object()
                    .ok_or_else(|| invalid("invalid fake disk paths"))?;
                for (prefix, value) in paths {
                    let prefix = Path::new(prefix);
                    let bytes = value
                        .as_u64()
                        .ok_or_else(|| invalid("invalid fake disk paths"))?;
                    if !prefix.is_absolute() {
                        return Err(invalid("invalid fake disk paths"));
                    }
                    let length = prefix.components().count();
                    if path.starts_with(prefix) && selected.is_none_or(|(best, _)| length > best) {
                        selected = Some((length, bytes));
                    }
                }
            }
            return Ok(selected.map_or(default, |(_, bytes)| bytes));
        }
        let p = existing(path)?;
        #[cfg(unix)]
        {
            use std::{ffi::CString, os::unix::ffi::OsStrExt};
            let name = CString::new(p.as_os_str().as_bytes()).map_err(invalid)?;
            let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) } != 0 {
                return Err(io(std::io::Error::last_os_error()));
            }
            let stat = unsafe { stat.assume_init() };
            Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
            let directory = if p.is_dir() {
                p.as_path()
            } else {
                p.parent().ok_or_else(|| io("path has no parent"))?
            };
            let wide: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut free = 0;
            if unsafe {
                GetDiskFreeSpaceExW(
                    wide.as_ptr(),
                    &mut free,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io(std::io::Error::last_os_error()));
            }
            Ok(free)
        }
    }
}
#[derive(Serialize, Debug)]
pub struct Check {
    pub kind: Kind,
    pub path: PathBuf,
    pub volume: String,
    pub need: Need,
    pub free_bytes: u64,
    pub reserved_bytes: u64,
    pub reserve_bytes: u64,
    pub ok: bool,
}
fn human(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut n = bytes as f64;
    let mut i = 0;
    while n >= 1024.0 && i < units.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{n:.1} {}", units[i])
    }
}
impl Check {
    pub fn line(&self) -> String {
        format!(
            "disk: {} needs {} on {} (+{} reserve); {} free, {} reserved by other runs; {}",
            self.kind.key(),
            human(self.need.bytes),
            self.volume,
            human(self.reserve_bytes),
            human(self.free_bytes),
            human(self.reserved_bytes),
            if self.ok {
                "ready"
            } else {
                "paused, free space then resume"
            }
        )
    }
}
// WAL initialization can race another first opener before SQLite's busy handler applies.
fn open_ledger(home: &Path) -> Res<Ledger> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Ledger::open(home) {
            Err(e)
                if e.kind == "storage"
                    && e.message.contains("locked")
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}
fn init(conn: &Connection) -> Res<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS preflight_reservations(id INTEGER PRIMARY KEY AUTOINCREMENT, volume TEXT NOT NULL, bytes TEXT NOT NULL, pid INTEGER NOT NULL, label TEXT NOT NULL, created_ms INTEGER NOT NULL)").map_err(storage)
}
fn reserved(conn: &Connection, volume: &str) -> Res<u64> {
    let mut stmt = conn
        .prepare("SELECT id, volume, bytes, pid FROM preflight_reservations")
        .map_err(storage)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(storage)?;
    let mut total = 0u64;
    let mut dead = Vec::new();
    for row in rows {
        let (id, v, bytes, pid) = row.map_err(storage)?;
        if u32::try_from(pid)
            .ok()
            .is_none_or(|p| !base::proc::pid_alive(p))
        {
            dead.push(id);
        } else if v == volume {
            total = total.saturating_add(bytes.parse::<u64>().map_err(storage)?);
        }
    }
    drop(stmt);
    for id in dead {
        conn.execute("DELETE FROM preflight_reservations WHERE id=?1", [id])
            .map_err(storage)?;
    }
    Ok(total)
}
fn perform(
    conn: &Connection,
    probe: &dyn Probe,
    policy: &Policy,
    path: &Path,
    kind: Kind,
    mut need: Need,
) -> Res<Check> {
    if need.bytes == 0 {
        need.bytes = kind.default_bytes(policy);
    }
    let volume = probe.volume(path)?;
    let free_bytes = probe.free_bytes(path)?;
    let reserved_bytes = reserved(conn, &volume)?;
    let ok = free_bytes.saturating_sub(reserved_bytes)
        >= need.bytes.saturating_add(policy.reserve_bytes);
    Ok(Check {
        kind,
        path: path.to_path_buf(),
        volume,
        need,
        free_bytes,
        reserved_bytes,
        reserve_bytes: policy.reserve_bytes,
        ok,
    })
}
pub fn check(
    home: &Path,
    probe: &dyn Probe,
    policy: &Policy,
    path: &Path,
    kind: Kind,
    need: Need,
) -> Res<Check> {
    let ledger = open_ledger(home)?;
    let tx = Transaction::new_unchecked(ledger.conn(), TransactionBehavior::Immediate)
        .map_err(storage)?;
    init(&tx)?;
    let result = perform(&tx, probe, policy, path, kind, need)?;
    tx.commit().map_err(storage)?;
    Ok(result)
}
pub struct Reservation {
    pub id: i64,
    home: PathBuf,
    active: bool,
}
impl std::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation").field("id", &self.id).finish()
    }
}
impl Reservation {
    fn remove(&mut self) -> Res<()> {
        if self.active {
            let ledger = open_ledger(&self.home)?;
            ledger
                .conn()
                .execute("DELETE FROM preflight_reservations WHERE id=?1", [self.id])
                .map_err(storage)?;
            self.active = false;
        }
        Ok(())
    }
    pub fn release(mut self) -> Res<()> {
        self.remove()
    }
    pub fn keep(mut self) {
        self.active = false;
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
pub fn reserve(
    home: &Path,
    probe: &dyn Probe,
    policy: &Policy,
    path: &Path,
    kind: Kind,
    need: Need,
    holder_pid: u32,
    label: &str,
) -> Res<Result<Reservation, Check>> {
    let ledger = open_ledger(home)?;
    let tx = Transaction::new_unchecked(ledger.conn(), TransactionBehavior::Immediate)
        .map_err(storage)?;
    init(&tx)?;
    let check = perform(&tx, probe, policy, path, kind, need)?;
    if !check.ok {
        tx.commit().map_err(storage)?;
        return Ok(Err(check));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    tx.execute("INSERT INTO preflight_reservations(volume,bytes,pid,label,created_ms) VALUES(?1,?2,?3,?4,?5)",params![check.volume,check.need.bytes.to_string(),i64::from(holder_pid),label,now]).map_err(storage)?;
    let id = tx.last_insert_rowid();
    tx.commit().map_err(storage)?;
    Ok(Ok(Reservation {
        id,
        home: home.to_path_buf(),
        active: true,
    }))
}
pub fn low_disk(
    home: &Path,
    probe: &dyn Probe,
    policy: &Policy,
    path: &Path,
) -> Res<Option<Check>> {
    let mut c = check(
        home,
        probe,
        policy,
        path,
        Kind::Wish,
        Need {
            bytes: policy.low_disk_bytes,
            source: NeedSource::Default,
        },
    )?;
    if c.free_bytes < policy.low_disk_bytes {
        c.need.bytes = policy.low_disk_bytes;
        c.ok = false;
        Ok(Some(c))
    } else {
        Ok(None)
    }
}
pub fn notify_paused(store: &Path, event: &str, check: &Check, pending: Value) -> Res<()> {
    events::append_event(
        store,
        event,
        None,
        None,
        json!({"check":check,"pending":pending}),
    )?;
    let dir = store.join("reports");
    fs::create_dir_all(&dir).map_err(io)?;
    let path = dir.join("disk.txt");
    archive::restore(&path).map_err(io)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io)?;
    writeln!(file, "{}", check.line()).map_err(io)
}
