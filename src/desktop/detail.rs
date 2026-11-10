//! The right-hand panel of the Run screen: an overview of the run, one module, or one unit.
use super::i18n::{self, tr};
use super::model::*;
use super::ornament;
use super::theme::{Tokened as _, Tokens};
use gpui_kit::base::SelectableText;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{ErrorKind, Read as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How much of a file is shown. Anything longer is cut and says so.
const SHOWN_BYTES: usize = 200 * 1024;
const RECENT_EVENTS: usize = 8;
/// How long Resume stays disabled after a click if the run does not visibly react.
const RESUME_SETTLE: Duration = Duration::from_secs(8);
const STAGE_NAMES: [&str; 7] = ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report"];
const BRIEF_FIELDS: [&str; 4] = ["GOAL", "SCOPE", "VERIFY", "CONTEXT"];

const TABLE: i18n::Table = &[
    ("Overview", "總覽"),
    ("No wish is open.", "目前沒有開啟的願望。"),
    ("The item you selected is no longer in this run.", "你選的項目已不在這次執行中。"),
    ("Running now", "正在進行"),
    ("Nothing is running right now.", "目前沒有正在進行的工作。"),
    ("Stopped until you respond.", "已暫停，等你回應。"),
    ("Waiting for the background batch to finish.", "正在等背景批次完成。"),
    ("Waiting for the usage limit to reset.", "正在等用量額度恢復。"),
    ("It stopped before finishing. Resume to continue.", "它在完成前就停止了。可以接續。"),
    ("This wish has ended. {done} of {total} units verified.", "這個願望已結束。已驗收 {done} / {total} 個單元。"),
    ("{stage}: in progress", "{stage} 進行中"),
    ("Interjection", "插話"),
    ("Resume now", "立即接續"),
    ("Decisions made for you", "替你做的決定"),
    ("Chronos has not decided anything on your behalf.", "Chronos 還沒有替你做任何決定。"),
    ("Recent events", "最近事件"),
    ("Nothing has been logged yet.", "還沒有任何紀錄。"),
    ("Module", "模組"),
    ("Purpose", "用途"),
    ("Contract", "介面約定"),
    ("No contract was written for this module.", "這個模組沒有寫介面約定。"),
    ("Depends on", "相依模組"),
    ("This module depends on nothing else.", "這個模組不依賴其他模組。"),
    ("Units", "單元"),
    ("This module has no units yet.", "這個模組還沒有單元。"),
    ("{done} of {total} verified", "已驗收 {done} / {total}"),
    ("Not attempted yet", "尚未嘗試"),
    ("1 attempt", "嘗試 1 次"),
    ("{n} attempts", "嘗試 {n} 次"),
    ("Brief", "任務說明"),
    ("Verify", "驗證"),
    ("Review", "審查"),
    ("No brief was saved for this unit.", "這個單元沒有留下任務說明。"),
    ("Not started yet.", "還沒開始。"),
    ("Still being built. The report comes when the work is finished.", "還在施工，完成後才會有報告。"),
    ("No report was left for this unit.", "這個單元沒有留下報告。"),
    ("Verification runs when the work is finished.", "完成施工後才會執行驗證。"),
    ("No verification output was saved.", "沒有留下驗證輸出。"),
    ("Not reviewed yet.", "尚未審查。"),
    ("No review was saved for this unit.", "這個單元沒有留下審查紀錄。"),
    ("Loading…", "讀取中…"),
    ("This file is no longer there.", "這個檔案已不存在。"),
    ("VS Code was not found, so this file was not opened.", "找不到 VS Code，所以沒有開啟這個檔案。"),
    ("This file could not be read.", "無法讀取這個檔案。"),
    ("This file is empty.", "這個檔案是空的。"),
    ("This text is not saved as a file.", "這段文字沒有存成檔案。"),
    ("Copied the part shown", "已複製顯示的部分"),
    ("Copy the part shown", "複製顯示的部分"),
    ("Showing the first {shown} of {total}.", "只顯示前 {shown}，完整內容共 {total}。"),
    ("Open the full file", "開啟完整檔案"),
    ("Verdict", "審查結果"),
    ("No findings.", "沒有發現問題。"),
    ("That file is outside this project, so it was not opened.", "這個檔案在專案資料夾之外，所以沒有開啟。"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocTab {
    Brief,
    Report,
    Verify,
    Review,
}

impl DocTab {
    const ALL: [DocTab; 4] = [DocTab::Brief, DocTab::Report, DocTab::Verify, DocTab::Review];

    fn label(self) -> &'static str {
        match self {
            DocTab::Brief => "Brief",
            DocTab::Report => "Report",
            DocTab::Verify => "Verify",
            DocTab::Review => "Review",
        }
    }

    fn source(self, files: &UnitFiles) -> Option<&TextSource> {
        match self {
            DocTab::Brief => files.brief.as_ref(),
            DocTab::Report => files.report.as_ref(),
            DocTab::Verify => files.verify.as_ref(),
            DocTab::Review => files.review.as_ref(),
        }
    }
}

/// Why a tab has nothing to show, in the unit's current state.
fn empty_reason(tab: DocTab, state: UnitState) -> &'static str {
    use UnitState::*;
    match (tab, state) {
        (DocTab::Brief, _) => "No brief was saved for this unit.",
        (DocTab::Report | DocTab::Verify, Pending) => "Not started yet.",
        (DocTab::Report, Running) => "Still being built. The report comes when the work is finished.",
        (DocTab::Report, _) => "No report was left for this unit.",
        (DocTab::Verify, Running) => "Verification runs when the work is finished.",
        (DocTab::Verify, _) => "No verification output was saved.",
        (DocTab::Review, Pending | Running | Built) => "Not reviewed yet.",
        (DocTab::Review, Verified | Failed) => "No review was saved for this unit.",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Overview,
    Module(usize),
    Unit(usize),
}

/// The pane for a selection. A selection that no longer points at anything shows the Overview.
fn pane_for(run: &Run, selection: &Selection) -> Pane {
    match *selection {
        Selection::Unit(i) if i < run.units.len() => Pane::Unit(i),
        Selection::Module(i) if i < run.modules.len() => Pane::Module(i),
        _ => Pane::Overview,
    }
}

/// The selected item by name, so a refresh that renumbers units can keep it selected.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Anchor {
    session: SharedString,
    selection: Selection,
    id: SharedString,
}

fn anchor_of(run: &Run, selection: &Selection) -> Option<Anchor> {
    let id = match *selection {
        Selection::Unit(i) => run.units.get(i)?.id.clone(),
        Selection::Module(i) => run.modules.get(i)?.id.clone(),
        Selection::None => return None,
    };
    Some(Anchor { session: run.session.clone(), selection: selection.clone(), id })
}

/// Where the selection must move after the model changed, when the item it named has moved or gone.
fn followed(anchor: &Anchor, run: &Run, selection: &Selection) -> Option<Selection> {
    if anchor.session != run.session || anchor.selection != *selection {
        return None;
    }
    if anchor_of(run, selection).is_some_and(|now| now.id == anchor.id) {
        return None;
    }
    Some(match selection {
        Selection::Unit(_) => run
            .units
            .iter()
            .position(|u| u.id == anchor.id)
            .map_or(Selection::None, Selection::Unit),
        Selection::Module(_) => run
            .modules
            .iter()
            .position(|m| m.id == anchor.id)
            .map_or(Selection::None, Selection::Module),
        Selection::None => Selection::None,
    })
}

/// Files the pane reads, so nothing else stays in memory.
fn wanted_paths(run: &Run, pane: Pane) -> Vec<PathBuf> {
    let sources: Vec<&TextSource> = match pane {
        Pane::Overview => Vec::new(),
        Pane::Module(i) => run.modules[i].contract.iter().collect(),
        Pane::Unit(i) => DocTab::ALL.iter().filter_map(|tab| tab.source(&run.units[i].files)).collect(),
    };
    sources
        .into_iter()
        .filter_map(|source| match source {
            TextSource::Path(path) => Some(path.clone()),
            TextSource::Inline(_) => None,
        })
        .collect()
}

fn module_counts(run: &Run, module: &Module) -> (usize, usize) {
    let done = module
        .units
        .iter()
        .filter(|&&u| run.units.get(u).is_some_and(|unit| unit.state == UnitState::Verified))
        .count();
    (done, module.units.len())
}

/// One state standing for a whole module: trouble first, then activity.
fn module_state(run: &Run, module: &Module) -> UnitState {
    let states: Vec<UnitState> = module.units.iter().filter_map(|&u| run.units.get(u)).map(|u| u.state).collect();
    let any = |state| states.contains(&state);
    if any(UnitState::Failed) {
        UnitState::Failed
    } else if any(UnitState::Running) {
        UnitState::Running
    } else if any(UnitState::Built) {
        UnitState::Built
    } else if !states.is_empty() && states.iter().all(|s| *s == UnitState::Verified) {
        UnitState::Verified
    } else {
        UnitState::Pending
    }
}

/// Units that are really moving: a run that stopped for the user is not running anything.
fn running_units(run: &Run) -> Vec<usize> {
    if !matches!(run.status, RunStatus::Active | RunStatus::Waiting | RunStatus::LimitWait) {
        return Vec::new();
    }
    (0..run.units.len()).filter(|&i| run.units[i].state == UnitState::Running).collect()
}

type Stamp = (u64, Option<SystemTime>);

#[derive(Clone, Debug)]
struct FileText {
    text: SharedString,
    /// The file's full size, when only its beginning is shown.
    cut_from: Option<u64>,
    stamp: Stamp,
}

#[derive(Clone, Debug, Default)]
enum Load {
    #[default]
    Reading,
    Text(FileText),
    Missing,
    Unreadable(SharedString),
}

#[derive(Default)]
struct Doc {
    load: Load,
    reading: Option<Task<()>>,
}

#[derive(Debug)]
enum Read {
    Unchanged,
    Text(FileText),
    Missing,
    Unreadable(String),
}

/// Reads the beginning of a file, unless it is unchanged since `known`.
fn read_capped(path: &Path, limit: usize, known: Option<Stamp>) -> Read {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Read::Missing,
        Err(e) => return Read::Unreadable(e.to_string()),
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(e) => return Read::Unreadable(e.to_string()),
    };
    let stamp = (metadata.len(), metadata.modified().ok());
    if known == Some(stamp) {
        return Read::Unchanged;
    }
    let mut bytes = Vec::new();
    if let Err(e) = (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes) {
        return Read::Unreadable(e.to_string());
    }
    let cut = bytes.len() > limit;
    bytes.truncate(limit);
    let text = decode(&bytes, cut);
    Read::Text(FileText {
        text: text.into(),
        cut_from: cut.then_some(metadata.len().max(limit as u64 + 1)),
        stamp,
    })
}

