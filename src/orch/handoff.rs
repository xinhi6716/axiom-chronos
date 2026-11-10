//! A dispatcher's handoff: a short note that lets a fresh dispatcher carry on a module.
//!
//! Units, claims and evidence already live in the store; the note adds only what they cannot
//! show: what was in flight, why something is stuck, what was decided and the next step.

use super::*;
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Each field's limit in characters; a longer value is refused, never cut.
pub const HANDOFF_FIELDS: [(&str, usize); 4] = [
    ("--next", 400),
    ("--in-flight", 600),
    ("--stuck", 600),
    ("--decisions", 800),
];
/// Upper bound of the resume text: the handoff, then the digest cut to fit.
pub const MAX_RESUME: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Handoff {
    pub module: Option<String>,
    pub ts: String,
    #[serde(default)]
    pub repo: Option<String>,
    pub next: String,
    #[serde(default)]
    pub in_flight: String,
    #[serde(default)]
    pub stuck: String,
    #[serde(default)]
    pub decisions: String,
    /// Set once a fresh dispatcher was started with this note.
    #[serde(default)]
    pub taken: bool,
    /// Consecutive handoffs of this module with no unit progress in the store since the one before.
    #[serde(default)]
    pub stalled: u32,
    /// The module's units (id and state) when this note was written; absent in older stores.
    #[serde(default)]
    pub snapshot: Option<String>,
}

/// Refuse instructions to evade checks or misrepresent their outcome, not statements of fact.
pub fn note_refusal(text: &str) -> Option<String> {
    struct Patterns {
        verbs: regex::Regex,
        object: regex::Regex,
        outcome: regex::Regex,
        as_outcome: regex::Regex,
        claim_instruction: regex::Regex,
        whether: regex::Regex,
        negation: regex::Regex,
    }
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    let p = PATTERNS.get_or_init(|| Patterns {
        verbs: regex::Regex::new(concat!(
            r"(?i)(?P<direct>不要回報|不要汇报|不要報告|不要报告)",
            r"|(?P<checks>\b(?:skip|omit|bypass|disable|weaken|loosen|hide|conceal|suppress)\b",
            r"|\b(?:don['’]t|do[ \t]+not|never)[ \t]+(?:mention|report)\b",
            r"|跳過|跳过|略過|略过|省略|繞過|绕过|禁用|停用|削弱|放寬|放宽|隱藏|隐藏|隱瞞|隐瞒|壓制|压制|不要提及)",
            r"|(?P<as_claim>\b(?:mark|treat)\b)",
            r"|(?P<claim>\b(?:claim|say|report)\b|聲稱|声称|標記|标记|視為|视为)"
        )).unwrap(),
        object: regex::Regex::new(concat!(
            r"(?i)^[^.;!?；。！？]{0,40}?(?P<object>",
            r"\b(?:review\w*|test\w*|ci|acceptance|verif\w*|report\w*|findings?|failures?)\b",
            r"|審查|审查|審核|审核|測試|测试|驗證|验证|回報|汇报|報告|报告|失敗|失败|驗收|验收)"
        )).unwrap(),
        outcome: regex::Regex::new(concat!(
            r"(?i)^[^.;!?；。！？]{0,40}?(?P<object>",
            r"\b(?:passed|verified|green)\b|通過|通过|已驗證|已验证|綠燈|绿灯)"
        )).unwrap(),
        as_outcome: regex::Regex::new(concat!(
            r"(?i)^[^.;!?；。！？]{0,40}?\bas\s+(?P<object>",
            r"passed|verified|green)\b"
        )).unwrap(),
        claim_instruction: regex::Regex::new(
            r"(?i)(?:^|[.;!?；。！？\r\n]|\b(?:please|then|and|to|should|must))\s*$"
        ).unwrap(),
        whether: regex::Regex::new(r"(?i)^\s*(?:whether|if)\b").unwrap(),
        negation: regex::Regex::new(
            r"(?i)(?:\b(?:do[ \t]+not|don['’]t|never|must[ \t]+not|no[ \t]+need[ \t]+to[ \t]+not)|不要|別|别|不可|切勿)$"
        ).unwrap(),
    });

    for captures in p.verbs.captures_iter(text) {
        let verb = captures.get(0).unwrap();
        // Whitespace immediately before distinct verbs is disjoint; the remaining look-behind
        // is bounded, keeping the scan linear even for long input.
        let before = text[..verb.start()].trim_end_matches([' ', '\t']);
        let prefix: String = before
            .chars()
            .rev()
            .take(64)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if p.negation.is_match(&prefix) {
            continue;
        }
        if captures.name("direct").is_some() {
            return Some(format!("matched '{}'", verb.as_str()));
        }
        let tail = &text[verb.end()..];
        if captures.name("checks").is_none()
            && ((!p.claim_instruction.is_match(&prefix) && verb.as_str().is_ascii())
                || p.whether.is_match(tail))
        {
            continue;
        }
        let object_pattern = if captures.name("checks").is_some() {
            &p.object
        } else if captures.name("as_claim").is_some() {
            &p.as_outcome
        } else {
            &p.outcome
        };
        let end = tail.char_indices().nth(128).map_or(tail.len(), |(i, _)| i);
        if let Some(object) = object_pattern.captures(&tail[..end]) {
            return Some(format!(
                "matched '{}' and '{}'",
                verb.as_str(),
                &object["object"]
            ));
        }
    }
    None
}

