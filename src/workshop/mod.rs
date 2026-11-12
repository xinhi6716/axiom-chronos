pub mod api;
pub mod approval;
pub mod host;
pub mod manifest;
pub mod permissions;
pub mod services;

pub use manifest::{Manifest, Panel};
pub use permissions::{DataDirs, Grants, Permission, Provider};

pub const NOT_OS_ISOLATION: &str = "Workshop permissions are enforced at the API boundary and at spawn. This is not OS isolation: a module process can reach anything its user account can.";
