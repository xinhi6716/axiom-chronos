use crate::base::{Failure, Res};
use std::{
    fs::{self, Metadata, ReadDir},
    path::Path,
};

fn guard() {
    #[cfg(any(test, debug_assertions))]
    if let Some(panel) = crate::desktop::components::panel::rendering_panel() {
        panic!("io during render {panel}");
    }
}
fn failure(e: std::io::Error) -> Failure {
    let kind = if e.kind() == std::io::ErrorKind::NotFound {
        "not_found"
    } else {
        "io"
    };
    Failure::new(kind, e.to_string())
}
pub fn read(path: impl AsRef<Path>) -> Res<Vec<u8>> {
    guard();
    fs::read(path).map_err(failure)
}
pub fn read_dir(path: impl AsRef<Path>) -> Res<ReadDir> {
    guard();
    fs::read_dir(path).map_err(failure)
}
pub fn metadata(path: impl AsRef<Path>) -> Res<Metadata> {
    guard();
    fs::metadata(path).map_err(failure)
}
