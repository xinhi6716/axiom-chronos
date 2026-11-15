use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Issue,
    PullRequest,
    CiFailure,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub id: String,
    pub repo: String,
    pub title: String,
    pub url: Option<String>,
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}

pub fn from_github(event_name: &str, payload: &Value) -> Res<Event> {
    let repo = payload
        .pointer("/repository/full_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("missing repository"))?;
    let (kind, object, id) = match event_name {
        "issues" | "pull_request" => {
            let key = if event_name == "issues" {
                "issue"
            } else {
                "pull_request"
            };
            let object = &payload[key];
            let number = payload
                .get("number")
                .or_else(|| object.get("number"))
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("missing number"))?;
            let action = payload
                .get("action")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("missing action"))?;
            let kind = if event_name == "issues" {
                EventKind::Issue
            } else {
                EventKind::PullRequest
            };
            (kind, object, format!("{repo}#{number}:{action}"))
        }
        "check_run" | "workflow_run" => {
            let object = &payload[event_name];
            if object.get("conclusion").and_then(Value::as_str) != Some("failure") {
                return Err(invalid("not a failure"));
            }
            let number = object
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("missing run id"))?;
            (EventKind::CiFailure, object, format!("{repo}:run:{number}"))
        }
        _ => return Err(invalid("unsupported GitHub event")),
    };
    let title = object
        .get("title")
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let url = object
        .get("html_url")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Event {
        kind,
        id,
        repo: repo.to_owned(),
        title,
        url,
    })
}

#[derive(Clone, Debug)]
struct Field {
    values: Vec<bool>,
    wildcard: bool,
}

#[derive(Clone, Debug)]
pub struct Cron {
    minute: Field,
    hour: Field,
    day: Field,
    month: Field,
    weekday: Field,
}

fn field(expr: &str, min: usize, max: usize) -> Res<Field> {
    let mut values = vec![false; max + 1];
    let number = |s: &str| -> Res<usize> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid("invalid cron number"));
        }
        let n = s
            .parse::<usize>()
            .map_err(|_| invalid("invalid cron number"))?;
        if n < min || n > max {
            return Err(invalid("cron value out of range"));
        }
        Ok(n)
    };
    for part in expr.split(',') {
        if part == "*" {
            for value in &mut values[min..=max] {
                *value = true;
            }
        } else if let Some(step) = part.strip_prefix("*/") {
            if step.is_empty() || !step.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid("invalid cron step"));
            }
            let step = step
                .parse::<usize>()
                .map_err(|_| invalid("invalid cron step"))?;
            if step == 0 {
                return Err(invalid("invalid cron step"));
            }
            for n in (min..=max).step_by(step) {
                values[n] = true;
            }
        } else if let Some((start, end)) = part.split_once('-') {
            let start = number(start)?;
            let end = number(end)?;
            if start > end {
                return Err(invalid("invalid cron range"));
            }
            for value in &mut values[start..=end] {
                *value = true;
            }
        } else {
            values[number(part)?] = true;
        }
    }
    Ok(Field {
        values,
        wildcard: expr.starts_with('*'),
    })
}

pub fn parse_cron(expr: &str) -> Res<Cron> {
    let parts: Vec<_> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(invalid("cron requires five fields"));
    }
    Ok(Cron {
        minute: field(parts[0], 0, 59)?,
        hour: field(parts[1], 0, 23)?,
        day: field(parts[2], 1, 31)?,
        month: field(parts[3], 1, 12)?,
        weekday: field(parts[4], 0, 6)?,
    })
}

impl Cron {
    pub fn due(&self, now_unix: i64) -> bool {
        let days = now_unix.div_euclid(86_400);
        let seconds = now_unix.rem_euclid(86_400);
        // Proleptic Gregorian calendar, with 1970-01-01 at day zero.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as usize;
        let month = (mp + if mp < 10 { 3 } else { -9 }) as usize;
        let weekday = (days + 4).rem_euclid(7) as usize;
        let day_match = self.day.values[day];
        let weekday_match = self.weekday.values[weekday];
        let date_match = if self.day.wildcard || self.weekday.wildcard {
            day_match && weekday_match
        } else {
            day_match || weekday_match
        };
        self.minute.values[(seconds / 60 % 60) as usize]
            && self.hour.values[(seconds / 3600) as usize]
            && self.month.values[month]
            && date_match
    }
}
