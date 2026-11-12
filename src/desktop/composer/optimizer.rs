use crate::{
    base::{self, Failure, Res},
    desktop::{
        i18n::Lang,
        ux::seams::{OptimizerRunner, Proposal},
    },
};
use std::{io::Write, process::Stdio, time::Duration};

pub(in crate::desktop) enum OptimizerState {
    Idle,
    Running,
    Proposed {
        original: String,
        proposal: Proposal,
        answers: Vec<String>,
    },
    Failed {
        original: String,
        timeout: bool,
        message: String,
    },
}

pub(in crate::desktop) fn compose(
    original: &str,
    p: &Proposal,
    answers: &[String],
    lang: Lang,
) -> String {
    let labels = match lang {
        Lang::En => [
            "Goal",
            "Scope",
            "Limits",
            "Acceptance",
            "Answered ambiguities",
            "Original wish",
        ],
        Lang::ZhTw => ["目標", "範圍", "限制", "驗收", "已回答的疑問", "原始願望"],
    };
    let answered = answers
        .iter()
        .enumerate()
        .map(|(i, a)| match p.ambiguities.get(i) {
            Some(q) => format!("{q}\n{a}"),
            None => a.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\n{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}\n\n{}\n{}",
        labels[0],
        p.goal,
        labels[1],
        p.scope,
        labels[2],
        p.limits,
        labels[3],
        p.acceptance,
        labels[4],
        answered,
        labels[5],
        original
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum DiffKind {
    Same,
    Added,
    Removed,
}

pub(in crate::desktop) fn diff(original: &str, proposed: &str) -> Vec<(DiffKind, String)> {
    let a: Vec<_> = original.lines().collect();
    let b: Vec<_> = proposed.lines().collect();
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                1 + lcs[i + 1][j + 1]
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push((DiffKind::Same, a[i].to_owned()));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push((DiffKind::Removed, a[i].to_owned()));
            i += 1;
        } else {
            out.push((DiffKind::Added, b[j].to_owned()));
            j += 1;
        }
    }
    out
}

pub(in crate::desktop) const OPTIMIZE_TIMEOUT: Duration = Duration::from_secs(90);
pub(in crate::desktop) fn claude_args() -> &'static [&'static str] {
    &[
        "-p",
        "--model",
        "claude-sonnet-5-5",
        "--tools",
        "",
        "--strict-mcp-config",
        "--setting-sources",
        "local",
        "--output-format",
        "text",
    ]
}

pub(in crate::desktop) struct ClaudeOptimizer;
impl OptimizerRunner for ClaudeOptimizer {
    fn propose(&self, original: &str, lang: Lang) -> Res<Proposal> {
        let mut command = base::proc::cmd("claude");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .args(claude_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Failure::new("process", e.to_string()))?;
        let language = if lang == Lang::ZhTw {
            "Traditional Chinese"
        } else {
            "English"
        };
        let input = format!("Improve the wish in {language}. Reply only with a JSON object containing string fields goal, scope, limits, acceptance and an array of strings ambiguities. Do not invent requirements.\nWish:\n{original}");
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Failure::new("process", "missing stdin"))?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
        let result = base::proc::capture(child, OPTIMIZE_TIMEOUT)?;
        if result.timed_out {
            return Err(Failure::new("timeout", "Optimizer timed out"));
        }
        if result.exit_code != 0 {
            return Err(Failure::new(
                "process",
                String::from_utf8_lossy(&result.stderr).into_owned(),
            ));
        }
        let v: serde_json::Value = serde_json::from_slice(&result.stdout)
            .map_err(|e| Failure::new("bad_format", e.to_string()))?;
        let parse = || -> Option<Proposal> {
            Some(Proposal {
                goal: v.get("goal")?.as_str()?.into(),
                scope: v.get("scope")?.as_str()?.into(),
                limits: v.get("limits")?.as_str()?.into(),
                acceptance: v.get("acceptance")?.as_str()?.into(),
                ambiguities: v
                    .get("ambiguities")?
                    .as_array()?
                    .iter()
                    .map(|x| x.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            })
        };
        parse().ok_or_else(|| Failure::new("bad_format", "Invalid proposal JSON"))
    }
}