/// The longest prefix that does not end inside a UTF-8 sequence.
fn char_boundary(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(4) {
        let byte = bytes[len - back];
        if byte & 0b1100_0000 != 0b1000_0000 {
            let needed = match byte {
                0xF0..=0xF7 => 4,
                0xE0..=0xEF => 3,
                0xC0..=0xDF => 2,
                _ => 1,
            };
            return if needed > back { len - back } else { len };
        }
    }
    len
}

/// Text for display. A cut lands on a line end when one is near, and never inside a character.
fn decode(bytes: &[u8], cut: bool) -> String {
    let mut end = bytes.len();
    if cut {
        end = char_boundary(bytes);
        if let Some(newline) = bytes[..end].iter().rposition(|b| *b == b'\n') {
            if end - newline <= 1024 {
                end = newline + 1;
            }
        }
    }
    let lossy = String::from_utf8_lossy(&bytes[..end]);
    let text = lossy.strip_prefix('\u{feff}').unwrap_or(&lossy);
    clean(text).into_owned()
}

/// Removes what a terminal would interpret but a text view would draw as noise.
fn clean(text: &str) -> Cow<'_, str> {
    if !text.contains(|c: char| c.is_control() && c != '\n') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' => out.push('\n'),
            '\t' => out.push_str("    "),
            '\u{1b}' if chars.peek() == Some(&'[') => {
                chars.next();
                for code in chars.by_ref() {
                    if ('@'..='~').contains(&code) {
                        break;
                    }
                }
            }
            c if c.is_control() && c != '\n' => {}
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let n = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if n < KB * KB {
        format!("{:.0} KB", n / KB)
    } else if n < KB * KB * KB {
        format!("{:.1} MB", n / KB / KB)
    } else {
        format!("{:.1} GB", n / KB / KB / KB)
    }
}

fn is_web_link(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// A file named in a document: `src/a.rs`, `src/a.rs:12`, `src/a.rs:12:5`, `src/a.rs#L12`, a
/// pytest id `tests/t.py::test_x`, or a Python traceback's `File "a.py", line 12`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cite {
    /// Where the reference is written, in bytes of the text it was found in.
    range: std::ops::Range<usize>,
    path: String,
    line: Option<u32>,
    column: Option<u32>,
}

/// Characters a path reference is made of. Letters outside ASCII are left out on purpose:
/// Chinese prose runs straight into a path without a space.
fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | '_' | '-' | ':' | '#' | '+' | '@' | '~')
}

/// The first number of `12`, `12-20` or `12-L20`.
fn line_number(text: &str) -> Option<u32> {
    let digits: &str = &text[..text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len())];
    let rest = &text[digits.len()..];
    let tail_ok = rest.is_empty()
        || rest.strip_prefix('-').is_some_and(|r| {
            let r = r.strip_prefix('L').unwrap_or(r);
            !r.is_empty() && r.chars().all(|c| c.is_ascii_digit())
        });
    if digits.is_empty() || !tail_ok {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

fn whole_number(text: &str) -> Option<u32> {
    (!text.is_empty() && text.chars().all(|c| c.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

fn drive_prefixed(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || matches!(b[2], b'/' | b'\\'))
}

/// Whether `path` looks like a file: its last part has a short extension with a letter in it.
/// Unless `lenient`, it must also name a folder or a line, so `e.g.`, `v1.2` or a bare `README.md`
/// in a sentence are not taken for references.
fn plausible_path(path: &str, has_line: bool, lenient: bool) -> bool {
    if path.is_empty() || path.contains("://") || path.starts_with(['~', '-', '+']) || path.contains('#') {
        return false;
    }
    let colons = path.matches(':').count();
    if colons > usize::from(drive_prefixed(path)) {
        return false;
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext_ok = (1..=10).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic());
    if stem.is_empty() || stem.ends_with('.') || !ext_ok {
        return false;
    }
    lenient || has_line || path.contains(['/', '\\'])
}

/// Reads one run of path characters as a reference, with its range inside `raw`.
fn parse_cite(raw: &str, lenient: bool) -> Option<Cite> {
    let lead = raw.len() - raw.trim_start_matches([':', '#', '-', '+', '@']).len();
    let body = raw[lead..].trim_end_matches(['.', ':', ',', '#', '-', '+', '@']);
    let mut rest = body;
    if let Some(at) = rest.find("::") {
        rest = &rest[..at];
    }
    let (mut line, mut column) = (None, None);
    if let Some((path, tail)) = rest.rsplit_once("#L") {
        line = Some(line_number(tail)?);
        rest = path;
    } else if let Some((path, tail)) = rest.rsplit_once(':') {
        if let Some(n) = line_number(tail) {
            rest = path;
            line = Some(n);
            if let (Some(col), Some((path, prev))) = (whole_number(tail), rest.rsplit_once(':')) {
                if let Some(n) = whole_number(prev) {
                    rest = path;
                    line = Some(n);
                    column = Some(col);
                }
            }
        }
    }
    plausible_path(rest, line.is_some(), lenient).then(|| Cite {
        range: lead..lead + body.len(),
        path: rest.to_owned(),
        line,
        column,
    })
}

/// `", line 12` right after a quoted path in a Python traceback.
fn traceback_line(after: &str) -> Option<u32> {
    let rest = after.strip_prefix("\", line ")?;
    whole_number(&rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())])
}

/// Every file reference in plain text, in order.
fn find_cites(text: &str) -> Vec<Cite> {
    let mut cites = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !path_char(c) {
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(i, c)) = chars.peek() {
            if !path_char(c) {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        let raw = &text[start..end];
        // A traceback names the file and its line apart: `File "app.py", line 12`.
        let traced = traceback_line(&text[end..]);
        let found = parse_cite(raw, false).or_else(|| traced.and_then(|_| parse_cite(raw, true)));
        if let Some(mut cite) = found {
            let whole = cite.range.end == raw.len();
            cite.range = start + cite.range.start..start + cite.range.end;
            if cite.line.is_none() && whole {
                cite.line = traced;
            }
            cites.push(cite);
        }
    }
    cites
}

/// Why a reference was not opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// It points outside the run's project folder.
    Outside,
    /// The file is not there.
    Missing,
    /// No editor is installed, and the file's type is not safe to hand to the system.
    NoEditor,
}

/// `path` with `.` and `..` worked out, without touching the disk.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Whether `path` is `root` or lies inside it. Windows paths compare without regard to case.
fn within(root: &Path, path: &Path) -> bool {
    let key = |part: std::path::Component| {
        let text = part.as_os_str().to_string_lossy().into_owned();
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    let mut parts = path.components().map(key);
    root.components().map(key).all(|want| parts.next().is_some_and(|have| have == want))
}

/// The file a reference names, if it lies inside `project`. Relative references are read from
/// the project folder. This only looks at the text; `checked_target` also asks the disk.
fn resolve_cite(path: &str, project: &Path) -> Result<PathBuf, Refusal> {
    if !project.is_absolute() || path.starts_with('~') || (!cfg!(windows) && drive_prefixed(path)) {
        return Err(Refusal::Outside);
    }
    let written = if cfg!(windows) { path.to_owned() } else { path.replace('\\', "/") };
    let target = lexical(&project.join(written));
    let root = lexical(project);
    if within(&root, &target) && target != root {
        Ok(target)
    } else {
        Err(Refusal::Outside)
    }
}

/// The reference's file, checked against the disk at the moment it is opened: it must exist,
/// and its real location must still be inside the project, so a link cannot lead out through a
/// symbolic link or junction.
fn checked_target(path: &str, project: &Path) -> Result<PathBuf, Refusal> {
    let target = resolve_cite(path, project)?;
    let real = std::fs::canonicalize(&target).map_err(|_| Refusal::Missing)?;
    let root = std::fs::canonicalize(project).map_err(|_| Refusal::Missing)?;
    if !within(&root, &real) {
        return Err(Refusal::Outside);
    }
    if !real.is_file() {
        return Err(Refusal::Missing);
    }
    Ok(target)
}

const CITE_SCHEME: &str = "chronos-cite:";

/// A link target that carries the reference itself, so a click needs nothing else to open it.
fn cite_url(cite: &Cite) -> String {
    let mut url = CITE_SCHEME.to_owned();
    for b in cite.path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-') {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    if let Some(line) = cite.line {
        url.push_str(&format!("#{line}"));
        if let Some(column) = cite.column {
            url.push_str(&format!(":{column}"));
        }
    }
    url
}

/// The path, line and column a `cite_url` carries.
fn parse_cite_url(url: &str) -> Option<(String, Option<u32>, Option<u32>)> {
    let rest = url.strip_prefix(CITE_SCHEME)?;
    let (encoded, place) = rest.split_once('#').map_or((rest, None), |(p, l)| (p, Some(l)));
    let mut bytes = Vec::with_capacity(encoded.len());
    let raw = encoded.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(bytes).ok().filter(|p| !p.is_empty())?;
    let (line, column) = match place {
        None => (None, None),
        Some(place) => match place.split_once(':') {
            Some((l, c)) => (Some(whole_number(l)?), Some(whole_number(c)?)),
            None => (Some(whole_number(place)?), None),
        },
    };
    Some((path, line, column))
}

/// The arguments that open `target` at its line in VS Code.
fn editor_args(target: &Path, line: Option<u32>, column: Option<u32>) -> Vec<String> {
    let mut place = target.display().to_string();
    if let Some(line) = line {
        place.push_str(&format!(":{line}"));
        if let Some(column) = column {
            place.push_str(&format!(":{column}"));
        }
    }
    vec!["-g".into(), place]
}

fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Plain text with each reference inside the project turned into a link. `escape` makes the
/// rest literal for Markdown.
fn link_text(text: &str, project: &Path, escape: bool, out: &mut String) {
    let plain = |s: &str, out: &mut String| {
        if escape {
            out.push_str(&escape_markdown(s));
        } else {
            out.push_str(s);
        }
    };
    let mut at = 0;
    for cite in find_cites(text) {
        if resolve_cite(&cite.path, project).is_err() {
            continue;
        }
        plain(&text[at..cite.range.start], out);
        out.push_str(&format!("[{}]({})", escape_markdown(&text[cite.range.clone()]), cite_url(&cite)));
        at = cite.range.end;
    }
    plain(&text[at..], out);
}

/// The link for a code span that holds one reference and nothing else, if it lies inside the project.
fn code_span_link(code: &str, project: &Path, lenient: bool) -> Option<String> {
    let trimmed = code.trim();
    let cite = parse_cite(trimmed, lenient).filter(|c| c.range == (0..trimmed.len()))?;
    resolve_cite(&cite.path, project).ok()?;
    Some(cite_url(&cite))
}

/// Markdown with file references inside the project made into links. Fenced code, existing
/// links and HTML are left as written; a code span that is exactly one reference becomes a link.
fn link_markdown(markdown: &str, project: &Path) -> String {
    let mut out = String::with_capacity(markdown.len() + markdown.len() / 8);
    let mut fence: Option<&str> = None;
    for line in markdown.split_inclusive('\n') {
        let opener = line.trim_start();
        let marker = ["```", "~~~"].into_iter().find(|m| opener.starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some(m);
                out.push_str(line);
                continue;
            }
            (Some(open), Some(m)) if open == m => {
                fence = None;
                out.push_str(line);
                continue;
            }
            (Some(_), _) => {
                out.push_str(line);
                continue;
            }
            (None, None) => {}
        }
        link_markdown_line(line, project, &mut out);
    }
    out
}

