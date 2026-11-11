use std::path::{Path, PathBuf};

pub fn expand(path: &str) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
            .map(|home| PathBuf::from(home).join(rest))
    } else {
        Some(PathBuf::from(path))
    }
}

pub fn folders_exist(paths: &[Option<PathBuf>]) -> bool {
    if paths.iter().all(|path| path.as_deref().is_some_and(Path::is_dir)) {
        true
    } else {
        eprintln!("skipping capture file checks: capture folders or home directory are unavailable");
        false
    }
}
