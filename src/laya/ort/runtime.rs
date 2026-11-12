use super::{
    api::{OrtApi, OrtApiBase},
    ORT_API_VERSION,
};
use crate::base::{Failure, Res};
use libloading::Library;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub fn runtime_lib_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

pub fn runtime_candidates(home: Option<&Path>, repo: Option<&Path>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Some(p) = std::env::var_os("AXIOM_LAYA_ORT_LIB").filter(|p| !p.is_empty()) {
        result.push(p.into());
    }
    if let Some(home) = home {
        let dir = super::super::laya_dir(home);
        if let Ok(bytes) = std::fs::read(dir.join("router.json")) {
            if let Some(p) = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| {
                    v.get("runtime_lib")
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
            {
                result.push(PathBuf::from(p));
            }
        }
        result.push(dir.join("runtime").join(runtime_lib_name()));
    }
    if let Some(repo) = repo {
        result.push(repo.join(".laya").join("runtime").join(runtime_lib_name()));
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        result.push(
            PathBuf::from(root)
                .join("System32")
                .join(runtime_lib_name()),
        );
    }
    result
}
pub fn find_runtime(home: Option<&Path>, repo: Option<&Path>) -> Res<PathBuf> {
    runtime_candidates(home, repo)
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            Failure::new(
                "laya_runtime_missing",
                "ONNX Runtime not found; set AXIOM_LAYA_ORT_LIB or use tools/laya/fetch.py",
            )
        })
}
pub struct Runtime {
    _lib: Library,
    #[allow(dead_code)]
    pub(crate) api: &'static OrtApi,
}
// The library remains loaded for the lifetime of its immutable ORT API table. ORT APIs are thread-safe.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

impl Runtime {
    pub fn load(lib: &Path) -> Res<Arc<Self>> {
        static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Runtime>>>> = OnceLock::new();
        let path = std::fs::canonicalize(lib)
            .map_err(|e| Failure::new("laya_runtime_load", format!("{}: {e}", lib.display())))?;
        let mut cache = CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(rt) = cache.get(&path) {
            return Ok(rt.clone());
        }
        let library = unsafe { Library::new(&path) }
            .map_err(|e| Failure::new("laya_runtime_load", format!("{}: {e}", path.display())))?;
        let get_base = unsafe {
            library.get::<unsafe extern "system" fn() -> *const OrtApiBase>(b"OrtGetApiBase\0")
        }
        .map_err(|e| Failure::new("laya_runtime_load", e.to_string()))?;
        let api = unsafe { api_from_base(get_base())? };
        let rt = Arc::new(Runtime { _lib: library, api });
        cache.insert(path, rt.clone());
        Ok(rt)
    }
    pub fn api_version(&self) -> u32 {
        ORT_API_VERSION
    }
}

unsafe fn api_from_base(base: *const OrtApiBase) -> Res<&'static OrtApi> {
    if base.is_null() {
        return Err(Failure::new(
            "laya_runtime_incompatible",
            "OrtGetApiBase returned null",
        ));
    }
    let api = ((*base).get_api)(ORT_API_VERSION);
    if api.is_null() {
        return Err(Failure::new(
            "laya_runtime_incompatible",
            format!("ONNX Runtime API {ORT_API_VERSION} is unavailable"),
        ));
    }
    Ok(&*api)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;
    unsafe extern "system" fn no_api(_: u32) -> *const OrtApi {
        std::ptr::null()
    }
    unsafe extern "system" fn version() -> *const c_char {
        std::ptr::null()
    }
    #[test]
    fn incompatible_api_is_reported() {
        let base = OrtApiBase {
            get_api: no_api,
            get_version_string: version,
        };
        assert_eq!(
            unsafe { api_from_base(&base) }.err().unwrap().kind,
            "laya_runtime_incompatible"
        );
        assert_eq!(
            unsafe { api_from_base(std::ptr::null()) }
                .err()
                .unwrap()
                .kind,
            "laya_runtime_incompatible"
        );
    }
    #[test]
    fn runtime_and_session_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime>();
        assert_send_sync::<super::super::Session>();
    }
}