fn link_markdown_line(line: &str, project: &Path, out: &mut String) {
    let bytes = line.as_bytes();
    let mut plain_from = 0;
    let mut i = 0;
    let flush = |from: usize, to: usize, out: &mut String| link_text(&line[from..to], project, false, out);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                let ticks = bytes[i..].iter().take_while(|b| **b == b'`').count();
                let fence = &line[i..i + ticks];
                let close = line[i + ticks..].find(fence).map(|at| i + ticks + at);
                match close.filter(|&at| bytes.get(at + ticks) != Some(&b'`')) {
                    Some(close) => {
                        flush(plain_from, i, out);
                        let span = &line[i..close + ticks];
                        match code_span_link(&line[i + ticks..close], project, false) {
                            Some(url) => out.push_str(&format!("[{span}]({url})")),
                            None => out.push_str(span),
                        }
                        i = close + ticks;
                        plain_from = i;
                    }
                    None => i += ticks,
                }
            }
            b'[' => {
                // An existing link or image stays as written, text and target alike.
                let start = if i > 0 && bytes[i - 1] == b'!' { i - 1 } else { i };
                let end = line[i..].find("](").and_then(|mid| line[i + mid..].find(')').map(|close| i + mid + close + 1));
                match end {
                    Some(end) => {
                        flush(plain_from, start, out);
                        out.push_str(&line[start..end]);
                        i = end;
                        plain_from = i;
                    }
                    None => i += 1,
                }
            }
            b'<' => match line[i..].find('>') {
                Some(close) if !line[i..i + close].contains(char::is_whitespace) => {
                    flush(plain_from, i, out);
                    out.push_str(&line[i..i + close + 1]);
                    i += close + 1;
                    plain_from = i;
                }
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    flush(plain_from, line.len().min(bytes.len()), out);
}

/// Output text such as a test log, as Markdown that reads exactly as written, line for line,
/// with references inside the project as links.
fn link_plain(text: &str, project: &Path) -> String {
    let mut out = String::with_capacity(text.len() * 5 / 4);
    let lines: Vec<&str> = text.split('\n').collect();
    for (n, line) in lines.iter().enumerate() {
        let body = line.trim_start_matches(' ');
        let indent = line.len() - body.len();
        if body.is_empty() {
            out.push('\u{a0}');
        } else {
            out.extend(std::iter::repeat('\u{a0}').take(indent));
            link_text(body, project, true, &mut out);
        }
        if n + 1 < lines.len() {
            out.push_str("\\\n");
        }
    }
    out
}

/// Whether the system's default app may be asked to open `target` when no editor is available.
/// Only plain text and source types qualify: anything else could be a program or a script that
/// the default handler would run.
fn system_open_allowed(target: &Path) -> bool {
    const TEXT: &[&str] = &[
        "md", "markdown", "txt", "log", "rs", "toml", "lock", "json", "yaml", "yml", "csv", "tsv", "ini", "cfg", "conf",
        "diff", "patch", "css", "c", "h", "cpp", "hpp", "go", "java", "kt", "swift", "tex", "rst",
    ];
    target
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| TEXT.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

/// Opens a followed reference in the editor at its line: VS Code when `code` is on PATH,
/// otherwise the system's default app, for plain text and source files only: a program or script
/// is never handed to the system, which could run it. Anything outside the project, or gone, is
/// not opened and the person is told why.
fn open_cite(url: &str, project: Option<&Path>, window: &mut Window, cx: &mut App) {
    let Some((path, line, column)) = parse_cite_url(url) else {
        return;
    };
    let project = project.map(Path::to_path_buf);
    let outside = tr(cx, TABLE, "That file is outside this project, so it was not opened.");
    let missing = tr(cx, TABLE, "This file is no longer there.");
    let no_editor = tr(cx, TABLE, "VS Code was not found, so this file was not opened.");
    window
        .spawn(cx, async move |cx| {
            let opened = cx
                .background_spawn(async move {
                    let target = checked_target(&path, project.as_deref().ok_or(Refusal::Outside)?)?;
                    let launched = crate::base::on_path("code").is_some_and(|code| {
                        crate::base::proc::cmd(code).args(editor_args(&target, line, column)).spawn().is_ok()
                    });
                    if !launched && !system_open_allowed(&target) {
                        return Err(Refusal::NoEditor);
                    }
                    Ok::<_, Refusal>((target, launched))
                })
                .await;
            let _ = cx.update(|window, cx| match opened {
                Ok((_, true)) => {}
                Ok((target, false)) => cx.open_with_system(&target),
                Err(refusal) => {
                    let message = match refusal {
                        Refusal::Outside => outside,
                        Refusal::Missing => missing,
                        Refusal::NoEditor => no_editor,
                    };
                    window.push_notification(Notification::warning(message), cx);
                }
            });
        })
        .detach();
}

fn brief_field(line: &str) -> Option<(&'static str, &str)> {
    BRIEF_FIELDS.iter().find_map(|name| {
        let rest = line.strip_prefix(name)?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some((*name, rest.trim()))
    })
}

/// `text` as a code span, with enough backticks that any inside it stay text.
fn fenced_inline(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') { " " } else { "" };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

fn fenced(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}")
}

/// A brief as Markdown with its fields as headings, or `None` when it has no fields.
fn brief_markdown(text: &str) -> Option<String> {
    let mut preamble: Vec<&str> = Vec::new();
    let mut sections: Vec<(&str, bool, Vec<&str>)> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        match (!in_fence).then(|| brief_field(line)).flatten() {
            Some((name, rest)) => {
                let lines = if rest.is_empty() { Vec::new() } else { vec![rest] };
                sections.push((name, !rest.is_empty(), lines));
            }
            None => match sections.last_mut() {
                Some((_, _, lines)) => lines.push(line),
                None => preamble.push(line),
            },
        }
    }
    if sections.is_empty() {
        return None;
    }
    let mut out = preamble.join("\n");
    for (name, inline, lines) in sections {
        let filled: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
        out.push_str(&format!("\n\n#### {name}\n\n"));
        match name {
            "SCOPE" => {
                let items: Vec<String> = filled.iter().map(|l| format!("- {l}")).collect();
                out.push_str(&items.join("\n"));
            }
            "VERIFY" => out.push_str(&fenced(&filled.join("\n"))),
            _ if inline => out.push_str(&filled.join("\n")),
            _ => out.push_str(lines.join("\n").trim_matches('\n')),
        }
    }
    Some(out.trim_start().to_owned())
}

/// A review stored as JSON, written out for reading. `None` when it is not one. Each finding's
/// location is a link when it names a file inside `project`.
fn review_markdown(text: &str, verdict_label: &str, no_findings: &str, project: Option<&Path>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let verdict = value.get("verdict")?.as_str()?;
    let word = |finding: &serde_json::Value, keys: &[&str]| {
        keys.iter().find_map(|key| finding.get(*key)?.as_str().map(str::to_owned))
    };
    let mut out = format!("**{verdict_label}: {verdict}**\n");
    let findings = value.get("findings").and_then(|f| f.as_array());
    let count = findings.map_or(0, |f| f.len());
    for finding in findings.into_iter().flatten() {
        let severity = word(finding, &["severity"]).unwrap_or_else(|| "note".into());
        out.push_str(&format!("\n- **{severity}**"));
        if let Some(place) = word(finding, &["location", "file"]) {
            let span = fenced_inline(&place);
            match project.and_then(|project| code_span_link(&place, project, true)) {
                Some(url) => out.push_str(&format!(" [{span}]({url})")),
                None => out.push_str(&format!(" {span}")),
            }
        }
        if let Some(body) = word(finding, &["evidence", "text", "check"]) {
            out.push_str(&format!(" {body}"));
        }
        if let Some(fix) = word(finding, &["fix"]) {
            out.push_str(&format!(" → {fix}"));
        }
    }
    if count == 0 {
        out.push_str(&format!("\n{no_findings}\n"));
    }
    Some(out)
}

