use crate::{
    base::{self, Failure, Res},
    goal, orch,
    storage::{
        self,
        preflight::{self, Check, Probe},
        Class, Ledger, Owner, Policy,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub kind: preflight::Kind,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub need_bytes: Option<u64>,
    pub session: Option<String>,
    pub progress_file: Option<PathBuf>,
    pub marker_file: Option<PathBuf>,
    pub register: Vec<(PathBuf, Class)>,
}

#[derive(Debug)]
pub enum Launched {
    Started { pid: u32, reservation: i64 },
    Paused { check: Check },
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn launch(home: &Path, probe: &dyn Probe, spec: &LaunchSpec) -> Res<Launched> {
    if !matches!(
        spec.kind,
        preflight::Kind::Batch | preflight::Kind::Unpack | preflight::Kind::CacheBuild
    ) {
        return Err(Failure::new(
            "invalid_argument",
            "unsupported managed launch kind",
        ));
    }
    if spec.argv.first().is_none_or(|arg| arg.is_empty()) {
        return Err(Failure::new("invalid_argument", "empty launch argv"));
    }
    let goal =
        match &spec.session {
            Some(session) => Some(goal::load(home, session)?.ok_or_else(|| {
                Failure::new("not_found", format!("no goal for session {session}"))
            })?),
            None => None,
        };
    let policy = Policy::load(home)?;
    let need = preflight::need(&policy, spec.kind, spec.need_bytes, Some(&spec.cwd), None)?;
    let reservation = match preflight::reserve(
        home,
        probe,
        &policy,
        &spec.cwd,
        spec.kind,
        need,
        std::process::id(),
        spec.session.as_deref().unwrap_or("none"),
    )? {
        Ok(reservation) => reservation,
        Err(check) => {
            if let Some(session) = &spec.session {
                let pending = serde_json::to_value(spec).map_err(io)?;
                goal::disk_pause(home, session, &check.line())?;
                let goal = goal::set_other(home, session, "pending_launch", pending.clone())?;
                if goal.project.is_some() {
                    let store = orch::store_for_goal(home, &goal)?;
                    preflight::notify_paused(
                        &store,
                        preflight::EVENT_PREFLIGHT_PAUSED,
                        &check,
                        pending,
                    )?;
                }
            }
            return Ok(Launched::Paused { check });
        }
    };
    let owner = Owner {
        project: goal
            .as_ref()
            .and_then(|g| g.project.clone())
            .unwrap_or_else(|| storage::SYSTEM_PROJECT.into()),
        wish: goal.and_then(|g| g.wish_id),
    };
    for (path, class) in &spec.register {
        storage::ensure_dir(home, path, *class, &owner)?;
    }
    let managed = home.join("storage").join("managed");
    fs::create_dir_all(&managed).map_err(io)?;
    let mut command = base::proc::command(&spec.argv);
    command
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
    }
    let mut child = base::proc::spawn_group(&mut command)
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let pid = child.id();
    let path = managed.join(format!(
        "{}-{pid}.json",
        spec.session.as_deref().unwrap_or("none")
    ));
    let record = json!({"pid": pid, "kind": spec.kind, "argv": spec.argv, "cwd": spec.cwd, "session": spec.session, "progress_file": spec.progress_file, "marker_file": spec.marker_file});
    let result = (|| -> Res<()> {
        let data = serde_json::to_vec(&record).map_err(io)?;
        base::atomic_write(&path, &data).map_err(io)?;
        // Transfer the existing reservation without a gap or a second space check.
        let ledger = Ledger::open(home)?;
        let updated = ledger
            .conn()
            .execute(
                "UPDATE preflight_reservations SET pid=?1 WHERE id=?2 AND pid=?3",
                rusqlite::params![pid, reservation.id, std::process::id()],
            )
            .map_err(|e| Failure::new("storage", e.to_string()))?;
        if updated != 1 {
            return Err(Failure::new("storage", "launch reservation disappeared"));
        }
        Ok(())
    })();
    if let Err(error) = result {
        base::proc::kill_tree(&mut child);
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    let id = reservation.id;
    reservation.keep();
    // Reap while this process lives; dropping a Child does not terminate it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(Launched::Started {
        pid,
        reservation: id,
    })
}

