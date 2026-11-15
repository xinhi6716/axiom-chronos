use super::{program_dir, Stage};
use crate::base::{atomic_write, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageEntry {
    pub stage: Stage,
    pub status: StageStatus,
    pub artifact: String,
    pub sha256: Option<String>,
    pub at: String,
    pub detail: Option<String>,
    pub members: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramState {
    pub run_id: String,
    pub question: String,
    pub brief_sha256: String,
    pub stages: Vec<StageEntry>,
    pub complete: bool,
}

pub fn load(store: &Path) -> Res<Option<ProgramState>> {
    let path = program_dir(store).join("state.json");
    match fs::read(&path) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| Failure::new("json", format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Failure::new("io", format!("{}: {e}", path.display()))),
    }
}

pub fn save(store: &Path, state: &ProgramState) -> Res<()> {
    let dir = program_dir(store);
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", format!("{}: {e}", dir.display())))?;
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|e| Failure::new("json", e.to_string()))?;
    let path = dir.join("state.json");
    atomic_write(&path, &bytes).map_err(|e| Failure::new("io", format!("{}: {e}", path.display())))
}

pub fn gate(store: &Path) -> Res<()> {
    let Some(state) = load(store)? else {
        return Ok(());
    };
    for stage in Stage::ALL {
        if !state
            .stages
            .iter()
            .any(|entry| entry.stage == stage && entry.status == StageStatus::Done)
        {
            return Err(Failure::new(
                "research_program_gate",
                format!("missing done stage {}", stage.as_str()),
            ));
        }
    }
    for stage in Stage::ALL {
        let entry = state
            .stages
            .iter()
            .find(|entry| entry.stage == stage && entry.status == StageStatus::Done)
            .unwrap();
        let path = program_dir(store).join(stage.artifact());
        let exists = if stage.artifact().ends_with('/') {
            path.is_dir()
        } else {
            path.is_file()
        };
        if entry.artifact != stage.artifact() || !exists {
            return Err(Failure::new(
                "research_program_gate",
                format!(
                    "missing artifact for {}: {}",
                    stage.as_str(),
                    stage.artifact()
                ),
            ));
        }
    }
    if !state.complete {
        return Err(Failure::new(
            "research_program_gate",
            "research program is not complete",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn full(store: &Path) -> ProgramState {
        let dir = program_dir(store);
        fs::create_dir_all(&dir).unwrap();
        let stages = Stage::ALL
            .into_iter()
            .map(|stage| {
                let path = dir.join(stage.artifact());
                if stage.artifact().ends_with('/') {
                    fs::create_dir_all(path).unwrap();
                } else {
                    fs::write(path, "{} ").unwrap();
                }
                StageEntry {
                    stage,
                    status: StageStatus::Done,
                    artifact: stage.artifact().into(),
                    sha256: None,
                    at: "now".into(),
                    detail: None,
                    members: BTreeMap::new(),
                }
            })
            .collect();
        ProgramState {
            run_id: "run".into(),
            question: "question".into(),
            brief_sha256: "hash".into(),
            stages,
            complete: true,
        }
    }

    #[test]
    fn no_state_preserves_old_flow() {
        let dir = tempdir().unwrap();
        assert!(gate(dir.path()).is_ok());
    }

    #[test]
    fn first_missing_done_stage_is_named() {
        let dir = tempdir().unwrap();
        let mut state = full(dir.path());
        state.stages[1].status = StageStatus::Failed;
        state.stages.remove(4);
        save(dir.path(), &state).unwrap();
        let err = gate(dir.path()).unwrap_err();
        assert_eq!(err.kind, "research_program_gate");
        assert!(err.message.contains("dossier"));
    }

    #[test]
    fn missing_artifact_and_incomplete_block_gate() {
        let dir = tempdir().unwrap();
        let mut state = full(dir.path());
        save(dir.path(), &state).unwrap();
        fs::remove_file(program_dir(dir.path()).join(Stage::RedTeam.artifact())).unwrap();
        let err = gate(dir.path()).unwrap_err();
        assert_eq!(err.kind, "research_program_gate");
        assert!(err.message.contains("red_team"));
        fs::write(
            program_dir(dir.path()).join(Stage::RedTeam.artifact()),
            "{}",
        )
        .unwrap();
        state.complete = false;
        save(dir.path(), &state).unwrap();
        assert_eq!(gate(dir.path()).unwrap_err().kind, "research_program_gate");
    }

    #[test]
    fn complete_state_and_negative_control() {
        let dir = tempdir().unwrap();
        let mut state = full(dir.path());
        save(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(state.clone()));
        assert!(gate(dir.path()).is_ok());
        state.stages.retain(|entry| entry.stage != Stage::Synthesis);
        save(dir.path(), &state).unwrap();
        let err = gate(dir.path()).unwrap_err();
        assert_eq!(err.kind, "research_program_gate");
        assert!(err.message.contains("synthesis"));
    }
}
