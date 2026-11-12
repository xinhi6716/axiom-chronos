use crate::base::{Failure, Res};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    path::Path,
};
pub struct InstanceLock {
    _file: File,
}
pub fn acquire(home: &Path) -> Res<InstanceLock> {
    fs::create_dir_all(home.join("service")).map_err(|e| Failure::new("io", e.to_string()))?;
    let home = home
        .canonicalize()
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join("service/service.lock"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    match file.try_lock() {
        Ok(()) => {
            let endpoint = crate::base::ipc::endpoint_for(&home)?;
            let record = serde_json::json!({"pid":std::process::id(),"started_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),"endpoint":endpoint.name,"instance":""});
            crate::base::atomic_write(
                &home.join("service/service.json"),
                record.to_string().as_bytes(),
            )
            .map_err(|e| Failure::new("io", e.to_string()))?;
            Ok(InstanceLock { _file: file })
        }
        Err(TryLockError::WouldBlock) => {
            let record = fs::read(home.join("service/service.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .unwrap_or_default();
            Err(Failure::new(
                "already_running",
                format!(
                    "already running (pid {}, endpoint {})",
                    record["pid"], record["endpoint"]
                ),
            ))
        }
        Err(TryLockError::Error(e)) => Err(Failure::new("io", e.to_string())),
    }
}
