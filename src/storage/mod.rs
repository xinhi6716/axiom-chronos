use crate::base::{Failure, Res};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub mod archive;
pub mod ledger;
pub mod policy;
pub mod preflight;
pub mod retention;
pub mod usage;
pub mod worktree;
pub use ledger::{Class, Entry, EntryState, Lease, Ledger, Owner};
pub use policy::Policy;
pub const SYSTEM_PROJECT: &str = "_chronos";
pub fn dir(home: &Path) -> PathBuf {
    home.join("storage")
}
fn log_error(home: &Path, error: &Failure) {
    let _ = fs::create_dir_all(dir(home));
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir(home).join("errors.log"))
    {
        let _ = writeln!(f, "{error}");
    }
}
pub fn register_quiet(home: &Path, path: &Path, class: Class, owner: &Owner, meta: Value) {
    if let Err(e) =
        Ledger::open(home).and_then(|l| l.register(path, class, owner, meta).map(|_| ()))
    {
        log_error(home, &e);
    }
}
pub fn ensure_dir(home: &Path, path: &Path, class: Class, owner: &Owner) -> Res<PathBuf> {
    fs::create_dir_all(path).map_err(|e| Failure::new("io", e.to_string()))?;
    register_quiet(home, path, class, owner, Value::Object(Default::default()));
    Ok(path.to_path_buf())
}
