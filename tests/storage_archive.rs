use axiom::storage::archive::{self, SUFFIX};
use flate2::{write::GzEncoder, Compression};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    time::{Duration, SystemTime},
};

fn gzip(path: &Path, bytes: &[u8]) {
    let mut encoder = GzEncoder::new(
        fs::File::create(archive::archived_path(path)).unwrap(),
        Compression::default(),
    );
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap();
}

#[test]
fn compress_round_trip_and_plain_wins() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("data");
    let bytes = b"hello\0\xff archive\n";
    fs::write(&path, bytes).unwrap();
    let result = archive::compress(&path).unwrap();
    let gz = archive::archived_path(&path);
    assert_eq!(SUFFIX, ".gz");
    assert_eq!(result.original_bytes, bytes.len() as u64);
    assert_eq!(result.archived_bytes, fs::metadata(&gz).unwrap().len());
    assert!(!path.exists());
    assert!(gz.is_file());
    assert!(archive::exists(&path));
    assert_eq!(archive::read(&path).unwrap(), bytes);
    assert_eq!(
        archive::read_to_string(&path).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fs::write(&path, b"plain").unwrap();
    assert_eq!(archive::read_to_string(&path).unwrap(), "plain");
    assert_eq!(archive::read(&path).unwrap(), b"plain");
    assert!(!archive::restore(&path).unwrap());
    assert!(gz.exists());
}

#[test]
fn read_and_restore_fallback() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("item.txt");
    assert!(!archive::exists(&path));
    assert_eq!(
        archive::read(&path).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!archive::restore(&path).unwrap());
    gzip(&path, b"archived text");
    assert_eq!(archive::read_to_string(&path).unwrap(), "archived text");
    assert!(archive::restore(&path).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"archived text");
    assert!(!archive::archived_path(&path).exists());
    assert!(!archive::restore(&path).unwrap());
    fs::remove_file(&path).unwrap();
    fs::write(archive::archived_path(&path), b"invalid gzip").unwrap();
    assert!(archive::restore(&path).is_err());
    assert!(!path.exists());
    assert!(archive::archived_path(&path).exists());
}

#[test]
fn list_filters_sorts_and_deduplicates() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a.log");
    let b = t.path().join("b.log");
    fs::write(&a, b"plain").unwrap();
    gzip(&a, b"archive");
    gzip(&b, b"archive");
    fs::write(t.path().join("c.txt"), b"other").unwrap();
    fs::create_dir(t.path().join("d.log")).unwrap();
    assert_eq!(
        archive::list(t.path(), ".log").unwrap(),
        vec![a.clone(), b.clone()]
    );
    assert_eq!(
        archive::list(t.path(), "").unwrap(),
        vec![a, b, t.path().join("c.txt")]
    );
    assert!(archive::list(t.path(), ".missing").unwrap().is_empty());
    assert!(archive::list(&t.path().join("missing"), ".log").is_err());
}

#[test]
fn eligible_checks_age_and_file_type() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("old");
    fs::write(&path, b"x").unwrap();
    let now = SystemTime::now();
    assert!(!archive::eligible(&path, now, 1));
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(now - Duration::from_secs(2 * 86400))
        .unwrap();
    assert!(archive::eligible(&path, now, 1));
    assert!(!archive::eligible(&path, now, 3));
    assert!(!archive::eligible(&path, now, u64::MAX));
    let gz = t.path().join("old.gz");
    fs::rename(&path, &gz).unwrap();
    assert!(!archive::eligible(&gz, now, 1));
    assert!(!archive::eligible(t.path(), now, 0));
    assert!(!archive::eligible(&path, now, 0));
}

#[test]
fn compression_rejects_non_regular_and_gzip() {
    let t = tempfile::tempdir().unwrap();
    let gz = t.path().join("file.gz");
    fs::write(&gz, b"unchanged").unwrap();
    assert!(archive::compress(&gz).is_err());
    assert_eq!(fs::read(&gz).unwrap(), b"unchanged");
    assert!(archive::compress(t.path()).is_err());
    let plain = t.path().join("file");
    fs::write(&plain, b"source").unwrap();
    let link = t.path().join("link");
    #[cfg(unix)]
    let made_link = std::os::unix::fs::symlink(&plain, &link).is_ok();
    #[cfg(windows)]
    let made_link = std::os::windows::fs::symlink_file(&plain, &link).is_ok();
    #[cfg(not(any(unix, windows)))]
    let made_link = false;
    if made_link {
        assert!(archive::compress(&link).is_err());
        assert!(!archive::eligible(
            &link,
            SystemTime::now() + Duration::from_secs(86400),
            0
        ));
        assert_eq!(fs::read(&plain).unwrap(), b"source");
        assert!(!archive::archived_path(&link).exists());
    }
}

#[test]
fn compression_failure_preserves_original() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("data");
    fs::write(&path, b"keep me").unwrap();
    fs::create_dir(archive::archived_path(&path)).unwrap();
    assert!(archive::compress(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"keep me");
    assert!(archive::archived_path(&path).is_dir());
    assert!(fs::read_dir(t.path()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}