fn file_label(path: &Path) -> String {
    let mut parts: Vec<String> = path
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.reverse();
    parts.join("/")
}

fn unit_icon(state: UnitState) -> IconName {
    match state {
        UnitState::Pending => IconName::Minus,
        UnitState::Running => IconName::LoaderCircle,
        UnitState::Built => IconName::Eye,
        UnitState::Verified => IconName::CircleCheck,
        UnitState::Failed => IconName::CircleX,
    }
}

fn level_icon(level: LogLevel) -> IconName {
    match level {
        LogLevel::Info => IconName::Info,
        LogLevel::Ok => IconName::CircleCheck,
        LogLevel::Warn => IconName::TriangleAlert,
        LogLevel::Error => IconName::CircleX,
    }
}

fn fill(cx: &App, en: &'static str, values: &[(&str, &str)]) -> String {
    let mut text = tr(cx, TABLE, en).to_owned();
    for (name, value) in values {
        text = text.replace(name, value);
    }
    text
}

fn attempts_text(attempts: u32, cx: &App) -> String {
    match attempts {
        0 => tr(cx, TABLE, "Not attempted yet").to_owned(),
        1 => tr(cx, TABLE, "1 attempt").to_owned(),
        n => fill(cx, "{n} attempts", &[("{n}", &n.to_string())]),
    }
}

fn tooltip_of(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| Tooltip::new(text.clone()).max_w(px(340.)).build(window, cx)
}

fn section_title(text: &'static str, t: &Tokens, cx: &App) -> Div {
    div().text_xs().font_weight(FontWeight::MEDIUM).text_color(t.muted).child(tr(cx, TABLE, text))
}

fn quiet_line(text: impl Into<SharedString>, t: &Tokens) -> Div {
    div().text_color(t.muted).child(text.into())
}

fn quiet_panel(icon: IconName, text: impl Into<SharedString>, detail: Option<SharedString>, t: &Tokens) -> Div {
    v_flex()
        .w_full()
        .items_center()
        .px_6()
        .py_10()
        .gap_2()
        .text_color(t.muted)
        .child(Icon::new(icon).size(px(18.)).text_color(t.idle))
        .child(div().text_center().child(text.into()))
        .when_some(detail, |this, detail| {
            this.child(div().w_full().text_xs().text_center().child(SelectableText::new("quiet-detail", detail)))
        })
}

/// The app's gold, which `.primary()` would not pick up because the theme leaves the kit's button tokens alone.
fn primary_button(t: &Tokens, cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(t.gold)
        .foreground(t.canvas)
        .hover(t.gold.opacity(0.9))
        .active(t.gold.opacity(0.8))
}

fn markdown_style() -> TextViewStyle {
    TextViewStyle::default()
        .paragraph_gap(rems(0.65))
        .heading_font_size(|level, _| match level {
            1 => px(17.),
            2 => px(15.),
            3 => px(14.),
            _ => px(12.),
        })
        .code_block(StyleRefinement::default().text_size(px(12.)).px_3().py_2())
}

/// CommonMark does not end bold when the closing `**` follows punctuation and a letter comes right
/// after it, which is how Chinese is written: `**建議：**把` shows its asterisks. A space is put
/// after such a closing `**`. Code spans (of any delimiter length) and fenced code (backtick or
/// tilde) are left alone.
fn mend_bold(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let marker = fence_marker(line);
        match (fence, marker) {
            (Some((ch, len)), Some((c, n, rest))) if c == ch && n >= len && rest.trim().is_empty() => {
                fence = None;
                out.push_str(line);
                continue;
            }
            (Some(_), _) => {
                out.push_str(line);
                continue;
            }
            (None, Some((c, n, rest))) if c != '`' || !rest.contains('`') => {
                fence = Some((c, n));
                out.push_str(line);
                continue;
            }
            _ => {}
        }
        if !line.contains("**") {
            out.push_str(line);
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let (mut open, mut i) = (false, 0);
        while i < chars.len() {
            if chars[i] == '`' {
                let ticks = chars[i..].iter().take_while(|c| **c == '`').count();
                let close = (i + ticks..chars.len()).find(|&j| {
                    chars[j] == '`'
                        && (j == 0 || chars[j - 1] != '`')
                        && chars[j..].iter().take_while(|c| **c == '`').count() == ticks
                });
                let end = close.map_or(i + ticks, |j| j + ticks);
                out.extend(&chars[i..end]);
                i = end;
                continue;
            }
            if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
                out.push_str("**");
                if open {
                    let punctuated = i > 0 && !chars[i - 1].is_alphanumeric() && !chars[i - 1].is_whitespace() && chars[i - 1] != '*';
                    if punctuated && chars.get(i + 2).is_some_and(|c| c.is_alphanumeric()) {
                        out.push(' ');
                    }
                }
                open = !open;
                i += 2;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// A fence line: its character, how many of them, and the rest of the line. Up to three spaces
/// of indentation are allowed.
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let c = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = body.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, &body[n..]))
}

/// Markdown, with web links opened in the browser and file references in the editor.
fn markdown_view(id: impl Into<ElementId>, text: SharedString, project: Option<PathBuf>) -> TextView {
    TextView::markdown(id, mend_bold(&text))
        .selectable(true)
        .style(markdown_style())
        .on_link_click(move |url, _, window, cx| {
            if url.starts_with(CITE_SCHEME) {
                open_cite(url, project.as_deref(), window, cx);
            } else if is_web_link(url) {
                cx.open_url(url);
            }
        })
}

/// The last tab text drawn with its references made into links.
struct Linked {
    key: usize,
    tab: DocTab,
    source: SharedString,
    project: Option<PathBuf>,
    text: SharedString,
}

pub struct Detail {
    state: Entity<AppState>,
    linked: RefCell<Option<Linked>>,
    tab: DocTab,
    anchor: Option<Anchor>,
    /// The run whose selected item disappeared; its Overview says so once.
    lost_in: Option<SharedString>,
    /// Files of the current selection only, so this stays small.
    docs: HashMap<PathBuf, Doc>,
    /// The run whose Resume was just pressed, so a second press cannot start it twice.
    resuming: Option<SharedString>,
    resume_settle: Option<Task<()>>,
    /// Demo only: lays the panel out at the minimum width.
    narrow: bool,
    /// Demo only: leaves files unread so the loading state can be captured.
    hold_reads: bool,
}