pub fn stop_managed(home: &Path, session: &str) -> Res<usize> {
    if session.is_empty()
        || session.len() > 64
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(Failure::new(
            "invalid_argument",
            format!("not a session id: {session:?}"),
        ));
    }
    let dir = home.join("storage").join("managed");
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(io(e)),
    };
    let prefix = format!("{session}-");
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(number) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = number.parse::<u32>() else {
            continue;
        };
        if pid == 0 {
            continue;
        }
        base::proc::kill_pid_tree(pid)?;
        fs::remove_file(entry.path()).map_err(io)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct FakeProbe(u64);
    impl Probe for FakeProbe {
        fn volume(&self, _: &Path) -> Res<String> {
            Ok("test-volume".into())
        }
        fn free_bytes(&self, _: &Path) -> Res<u64> {
            Ok(self.0)
        }
    }

    fn sleeping_spec(cwd: &Path, session: Option<&str>) -> LaunchSpec {
        #[cfg(windows)]
        let argv = vec!["cmd".into(), "/C".into(), "ping -n 3 127.0.0.1".into()];
        #[cfg(unix)]
        let argv = vec!["sleep".into(), "3".into()];
        LaunchSpec {
            kind: preflight::Kind::Batch,
            argv,
            cwd: cwd.into(),
            need_bytes: Some(100),
            session: session.map(str::to_owned),
            progress_file: None,
            marker_file: None,
            register: vec![],
        }
    }

    #[test]
    fn paused_records_pending_without_spawning() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        goal::open(home, "test-session", "wish", None).unwrap();
        let marker = home.join("marker");
        let mut spec = sleeping_spec(home, Some("test-session"));
        #[cfg(windows)]
        {
            spec.argv = vec![
                "cmd".into(),
                "/C".into(),
                format!("echo marker > \"{}\"", marker.display()),
            ];
        }
        #[cfg(unix)]
        {
            spec.argv = vec!["touch".into(), marker.to_string_lossy().into_owned()];
        }
        let result = launch(home, &FakeProbe(99), &spec).unwrap();
        assert!(matches!(result, Launched::Paused { .. }));
        assert!(!marker.exists());
        assert!(!home.join("storage/managed").exists());
        let goal = goal::load(home, "test-session").unwrap().unwrap();
        assert_eq!(goal.status, "blocked");
        assert_eq!(goal.reason.as_deref(), Some("disk_space"));
        assert_eq!(
            goal.other["pending_launch"],
            serde_json::to_value(&spec).unwrap()
        );
    }

    #[test]
    fn boundary_reserves_for_child_and_stop_kills_it() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let spec = sleeping_spec(home, None);
        let required = Policy::load(home).unwrap().reserve_bytes + spec.need_bytes.unwrap();
        assert!(matches!(
            launch(home, &FakeProbe(required - 1), &spec).unwrap(),
            Launched::Paused { .. }
        ));
        let Launched::Started { pid, reservation } =
            launch(home, &FakeProbe(required), &spec).unwrap()
        else {
            panic!("expected launch")
        };
        let path = home
            .join("storage/managed")
            .join(format!("none-{pid}.json"));
        let record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["pid"], pid);
        assert_eq!(record["argv"], json!(spec.argv));
        let ledger = Ledger::open(home).unwrap();
        let holder: u32 = ledger
            .conn()
            .query_row(
                "SELECT pid FROM preflight_reservations WHERE id=?1",
                [reservation],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(holder, pid);
        let Launched::Paused { check } = launch(home, &FakeProbe(required), &spec).unwrap() else {
            panic!("a live child must hold its reservation")
        };
        assert_eq!(check.reserved_bytes, spec.need_bytes.unwrap());
        assert_eq!(stop_managed(home, "none").unwrap(), 1);
        assert!(!path.exists());
        assert!(base::proc::wait_pid_exit(pid, Duration::from_secs(10)));
        assert_eq!(stop_managed(home, "none").unwrap(), 0);
    }

    #[test]
    fn project_notification_and_registered_owner() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let mut goal = goal::open(home, "project-session", "wish", None).unwrap();
        goal.project = Some("project".into());
        fs::write(
            home.join("goals/project-session.json"),
            serde_json::to_vec(&goal).unwrap(),
        )
        .unwrap();
        let dir = home.join("registered");
        let mut spec = sleeping_spec(home, Some("project-session"));
        spec.register.push((dir.clone(), Class::Scratch));
        assert!(matches!(
            launch(home, &FakeProbe(0), &spec).unwrap(),
            Launched::Paused { .. }
        ));
        let store = orch::store_for_goal(home, &goal).unwrap();
        let event: serde_json::Value =
            serde_json::from_slice(&fs::read(store.join("events.jsonl")).unwrap()).unwrap();
        assert_eq!(event["kind"], preflight::EVENT_PREFLIGHT_PAUSED);
        assert_eq!(
            event["data"]["pending"],
            serde_json::to_value(&spec).unwrap()
        );
        assert!(!dir.exists());
        let Launched::Started { pid, .. } = launch(home, &FakeProbe(u64::MAX), &spec).unwrap()
        else {
            panic!("expected launch")
        };
        let entry = Ledger::open(home).unwrap().get(&dir).unwrap().unwrap();
        assert_eq!(entry.owner.project, "project");
        assert_eq!(entry.owner.wish, goal.wish_id);
        assert_eq!(stop_managed(home, "project-session").unwrap(), 1);
        assert!(base::proc::wait_pid_exit(pid, Duration::from_secs(10)));
    }
}
