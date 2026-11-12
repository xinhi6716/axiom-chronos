use axiom::{
    base::Res,
    orch::events,
    storage::{
        archive,
        preflight::{self, Kind, Need, NeedSource, Probe},
        Policy,
    },
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier, Mutex,
    },
};
use tempfile::tempdir;

struct Fake {
    free: AtomicU64,
    volume: &'static str,
}
impl Fake {
    fn new(free: u64, volume: &'static str) -> Self {
        Self {
            free: AtomicU64::new(free),
            volume,
        }
    }
}
impl Probe for Fake {
    fn volume(&self, _: &Path) -> Res<String> {
        Ok(self.volume.into())
    }
    fn free_bytes(&self, _: &Path) -> Res<u64> {
        Ok(self.free.load(Ordering::SeqCst))
    }
}
fn policy() -> Policy {
    let mut p = Policy::default();
    p.reserve_bytes = 20;
    p.low_disk_bytes = 50;
    p
}
fn n(bytes: u64) -> Need {
    Need {
        bytes,
        source: NeedSource::Override,
    }
}

#[test]
fn parse_size_units() {
    for (s, value) in [
        ("512M", 512 << 20),
        ("20gB", 20 << 30),
        ("1.5T", 3 << 39),
        ("123", 123),
        ("1k", 1024),
    ] {
        assert_eq!(preflight::parse_size(s), Some(value));
    }
    for s in ["", "-1G", "1XB", "1.2.3G", "18446744073709551616"] {
        assert_eq!(preflight::parse_size(s), None);
    }
}
#[test]
fn project_estimate_reaches_batch_and_cache_preflight() {
    let repo = tempdir().unwrap();
    fs::create_dir(repo.path().join(".axiom")).unwrap();
    let file = repo.path().join(".axiom/setup.json");
    fs::write(&file, r#"{"version":1,"estimates":{"batch":"2G","cache_build":123},"steps":[{"id":"build","argv":["x"],"estimate":"512M"}]}"#).unwrap();
    let p = policy();
    for (kind, step, bytes, source) in [
        (Kind::Batch, None, 2 << 30, NeedSource::Project),
        (Kind::CacheBuild, None, 123, NeedSource::Project),
        (
            Kind::SetupStep,
            Some("build"),
            512 << 20,
            NeedSource::ProjectStep,
        ),
        (Kind::Wish, None, p.estimates.wish, NeedSource::Default),
    ] {
        let v = preflight::need(&p, kind, None, Some(repo.path()), step).unwrap();
        assert_eq!((v.bytes, v.source), (bytes, source));
    }
    assert_eq!(
        preflight::need(&p, Kind::Batch, Some(4), Some(repo.path()), Some("build"))
            .unwrap()
            .bytes,
        4
    );
    assert_eq!(
        preflight::need(&p, Kind::Batch, None, Some(repo.path()), Some("build"))
            .unwrap()
            .source,
        NeedSource::ProjectStep
    );
    assert_eq!(
        preflight::need(&p, Kind::Batch, Some(0), None, None)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    fs::write(
        &file,
        r#"{"estimates":{"batch":0},"steps":[{"id":"build","estimate":0}]}"#,
    )
    .unwrap();
    assert!(
        preflight::need(&p, Kind::Batch, None, Some(repo.path()), Some("build"))
            .unwrap()
            .bytes
            > 0
    );
    fs::write(&file, r#"{"estimates":{"batch":"nonsense"}}"#).unwrap();
    assert_eq!(
        preflight::project_estimate(repo.path(), Kind::Batch, None)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}
static ENV_LOCK: Mutex<()> = Mutex::new(());
#[test]
fn preflight_blocks_fake_unpack() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let file = home.path().join("disk.json");
    fs::write(&file, r#"{"free_bytes":119}"#).unwrap();
    std::env::set_var("AXIOM_FAKE_DISK", &file);
    let probe = preflight::os_probe();
    let p = policy();
    let c = preflight::check(
        home.path(),
        probe.as_ref(),
        &p,
        home.path(),
        Kind::Unpack,
        n(100),
    )
    .unwrap();
    assert!(!c.ok);
    assert_eq!(c.volume, "fake");
    for word in ["unpack", "100", "119", "paused"] {
        assert!(c.line().contains(word), "{}", c.line());
    }
    fs::write(&file, r#"{"free_bytes":120}"#).unwrap();
    assert!(
        preflight::check(
            home.path(),
            probe.as_ref(),
            &p,
            home.path(),
            Kind::Unpack,
            n(100)
        )
        .unwrap()
        .ok
    );
    std::env::remove_var("AXIOM_FAKE_DISK");
}
#[test]
fn parallel_preflights_respect_volume_reservations() {
    let home = tempdir().unwrap();
    let probe = Arc::new(Fake::new(170, "a"));
    let barrier = Arc::new(Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let h = home.path().to_owned();
        let f = probe.clone();
        let b = barrier.clone();
        threads.push(std::thread::spawn(move || {
            b.wait();
            preflight::reserve(
                &h,
                f.as_ref(),
                &policy(),
                &h,
                Kind::Batch,
                n(100),
                std::process::id(),
                "test",
            )
            .unwrap()
        }));
    }
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let held = results.into_iter().find_map(Result::ok).unwrap();
    let other = Fake::new(170, "b");
    assert!(preflight::reserve(
        home.path(),
        &other,
        &policy(),
        home.path(),
        Kind::Batch,
        n(100),
        std::process::id(),
        "other"
    )
    .unwrap()
    .is_ok());
    held.release().unwrap();
    assert!(preflight::reserve(
        home.path(),
        probe.as_ref(),
        &policy(),
        home.path(),
        Kind::Batch,
        n(100),
        std::process::id(),
        "next"
    )
    .unwrap()
    .is_ok());
    let db = rusqlite::Connection::open(home.path().join("storage/ledger.sqlite3")).unwrap();
    db.execute("INSERT INTO preflight_reservations(volume,bytes,pid,label,created_ms) VALUES('a',100,4294967295,'dead',0)",[]).unwrap();
    assert!(preflight::reserve(
        home.path(),
        probe.as_ref(),
        &policy(),
        home.path(),
        Kind::Batch,
        n(100),
        std::process::id(),
        "prune"
    )
    .unwrap()
    .is_ok());
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM preflight_reservations WHERE label='dead'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn low_disk_threshold() {
    let home = tempdir().unwrap();
    let f = Fake::new(49, "a");
    let p = policy();
    let c = preflight::low_disk(home.path(), &f, &p, home.path())
        .unwrap()
        .unwrap();
    assert!(!c.ok);
    assert_eq!(c.need.bytes, 50);
    assert_eq!(c.kind, Kind::Wish);
    f.free.store(50, Ordering::SeqCst);
    assert!(preflight::low_disk(home.path(), &f, &p, home.path())
        .unwrap()
        .is_none());
}
#[test]
fn notify_paused_writes_event_and_report() {
    let home = tempdir().unwrap();
    let f = Fake::new(1, "a");
    let c = preflight::check(home.path(), &f, &policy(), home.path(), Kind::Wish, n(100)).unwrap();
    let report = home.path().join("reports/disk.txt");
    fs::create_dir_all(report.parent().unwrap()).unwrap();
    fs::write(&report, "previous\n").unwrap();
    archive::compress(&report).unwrap();
    preflight::notify_paused(
        home.path(),
        preflight::EVENT_DISK_PAUSE,
        &c,
        json!({"job":1}),
    )
    .unwrap();
    let ev = events::read_events(home.path(), 0).unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].kind, preflight::EVENT_DISK_PAUSE);
    assert_eq!(ev[0].data["pending"]["job"], 1);
    assert_eq!(ev[0].data["check"]["need"]["bytes"], 100);
    let text = fs::read_to_string(&report).unwrap();
    assert!(text.starts_with("previous\n"));
    assert!(text.contains(&c.line()));
    assert!(!archive::archived_path(&report).exists());
}
#[test]
fn exact_boundary_and_one_byte_less() {
    let home = tempdir().unwrap();
    let f = Fake::new(170, "a");
    let p = policy();
    let reservation = preflight::reserve(
        home.path(),
        &f,
        &p,
        home.path(),
        Kind::Wish,
        n(50),
        std::process::id(),
        "held",
    )
    .unwrap()
    .unwrap();
    assert!(
        preflight::check(home.path(), &f, &p, home.path(), Kind::Batch, n(100))
            .unwrap()
            .ok
    );
    f.free.store(169, Ordering::SeqCst);
    assert!(
        !preflight::check(home.path(), &f, &p, home.path(), Kind::Batch, n(100))
            .unwrap()
            .ok
    );
    drop(reservation);
}

#[test]
fn os_probe_existing_and_missing_paths() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let file = home.path().join("file");
    fs::write(&file, b"x").unwrap();
    let probe = preflight::os_probe();
    let volume = probe.volume(home.path()).unwrap();
    assert!(!volume.is_empty());
    assert_eq!(probe.volume(&file).unwrap(), volume);
    assert_eq!(
        probe.volume(&home.path().join("missing/nested")).unwrap(),
        volume
    );
    probe.free_bytes(&file).unwrap();
    probe
        .free_bytes(&home.path().join("missing/nested"))
        .unwrap();
}

#[test]
fn stale_guard_cannot_release_replacement_and_keep_persists() {
    let home = tempdir().unwrap();
    let f = Fake::new(170, "a");
    let p = policy();
    let stale = preflight::reserve(
        home.path(),
        &f,
        &p,
        home.path(),
        Kind::Batch,
        n(100),
        u32::MAX,
        "dead",
    )
    .unwrap()
    .unwrap();
    let live = preflight::reserve(
        home.path(),
        &f,
        &p,
        home.path(),
        Kind::Batch,
        n(100),
        std::process::id(),
        "live",
    )
    .unwrap()
    .unwrap();
    drop(stale);
    assert!(
        !preflight::check(home.path(), &f, &p, home.path(), Kind::Batch, n(100))
            .unwrap()
            .ok
    );
    live.keep();
    assert!(
        !preflight::check(home.path(), &f, &p, home.path(), Kind::Batch, n(100))
            .unwrap()
            .ok
    );
}