impl Detail {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, _, cx| this.model_changed(cx)).detach();
        let scene = state.read(cx).scene.clone();
        let mut this = Self {
            state,
            linked: RefCell::new(None),
            tab: DocTab::Brief,
            anchor: None,
            lost_in: None,
            docs: HashMap::new(),
            resuming: None,
            resume_settle: None,
            narrow: false,
            hold_reads: false,
        };
        if let Some(scene) = scene {
            this.apply_scene(&scene, cx);
        }
        this.model_changed(cx);
        this
    }

    fn model_changed(&mut self, cx: &mut Context<Self>) {
        let app = self.state.read(cx);
        let correction = match (&self.anchor, app.run()) {
            (Some(anchor), Some(run)) => followed(anchor, run, &app.selection),
            _ => None,
        };
        if let Some(selection) = correction {
            if selection == Selection::None {
                self.lost_in = app.run().map(|run| run.session.clone());
            }
            self.state.update(cx, |app, cx| app.select(selection, cx));
            return;
        }
        self.anchor = app.run().and_then(|run| anchor_of(run, &app.selection));
        if self.anchor.is_some() {
            self.lost_in = None;
        }
        let settled = match (&self.resuming, app.run()) {
            (Some(session), Some(run)) => *session != run.session || run.needs.is_empty(),
            _ => true,
        };
        if settled {
            self.resuming = None;
            self.resume_settle = None;
        }
        let wanted = app
            .run()
            .map(|run| wanted_paths(run, pane_for(run, &app.selection)))
            .unwrap_or_default();
        self.docs.retain(|path, _| wanted.contains(path));
        if !self.hold_reads {
            for path in wanted {
                self.read_doc(path, cx);
            }
        }
        cx.notify();
    }

    fn read_doc(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let doc = self.docs.entry(path.clone()).or_default();
        let known = match &doc.load {
            Load::Text(file) => Some(file.stamp),
            _ => None,
        };
        let source = path.clone();
        let reader = cx.background_spawn(async move { read_capped(&source, SHOWN_BYTES, known) });
        doc.reading = Some(cx.spawn(async move |this, cx| {
            let read = reader.await;
            this.update(cx, |this, cx| this.finish_read(&path, read, cx)).ok();
        }));
    }

    fn finish_read(&mut self, path: &Path, read: Read, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        doc.load = match read {
            Read::Unchanged => return,
            Read::Text(file) => Load::Text(file),
            Read::Missing => Load::Missing,
            Read::Unreadable(reason) => Load::Unreadable(reason.into()),
        };
        cx.notify();
    }

    fn resume(&mut self, cx: &mut Context<Self>) {
        self.resuming = self.state.read(cx).run().map(|run| run.session.clone());
        self.state.update(cx, |app, cx| app.resume_current(None, cx));
        self.resume_settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESUME_SETTLE).await;
            this.update(cx, |this, cx| {
                this.resuming = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn go(&self, selection: Selection) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let state = self.state.clone();
        move |_, _, cx| state.update(cx, |app, cx| app.select(selection.clone(), cx))
    }

    fn load_of(&self, source: &TextSource) -> Load {
        match source {
            TextSource::Inline(text) => Load::Text(FileText {
                text: match clean(text) {
                    Cow::Borrowed(_) => text.clone(),
                    Cow::Owned(cleaned) => cleaned.into(),
                },
                cut_from: None,
                stamp: (0, None),
            }),
            TextSource::Path(path) => self.docs.get(path).map_or(Load::Reading, |doc| doc.load.clone()),
        }
    }

    fn back_button(&self, cx: &App) -> Button {
        Button::new("detail-back")
            .ghost()
            .xsmall()
            .icon(IconName::ChevronLeft)
            .label(tr(cx, TABLE, "Overview"))
            .on_click(self.go(Selection::None))
    }

    fn overview(&self, run: &Run, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let lost = self.lost_in.as_ref() == Some(&run.session);
        v_flex()
            .size_full()
            .p_4()
            .gap_5()
            .overflow_y_scrollbar()
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, TABLE, "Overview")))
            .when(lost, |this| {
                this.child(quiet_line(tr(cx, TABLE, "The item you selected is no longer in this run."), &t))
            })
            .children(super::usage::warning_texts(run, *cx.global::<i18n::Lang>()).into_iter().enumerate().map(|(i, warning)| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_color(t.muted)
                    .child(Icon::new(IconName::TriangleAlert).size(px(14.)).text_color(t.review).mt(px(2.)))
                    .child(div().flex_1().min_w_0().child(SelectableText::new(("warning", i), warning)))
            }))
            .child(self.running_section(run, cx))
            .when(!run.needs.is_empty(), |this| this.child(self.needs_section(run, cx)))
            .child(self.decisions_section(run, cx))
            .child(self.events_section(run, cx))
            .into_any_element()
    }

    fn running_section(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let units = running_units(run);
        let stage = run.stages.iter().find(|s| s.phase == Phase::Running);
        let line: Option<SharedString> = if !units.is_empty() {
            None
        } else {
            Some(match run.status {
                RunStatus::Blocked => tr(cx, TABLE, "Stopped until you respond.").into(),
                RunStatus::Waiting => tr(cx, TABLE, "Waiting for the background batch to finish.").into(),
                RunStatus::LimitWait => tr(cx, TABLE, "Waiting for the usage limit to reset.").into(),
                RunStatus::Interrupted => tr(cx, TABLE, "It stopped before finishing. Resume to continue.").into(),
                RunStatus::Done => {
                    let done = run.count(UnitState::Verified).to_string();
                    let total = run.units.len().to_string();
                    fill(
                        cx,
                        "This wish has ended. {done} of {total} units verified.",
                        &[("{done}", &done), ("{total}", &total)],
                    )
                    .into()
                }
                RunStatus::Active => match stage {
                    Some(stage) => {
                        let key = STAGE_NAMES.iter().find(|name| **name == stage.name.as_ref());
                        let name: SharedString = key.map_or(stage.name.clone(), |key| tr(cx, TABLE, key).into());
                        fill(cx, "{stage}: in progress", &[("{stage}", name.as_ref())]).into()
                    }
                    None => tr(cx, TABLE, "Nothing is running right now.").into(),
                },
            })
        };
        v_flex()
            .gap_1()
            .child(section_title("Running now", &t, cx))
            .when_some(line, |this, line| this.child(quiet_line(line, &t)))
            .children(units.into_iter().map(|i| self.unit_row(run, i, cx)))
    }

    fn unit_row(&self, run: &Run, index: usize, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let unit = &run.units[index];
        let detail = format!("{} · {}", unit.id, attempts_text(unit.attempts, cx));
        Button::new(("unit-row", index))
            .ghost()
            .w_full()
            .h_auto()
            .px_1()
            .py_1()
            .tooltip(unit.title.clone())
            .on_click(self.go(Selection::Unit(index)))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(unit_icon(unit.state)).size(px(14.)).text_color(t.unit(unit.state)).mt(px(2.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).text_color(t.text).truncate().child(unit.title.clone()))
                            .child(div().text_xs().text_color(t.muted).truncate().child(detail)),
                    ),
            )
    }

    fn needs_section(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let last = run.needs.len() - 1;
        v_flex()
            .gap_2()
            .children(run.needs.iter().enumerate().map(|(i, need)| self.need_card(run, i, need, i == last, cx)))
    }

    fn need_card(&self, run: &Run, index: usize, need: &Need, with_action: bool, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let (icon, color, kind) = match need.kind {
            NeedKind::Blocked => (IconName::CircleAlert, t.fail, "Needs you"),
            NeedKind::Waiting => (IconName::Pause, t.review, "Waiting on batch"),
            NeedKind::Interjection => (IconName::User, t.run, "Interjection"),
        };
        let label = if need.kind == NeedKind::Waiting { "Resume now" } else { "Resume" };
        let resuming = self.resuming.as_ref() == Some(&run.session);
        ornament::frame(&t, color.opacity(0.6), t.surface.blend(color.opacity(0.06)))
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .child(
                h_flex()
                    .gap_1p5()
                    .text_xs()
                    .text_color(color)
                    .child(Icon::new(icon).size(px(14.)))
                    .child(tr(cx, TABLE, kind)),
            )
            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(need.title.clone()))
            .child(div().text_color(t.muted).child(SelectableText::new(("need-note", index), need.note.clone())))
            .when(with_action, |this| {
                this.child(
                    h_flex().justify_end().child(
                        Button::new(("need-resume", index))
                            .custom(primary_button(&t, cx))
                            .small()
                            .label(tr(cx, TABLE, label))
                            .loading(resuming)
                            .disabled(resuming)
                            .on_click(cx.listener(|this, _, _, cx| this.resume(cx))),
                    ),
                )
            })
    }

    fn decisions_section(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        v_flex()
            .gap_2()
            .child(section_title("Decisions made for you", &t, cx))
            .when(run.decisions.is_empty(), |this| {
                this.child(quiet_line(tr(cx, TABLE, "Chronos has not decided anything on your behalf."), &t))
            })
            .children(run.decisions.iter().enumerate().map(|(i, decision)| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().mt(px(7.)).size(px(4.)).flex_none().rounded_full().bg(t.gold))
                    .child(div().flex_1().min_w_0().child(SelectableText::new(("decision", i), decision.clone())))
            }))
    }

    fn events_section(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        v_flex()
            .gap_2()
            .child(section_title("Recent events", &t, cx))
            .when(run.log.is_empty(), |this| {
                this.child(quiet_line(tr(cx, TABLE, "Nothing has been logged yet."), &t))
            })
            .children(run.log.iter().rev().take(RECENT_EVENTS).enumerate().map(|(i, line)| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(level_icon(line.level)).size(px(14.)).text_color(t.log(line.level)).mt(px(2.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(div().flex_none().font_family(mono.clone()).child(line.time.clone()))
                                    .child(div().min_w_0().truncate().child(super::control::source_label(&line.source, cx))),
                            )
                            .child(
                                div()
                                    .id(("event", i))
                                    .line_clamp(3)
                                    .tooltip(tooltip_of(line.text.clone()))
                                    .child(line.text.clone()),
                            ),
                    )
            }))
    }

    fn module_page(&self, run: &Run, index: usize, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let module = &run.modules[index];
        let (done, total) = module_counts(run, module);
        let progress = fill(
            cx,
            "{done} of {total} verified",
            &[("{done}", &done.to_string()), ("{total}", &total.to_string())],
        );
        let depends: Vec<usize> = module.depends.iter().copied().filter(|&d| d < run.modules.len()).collect();
        let units: Vec<usize> = module.units.iter().copied().filter(|&u| u < run.units.len()).collect();
        v_flex()
            .size_full()
            .p_4()
            .gap_5()
            .overflow_y_scrollbar()
            .id(("module-page", index))
            .child(
                v_flex()
                    .gap_2()
                    .items_start()
                    .child(self.back_button(cx))
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(div().text_xs().text_color(t.muted).child(tr(cx, TABLE, "Module")))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(SelectableText::new(("module-id", index), module.id.clone())),
                            )
                            .child(div().child(SelectableText::new(("module-purpose", index), module.purpose.clone())))
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(t.unit(module_state(run, module)))
                                    .child(Icon::new(unit_icon(module_state(run, module))).size(px(13.)))
                                    .child(progress),
                            ),
                    ),
            )
            .child(self.contract_section(module, index, cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(section_title("Depends on", &t, cx))
                    .when(depends.is_empty(), |this| {
                        this.child(quiet_line(tr(cx, TABLE, "This module depends on nothing else."), &t))
                    })
                    .children(depends.into_iter().map(|d| self.module_row(run, d, cx))),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(section_title("Units", &t, cx))
                    .when(units.is_empty(), |this| {
                        this.child(quiet_line(tr(cx, TABLE, "This module has no units yet."), &t))
                    })
                    .children(units.into_iter().map(|u| self.unit_row(run, u, cx))),
            )
            .into_any_element()
    }

    fn contract_section(&self, module: &Module, index: usize, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let load = module.contract.as_ref().map(|source| self.load_of(source));
        let key = 1_000_000 + index;
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .child(section_title("Contract", &t, cx))
                    .when_some(module.contract.as_ref().zip(load.as_ref()), |this, (source, load)| {
                        this.child(self.doc_actions(key, source, load, cx))
                    }),
            )
            .map(|this| match (&module.contract, load) {
                (None, _) | (_, None) => this.child(quiet_line(tr(cx, TABLE, "No contract was written for this module."), &t)),
                (Some(source), Some(load)) => this.child(self.doc_content(key, DocTab::Report, source, load, false, cx)),
            })
    }

    fn module_row(&self, run: &Run, index: usize, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let module = &run.modules[index];
        let state = module_state(run, module);
        let (done, total) = module_counts(run, module);
        Button::new(("module-row", index))
            .ghost()
            .w_full()
            .h_auto()
            .px_1()
            .py_1()
            .tooltip(module.purpose.clone())
            .on_click(self.go(Selection::Module(index)))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(unit_icon(state)).size(px(14.)).text_color(t.unit(state)).mt(px(2.)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(13.)).text_color(t.text).truncate().child(module.id.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .truncate()
                                    .child(format!("{} · {done}/{total}", module.purpose)),
                            ),
                    ),
            )
    }

    fn unit_page(&self, run: &Run, index: usize, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let unit = &run.units[index];
        let selected = DocTab::ALL.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let module = run.modules.get(unit.module);
        let label = tr(cx, &[], unit.state.label());
        let raw_differs = !unit.raw_state.eq_ignore_ascii_case(label);
        let color = t.unit(unit.state);
        v_flex()
            .size_full()
            .child(
                v_flex()
                    .w_full()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .gap_2()
                    .items_start()
                    .child(self.back_button(cx))
                    .child(
                        div()
                            .id("unit-title")
                            .w_full()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_clamp(3)
                            .tooltip(tooltip_of(unit.title.clone()))
                            .child(unit.title.clone()),
                    )
                    .when(unit.id != unit.title, |this| {
                        this.child(
                            div()
                                .w_full()
                                .text_xs()
                                .text_color(t.muted)
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(SelectableText::new(("unit-id", index), unit.id.clone())),
                        )
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .items_center()
                            .gap_x_2()
                            .gap_y_1()
                            .child(
                                Tag::custom(color.opacity(0.12), color, color.opacity(0.35))
                                    .rounded_full()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(unit_icon(unit.state)).size(px(12.)))
                                            .child(label)
                                            .when(raw_differs, |this| {
                                                this.child(div().opacity(0.7).child(unit.raw_state.clone()))
                                            }),
                                    ),
                            )
                            .child(div().text_xs().text_color(t.muted).child(attempts_text(unit.attempts, cx)))
                            .when_some(module, |this, module| {
                                this.child(
                                    Button::new("unit-module")
                                        .ghost()
                                        .xsmall()
                                        .label(format!("{} {}", tr(cx, TABLE, "Module"), module.id))
                                        .child(Icon::new(IconName::ChevronRight).size(px(12.)))
                                        .on_click(self.go(Selection::Module(unit.module))),
                                )
                            }),
                    ),
            )
            .child(
                TabBar::new("unit-tabs")
                    .underline()
                    .small()
                    .prefix(div().w_4())
                    .selected_index(selected)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(tab) = DocTab::ALL.get(*ix) {
                            this.tab = *tab;
                            cx.notify();
                        }
                    }))
                    .children(DocTab::ALL.iter().map(|tab| Tab::new().label(tr(cx, TABLE, tab.label())))),
            )
            .child(div().flex_1().min_h_0().child(self.tab_body(unit, index * DocTab::ALL.len() + selected, cx)))
            .into_any_element()
    }

    fn tab_body(&self, unit: &Unit, key: usize, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let Some(source) = self.tab.source(&unit.files) else {
            return quiet_panel(IconName::FileText, tr(cx, TABLE, empty_reason(self.tab, unit.state)), None, &t)
                .into_any_element();
        };
        let load = self.load_of(source);
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .w_full()
                    .h_8()
                    .flex_none()
                    .px_4()
                    .gap_2()
                    .justify_between()
                    .child(self.source_label(key, source, cx))
                    .child(self.doc_actions(key, source, &load, cx)),
            )
            .child(div().flex_1().min_h_0().child(self.doc_content(key, self.tab, source, load, true, cx)))
            .into_any_element()
    }

    fn source_label(&self, key: usize, source: &TextSource, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        match source {
            TextSource::Inline(_) => div().into_any_element(),
            TextSource::Path(path) => div()
                .id(("doc-name", key))
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(t.muted)
                .truncate()
                .tooltip(tooltip_of(path.display().to_string()))
                .child(file_label(path))
                .into_any_element(),
        }
    }

    fn doc_actions(&self, key: usize, source: &TextSource, load: &Load, cx: &Context<Self>) -> impl IntoElement {
        let file = match load {
            Load::Text(file) if !file.text.trim().is_empty() => Some(file.clone()),
            _ => None,
        };
        let path = match (source, load) {
            (TextSource::Path(_), Load::Missing) => None,
            (TextSource::Path(path), _) => Some(path.clone()),
            (TextSource::Inline(_), _) => None,
        };
        let open_tip = match (source, load) {
            (TextSource::Inline(_), _) => tr(cx, TABLE, "This text is not saved as a file."),
            (_, Load::Missing) => tr(cx, TABLE, "This file is no longer there."),
            _ => tr(cx, &[], "Open in editor"),
        };
        let copy_tip = if file.as_ref().is_some_and(|f| f.cut_from.is_some()) {
            tr(cx, TABLE, "Copy the part shown")
        } else {
            tr(cx, &[], "Copy")
        };
        h_flex()
            .flex_none()
            .gap_0p5()
            .map(|this| match file {
                Some(file) => this.child(Clipboard::new(("doc-copy", key)).value(file.text.clone()).tooltip(copy_tip)),
                None => this.child(Button::new(("doc-copy", key)).ghost().xsmall().icon(IconName::Copy).disabled(true)),
            })
            .child(
                Button::new(("doc-open", key))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ExternalLink)
                    .tooltip(open_tip)
                    .disabled(path.is_none())
                    .when_some(path, |this, path| {
                        this.on_click(move |_, _, cx| cx.open_with_system(&path))
                    }),
            )
    }

    fn doc_content(&self, key: usize, tab: DocTab, source: &TextSource, load: Load, scrolls: bool, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        match load {
            Load::Reading => quiet_panel(IconName::Loader, tr(cx, TABLE, "Loading…"), None, &t).into_any_element(),
            Load::Missing => quiet_panel(
                IconName::File,
                tr(cx, TABLE, "This file is no longer there."),
                source_path(source),
                &t,
            )
            .into_any_element(),
            Load::Unreadable(reason) => quiet_panel(
                IconName::File,
                tr(cx, TABLE, "This file could not be read."),
                Some(reason),
                &t,
            )
            .into_any_element(),
            Load::Text(file) if file.text.trim().is_empty() => {
                quiet_panel(IconName::File, tr(cx, TABLE, "This file is empty."), None, &t).into_any_element()
            }
            Load::Text(file) => v_flex()
                .size_full()
                .when_some(file.cut_from, |this, total| {
                    this.child(self.cut_note(key, source, total, cx))
                })
                .child(div().flex_1().min_h_0().child(self.text_view(key, tab, &file, scrolls, cx)))
                .into_any_element(),
        }
    }

    fn cut_note(&self, key: usize, source: &TextSource, total: u64, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let message = fill(
            cx,
            "Showing the first {shown} of {total}.",
            &[("{shown}", &human_bytes(SHOWN_BYTES as u64)), ("{total}", &human_bytes(total))],
        );
        let path = match source {
            TextSource::Path(path) => Some(path.clone()),
            TextSource::Inline(_) => None,
        };
        h_flex()
            .mx_4()
            .mb_2()
            .px_2p5()
            .py_1p5()
            .flex_wrap()
            .gap_x_2()
            .gap_y_1()
            .items_center()
            .rounded(px(6.))
            .bg(t.review.opacity(0.1))
            .text_xs()
            .child(Icon::new(IconName::TriangleAlert).size(px(13.)).text_color(t.review))
            .child(div().flex_1().min_w(px(120.)).child(message))
            .when_some(path, |this, path| {
                this.child(
                    Button::new(("doc-open-full", key))
                        .link()
                        .xsmall()
                        .label(tr(cx, TABLE, "Open the full file"))
                        .on_click(move |_, _, cx| cx.open_with_system(&path)),
                )
            })
    }

    /// The project folder of the open run: references are followed only inside it.
    fn project(&self, cx: &App) -> Option<PathBuf> {
        self.state.read(cx).run().and_then(|run| run.cwd.clone()).filter(|cwd| cwd.is_absolute())
    }

    /// The text a tab draws, with its file references as links. Kept until the text, the tab or
    /// the project changes, so a large log is not scanned again on every frame.
    fn linked_text(&self, key: usize, tab: DocTab, file: &FileText, project: Option<&Path>, cx: &App) -> SharedString {
        if let Some(cached) = self.linked.borrow().as_ref() {
            if cached.key == key && cached.tab == tab && cached.source == file.text && cached.project.as_deref() == project {
                return cached.text.clone();
            }
        }
        let text: String = match tab {
            DocTab::Verify => match project {
                Some(project) => link_plain(&file.text, project),
                None => file.text.to_string(),
            },
            DocTab::Brief | DocTab::Report | DocTab::Review => {
                let shaped = match tab {
                    DocTab::Brief => brief_markdown(&file.text),
                    DocTab::Review => review_markdown(&file.text, tr(cx, TABLE, "Verdict"), tr(cx, TABLE, "No findings."), project),
                    _ => None,
                }
                .unwrap_or_else(|| file.text.to_string());
                match project {
                    Some(project) => link_markdown(&shaped, project),
                    None => shaped,
                }
            }
        };
        let text: SharedString = text.into();
        *self.linked.borrow_mut() = Some(Linked {
            key,
            tab,
            source: file.text.clone(),
            project: project.map(Path::to_path_buf),
            text: text.clone(),
        });
        text
    }

    fn text_view(&self, key: usize, tab: DocTab, file: &FileText, scrolls: bool, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let project = self.project(cx);
        if tab == DocTab::Verify {
            let mono = cx.theme().mono_font_family.clone();
            // With a project, the log is drawn as Markdown so its references can be links;
            // without one there is nothing to follow and it stays plain selectable text.
            let body = match project.as_deref() {
                Some(root) => markdown_view(("doc-mono", key), self.linked_text(key, tab, file, Some(root), cx), project.clone())
                    .font_family(mono.clone())
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .into_any_element(),
                None => SelectableText::new(("doc-mono", key), file.text.clone()).into_any_element(),
            };
            return div()
                .size_full()
                .overflow_y_scrollbar()
                .id(("doc-scroll", key))
                .child(
                    div()
                        .mx_4()
                        .mb_4()
                        .px_3()
                        .py_2()
                        .rounded(px(6.))
                        .bg(t.canvas)
                        .border_1()
                        .border_color(t.border)
                        .font_family(mono)
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .child(body),
                )
                .into_any_element();
        }
        let text = self.linked_text(key, tab, file, project.as_deref(), cx);
        markdown_view(("doc-text", key), text, project)
            .scrollable(scrolls)
            .when(scrolls, |this| this.size_full().px_4())
            .into_any_element()
    }

    /// Demo only: puts the panel into a named state so it can be captured. Words join with `+`.
    fn apply_scene(&mut self, scene: &str, cx: &mut Context<Self>) {
        let words: Vec<&str> = scene.split('+').collect();
        let has = |word: &str| words.contains(&word);
        self.narrow = has("narrow");
        self.hold_reads = has("loading");
        for (word, tab) in [("report", DocTab::Report), ("verify", DocTab::Verify), ("review", DocTab::Review)] {
            if has(word) {
                self.tab = tab;
            }
        }
        let links = has("links");
        let files = links || has("files") || has("cut") || has("missing") || has("unreadable") || has("loading");
        let dir = std::env::temp_dir().join("chronos-detail-scene");
        if files {
            let _ = std::fs::create_dir_all(&dir);
        }
        self.state.update(cx, |app, _| {
            if has("waiting") {
                app.current = Some(1);
            }
            if has("blocked") {
                app.current = Some(2);
            }
            let unit_id = [("unit", "pdftext"), ("unreviewed", "d10_scorer"), ("pending", "causal_fix"), ("failed", "causal")]
                .iter()
                .find(|(word, _)| has(word))
                .map(|(_, id)| *id);
            let selection = app.current.and_then(|c| app.runs.get(c)).and_then(|run| {
                if has("lost") {
                    Some(Selection::Unit(9999))
                } else if has("module") {
                    run.modules.iter().position(|m| m.id == "read").map(Selection::Module)
                } else {
                    unit_id.and_then(|id| run.units.iter().position(|u| u.id == id)).map(Selection::Unit)
                }
            });
            if let Some(selection) = selection {
                app.selection = selection;
            }
            let selected = app.selection.clone();
            let Some(run) = app.current.and_then(|c| app.runs.get_mut(c)) else {
                return;
            };
            if has("interjection") {
                run.needs.push(Need {
                    kind: NeedKind::Interjection,
                    title: "Your message is waiting for the next step".into(),
                    note: "順便把 tests/test_pdftext.py 的頁序也驗一下".into(),
                });
            }
            if has("warning") {
                run.warnings.push("units.tsv could not be read; showing the last good copy.".into());
            }
            if has("long") {
                let long = "very_long_token_without_any_spaces_".repeat(12);
                run.decisions.push(long.clone().into());
                run.log.push(LogLine {
                    time: "09:52:00".into(),
                    source: long.clone().into(),
                    text: long.clone().into(),
                    level: LogLevel::Error,
                });
                run.needs.push(Need { kind: NeedKind::Blocked, title: long.clone().into(), note: long.clone().into() });
                for unit in &mut run.units {
                    unit.title = format!("{} {}", unit.title, "一個非常長的單元標題，用來檢查窄版面時是否正常換行與截斷。".repeat(3)).into();
                }
                for module in &mut run.modules {
                    module.purpose = format!("{} {}", module.purpose, "很長的用途說明。".repeat(12)).into();
                }
                if let Some(unit) = run.units.iter_mut().find(|u| u.id == "pdftext") {
                    let lines = format!("{long}\n$ {}\nFAILED tests/test_pdftext.py::test_tile - AssertionError: {}\n", "x".repeat(600), "錯誤訊息".repeat(90));
                    unit.files.verify = Some(TextSource::Inline(lines.repeat(4).into()));
                    let table = format!("# Very long heading {long}\n\nParagraph {long}\n\n```\n{long}\n```\n\n| a | b |\n|---|---|\n| {long} | {long} |\n");
                    unit.files.report = Some(TextSource::Inline(table.into()));
                }
            }
            if !files {
                return;
            }
            let write = |name: &str, body: &str| {
                let path = dir.join(name);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, body);
                TextSource::Path(path)
            };
            if links {
                // The scene folder stands in for the project, holding the files the documents cite.
                run.cwd = Some(dir.clone());
                for name in SCENE_CITED {
                    write(name, "# cited by the scene\n");
                }
            }
            let Selection::Unit(i) = selected else {
                return;
            };
            let Some(unit) = run.units.get_mut(i) else {
                return;
            };
            if has("files") || has("loading") || links {
                unit.files.brief = Some(write("brief.md", SCENE_BRIEF));
                unit.files.report = Some(write("report.md", SCENE_REPORT));
                unit.files.verify = Some(write("verify.log", SCENE_VERIFY));
                unit.files.review = Some(write("review.json", SCENE_REVIEW));
            }
            if has("cut") {
                let body = "2026-11-09 09:41:45 INFO  read_batch  screening 1,467 included, 878 excluded, 5,258 pending 測試資料\n".repeat(36_000);
                unit.files.report = Some(write("huge-report.md", &body));
                unit.files.verify = Some(write("huge-verify.log", &body));
            }
            if has("missing") {
                unit.files.brief = Some(TextSource::Path(dir.join("no-such-file.md")));
                unit.files.report = Some(TextSource::Path(dir.join("no-such-file.md")));
            }
            if has("unreadable") {
                unit.files.report = Some(TextSource::Path(dir.clone()));
            }
        });
    }
}

