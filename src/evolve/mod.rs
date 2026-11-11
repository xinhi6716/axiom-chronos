use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub mod collect;
pub mod digest;
pub mod history;
pub mod launch;
pub mod lint;
pub mod repo;

pub const DEFAULT_SINCE_DAYS: u32 = 7;
pub const MAX_SINCE_DAYS: u32 = 365;

use crate::{
    base::{Failure, Res},
    goal, session, usage,
};
use digest::Digest;
use history::{MetricValue, RunRecord};

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub digest: Digest,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StartOutcome {
    NoOp { reason: String },
    Ran { record: RunRecord, exit_code: i32 },
}

fn validate(inputs: &Inputs) -> Res<()> {
    if !(1..=MAX_SINCE_DAYS).contains(&inputs.since_days) {
        return Err(Failure::new(
            "invalid_argument",
            "since_days must be between 1 and 365",
        ));
    }
    Ok(())
}

pub fn preview(inputs: &Inputs) -> Res<Preview> {
    validate(inputs)?;
    let collected = collect::collect(inputs)?;
    let period = usage::chronos_period(
        &usage::Locations {
            home: inputs.home.clone(),
            projects: inputs.projects.clone(),
        },
        inputs.now_ms,
        inputs.since_days,
    )?;
    let earlier = history::list(&inputs.home)?;
    let digest = digest::build(
        collected,
        &period,
        &earlier,
        inputs.now_ms,
        inputs.since_days,
    );
    let text = digest::render(&digest);
    Ok(Preview { digest, text })
}

pub fn list_runs(home: &Path) -> Res<Vec<RunRecord>> {
    history::list(home)
}

pub fn start(
    inputs: &Inputs,
    repo: Option<&Path>,
    config: &session::Config,
    input: Box<dyn BufRead + Send>,
    out: &mut dyn Write,
) -> Res<StartOutcome> {
    validate(inputs)?;
    if config.home != inputs.home {
        return Err(Failure::new(
            "invalid_argument",
            "config.home must equal inputs.home",
        ));
    }
    let id = history::new_id(inputs.now_ms);
    let calendar = id.rsplitn(3, '-').nth(2).expect("run id contains a date");
    let month = calendar.len() - 4;
    let date = format!(
        "{}-{}-{}",
        &calendar[..month],
        &calendar[month..month + 2],
        &calendar[month + 2..]
    );
    let dir = repo::resolve(&inputs.home, repo)?;
    let info = repo::check(&dir, &date)?;
    let preview = preview(inputs)?;
    if preview.digest.problems.is_empty() {
        return Ok(StartOutcome::NoOp {
            reason: "No recorded problems in the period.".into(),
        });
    }
    repo::create_branch(&info)?;
    let mut record = RunRecord {
        id,
        started_ms: inputs.now_ms,
        repo: info.root.to_string_lossy().into_owned(),
        base_commit: info.base_commit,
        branch: info.branch,
        since_days: inputs.since_days,
        start_ms: preview.digest.start_ms,
        end_ms: inputs.now_ms,
        problems: preview
            .digest
            .problems
            .iter()
            .take(digest::MAX_PROBLEMS)
            .map(|p| p.id.clone())
            .collect(),
        metrics: preview
            .digest
            .problems
            .iter()
            .map(|p| MetricValue {
                problem: p.id.clone(),
                value: p.metric.value,
                samples: p.metric.samples,
            })
            .collect(),
        state: "started".into(),
        session: None,
        goal_status: None,
        exit_code: None,
        error: None,
    };
    history::save(&inputs.home, &record)?;
    let wish = launch::wish_text(&preview.digest, &info.root);
    let result = session::wait::run_observing(
        config,
        "wish",
        &wish,
        &info.root,
        None,
        input,
        out,
        &session::run::Launch::default(),
        &mut |id| record.session = Some(id.to_owned()),
    );
    let exit_code = match result {
        Ok(code) => code,
        Err(error) => {
            let message = error.to_string();
            record.state = "launch-failed".into();
            record.error = Some(message.clone());
            history::save(&inputs.home, &record)?;
            return Err(Failure::new("launch_failed", message));
        }
    };
    record.goal_status = match &record.session {
        Some(id) => goal::load(&inputs.home, id)?.map(|goal| goal.status),
        None => None,
    };
    record.exit_code = Some(exit_code);
    record.state = "ended".into();
    history::save(&inputs.home, &record)?;
    repo::remember(&inputs.home, &info.root)?;
    Ok(StartOutcome::Ran { record, exit_code })
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Inputs {
    pub home: PathBuf,
    pub projects: PathBuf,
    pub now_ms: i64,
    pub since_days: u32,
}
impl Inputs {
    pub fn from_env(since_days: u32) -> Inputs {
        Inputs {
            home: crate::base::axiom_home(),
            projects: crate::usage::Locations::from_env().projects,
            now_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
                .unwrap_or_default(),
            since_days,
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Source {
    pub family: String,
    pub location: String,
    pub session: Option<String>,
    pub project: Option<String>,
    pub unit: Option<String>,
    pub ts_ms: Option<i64>,
    pub key: String,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Event {
    pub problem: String,
    pub title: String,
    pub count: u64,
    pub cost_usd: Option<f64>,
    pub unpriced: u64,
    pub detail: String,
    pub snapshot: bool,
    pub source: Source,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Gap {
    pub family: String,
    pub location: String,
    pub reason: String,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SourceStatus {
    pub family: String,
    pub state: String,
    pub samples: u64,
    pub note: String,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Collected {
    pub events: Vec<Event>,
    pub gaps: Vec<Gap>,
    pub sources: Vec<SourceStatus>,
}
