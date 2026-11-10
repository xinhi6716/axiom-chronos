use super::{Collected, Event, Gap, Inputs, Source, SourceStatus};
use crate::{
    base::{Failure, Res},
    goal::Goal,
    orch::handoff::Handoff,
    usage::{self, Warning},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

fn slug(s: &str) -> String {
    let needs_suffix = !s.is_ascii();
    let hash = format!("{:x}", Sha256::digest(s.as_bytes()));
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        format!("unknown-{}", &hash[..8])
    } else if needs_suffix {
        format!("{s}-{}", &hash[..8])
    } else {
        s
    }
}
fn paths(dir: &Path, gaps: &mut Vec<Gap>, family: &str) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(entries) => {
            let mut out = Vec::new();
            for e in entries {
                match e {
                    Ok(e) => out.push(e.path()),
                    Err(e) => gaps.push(Gap {
                        family: family.into(),
                        location: dir.display().to_string(),
                        reason: e.to_string(),
                    }),
                }
            }
            out.sort();
            out
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => {
            gaps.push(Gap {
                family: family.into(),
                location: dir.display().to_string(),
                reason: e.to_string(),
            });
            vec![]
        }
    }
}
fn gap(gaps: &mut Vec<Gap>, family: &str, location: &str, reason: impl ToString) {
    gaps.push(Gap {
        family: family.into(),
        location: location.into(),
        reason: reason.to_string(),
    });
}
fn status(family: &str, samples: u64) -> SourceStatus {
    SourceStatus {
        family: family.into(),
        state: if samples == 0 {
            "unobserved"
        } else {
            "observed"
        }
        .into(),
        samples,
        note: String::new(),
    }
}
pub(super) fn source_key(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|field| format!("{}:{field}", field.len()))
        .collect()
}
fn source(
    family: &str,
    location: String,
    suffix: &[&str],
    project: Option<String>,
    unit: Option<String>,
    session: Option<String>,
    ts_ms: Option<i64>,
) -> Source {
    Source {
        family: family.into(),
        key: source_key(&[&[family, &location][..], suffix].concat()),
        location,
        session,
        project,
        unit,
        ts_ms,
    }
}
fn event(problem: String, detail: String, snapshot: bool, source: Source) -> Event {
    Event {
        title: problem.clone(),
        problem,
        count: 1,
        cost_usd: None,
        unpriced: 0,
        detail,
        snapshot,
        source,
    }
}
fn modified(path: &Path) -> Option<i64> {
    let time = fs::metadata(path).ok()?.modified().ok()?;
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok(),
        Err(e) => i64::try_from(e.duration().as_millis()).ok()?.checked_neg(),
    }
}
// Accept RFC 3339 timestamps, including offsets and fractional seconds.
fn parse_ms(s: &str) -> Option<i64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?([Zz]|[+-]\d{2}:\d{2})$",
    ).expect("constant timestamp regex")
    });
    let c = re.captures(s)?;
    let n = |i| c.get(i)?.as_str().parse::<i64>().ok();
    let (year, month, day, hour, minute, second) = (n(1)?, n(2)?, n(3)?, n(4)?, n(5)?, n(6)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let maxday = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > maxday || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let fraction = c
        .get(7)
        .map(|v| {
            format!("{}00", v.as_str())
                .chars()
                .take(3)
                .collect::<String>()
                .parse::<i64>()
                .ok()
        })
        .unwrap_or(Some(0))?;
    let zone = c.get(8)?.as_str();
    let offset = if zone.eq_ignore_ascii_case("z") {
        0
    } else {
        let h: i64 = zone[1..3].parse().ok()?;
        let m: i64 = zone[4..6].parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        (h * 60 + m) * if zone.starts_with('-') { -1 } else { 1 }
    };
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Some(
        days * 86_400_000
            + hour * 3_600_000
            + (minute - offset) * 60_000
            + second * 1000
            + fraction,
    )
}
fn log_has_session(path: &Path, session: &str) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    BufReader::new(file)
        .lines()
        .take(20)
        .filter_map(Result::ok)
        .any(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                return false;
            };
            v["type"] == "system"
                && v["subtype"] == "init"
                && v["session_id"].as_str() == Some(session)
        })
}
fn json_file<T: serde::de::DeserializeOwned>(
    path: &Path,
    family: &str,
    gaps: &mut Vec<Gap>,
) -> Option<T> {
    let loc = path.display().to_string();
    match fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(v) => Some(v),
        Err(e) => {
            gap(gaps, family, &loc, e);
            None
        }
    }
}
fn timestamp(ts: Option<i64>, family: &str, location: &str, gaps: &mut Vec<Gap>) -> Option<i64> {
    if ts.is_none() {
        gap(gaps, family, location, "missing or invalid timestamp");
    }
    ts
}

