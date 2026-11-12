use crate::{
    base::{self, Failure, Res},
    goal,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    pub instance: String,
    pub pid: u32,
    pub sessions: Vec<Session>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    pub handle: String,
    pub session: Option<String>,
    pub cwd: String,
    pub text: String,
    pub state: String,
    #[serde(default)]
    pub user_stopped: bool,
    #[serde(default)]
    pub recovered: bool,
}
pub fn load(home: &Path) -> Res<Store> {
    match std::fs::read(home.join("service/hosted.json")) {
        Ok(b) => {
            serde_json::from_slice(&b).map_err(|e| Failure::new("invalid_json", e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}
pub fn save(home: &Path, store: &Store) -> Res<()> {
    let b = serde_json::to_vec(store).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&home.join("service/hosted.json"), &b)
        .map_err(|e| Failure::new("io", e.to_string()))
}
pub fn recover(home: &Path, store: &mut Store) -> Res<()> {
    for s in &mut store.sessions {
        if matches!(s.state.as_str(), "starting" | "running" | "limit_wait") {
            if let Some(id) = &s.session {
                if goal::interrupt(home, id, "the background service ended unexpectedly")?.is_some()
                {
                    s.recovered = true;
                }
            }
            if s.state != "limit_wait" {
                s.state = "ended".into();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_store_round_trip_and_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("service")).unwrap();
        let id = "12345678-1234-1234-1234-123456789abc";
        goal::open(tmp.path(), id, "wish", Some(tmp.path().to_str().unwrap())).unwrap();
        let mut store = Store {
            instance: "old".into(),
            pid: 12,
            sessions: vec![Session {
                handle: "h".into(),
                session: Some(id.into()),
                cwd: "p".into(),
                text: "wish".into(),
                state: "running".into(),
                user_stopped: false,
                recovered: false,
            }],
        };
        save(tmp.path(), &store).unwrap();
        recover(tmp.path(), &mut store).unwrap();
        save(tmp.path(), &store).unwrap();
        let saved = load(tmp.path()).unwrap();
        assert!(saved.sessions[0].recovered);
        assert_eq!(
            goal::load(tmp.path(), id).unwrap().unwrap().status,
            "blocked"
        );
        assert_eq!(
            std::fs::read_dir(tmp.path().join("service"))
                .unwrap()
                .count(),
            1
        );
    }
}