/// Check fields in the order they are presented to the successor.
pub fn check_note(h: &Handoff) -> Result<(), (&'static str, String)> {
    for (flag, value) in [
        ("--next", &h.next),
        ("--in-flight", &h.in_flight),
        ("--stuck", &h.stuck),
        ("--decisions", &h.decisions),
    ] {
        if let Some(reason) = note_refusal(value) {
            return Err((flag, reason));
        }
    }
    Ok(())
}

pub const STALL_ENV: &str = "AXIOM_HANDOFF_STALLS";
pub const DEFAULT_STALL_LIMIT: u32 = 3;

/// Handoffs in a row without progress after which a module gets no new dispatcher:
/// `AXIOM_HANDOFF_STALLS`, 0 to turn the guard off, 3 when unset.
pub fn stall_limit() -> Option<u32> {
    let raw = std::env::var(STALL_ENV).unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(DEFAULT_STALL_LIMIT);
    }
    match raw.parse::<u32>() {
        Ok(0) => None,
        Ok(n) => Some(n),
        Err(_) => {
            eprintln!("{STALL_ENV}={raw} is not a count; using {DEFAULT_STALL_LIMIT}");
            Some(DEFAULT_STALL_LIMIT)
        }
    }
}

/// The module's units as `id=state` lines, so a new, removed or changed unit shows as a difference.
fn snapshot_of(store: &Path, module: &str) -> Option<String> {
    let mut rows: Vec<String> = read_units(store)
        .ok()?
        .iter()
        .filter(|u| u.track == module)
        .map(|u| format!("{}={}", u.id, u.state))
        .collect();
    rows.sort();
    Some(rows.join(";"))
}

/// The handoff, when its module has reached the stall limit and the store still shows no progress since.
pub fn stalled_out(store: &Path, h: &Handoff) -> Option<String> {
    let module = h.module.as_deref()?;
    let limit = stall_limit()?;
    if h.stalled < limit || h.snapshot.is_none() || snapshot_of(store, module) != h.snapshot {
        return None;
    }
    let units = read_units(store).ok()?;
    let states: Vec<String> = units
        .iter()
        .filter(|u| u.track == module)
        .map(|u| format!("{} {}", u.id, u.state))
        .collect();
    Some(format!(
        "Module {module} was handed off {} times in a row with no unit progress, so no new dispatcher starts on it. Close the goal as blocked with the tool `goal`, status blocked, and this note: stuck: {}; units: {}.",
        h.stalled + 1,
        if h.stuck.is_empty() { "not stated" } else { &h.stuck },
        if states.is_empty() { "none".to_string() } else { states.join(", ") }
    ))
}

fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn path(store: &Path, module: Option<&str>) -> PathBuf {
    let dir = store.join("handoffs");
    match module {
        Some(m) => dir.join("module").join(format!("{m}.json")),
        None => dir.join("wish.json"),
    }
}
pub(crate) fn map_modules(store: &Path) -> Res<Option<Vec<String>>> {
    let file = store.join("modules.md");
    if !file.exists() {
        return Ok(None);
    }
    let map = parse_map(&std::fs::read_to_string(file).map_err(io)?).map_err(invalid)?;
    Ok(Some(map.modules.into_iter().map(|m| m.id).collect()))
}
fn check_module(store: &Path, module: Option<&str>) -> Res<()> {
    let Some(m) = module else { return Ok(()) };
    base::require_slug("module", m)?;
    match map_modules(store)? {
        Some(ids) if !ids.iter().any(|id| id == m) => Err(invalid(format!(
            "module {m} is not in the module map; modules: {}",
            ids.join(", ")
        ))),
        _ => Ok(()),
    }
}

/// `orch handoff [--module <m>] --next <text> [--in-flight <text>] [--stuck <text>] [--decisions <text>] [--repo <dir>]`
pub fn record_handoff(store: &Path, args: &[String]) -> Res<Handoff> {
    let mut module = None;
    let mut repo = None;
    let mut fields: Vec<(&str, String)> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| invalid(format!("{flag} requires a value")))?
            .clone();
        if fields.iter().any(|(f, _)| *f == flag)
            || (flag == "--module" && module.is_some())
            || (flag == "--repo" && repo.is_some())
        {
            return Err(invalid(format!("{flag} is given twice")));
        }
        match flag {
            "--module" => module = Some(value),
            "--repo" => repo = Some(value),
            _ => {
                let Some((name, limit)) = HANDOFF_FIELDS.iter().find(|(f, _)| *f == flag) else {
                    return Err(invalid(format!(
                        "unknown handoff option {flag}; options: --module, --repo, --next, --in-flight, --stuck, --decisions"
                    )));
                };
                let value = value.trim().to_string();
                let length = value.chars().count();
                if length > *limit {
                    return Err(invalid(format!(
                        "{name} is {length} characters; the limit is {limit}. Shorten it: unit states and reports are already in the store."
                    )));
                }
                fields.push((name, value));
            }
        }
        i += 2;
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(f, _)| *f == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let next = field("--next");
    if next.is_empty() {
        return Err(invalid(
            "--next is required: the step the fresh dispatcher takes first",
        ));
    }
    check_module(store, module.as_deref())?;
    let mut note = Handoff {
        module,
        ts: base::now_iso(),
        repo,
        next,
        in_flight: field("--in-flight"),
        stuck: field("--stuck"),
        decisions: field("--decisions"),
        taken: false,
        stalled: 0,
        snapshot: None,
    };
    if let Err((flag, reason)) = check_note(&note) {
        return Err(invalid(format!(
            "{flag} is refused: {reason}. Write facts only; the store holds units, evidence and reviews."
        )));
    }
    let file = path(store, note.module.as_deref());
    with_lock(store, false, || {
        if let Some(m) = note.module.clone() {
            note.snapshot = snapshot_of(store, &m);
            // Only a note that carries a snapshot too can show that nothing moved.
            if let Some(prev) = read_handoff(store, Some(&m))? {
                if prev.snapshot.is_some() && prev.snapshot == note.snapshot {
                    note.stalled = prev.stalled + 1;
                }
            }
        }
        std::fs::create_dir_all(file.parent().unwrap()).map_err(io)?;
        base::atomic_write(&file, &serde_json::to_vec_pretty(&note).unwrap()).map_err(io)
    })?;
    Ok(note)
}