fn source_path(source: &TextSource) -> Option<SharedString> {
    match source {
        TextSource::Path(path) => Some(path.display().to_string().into()),
        TextSource::Inline(_) => None,
    }
}

const SCENE_BRIEF: &str = "GOAL\nModule m01-core passes its ACCEPT test: Shared infrastructure that every later module uses.\n\nSCOPE\n`hf6_study/hf6/core/` (`__init__.py`, `paths.py`, `legacy.py`), `hf6_study/data/manifest.json`\n(all paths relative to E:/codex/ML_RRAM; create parents as needed)\n\nVERIFY\npython -m pytest -q E:/codex/ML_RRAM/hf6_study/hf6/core\n\nCONTEXT\nLegacy sources to import read-only through `legacy.py`: tools/C31/V7/sealed_preflight.py, tools/C31/V7/guard.py.\n\nStudy-wide decisions:\n\n1. **Own directory, own repo.** The whole study lives in `ML_RRAM/hf6_study/`.\n2. **Python layout.** Package `hf6` at `hf6_study/hf6/`, one subpackage per module.\n";
const SCENE_REPORT: &str = "# 結果\n\n把頁面算圖改走快取，並補上版面測試。\n\n- 改動 `src/pdftext/render.py:42` 的 `render_page`，快取鍵加上 dpi\n- 新增 tests/test_pdftext.py::test_tile 與 tests/test_cache.py:17\n- 18 tests passed\n- SCOPE 之外沒有改動；C:/Windows/win.ini 與 ../outside/notes.md 不在專案內，不會變成連結\n\n```text\nrender_page(pdf, sha, page, cache, dpi=300)\n```\n\n詳見 [說明](https://example.com/notes)。\n";
const SCENE_VERIFY: &str = "$ python -m pytest tests/test_pdftext.py -q\r\n..................\t[100%]\r\n\u{1b}[32m18 passed\u{1b}[0m in 2.41s\r\n  File \"src/pdftext/render.py\", line 42, in render_page\r\nprogress 10%\rprogress 100%\n";
/// Files the scene's documents cite, created in its folder so the links lead somewhere.
const SCENE_CITED: [&str; 5] = [
    "src/pdftext/render.py",
    "tests/test_pdftext.py",
    "tests/test_cache.py",
    "tests/bridge_tools.rs",
    "tests/adoption_doc.rs",
];
const SCENE_REVIEW: &str = "{\"unit\":\"bridge-no-gemini\",\"verdict\":\"pass\",\"findings\":[{\"check\":\"untested\",\"severity\":\"minor\",\"location\":\"tests/bridge_tools.rs\",\"evidence\":\"bridge_status description not asserted\",\"fix\":\"assert description\"},{\"severity\":\"minor\",\"file\":\"tests/adoption_doc.rs\",\"text\":\"14/167 label not tied to figure\"}]}";

