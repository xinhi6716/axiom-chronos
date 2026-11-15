use crate::base::Res;
pub struct RoutineRow { pub id: String, pub name: String, pub enabled: bool, pub schedule: String, pub next_run_ms: Option<i64>, pub last_run_ms: Option<i64>, pub last_outcome: Option<String>, pub last_error: Option<String> }
pub trait RoutineSeam: Send + Sync + 'static { fn list(&self) -> Res<Vec<RoutineRow>>; fn put(&self, row: &RoutineRow) -> Res<()>; fn set_enabled(&self, id: &str, on: bool) -> Res<()>; fn retry(&self, id: &str) -> Res<()>; fn remove(&self, id: &str) -> Res<()>; }
