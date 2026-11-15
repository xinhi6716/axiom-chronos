use crate::{
    base,
    desktop::{
        i18n::Lang,
        ux::seams::{GhOutcome, GhRunner},
    },
};
use std::{process::Stdio, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::desktop) struct IssueRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum ImportError {
    InvalidReference,
    GhMissing,
    NotLoggedIn,
    NotFound,
    TimedOut,
    BadFormat,
    Failed(String),
}
impl ImportError {
    pub(in crate::desktop) fn message(&self, lang: Lang) -> String {
        let (en, zh) = match self {
            Self::InvalidReference => ("Invalid issue reference.", "議題參照格式無效。"),
            Self::GhMissing => ("GitHub CLI is not installed.", "尚未安裝 GitHub CLI。"),
            Self::NotLoggedIn => ("Sign in with gh auth login.", "請使用 gh auth login 登入。"),
            Self::NotFound => ("Issue not found.", "找不到議題。"),
            Self::TimedOut => ("Import timed out.", "匯入逾時。"),
            Self::BadFormat => (
                "GitHub returned an invalid issue.",
                "GitHub 回傳的議題格式無效。",
            ),
            Self::Failed(_) => ("Import failed", "匯入失敗"),
        };
        let label = if lang == Lang::ZhTw { zh } else { en };
        match self {
            Self::Failed(detail) => format!("{label}: {detail}"),
            _ => label.into(),
        }
    }
}

pub(in crate::desktop) fn parse_reference(text: &str) -> Result<IssueRef, ImportError> {
    let text = text.trim();
    let parts = if let Some(path) = text.strip_prefix("https://github.com/") {
        let mut p = path.split('/');
        match (p.next(), p.next(), p.next(), p.next(), p.next()) {
            (Some(o), Some(r), Some("issues"), Some(n), None) => Some((o, r, n)),
            _ => None,
        }
    } else {
        text.split_once('#')
            .and_then(|(path, n)| path.split_once('/').map(|(o, r)| (o, r, n)))
    };
    let (owner, repo, number) = parts.ok_or(ImportError::InvalidReference)?;
    let valid = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    };
    if !valid(owner)
        || !valid(repo)
        || number.is_empty()
        || !number.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(ImportError::InvalidReference);
    }
    let number = number
        .parse::<u64>()
        .map_err(|_| ImportError::InvalidReference)?;
    if number == 0 {
        return Err(ImportError::InvalidReference);
    }
    Ok(IssueRef {
        owner: owner.into(),
        repo: repo.into(),
        number,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::desktop) struct IssueDraft {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub labels: Vec<String>,
}

pub(in crate::desktop) fn import(
    gh: &dyn GhRunner,
    r: &IssueRef,
    timeout: Duration,
) -> Result<IssueDraft, ImportError> {
    let args = vec![
        "issue".into(),
        "view".into(),
        r.number.to_string(),
        "--repo".into(),
        format!("{}/{}", r.owner, r.repo),
        "--json".into(),
        "number,title,body,url,labels".into(),
    ];
    let stdout = match gh.run(&args, timeout) {
        GhOutcome::Missing => return Err(ImportError::GhMissing),
        GhOutcome::TimedOut => return Err(ImportError::TimedOut),
        GhOutcome::Exit {
            code: 0, stdout, ..
        } => stdout,
        GhOutcome::Exit { stderr, .. } => {
            let lower = stderr.to_ascii_lowercase();
            return Err(if lower.contains("gh auth login") {
                ImportError::NotLoggedIn
            } else if lower.contains("could not resolve") || lower.contains("not found") {
                ImportError::NotFound
            } else {
                ImportError::Failed(stderr)
            });
        }
    };
    let v: serde_json::Value = serde_json::from_str(&stdout).map_err(|_| ImportError::BadFormat)?;
    let parse = || -> Option<IssueDraft> {
        Some(IssueDraft {
            number: v.get("number")?.as_u64()?,
            title: v.get("title")?.as_str()?.into(),
            body: v.get("body")?.as_str()?.into(),
            url: v.get("url")?.as_str()?.into(),
            labels: v
                .get("labels")?
                .as_array()?
                .iter()
                .map(|x| x.get("name")?.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        })
    };
    parse().ok_or(ImportError::BadFormat)
}

pub(in crate::desktop) fn draft_text(d: &IssueDraft, lang: Lang) -> String {
    let source = if lang == Lang::ZhTw {
        "來源："
    } else {
        "Source: "
    };
    format!("{}\n\n{}\n\n{source}{}", d.title, d.body, d.url)
}

pub(in crate::desktop) struct RealGh;
impl GhRunner for RealGh {
    fn run(&self, args: &[String], timeout: Duration) -> GhOutcome {
        let mut command = base::proc::cmd("gh");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = match command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return GhOutcome::Missing,
        };
        match base::proc::capture(child, timeout) {
            Ok(result) if result.timed_out => GhOutcome::TimedOut,
            Ok(result) => GhOutcome::Exit {
                code: result.exit_code,
                stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            },
            Err(e) => GhOutcome::Exit {
                code: -1,
                stdout: String::new(),
                stderr: e.message,
            },
        }
    }
}
