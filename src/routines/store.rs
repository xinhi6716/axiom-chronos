use super::{validate, Cause, Routine};
use crate::base::{atomic_write, Failure, Res};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub last_due_ms: BTreeMap<String, i64>,
    pub seen: BTreeSet<String>,
    pub log: Vec<LaunchRecord>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LaunchRecord {
    pub routine: String,
    pub cause: Cause,
    pub outcome: Outcome,
    pub at_ms: i64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    Launched { id: String },
    Denied,
    Failed { message: String },
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
impl Store {
    pub fn open(home: impl AsRef<Path>) -> Res<Self> {
        let dir = home.as_ref().join("routines");
        fs::create_dir_all(&dir).map_err(error)?;
        Ok(Self { dir })
    }
    fn lock(&self) -> Res<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(self.dir.join("lock"))
            .map_err(error)?;
        file.lock().map_err(error)?;
        Ok(file)
    }
    fn read<T: DeserializeOwned + Default>(&self, name: &str) -> Res<T> {
        match fs::read(self.dir.join(name)) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| Failure::new("json", e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(e) => Err(error(e)),
        }
    }
    fn write<T: Serialize>(&self, name: &str, value: &T) -> Res<()> {
        let bytes =
            serde_json::to_vec_pretty(value).map_err(|e| Failure::new("json", e.to_string()))?;
        atomic_write(&self.dir.join(name), &bytes).map_err(error)
    }
    pub fn routines(&self) -> Res<Vec<Routine>> {
        let _lock = self.lock()?;
        self.read("routines.json")
    }
    pub fn put(&self, routine: &Routine) -> Res<()> {
        validate(routine)?;
        let _lock = self.lock()?;
        let mut routines: Vec<Routine> = self.read("routines.json")?;
        if let Some(existing) = routines.iter_mut().find(|r| r.id == routine.id) {
            *existing = routine.clone();
        } else {
            routines.push(routine.clone());
        }
        self.write("routines.json", &routines)
    }
    pub fn remove(&self, id: &str) -> Res<()> {
        let _lock = self.lock()?;
        let mut routines: Vec<Routine> = self.read("routines.json")?;
        routines.retain(|routine| routine.id != id);
        self.write("routines.json", &routines)
    }
    pub fn state(&self) -> Res<State> {
        let _lock = self.lock()?;
        self.read("state.json")
    }
    pub fn save_state(&self, state: &State) -> Res<()> {
        let _lock = self.lock()?;
        self.write("state.json", state)
    }
    pub(crate) fn update_state<T>(&self, update: impl FnOnce(&mut State) -> T) -> Res<T> {
        let _lock = self.lock()?;
        let mut state = self.read("state.json")?;
        let result = update(&mut state);
        self.write("state.json", &state)?;
        Ok(result)
    }
}
