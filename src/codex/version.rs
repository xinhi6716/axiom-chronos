use crate::base::{proc, Failure, Res};
use serde::Serialize;
use std::{fmt, process::Stdio, time::Duration};

pub const PROTOCOL_MISMATCH: &str = "codex_protocol";
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    // Last, so that a pre-release (false) orders below its release (true).
    is_release: bool,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            is_release: true,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub struct TestedVersions {
    pub minimum: Version,
    pub last_tested: Version,
}

// Raise `last_tested` only after running Chronos on that release. 0.159.3 is the only release with
// any record of a run so far, so it is also the floor until an older one is actually tried.
pub const TESTED_CODEX: TestedVersions = TestedVersions {
    minimum: Version::new(0, 159, 3),
    last_tested: Version::new(0, 159, 3),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Fail,
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionCheck {
    pub verdict: Verdict,
    pub installed: Option<String>,
    pub minimum: String,
    pub last_tested: String,
    pub message: String,
}

pub struct VersionProbe {
    pub exit_code: i32,
    pub last_line: Option<String>,
    pub check: VersionCheck,
}

fn finding(
    verdict: Verdict,
    installed: Option<String>,
    message: String,
    tested: &TestedVersions,
) -> VersionCheck {
    VersionCheck {
        verdict,
        installed,
        minimum: tested.minimum.to_string(),
        last_tested: tested.last_tested.to_string(),
        message,
    }
}

fn parse_line(line: &str) -> Option<(String, Version)> {
    let mut words = line.split_whitespace();
    if words.next()? != "codex-cli" {
        return None;
    }
    let text = words.next()?;
    if words.next().is_some() {
        return None;
    }
    let core_end = text.find(['-', '+']).unwrap_or(text.len());
    let mut parts = text[..core_end].split('.').map(str::parse::<u64>);
    let (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let version = Version {
        major,
        minor,
        patch,
        is_release: !text[core_end..].starts_with('-'),
    };
    Some((text.to_owned(), version))
}

pub fn parse_version(output: &str) -> Option<(String, Version)> {
    output.lines().rev().find_map(parse_line)
}

fn tail(text: &str) -> &str {
    let text = text.trim();
    let start = text.char_indices().rev().nth(199).map_or(0, |(i, _)| i);
    &text[start..]
}

pub fn assess_version(exit_code: i32, output: &str, tested: &TestedVersions) -> VersionCheck {
    let (minimum, last_tested) = (&tested.minimum, &tested.last_tested);
    if exit_code != 0 {
        let message = format!("`codex --version` exited with {exit_code}: {}", tail(output));
        return finding(Verdict::Fail, None, message, tested);
    }
    let Some((installed, version)) = parse_version(output) else {
        let message = format!(
            "cannot read a version from `codex --version` output {:?}; Chronos needs codex-cli {minimum} or newer",
            tail(output)
        );
        return finding(Verdict::Fail, None, message, tested);
    };
    let (verdict, message) = if version < *minimum {
        (
            Verdict::Fail,
            format!("codex-cli {installed} is older than {minimum}, the oldest version Chronos supports (last tested {last_tested}); update Codex"),
        )
    } else if version > *last_tested {
        (
            Verdict::Warn,
            format!("codex-cli {installed} is newer than {last_tested}, the last version tested with Chronos; runs are not blocked, but an app-server protocol change could break them"),
        )
    } else {
        (
            Verdict::Ok,
            format!("codex-cli {installed}, tested with Chronos"),
        )
    };
    finding(verdict, Some(installed), message, tested)
}

pub fn probe_version(command: &[String]) -> Res<VersionProbe> {
    probe_version_within(command, PROBE_TIMEOUT)
}

#[doc(hidden)]
pub fn probe_version_within(command: &[String], timeout: Duration) -> Res<VersionProbe> {
    let failed = |message: String| Failure::new("probe_failed", message);
    let child = proc::command(command)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            failed(format!(
                "{} failed: {e}",
                command.first().map_or("codex", String::as_str)
            ))
        })?;
    let run = proc::capture(child, timeout).map_err(|e| failed(e.message))?;
    if run.timed_out {
        return Err(failed(format!(
            "`codex --version` did not answer within {timeout:?}"
        )));
    }
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    let (first, second) = if run.exit_code == 0 {
        (stdout, stderr)
    } else {
        (stderr, stdout)
    };
    let output = if first.trim().is_empty() { second } else { first };
    Ok(VersionProbe {
        exit_code: run.exit_code,
        last_line: output.trim().lines().last().map(str::to_owned),
        check: assess_version(run.exit_code, &output, &TESTED_CODEX),
    })
}

pub fn check_installed_version(command: &[String]) -> VersionCheck {
    match probe_version(command) {
        Ok(probe) => probe.check,
        Err(e) => finding(Verdict::Fail, None, e.message, &TESTED_CODEX),
    }
}

pub(crate) fn suggests_mismatch(code: Option<i64>, message: &str) -> bool {
    match code {
        Some(-32700 | -32601 | -32602) => true,
        // Codex 0.159.3 answers unknown methods and malformed params with -32600 and this prefix,
        // and uses -32600 without it for ordinary errors such as "thread not found".
        Some(-32600) => message.starts_with("Invalid request:"),
        _ => false,
    }
}

pub(crate) fn mismatch_failure(command: &[String], detail: &str) -> Failure {
    let check = check_installed_version(command);
    let installed = check.installed.map_or_else(
        || "installed version unknown".to_owned(),
        |v| format!("installed codex-cli {v}"),
    );
    Failure::new(
        PROTOCOL_MISMATCH,
        format!(
            "Codex app-server protocol mismatch suspected ({installed}, last tested with Chronos {}); run `axiom doctor`. Raw error: {detail:.200}",
            check.last_tested
        ),
    )
}

pub(crate) fn note_untested_version(command: &[String], message: &str) -> String {
    let check = check_installed_version(command);
    if check.verdict == Verdict::Ok {
        message.to_owned()
    } else {
        format!("{message}; {}", check.message)
    }
}