/// The latest handoff of a module, or of the whole wish when `module` is None.
pub fn read_handoff(store: &Path, module: Option<&str>) -> Res<Option<Handoff>> {
    if let Some(m) = module {
        base::require_slug("module", m)?;
    }
    match std::fs::read(path(store, module)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            invalid(format!(
                "handoff of {} is unreadable: {e}",
                module.unwrap_or("the wish")
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}

/// Handoffs no fresh dispatcher has taken yet, the whole wish's first.
pub fn pending_handoffs(store: &Path) -> Res<Vec<Handoff>> {
    let mut out: Vec<Handoff> = read_handoff(store, None)?.into_iter().collect();
    let dir = store.join("handoffs").join("module");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(keep_pending(out)),
        Err(e) => return Err(io(e)),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .filter(|m| base::require_slug("module", m).is_ok())
        .collect();
    names.sort();
    for m in names {
        out.extend(read_handoff(store, Some(&m))?);
    }
    Ok(keep_pending(out))
}
fn keep_pending(all: Vec<Handoff>) -> Vec<Handoff> {
    all.into_iter().filter(|h| !h.taken).collect()
}

/// Marks a handoff as taken by a fresh dispatcher.
pub fn take_handoff(store: &Path, module: Option<&str>) -> Res<()> {
    take(store, module, None)
}

/// Marks `h` taken once a dispatcher was seen running with it; a newer note of the module stays pending.
pub fn take_delivered(store: &Path, h: &Handoff) -> Res<()> {
    take(store, h.module.as_deref(), Some(&h.ts))
}

fn take(store: &Path, module: Option<&str>, ts: Option<&str>) -> Res<()> {
    with_lock(store, false, || {
        let Some(mut note) = read_handoff(store, module)? else {
            return Ok(());
        };
        if note.taken || ts.is_some_and(|ts| ts != note.ts) {
            return Ok(());
        }
        note.taken = true;
        base::atomic_write(
            &path(store, module),
            &serde_json::to_vec_pretty(&note).unwrap(),
        )
        .map_err(io)
    })
}

/// The first line of a handoff's text without its pending mark; it names the note by module and time.
pub fn handoff_header(h: &Handoff) -> String {
    format!(
        "HANDOFF {} at {}",
        h.module
            .as_deref()
            .map_or("for the whole wish".to_string(), |m| format!(
                "for module {m}"
            )),
        h.ts
    )
}

pub fn handoff_text(h: &Handoff) -> String {
    if let Err((flag, _)) = check_note(h) {
        let label = match flag {
            "--next" => "next",
            "--in-flight" => "in flight",
            "--stuck" => "stuck",
            "--decisions" => "decisions",
            _ => unreachable!(),
        };
        return format!(
            "{}{}\nnote withheld: its {label} field tells the successor to skip, weaken or hide checks; work from the store and the digest.\n",
            handoff_header(h),
            if h.taken { "" } else { " (pending)" },
        );
    }
    let mut text = format!(
        "{}{}\nnext: {}\n",
        handoff_header(h),
        if h.taken { "" } else { " (pending)" },
        h.next
    );
    for (label, value) in [
        ("in flight", &h.in_flight),
        ("stuck", &h.stuck),
        ("decisions", &h.decisions),
    ] {
        if !value.is_empty() {
            text.push_str(&format!("{label}: {value}\n"));
        }
    }
    text
}

/// The latest handoff (all pending ones when `module` is None) and the digest, within MAX_RESUME.
pub fn resume_text(store: &Path, repo: Option<&Path>, module: Option<&str>) -> Res<String> {
    let notes: Vec<Handoff> = match module {
        Some(m) => read_handoff(store, Some(m))?.into_iter().collect(),
        None => {
            let mut notes: Vec<Handoff> = read_handoff(store, None)?.into_iter().collect();
            notes.extend(
                pending_handoffs(store)?
                    .into_iter()
                    .filter(|h| h.module.is_some()),
            );
            notes
        }
    };
    let mut out = String::new();
    if notes.is_empty() {
        out.push_str("HANDOFF none recorded\n");
    }
    for note in &notes {
        out.push_str(&handoff_text(note));
    }
    let repo = repo.map(Path::to_path_buf).or_else(|| {
        notes
            .iter()
            .find_map(|h| h.repo.as_ref().map(PathBuf::from))
    });
    let digest = match repo {
        None => "unavailable: pass --repo <project directory>\n".to_string(),
        Some(repo) => match digest(store, &repo, module) {
            Ok(text) => text,
            Err(e) if e.kind == "not_found" => format!("{}\n", e.message),
            Err(e) => return Err(e),
        },
    };
    out.push_str("DIGEST\n");
    let room = MAX_RESUME.saturating_sub(out.chars().count());
    let mut chars = digest.chars();
    let head: String = chars.by_ref().take(room).collect();
    out.push_str(&head);
    if chars.next().is_some() {
        let marker = "\n... cut; run orch report for the rest\n";
        let keep = MAX_RESUME.saturating_sub(marker.len());
        out = out.chars().take(keep).collect();
        out.push_str(marker);
    }
    Ok(out)
}