/// Keep the first event for each source key, preserving input order.
pub fn dedup(mut events: Vec<Event>) -> Vec<Event> {
    let mut seen = HashSet::new();
    events.retain(|e| seen.insert(e.source.key.clone()));
    events
}

pub fn collect(inputs: &Inputs) -> Res<Collected> {
    if inputs.since_days == 0 {
        return Err(Failure::new(
            "invalid_argument",
            "since_days must be greater than zero",
        ));
    }
    let start = inputs
        .now_ms
        .checked_sub(i64::from(inputs.since_days) * usage::DAY_MS)
        .ok_or_else(|| Failure::new("invalid_argument", "period start is out of range"))?;
    let inside = |ts: i64| (start..=inputs.now_ms).contains(&ts);
    let mut out = Collected {
        events: vec![],
        gaps: vec![],
        sources: vec![],
    };
    let locs = usage::Locations {
        home: inputs.home.clone(),
        projects: inputs.projects.clone(),
    };
    let mut usage_samples = 0;
    match usage::period(&locs, inputs.now_ms, inputs.since_days) {
        Ok(period) => {
            usage_samples = period.history.len() as u64;
            if period.undated_calls > 0 {
                gap(
                    &mut out.gaps,
                    "usage",
                    &inputs.projects.display().to_string(),
                    format!("{} undated calls", period.undated_calls),
                );
            }
            if period.skipped_lines > 0 {
                gap(
                    &mut out.gaps,
                    "usage",
                    &inputs.projects.display().to_string(),
                    format!("{} unparseable lines", period.skipped_lines),
                );
            }
            for run in period.runs {
                let file = if run.ledger.source == "transcript" {
                    let direct = run.cwd.as_ref().map(|cwd| {
                        inputs
                            .projects
                            .join(usage::transcript_dir_name(cwd))
                            .join(format!("{}.jsonl", run.session))
                    });
                    direct
                        .filter(|p| p.is_file())
                        .or_else(|| {
                            paths(&inputs.projects, &mut out.gaps, "usage")
                                .into_iter()
                                .map(|p| p.join(format!("{}.jsonl", run.session)))
                                .find(|p| p.is_file())
                        })
                        .unwrap_or_else(|| inputs.projects.clone())
                } else {
                    paths(&inputs.home.join("logs"), &mut out.gaps, "usage")
                        .into_iter()
                        .find(|p| log_has_session(p, &run.session))
                        .unwrap_or_else(|| inputs.home.join("logs"))
                };
                for w in &run.ledger.warnings {
                    let (kind, agent, instance, count, detail, cache_cost) = match w {
                        Warning::ContextHigh {
                            agent_type,
                            instance,
                            model,
                            peak_context,
                        } => (
                            "context-high",
                            agent_type,
                            instance,
                            1,
                            format!("{model}: peak context {peak_context}"),
                            None,
                        ),
                        Warning::CacheRewritesAfterGap {
                            agent_type,
                            instance,
                            count,
                            tokens,
                            usd,
                        } => (
                            "cache-gap",
                            agent_type,
                            instance,
                            *count,
                            format!("{tokens} cache-write tokens"),
                            Some(*usd),
                        ),
                        Warning::LargeFileRead {
                            agent_type,
                            instance,
                            path,
                            reads,
                            largest_bytes,
                        } => (
                            "large-read",
                            agent_type,
                            instance,
                            *reads,
                            format!("{path}: largest read {largest_bytes} bytes"),
                            None,
                        ),
                        Warning::CopiedHistoryNotSeparable { file, zeroed_calls } => {
                            gap(
                                &mut out.gaps,
                                "usage",
                                file,
                                format!("{zeroed_calls} copied calls not separable"),
                            );
                            continue;
                        }
                    };
                    let source_file = if run.ledger.source == "transcript" && instance != "main" {
                        let agent_file = file
                            .with_extension("")
                            .join("subagents")
                            .join(format!("agent-{instance}.jsonl"));
                        if agent_file.is_file() {
                            &agent_file
                        } else {
                            &file
                        }
                        .to_path_buf()
                    } else {
                        file.clone()
                    };
                    let tally = run
                        .ledger
                        .instances
                        .iter()
                        .find(|i| &i.id == instance)
                        .map(|i| &i.tally);
                    let mut e = event(
                        format!("usage.{kind}/{}", slug(agent)),
                        detail.clone(),
                        false,
                        source(
                            "usage",
                            source_file.display().to_string(),
                            &[&run.session, kind, instance, &detail],
                            run.cwd.clone(),
                            None,
                            Some(run.session.clone()),
                            run.last_ms,
                        ),
                    );
                    e.count = count;
                    if let Some(t) = tally {
                        e.unpriced = t.unpriced_calls;
                        e.cost_usd = if e.unpriced == 0 {
                            cache_cost.or(t.usd)
                        } else if t.priced_usd > 0.0 {
                            Some(cache_cost.unwrap_or(t.priced_usd))
                        } else {
                            None
                        };
                    }
                    out.events.push(e);
                }
            }
        }
        Err(e) => gap(
            &mut out.gaps,
            "usage",
            &inputs.home.display().to_string(),
            e.message,
        ),
    }
    out.sources.push(status("usage", usage_samples));
    let mut dirs = Vec::new();
    let mut goal_samples = 0;
    for path in paths(&inputs.home.join("goals"), &mut out.gaps, "goal")
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
    {
        goal_samples += 1;
        let loc = path.display().to_string();
        let Some(goal): Option<Goal> = json_file(&path, "goal", &mut out.gaps) else {
            continue;
        };
        let Some(ts) = timestamp(modified(&path), "goal", &loc, &mut out.gaps) else {
            continue;
        };
        if !inside(ts) {
            continue;
        }
        if let Some(cwd) = &goal.cwd {
            let p = PathBuf::from(cwd);
            if p.is_dir() {
                dirs.push(p);
            }
        }
        if ["blocked", "waiting"].contains(&goal.status.as_str()) {
            let detail = [goal.note.as_deref(), goal.reason.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ");
            out.events.push(event(
                format!("goal.{}", goal.status),
                detail,
                true,
                source(
                    "goal",
                    loc,
                    &["goal"],
                    goal.project,
                    None,
                    path.file_stem().map(|s| s.to_string_lossy().into_owned()),
                    Some(ts),
                ),
            ));
        }
    }
    out.sources.push(status("goal", goal_samples));
    let (mut orch_samples, mut review_samples, mut handoff_samples) = (0, 0, 0);
    for store in paths(&inputs.home.join("orchestrate"), &mut out.gaps, "orch")
        .into_iter()
        .filter(|p| p.is_dir())
    {
        let project = store
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for path in paths(&store.join("reports"), &mut out.gaps, "orch") {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let loc = path.display().to_string();
            if let Some(unit) = name.strip_suffix(".evidence.tsv") {
                orch_samples += 1;
                let text = match fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        gap(&mut out.gaps, "orch", &loc, e);
                        continue;
                    }
                };
                let mut lines = text.lines();
                if lines.next() != Some("kind\tcommand\texit_code\tcommit\tts") {
                    gap(&mut out.gaps, "orch", &loc, "invalid evidence header");
                    continue;
                }
                let mut reviews = 0;
                let mut review_ts = None;
                for (i, line) in lines.enumerate() {
                    if line.is_empty() {
                        continue;
                    }
                    let rowloc = format!("{loc}:{}", i + 2);
                    let c: Vec<_> = line.split('\t').collect();
                    if c.len() != 5 {
                        gap(&mut out.gaps, "orch", &rowloc, "malformed evidence row");
                        continue;
                    }
                    let Ok(code) = c[2].parse::<i32>() else {
                        gap(&mut out.gaps, "orch", &rowloc, "invalid exit code");
                        continue;
                    };
                    let Some(ts) = timestamp(parse_ms(c[4]), "orch", &rowloc, &mut out.gaps) else {
                        continue;
                    };
                    if !inside(ts) {
                        continue;
                    }
                    if c[0] == "review" {
                        reviews += 1;
                        review_ts = Some(ts);
                    }
                    if code != 0 && ["run", "verify", "accept", "review"].contains(&c[0]) {
                        out.events.push(event(
                            format!("orch.{}-failed", c[0]),
                            format!("{} (exit {code}; commit {})", c[1], c[3]),
                            false,
                            source(
                                "orch",
                                rowloc,
                                &["failure"],
                                Some(project.clone()),
                                Some(unit.into()),
                                None,
                                Some(ts),
                            ),
                        ));
                    }
                }
                if reviews >= 2 {
                    let mut e = event(
                        "orch.re-review".into(),
                        format!("{reviews} review rows"),
                        false,
                        source(
                            "orch",
                            loc,
                            &["re-review"],
                            Some(project.clone()),
                            Some(unit.into()),
                            None,
                            review_ts,
                        ),
                    );
                    e.count = 1;
                    out.events.push(e);
                }
            } else if let Some(unit) = name.strip_suffix(".verdict.json") {
                review_samples += 1;
                let Some(v): Option<serde_json::Value> = json_file(&path, "review", &mut out.gaps)
                else {
                    continue;
                };
                let Some(findings) = v.get("findings").and_then(|v| v.as_array()) else {
                    gap(&mut out.gaps, "review", &loc, "missing findings array");
                    continue;
                };
                let Some(ts) = timestamp(modified(&path), "review", &loc, &mut out.gaps) else {
                    continue;
                };
                if !inside(ts) {
                    continue;
                }
                for (i, f) in findings.iter().enumerate() {
                    let Some(severity) = f
                        .get("severity")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    else {
                        gap(
                            &mut out.gaps,
                            "review",
                            &loc,
                            format!("finding {i} missing severity"),
                        );
                        continue;
                    };
                    out.events.push(event(
                        format!("review.finding/{}", slug(severity)),
                        f.to_string(),
                        true,
                        source(
                            "review",
                            loc.clone(),
                            &["finding", &i.to_string()],
                            Some(project.clone()),
                            Some(unit.into()),
                            None,
                            Some(ts),
                        ),
                    ));
                }
            }
        }
        let handoffs = store.join("handoffs");
        let wish = handoffs.join("wish.json");
        let mut files = if wish.exists() { vec![wish] } else { vec![] };
        files.extend(
            paths(&handoffs.join("module"), &mut out.gaps, "handoff")
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "json")),
        );
        for path in files {
            handoff_samples += 1;
            let loc = path.display().to_string();
            let Some(h): Option<Handoff> = json_file(&path, "handoff", &mut out.gaps) else {
                continue;
            };
            let Some(ts) = timestamp(parse_ms(&h.ts), "handoff", &loc, &mut out.gaps) else {
                continue;
            };
            if !inside(ts) {
                continue;
            }
            out.events.push(event(
                format!("handoff.{}", if h.taken { "taken" } else { "pending" }),
                format!("next: {}; stuck: {}", h.next, h.stuck),
                true,
                source(
                    "handoff",
                    loc.clone(),
                    &["handoff"],
                    Some(project.clone()),
                    h.module.clone(),
                    None,
                    Some(ts),
                ),
            ));
            if h.stalled > 0 {
                let mut e = event(
                    format!(
                        "handoff.stall/{}",
                        slug(h.module.as_deref().unwrap_or("wish"))
                    ),
                    h.stuck,
                    true,
                    source(
                        "handoff",
                        loc,
                        &["stall"],
                        Some(project.clone()),
                        h.module,
                        None,
                        Some(ts),
                    ),
                );
                e.count = u64::from(h.stalled);
                out.events.push(e);
            }
        }
    }
    out.sources.push(status("orch", orch_samples));
    out.sources.push(status("handoff", handoff_samples));
    out.sources.push(status("review", review_samples));
    let hook = inputs.home.join("hook/refusals.jsonl");
    let mut hook_status = status("hook", 0);
    if !hook.exists() {
        hook_status.state = "not logged".into();
    } else {
        match fs::read_to_string(&hook) {
            Ok(text) => {
                for (i, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    hook_status.samples += 1;
                    let loc = format!("{}:{}", hook.display(), i + 1);
                    let v: serde_json::Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(e) => {
                            gap(&mut out.gaps, "hook", &loc, e);
                            continue;
                        }
                    };
                    let Some(reason) = v.get("reason").and_then(|v| v.as_str()) else {
                        gap(&mut out.gaps, "hook", &loc, "missing reason");
                        continue;
                    };
                    let Some(ts) = timestamp(
                        v.get("ts").and_then(|v| v.as_str()).and_then(parse_ms),
                        "hook",
                        &loc,
                        &mut out.gaps,
                    ) else {
                        continue;
                    };
                    if inside(ts) {
                        out.events.push(event(
                            "hook.refusal".into(),
                            reason.into(),
                            false,
                            source(
                                "hook",
                                loc,
                                &["refusal"],
                                None,
                                None,
                                v.get("session").and_then(|v| v.as_str()).map(str::to_owned),
                                Some(ts),
                            ),
                        ));
                    }
                }
            }
            Err(e) => gap(&mut out.gaps, "hook", &hook.display().to_string(), e),
        }
        if hook_status.samples > 0 {
            hook_status.state = "observed".into();
        }
    }
    out.sources.push(hook_status);
    match super::lint::collect_lint(&inputs.home, &dirs) {
        Ok((events, status)) => {
            out.events.extend(events);
            out.sources.push(status);
        }
        Err(e) => {
            gap(
                &mut out.gaps,
                "lint",
                &inputs.home.join("memory/memory.db").display().to_string(),
                e.message,
            );
            out.sources.push(status("lint", 0));
        }
    }
    out.events = dedup(out.events);
    Ok(out)
}