impl Render for Detail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cx: &Context<Self> = cx;
        let t = cx.tokens();
        let app = self.state.read(cx);
        let body = match app.run() {
            None => quiet_panel(IconName::Inbox, tr(cx, TABLE, "No wish is open."), None, &t).into_any_element(),
            Some(run) => match pane_for(run, &app.selection) {
                Pane::Overview => self.overview(run, cx),
                Pane::Module(i) => self.module_page(run, i, cx),
                Pane::Unit(i) => self.unit_page(run, i, cx),
            },
        };
        div()
            .size_full()
            .when(self.narrow, |this| this.w(px(320.)))
            .bg(t.surface)
            .text_color(t.text)
            .text_size(px(13.))
            .overflow_hidden()
            .child(body)
    }
}

#[cfg(test)]
mod cite_tests {
    use super::*;
    use std::prelude::v1::test;

    fn project() -> PathBuf {
        std::env::temp_dir().join("chronos-cite-project")
    }

    fn found(text: &str) -> Vec<(String, Option<u32>, Option<u32>, &str)> {
        find_cites(text).into_iter().map(|c| (c.path.clone(), c.line, c.column, &text[c.range])).collect()
    }

    #[test]
    fn references_are_found_with_their_line_and_column() {
        let text = "改了 src/desktop/mesh.rs:120 和 `tests/a.rs:3:7`，見 docs/app-design.md#L26 與 tests/test_x.py::test_tile.";
        assert_eq!(
            found(text),
            vec![
                ("src/desktop/mesh.rs".to_owned(), Some(120), None, "src/desktop/mesh.rs:120"),
                ("tests/a.rs".to_owned(), Some(3), Some(7), "tests/a.rs:3:7"),
                ("docs/app-design.md".to_owned(), Some(26), None, "docs/app-design.md#L26"),
                ("tests/test_x.py".to_owned(), None, None, "tests/test_x.py::test_tile"),
            ]
        );
        assert_eq!(found("main.rs:12-20 failed")[0].1, Some(12));
        assert_eq!(found(r"see E:\proj\src\lib.rs:9:")[0], (r"E:\proj\src\lib.rs".to_owned(), Some(9), None, r"E:\proj\src\lib.rs:9"));
        let traceback = "  File \"pkg/app.py\", line 42, in main";
        assert_eq!(found(traceback), vec![("pkg/app.py".to_owned(), Some(42), None, "pkg/app.py")]);
        assert_eq!(found("  File \"app.py\", line 7, in x")[0].1, Some(7));
    }

