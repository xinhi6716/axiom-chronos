use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Estimates {
    pub wish: u64,
    pub batch: u64,
    pub unpack: u64,
    pub cache_build: u64,
    pub setup_step: u64,
}
impl Default for Estimates {
    fn default() -> Self {
        let gib = 1024 * 1024 * 1024;
        Self {
            wish: 2 * gib,
            batch: 10 * gib,
            unpack: 10 * gib,
            cache_build: 10 * gib,
            setup_step: 512 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub build_cache_days: u64,
    pub compress_after_days: u64,
    pub low_disk_bytes: u64,
    pub reserve_bytes: u64,
    pub estimates: Estimates,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            build_cache_days: 7,
            compress_after_days: 30,
            low_disk_bytes: 5 * 1024 * 1024 * 1024,
            reserve_bytes: 2 * 1024 * 1024 * 1024,
            estimates: Estimates::default(),
        }
    }
}
impl Policy {
    pub fn load(home: &Path) -> Res<Self> {
        match fs::read(super::dir(home).join("policy.json")) {
            Ok(data) => {
                serde_json::from_slice(&data).map_err(|e| Failure::new("config", e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Failure::new("io", e.to_string())),
        }
    }
}
