//! Local, content-based progress tracking. Guard bookkeeping is never itself progress.
use super::*;
use sha2::{Digest, Sha256};
use std::fs;

#[derive(Default, Serialize, Deserialize)]
struct Progress {
    stop: Option<String>,
    continued: Option<String>,
    idle_continues: u32,
}

fn state_path(home: &Path, session: &str) -> Res<PathBuf> {
    Ok(path(home, session)?.with_extension("progress"))
}

pub(super) fn clear(home: &Path, session: &str) -> Res<()> {
    match fs::remove_file(state_path(home, session)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(e)),
    }
}

fn fingerprint(home: &Path, session: &str) -> Res<String> {
    let goal = required(home, session)?;
    let mut files = vec![path(home, session)?];
    if let Some(project) = goal.project {
        let store = crate::orch::store_for(home, &project)?;
        files.extend(["units.tsv", "ledger.tsv", "claims.tsv"].map(|name| store.join(name)));
        for dir in [store.join("handoffs"), store.join("handoffs/module")] {
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(e)),
            };
            for entry in entries {
                let entry = entry.map_err(io_error)?;
                if entry.file_type().map_err(io_error)?.is_file()
                    && entry.path().extension().is_some_and(|ext| ext == "json")
                {
                    files.push(entry.path());
                }
            }
        }
    }
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file.to_string_lossy().as_bytes());
        hash.update([0]);
        match fs::read(file) {
            Ok(bytes) => {
                hash.update([1]);
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(e) => return Err(io_error(e)),
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn update<T>(home: &Path, session: &str, f: impl FnOnce(&mut Progress, String) -> T) -> Res<T> {
    let _lock = lock(home)?;
    let file = state_path(home, session)?;
    let mut state = match fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io_error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Progress::default(),
        Err(e) => return Err(io_error(e)),
    };
    let result = f(&mut state, fingerprint(home, session)?);
    base::atomic_write(&file, &serde_json::to_vec(&state).map_err(io_error)?).map_err(io_error)?;
    Ok(result)
}

/// Only the first Stop on a given work state blocks; separate hook processes share this record.
pub(crate) fn block_stop(home: &Path, session: &str) -> Res<bool> {
    update(home, session, |state, current| {
        let changed = state.stop.as_ref() != Some(&current);
        state.stop = Some(current);
        changed
    })
}

/// Results after continues with unchanged work. The first result establishes the baseline.
pub(crate) fn idle_continues(home: &Path, session: &str) -> Res<u32> {
    update(home, session, |state, current| {
        state.idle_continues = if state.continued.as_ref() == Some(&current) {
            state.idle_continues.saturating_add(1)
        } else {
            0
        };
        state.continued = Some(current);
        state.idle_continues
    })
}

/// A positive count is required: invalid or zero values retain the default safety bound.
pub(crate) fn idle_limit() -> u32 {
    std::env::var("AXIOM_IDLE_CONTINUES")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(3)
}
