use std::path::PathBuf;
use crate::{base::{Failure, Res}, desktop::ux::seams::{RoutineRow, RoutineSeam}};
pub struct StoreRoutines { _home: PathBuf }
impl StoreRoutines { pub fn new(home: PathBuf) -> Self { Self { _home: home } } }
fn unsupported<T>() -> Res<T> { Err(Failure::new("unsupported", "Routines are not implemented")) }
impl RoutineSeam for StoreRoutines {
 fn list(&self) -> Res<Vec<RoutineRow>> { unsupported() }
 fn put(&self, _: &RoutineRow) -> Res<()> { unsupported() }
 fn set_enabled(&self, _: &str, _: bool) -> Res<()> { unsupported() }
 fn retry(&self, _: &str) -> Res<()> { unsupported() }
 fn remove(&self, _: &str) -> Res<()> { unsupported() }
}
