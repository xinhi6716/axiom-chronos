use super::{Event, Source, SourceStatus};
use crate::{
    base::{self, Res},
    memory::Memory,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn collect_lint(home: &Path, project_dirs: &[PathBuf]) -> Res<(Vec<Event>, SourceStatus)> {
    let db = home.join("memory/memory.db");
    let mut status = SourceStatus {
        family: "lint".into(),
        state: "unobserved".into(),
        samples: 0,
        note: String::new(),
    };
    if !db.is_file() {
        return Ok((vec![], status));
    }
    let memory = Memory::open(home)?;
    let mut events = Vec::new();
    let dirs: BTreeSet<_> = project_dirs.iter().filter(|p| p.is_dir()).collect();
    for dir in dirs {
        status.samples += 1;
        let project = base::slug_for_dir(dir);
        for (i, problem) in memory.lint(&project, dir)?.into_iter().enumerate() {
            let location = db.display().to_string();
            events.push(Event {
                problem: format!("lint.{}", problem.kind),
                title: problem.kind.replace('_', " "),
                count: 1,
                cost_usd: None,
                unpriced: 0,
                detail: format!("current snapshot: {}", problem.message),
                snapshot: true,
                source: Source {
                    family: "lint".into(),
                    location: location.clone(),
                    session: None,
                    project: Some(project.clone()),
                    unit: None,
                    ts_ms: None,
                    key: super::collect::source_key(&[
                        "lint",
                        &location,
                        &dir.display().to_string(),
                        &i.to_string(),
                    ]),
                },
            });
        }
    }
    if status.samples > 0 {
        status.state = "observed".into();
    }
    Ok((events, status))
}
