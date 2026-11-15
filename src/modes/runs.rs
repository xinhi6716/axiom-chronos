use super::{Mode, Run, Session};
use crate::{
    base::{self, Failure, Res},
    codex::slots,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    path::{Component, Path},
};

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn read<T: DeserializeOwned>(path: &Path) -> Res<T> {
    let bytes = fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Failure::new("not_found", format!("not found: {}", path.display()))
        } else {
            error(e)
        }
    })?;
    serde_json::from_slice(&bytes).map_err(error)
}
fn write(path: &Path, value: &impl Serialize) -> Res<()> {
    fs::create_dir_all(path.parent().expect("record has parent")).map_err(error)?;
    let bytes = serde_json::to_vec(value).map_err(error)?;
    base::atomic_write(path, &bytes).map_err(error)
}
pub(crate) fn relative(value: &str) -> bool {
    let path = Path::new(value);
    !value.starts_with(['/', '\\'])
        && !value.contains(':')
        && !value.split(['/', '\\']).any(|part| part == "..")
        && !path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}
fn session_path(home: &Path, id: &str) -> Res<std::path::PathBuf> {
    base::require_slug("session", id)?;
    Ok(home
        .join("modes")
        .join("sessions")
        .join(id)
        .join("session.json"))
}
fn run_path(home: &Path, id: &str) -> Res<std::path::PathBuf> {
    base::require_slug("run", id)?;
    Ok(home.join("modes").join("runs").join(format!("{id}.json")))
}
pub fn create_session(home: &Path, project: Option<&Path>, scope: &[String]) -> Res<Session> {
    if scope.iter().any(|entry| !relative(entry)) {
        return Err(Failure::new(
            "invalid_argument",
            "scope must contain relative paths without parent components",
        ));
    }
    let session = Session {
        id: base::unique(),
        created: base::now_iso(),
        project: project.map(Path::to_path_buf),
        scope: scope.to_vec(),
    };
    write(&session_path(home, &session.id)?, &session)?;
    Ok(session)
}
pub fn load_session(home: &Path, id: &str) -> Res<Session> {
    read(&session_path(home, id)?)
}
pub fn start_run(
    home: &Path,
    session: &str,
    mode: Mode,
    parent: Option<&str>,
    reference: Option<&str>,
) -> Res<Run> {
    load_session(home, session)?;
    if let Some(id) = parent {
        load_run(home, id)?;
    }
    let run = Run {
        id: base::unique(),
        session: session.into(),
        parent: parent.map(str::to_owned),
        mode,
        started: base::now_iso(),
        finished: None,
        outcome: None,
        reference: reference.map(str::to_owned),
    };
    write(&run_path(home, &run.id)?, &run)?;
    Ok(run)
}
pub fn finish_run(home: &Path, id: &str, outcome: &str) -> Res<Run> {
    let mut run = load_run(home, id)?;
    if run.finished.is_some() {
        return Err(Failure::new("invalid_argument", "run is already finished"));
    }
    run.finished = Some(base::now_iso());
    run.outcome = Some(outcome.into());
    write(&run_path(home, id)?, &run)?;
    Ok(run)
}
pub fn load_run(home: &Path, id: &str) -> Res<Run> {
    read(&run_path(home, id)?)
}
pub fn session_runs(home: &Path, session: &str) -> Res<Vec<Run>> {
    load_session(home, session)?;
    let dir = home.join("modes").join("runs");
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(e)),
    };
    let mut runs = Vec::new();
    for entry in entries {
        let path = entry.map_err(error)?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let run: Run = read(&path)?;
            if run.session == session {
                runs.push(run);
            }
        }
    }
    runs.sort_by(|a, b| (&a.started, &a.id).cmp(&(&b.started, &b.id)));
    Ok(runs)
}
pub fn record_launch(home: &Path, skill: &str, reference: &str) {
    let mode = match skill {
        "wish" => Mode::Wish,
        "research" => Mode::Research,
        _ => return,
    };
    let _ = create_session(home, None, &[])
        .and_then(|session| start_run(home, &session.id, mode, None, Some(reference)));
}
pub fn acquire_slot(home: &Path, limit: u64) -> Res<slots::Slot> {
    let mut checks = 0u64;
    slots::acquire(home, limit, &mut slots::Wait::default(), || {
        if checks > limit || limit == 0 {
            return Err(Failure::new("codex_slots", "Codex slot limit reached"));
        }
        checks = checks.saturating_add(1);
        Ok(())
    })
}
