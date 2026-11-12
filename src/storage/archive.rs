//! Gzip archives for individual storage files.
use crate::base::{self, Failure, Res};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

pub const SUFFIX: &str = ".gz";

pub fn archived_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(SUFFIX);
    PathBuf::from(name)
}

fn has_suffix(path: &Path, suffix: &str) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .ends_with(suffix.as_bytes())
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut bytes = Vec::new();
            GzDecoder::new(fs::File::open(archived_path(path))?).read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        Err(e) => Err(e),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn exists(path: &Path) -> bool {
    path.exists() || archived_path(path).exists()
}

pub fn restore(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let archive = archived_path(path);
    let file = match fs::File::open(&archive) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    GzDecoder::new(file).read_to_end(&mut bytes)?;
    base::atomic_write(path, &bytes)?;
    fs::remove_file(archive)?;
    Ok(true)
}

pub fn list(dir: &Path, suffix: &str) -> io::Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let bytes = name.as_encoded_bytes();
        let archived = if bytes.ends_with(SUFFIX.as_bytes()) {
            // Removing an ASCII suffix leaves a valid self-synchronizing encoded string.
            Some(unsafe {
                OsStr::from_encoded_bytes_unchecked(&bytes[..bytes.len() - SUFFIX.len()])
            })
        } else {
            None
        };
        if let Some(logical) =
            archived.filter(|s| s.as_encoded_bytes().ends_with(suffix.as_bytes()))
        {
            paths.insert(dir.join(logical));
        } else if bytes.ends_with(suffix.as_bytes()) {
            paths.insert(dir.join(&name));
        }
    }
    Ok(paths.into_iter().collect())
}

#[derive(Debug)]
pub struct Compressed {
    pub original_bytes: u64,
    pub archived_bytes: u64,
}

pub fn compress(path: &Path) -> Res<Compressed> {
    compress_inner(path).map_err(|e| Failure::new("io", e.to_string()))
}

fn compress_inner(path: &Path) -> io::Result<Compressed> {
    if has_suffix(path, SUFFIX) || !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular uncompressed file",
        ));
    }
    let original = fs::read(path)?;
    let archive = archived_path(path);
    let mut temp_name = archive.as_os_str().to_os_string();
    temp_name.push(format!(".{}.tmp", base::unique()));
    let temp = PathBuf::from(temp_name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&original)?;
        let mut file = encoder.finish()?;
        file.flush()?;
        drop(file);
        let mut verified = Vec::new();
        GzDecoder::new(fs::File::open(&temp)?).read_to_end(&mut verified)?;
        if verified != original {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive verification failed",
            ));
        }
        let archived_bytes = fs::metadata(&temp)?.len();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs::rename(&temp, &archive) {
                Err(e)
                    if cfg!(windows)
                        && matches!(e.raw_os_error(), Some(5 | 32))
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => break result?,
            }
        }
        fs::remove_file(path)?;
        Ok(Compressed {
            original_bytes: original.len() as u64,
            archived_bytes,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn eligible(path: &Path, now: SystemTime, after_days: u64) -> bool {
    if has_suffix(path, SUFFIX) {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    let Some(seconds) = after_days.checked_mul(24 * 60 * 60) else {
        return false;
    };
    metadata
        .modified()
        .ok()
        .and_then(|mtime| now.duration_since(mtime).ok())
        .is_some_and(|age| age > Duration::from_secs(seconds))
}
