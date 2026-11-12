//! ONNX runtime integration. Owned by module ort.

mod api;
mod artifacts;
mod runtime;
mod session;
#[cfg(test)]
mod tests;

pub use artifacts::{read_manifest, sha256_file, verify_dir, write_manifest, Manifest};
pub use session::{ElemType, IoInfo, Session, Tensor};
pub use runtime::{find_runtime, runtime_candidates, runtime_lib_name, Runtime};
pub const ORT_API_VERSION: u32 = 14;