    #[test]
    fn words_that_only_look_like_files_are_not_references() {
        for text in [
            "e.g. this",
            "version v1.2.3 and 2.41s",
            "README.md alone in a sentence",
            "https://example.com/notes/a.html",
            "hf6_study/hf6/core/ is a folder",
            "and/or TCP/IP",
            "~/secret/key.pem",
            "a:b/c.rs",
            "18 passed in 2.41s",
        ] {
            assert!(find_cites(text).is_empty(), "{text}: {:?}", find_cites(text));
        }
    }

    #[test]
    fn only_paths_inside_the_project_resolve() {
        let root = project();
        assert_eq!(resolve_cite("src/a.rs", &root), Ok(root.join("src").join("a.rs")));
        assert_eq!(resolve_cite("./src/../src/a.rs", &root), Ok(root.join("src").join("a.rs")));
        let inside = root.join("tests").join("t.py");
        assert_eq!(resolve_cite(&inside.display().to_string(), &root), Ok(inside));
        for outside in ["../a.rs", "src/../../a.rs", "../chronos-cite-project-evil/a.rs", "~/a.rs"] {
            assert_eq!(resolve_cite(outside, &root), Err(Refusal::Outside), "{outside}");
        }
        let elsewhere = std::env::temp_dir().join("elsewhere").join("x.rs");
        assert_eq!(resolve_cite(&elsewhere.display().to_string(), &root), Err(Refusal::Outside));
        assert_eq!(resolve_cite("src/a.rs", Path::new("relative/project")), Err(Refusal::Outside));
        #[cfg(windows)]
        {
            assert_eq!(resolve_cite(r"C:\Windows\win.ini", &root), Err(Refusal::Outside));
            assert_eq!(resolve_cite("/Windows/win.ini", &root), Err(Refusal::Outside));
            let upper = root.display().to_string().to_uppercase();
            assert!(resolve_cite(&format!(r"{upper}\src\a.rs"), &root).is_ok(), "drive letters and folders ignore case");
        }
    }

    #[test]
    fn opening_checks_the_disk_again() {
        let root = std::env::temp_dir().join(format!("chronos-cite-open-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("a.rs"), "fn a() {}\n").unwrap();
        assert_eq!(checked_target("src/a.rs", &root), Ok(root.join("src").join("a.rs")));
        assert_eq!(checked_target("src/gone.rs", &root), Err(Refusal::Missing));
        assert_eq!(checked_target("src", &root), Err(Refusal::Missing), "a folder is not a file to open");
        assert_eq!(checked_target("../a.rs", &root), Err(Refusal::Outside));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn links_carry_the_reference_and_editor_gets_the_line() {
        let cite = Cite { range: 0..0, path: "src/a b#c.rs".into(), line: Some(12), column: Some(4) };
        let url = cite_url(&cite);
        assert!(!url.contains(' ') && !url.contains('(') && !url.contains(')'), "{url}");
        assert_eq!(parse_cite_url(&url), Some(("src/a b#c.rs".to_owned(), Some(12), Some(4))));
        let drive = Cite { range: 0..0, path: r"E:\p\x.rs".into(), line: None, column: None };
        assert_eq!(parse_cite_url(&cite_url(&drive)), Some((r"E:\p\x.rs".to_owned(), None, None)));
        assert_eq!(parse_cite_url("https://example.com"), None);
        assert_eq!(parse_cite_url("chronos-cite:a.rs#x"), None);
        let target = project().join("a.rs");
        assert_eq!(editor_args(&target, Some(12), Some(4)), vec!["-g".to_owned(), format!("{}:12:4", target.display())]);
        assert_eq!(editor_args(&target, None, Some(4)), vec!["-g".to_owned(), target.display().to_string()]);
    }

    #[test]
    fn markdown_links_references_and_leaves_code_links_and_outside_paths_alone() {
        let root = project();
        let text = "改了 src/a.rs:3 與 `tests/b.rs`。\n\n```\nsrc/in_fence.rs:1\n```\n\n[docs](docs/x.md) <https://e.com/a.rs> ../out.rs:2 `not a path`\n";
        let linked = link_markdown(text, &root);
        assert!(linked.contains("[src\\/a\\.rs\\:3](chronos-cite:src/a.rs#3)"), "{linked}");
        assert!(linked.contains("[`tests/b.rs`](chronos-cite:tests/b.rs)"), "{linked}");
        assert!(linked.contains("```\nsrc/in_fence.rs:1\n```"), "fenced code stays as written: {linked}");
        assert!(linked.contains("[docs](docs/x.md)") && linked.contains("<https://e.com/a.rs>"), "{linked}");
        assert!(linked.contains(" ../out.rs:2 ") && !linked.contains("out.rs#2"), "outside paths are not links: {linked}");
        assert!(linked.contains("`not a path`"));
        assert_eq!(link_markdown("no references here.\n", &root), "no references here.\n");
    }

    #[test]
    fn a_log_keeps_its_lines_and_characters_and_gains_links() {
        let root = project();
        let linked = link_plain("FAILED tests/t.py::test_a - *x*\n\n  [2] done", &root);
        assert_eq!(
            linked,
            "FAILED [tests\\/t\\.py\\:\\:test\\_a](chronos-cite:tests/t.py) \\- \\*x\\*\\\n\u{a0}\\\n\u{a0}\u{a0}\\[2\\] done"
        );
    }

    #[test]
    fn a_review_finding_shows_its_location_as_a_link() {
        let root = project();
        let review = r#"{"verdict":"pass","findings":[{"severity":"minor","location":"bridge.rs","evidence":"x"},{"severity":"minor","location":"../../etc/passwd.txt"}]}"#;
        let text = review_markdown(review, "Verdict", "No findings.", Some(&root)).unwrap();
        assert!(text.contains("[`bridge.rs`](chronos-cite:bridge.rs)"), "{text}");
        assert!(text.contains(" `../../etc/passwd.txt`") && !text.contains("passwd.txt)"), "{text}");
        let plain = review_markdown(review, "Verdict", "No findings.", None).unwrap();
        assert!(plain.contains(" `bridge.rs`") && !plain.contains("chronos-cite"));
    }
}

#[cfg(test)]
mod system_open_tests {
    use super::system_open_allowed;
    use std::path::Path;
    use std::prelude::v1::test;

    #[test]
    fn programs_and_scripts_are_never_handed_to_the_system() {
        for name in [
            "run.bat", "run.CMD", "tool.exe", "x.com", "x.scr", "x.msi", "x.lnk", "x.ps1", "x.vbs", "x.js", "x.jse",
            "x.wsf", "x.hta", "x.reg", "x.py", "x.sh", "x.jar", "x.url", "x.appref-ms", "x.pif", "noext", "x.md.exe",
            "x.bat.", "x.bat ", "x.txt:evil.bat", "x.",
        ] {
            assert!(!system_open_allowed(Path::new(&format!("C:/proj/{name}"))), "{name}");
        }
    }

    #[test]
    fn plain_text_and_source_files_may_use_the_system_fallback() {
        for name in ["README.md", "notes.TXT", "src/lib.rs", "Cargo.toml", "a.json", "run.log", "b.yaml"] {
            assert!(system_open_allowed(Path::new(&format!("C:/proj/{name}"))), "{name}");
        }
    }
}

#[cfg(test)]
mod bold_tests {
    use super::mend_bold;
    use std::prelude::v1::test;

    #[test]
    fn bold_closed_after_punctuation_before_a_letter_gets_a_space() {
        assert_eq!(mend_bold("**建議：**把檔案拆開"), "**建議：** 把檔案拆開");
        assert_eq!(mend_bold("- **嚴重**（高）與 **建議：**照做"), "- **嚴重**（高）與 **建議：** 照做");
    }

    #[test]
    fn bold_that_already_closes_and_code_stay_as_written() {
        for text in ["**建議：** 把", "**done**，ok", "**a:**\nnext", "`**x：**y`", "```\n**x：**y\n```\n"] {
            assert_eq!(mend_bold(text), text);
        }
    }

    #[test]
    fn double_backtick_spans_and_tilde_fences_are_code() {
        for text in [
            "``**x：**y``",
            "a ``code ` **x：**y`` b",
            "~~~\n**x：**y\n~~~\n",
            "````\n```\n**x：**y\n````\n",
            "~~~~\n~~~\n**x：**y\n~~~~\n",
        ] {
            assert_eq!(mend_bold(text), text, "{text:?}");
        }
        assert_eq!(mend_bold("``a`` **建議：**把"), "``a`` **建議：** 把");
        assert_eq!(mend_bold("~~~\ncode\n~~~\n**建議：**把"), "~~~\ncode\n~~~\n**建議：** 把");
        assert_eq!(mend_bold("```\n~~~\n```\n**建議：**把"), "```\n~~~\n```\n**建議：** 把");
    }
}
