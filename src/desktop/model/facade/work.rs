//! View-free work facade.
use gpui_kit::*;
use crate::desktop::{i18n::{Lang, fill, translate, Table}, model::{AppState, Run, RunStatus, Selection, UnitState}, ux::ReadFilter};
use crate::desktop::model::facade::settings::{Store, UxPrefs, ReadMark};
use crate::base;
use sha2::{Digest, Sha256};
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet, VecDeque}, io::Write, path::{Path, PathBuf}, process::Stdio, rc::Rc, sync::Arc, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use crate::desktop::theme::{self, Tokened as _, TypeSize};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;

// ---- Activity and events
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
    pub last_seq: u64,
    pub last_activity_ms: Option<i64>,
    pub last_seen_ms: Option<i64>,
    pub open: Vec<OpenActivity>,
    pub claimed: Vec<SharedString>,
    pub running_units: Vec<SharedString>,
    pub council_round: Option<u32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OpenActivity {
    pub id: SharedString,
    pub module: SharedString,
    pub actor: SharedString,
    pub phase: SharedString,
    pub since_ms: i64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pulse {
    Unknown,
    Live { last_activity_ms: Option<i64> },
    Lost { last_seen_ms: i64 },
}
pub const LOST_AFTER_MS: i64 = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Ok,
    Warn,
    Error,
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EventSummary {
    pub tone: Tone,
    pub en: String,
    pub zh: String,
    pub raw: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeKind {
    MaxTurns {
        turn_limit: Option<u32>,
    },
    DegradedCouncil {
        member: String,
        phase: String,
        round: u64,
        status: String,
        reason: String,
    },
    ScopeFailure {
        unit: String,
        allowed: Vec<String>,
        outside: Vec<String>,
        outside_count: usize,
        ignored: Vec<(String, String)>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub key: String,
    pub kind: NoticeKind,
    pub at_ms: Option<i64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeAction {
    Resume,
    OpenCouncil,
    OpenUnit(String),
}

// ---- Read state
/// FNV-1a over a versioned, length-prefixed canonical UTF-8 representation.
/// Sorting makes the attention key independent of source ordering.
pub fn attention_key(run: &Run) -> u64 {
    let mut canonical = String::from("attention-v1;");
    fn field(s: &mut String, value: &str) { s.push_str(&format!("{}:{};", value.len(), value)); }
    field(&mut canonical, match run.status { RunStatus::Active => "active", RunStatus::Waiting => "waiting", RunStatus::LimitWait => "limit_wait", RunStatus::Interrupted => "interrupted", RunStatus::Blocked => "blocked", RunStatus::Done => "done" });
    let mut needs: Vec<_> = run.needs.iter().map(|n| n.title.as_ref()).collect(); needs.sort_unstable();
    field(&mut canonical, &needs.len().to_string()); for title in needs { field(&mut canonical, title); }
    let mut failed: Vec<_> = run.units.iter().filter(|u| u.state == UnitState::Failed).map(|u| u.id.as_ref()).collect(); failed.sort_unstable();
    field(&mut canonical, &failed.len().to_string()); for id in failed { field(&mut canonical, id); }
    field(&mut canonical, if run.verified_end() { "1" } else { "0" });
    field(&mut canonical, if run.report.is_some() { "1" } else { "0" });
    canonical.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
}
pub fn is_unread(run: &Run, prefs: &UxPrefs) -> bool { prefs.read.get(run.session.as_ref()).map_or(true, |m| m.key != attention_key(run)) }
pub fn is_archived(run: &Run, prefs: &UxPrefs) -> bool { prefs.read.get(run.session.as_ref()).is_some_and(|m| m.archived && m.key == attention_key(run)) }
pub fn visible(run: &Run, prefs: &UxPrefs, f: ReadFilter) -> bool {
    match f { ReadFilter::All => !is_archived(run, prefs), ReadFilter::Unread => is_unread(run, prefs), ReadFilter::Archived => is_archived(run, prefs) }
}
pub fn unread_count(runs: &[Run], prefs: &UxPrefs) -> usize { runs.iter().filter(|r| is_unread(r, prefs)).count() }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAction { MarkRead, MarkUnread, Archive, Unarchive }
pub fn read_actions(run: &Run, prefs: &UxPrefs) -> Vec<ReadAction> {
    vec![if is_unread(run, prefs) { ReadAction::MarkRead } else { ReadAction::MarkUnread }, if is_archived(run, prefs) { ReadAction::Unarchive } else { ReadAction::Archive }]
}
pub fn action_label(a: ReadAction, lang: Lang) -> &'static str {
    match (a, lang) { (ReadAction::MarkRead, Lang::En) => "Mark as read", (ReadAction::MarkRead, Lang::ZhTw) => "標為已讀", (ReadAction::MarkUnread, Lang::En) => "Mark as unread", (ReadAction::MarkUnread, Lang::ZhTw) => "標為未讀", (ReadAction::Archive, Lang::En) => "Archive", (ReadAction::Archive, Lang::ZhTw) => "封存", (ReadAction::Unarchive, Lang::En) => "Move back from archive", (ReadAction::Unarchive, Lang::ZhTw) => "移出封存" }
}
/// Apply an action against the current run snapshot; stale archives never hide updates.
pub fn change_mark(run: &Run, prefs: &mut UxPrefs, action: ReadAction) {
    match action {
        ReadAction::MarkRead => { let archived = is_archived(run, prefs); prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(run), archived }); }
        ReadAction::MarkUnread => { prefs.read.remove(run.session.as_ref()); }
        ReadAction::Archive => { prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(run), archived: true }); }
        ReadAction::Unarchive => { if let Some(mark) = prefs.read.get_mut(run.session.as_ref()) { mark.archived = false; } }
    }
}
pub fn apply_read_action(state: &Entity<AppState>, session: &str, action: ReadAction, cx: &mut App) {
    let run = state.read(cx).runs.iter().find(|r| r.session.as_ref() == session).cloned();
    if let Some(run) = run { Store::edit(cx, |s| change_mark(&run, &mut s.ux, action)); state.update(cx, |_, cx| cx.notify()); }
}
pub fn mark_read(state: &Entity<AppState>, session: &str, cx: &mut App) { apply_read_action(state, session, ReadAction::MarkRead, cx); }
pub fn mark_unread(state: &Entity<AppState>, session: &str, cx: &mut App) { apply_read_action(state, session, ReadAction::MarkUnread, cx); }
pub fn archive(state: &Entity<AppState>, session: &str, cx: &mut App) { apply_read_action(state, session, ReadAction::Archive, cx); }
pub fn unarchive(state: &Entity<AppState>, session: &str, cx: &mut App) { apply_read_action(state, session, ReadAction::Unarchive, cx); }
pub fn mark_all_read(state: &Entity<AppState>, cx: &mut App) {
    let runs = state.read(cx).runs.clone();
    Store::edit(cx, |s| for run in &runs { if !is_archived(run, &s.ux) { change_mark(run, &mut s.ux, ReadAction::MarkRead); } });
    state.update(cx, |_, cx| cx.notify());
}
pub fn set_read_filter(state: &Entity<AppState>, f: ReadFilter, cx: &mut App) { Store::edit(cx, |s| s.ux.read_filter = f); state.update(cx, |_, cx| cx.notify()); }

// ---- Wishes
/// Which runs the Wishes screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WishFilter {
    All,
    Running,
    NeedsYou,
    Done,
}

impl WishFilter {
    pub const ALL: [WishFilter; 4] = [WishFilter::All, WishFilter::Running, WishFilter::NeedsYou, WishFilter::Done];

    /// A run waiting on a batch counts as running: the work is going on and nothing is asked of the user yet.
    pub fn admits(self, status: RunStatus) -> bool {
        match self {
            WishFilter::All => true,
            WishFilter::Running => matches!(status, RunStatus::Active | RunStatus::Waiting | RunStatus::LimitWait),
            WishFilter::NeedsYou => matches!(status, RunStatus::Blocked | RunStatus::Interrupted),
            WishFilter::Done => status == RunStatus::Done,
        }
    }

}
/// Lowercased search terms; a row must contain all of them.
pub fn search_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// True when every term appears in at least one of `fields`.
pub fn matches_terms(fields: &[&str], terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let fields: Vec<String> = fields.iter().map(|field| field.to_lowercase()).collect();
    terms
        .iter()
        .all(|term| fields.iter().any(|field| field.contains(term.as_str())))
}

fn run_matches(run: &Run, terms: &[String]) -> bool {
    matches_terms(&[&run.wish, &run.project], terms)
}

/// How many runs each of [`WishFilter::ALL`] holds, within the current search.
pub fn filter_counts(runs: &[Run], terms: &[String]) -> [usize; 4] {
    let mut counts = [0; 4];
    for run in runs.iter().filter(|run| run_matches(run, terms)) {
        for (count, filter) in counts.iter_mut().zip(WishFilter::ALL) {
            *count += usize::from(filter.admits(run.status));
        }
    }
    counts
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `(month, day, hour, minute)` of a start time written like "Nov 3, 05:15".
fn parse_started(started: &str) -> Option<(u8, u8, u8, u8)> {
    let (month, rest) = started.trim().split_once(' ')?;
    let (day, time) = rest.split_once(',')?;
    let (hour, minute) = time.trim().split_once(':')?;
    let month = MONTHS.iter().position(|name| *name == month)? as u8 + 1;
    Some((
        month,
        day.trim().parse().ok()?,
        hour.parse().ok()?,
        minute.parse().ok()?,
    ))
}

/// The start time as the interface language writes it; anything unreadable stays as written.
pub fn started_label(started: &SharedString, lang: Lang) -> SharedString {
    match (lang, parse_started(started)) {
        (Lang::ZhTw, Some((month, day, hour, minute))) => {
            format!("{month}月{day}日 {hour:02}:{minute:02}").into()
        }
        _ => started.clone(),
    }
}

pub fn can_resume(status: RunStatus) -> bool {
    matches!(status, RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted)
}

/// `text` on a single line, for rows that have room for only one.
pub fn one_line(text: &SharedString) -> SharedString {
    if text.contains(['\n', '\r', '\t']) {
        text.split_whitespace().collect::<Vec<_>>().join(" ").into()
    } else {
        text.clone()
    }
}

pub fn glyph_state(status: RunStatus, verified_end: bool) -> UnitState {
    match status {
        RunStatus::Active => UnitState::Running,
        RunStatus::Done if verified_end => UnitState::Verified,
        RunStatus::Done | RunStatus::Blocked => UnitState::Built,
        RunStatus::Waiting | RunStatus::LimitWait => UnitState::Pending,
        RunStatus::Interrupted => UnitState::Failed,
    }
}

pub fn one_line_limit(text: &str, limit: usize) -> SharedString {
    let start: String = text.trim_start().chars().take(limit).collect();
    start.split_whitespace().collect::<Vec<_>>().join(" ").into()
}


// ---- Control
const RUNNING_ENTRY_HEIGHT_PX: f32 = 56.;
const ASK_WIDTH_PX: f32 = 420.;
pub const RUNNING_ENTRY_HEIGHT: Pixels = px(RUNNING_ENTRY_HEIGHT_PX);
const T: Table = &[
    ("About {}: ", "關於 {}："),
    ("goal {}", "目標 {}"),
    ("module {}", "模組 {}"),
    ("unit {}", "單元 {}"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    Send,
    /// Waiting on a usage limit: nothing can be said to claude, but the wish can be stopped.
    StopOnly,
    Resume,
    ResumeNow,
    AnotherWish,
}

/// What the text box is for in the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Hidden,
    Interjection,
    Reply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportOffer {
    Hidden,
    Ready,
    NotWritten,
}

/// Everything the bar offers for one state of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub primary: Option<Primary>,
    pub entry: Entry,
    pub stop: bool,
    pub open_folder: bool,
    pub report: ReportOffer,
    pub batch: bool,
    pub view_only: bool,
}

/// The design's Control bar table: one primary action per state, the rest quiet.
pub fn offer(status: RunStatus, hosted: bool, has_folder: bool, has_report: bool) -> Offer {
    let nothing = Offer {
        primary: None,
        entry: Entry::Hidden,
        stop: false,
        open_folder: false,
        report: ReportOffer::Hidden,
        batch: false,
        view_only: false,
    };
    match status {
        RunStatus::Active if hosted => Offer {
            primary: Some(Primary::Send),
            entry: Entry::Interjection,
            stop: true,
            ..nothing
        },
        RunStatus::Active => Offer { view_only: true, ..nothing },
        RunStatus::LimitWait if hosted => Offer {
            primary: Some(Primary::StopOnly),
            stop: true,
            ..nothing
        },
        RunStatus::LimitWait => Offer { view_only: true, ..nothing },
        RunStatus::Blocked | RunStatus::Interrupted => Offer {
            primary: Some(Primary::Resume),
            entry: Entry::Reply,
            open_folder: has_folder,
            ..nothing
        },
        RunStatus::Waiting => Offer {
            primary: Some(Primary::ResumeNow),
            batch: true,
            ..nothing
        },
        RunStatus::Done => Offer {
            primary: Some(Primary::AnotherWish),
            open_folder: has_folder,
            report: if has_report { ReportOffer::Ready } else { ReportOffer::NotWritten },
            ..nothing
        },
    }
}

/// What a person's message is about: the selected module or unit, and the goal it serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    pub goal: Option<SharedString>,
    pub module: SharedString,
    pub unit: Option<SharedString>,
}

/// The selected item of `run`, or `None` when nothing is selected or the selection is gone.
pub fn subject_of(run: &Run, selection: &Selection) -> Option<Subject> {
    let (module, unit) = match *selection {
        Selection::None => return None,
        Selection::Module(m) => (m, None),
        Selection::Unit(u) => {
            let unit = run.units.get(u)?;
            (unit.module, Some(unit.id.clone()))
        }
    };
    let found = run.modules.get(module)?;
    let goal = run.goals.iter().find(|g| g.modules.contains(&module)).map(|g| g.id.clone());
    Some(Subject { goal, module: found.id.clone(), unit })
}

/// The words that open the box, such as "About goal G1 · module m1 · unit a1: ". The engine is
/// not changed: the run sees them as the first words of the message.
pub fn address_line(subject: &Subject, lang: Lang) -> String {
    let part = |key: &'static str, id: &str| fill(translate(lang, T, key), &[id]);
    let mut parts = Vec::with_capacity(3);
    if let Some(goal) = &subject.goal {
        parts.push(part("goal {}", goal));
    }
    parts.push(part("module {}", &subject.module));
    if let Some(unit) = &subject.unit {
        parts.push(part("unit {}", unit));
    }
    fill(translate(lang, T, "About {}: "), &[&parts.join(" · ")])
}

pub fn address_text(subject: &Subject, cx: &App) -> String {
    address_line(subject, *cx.global::<Lang>())
}

pub fn clock(local_secs: i64) -> String {
    let day = local_secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

pub fn now_clock() -> SharedString {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    clock(secs + i64::from(crate::session::local_offset_secs(secs))).into()
}

/// A question asked before something that cannot be taken back or would be a surprise.
pub struct Ask {
    pub title: &'static str,
    pub detail: &'static str,
    pub confirm: &'static str,
    pub back: &'static str,
    pub danger: bool,
    /// Which button Enter presses when the question appears.
    pub focus_confirm: bool,
}

/// Opens the question as a dialog. Escape and the back button leave without doing anything.
pub fn ask(question: Ask, window: &mut Window, cx: &mut App, confirmed: impl Fn(&mut Window, &mut App) + 'static) {
    let confirmed = Rc::new(confirmed);
    let confirm_focus = cx.focus_handle();
    let back_focus = cx.focus_handle();
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let handle = if question.focus_confirm { &confirm_focus } else { &back_focus };
            handle.focus(window, cx);
        }
        let d = theme::density(cx);
        let confirmed = confirmed.clone();
        let muted = cx.tokens().muted;
        dialog
            .w(px(ASK_WIDTH_PX))
            .close_button(false)
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .entrance(crate::desktop::motion::DIALOG_ENTRANCE)
            .title(question.title)
            .child(crate::desktop::components::DialogFrame::new("ask-frame", &cx.tokens()).child(crate::desktop::motion::settle_in("ask-detail", div().text_size(theme::font(d, TypeSize::Body)).text_color(muted).child(question.detail))))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("ask-back")
                            .label(question.back)
                            .track_focus(&back_focus)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("ask-confirm")
                            .map(|b| if question.danger { b.danger() } else { theme::gold_leaf(b, &cx.tokens()) })
                            .label(question.confirm)
                            .track_focus(&confirm_focus)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirmed(window, cx);
                            }),
                    ),
            )
    });
}


// ---- Live display
pub fn worker_session<'a>(run: &str, unit: Option<&str>, sessions: &'a [crate::desktop::ux::seams::sessions::WorkerSession]) -> Option<&'a str> {
    let unit = unit?;
    sessions.iter().find(|s| s.run == run && s.unit.as_deref() == Some(unit)).map(|s| s.id.as_str())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerKind { Claude, Codex, Council }
#[derive(Clone, Debug, PartialEq)]
pub struct Worker { pub kind: WorkerKind, pub who: SharedString, pub what: SharedString, pub since_ms: Option<i64>, pub unit: Option<SharedString> }
#[derive(Clone, Debug, PartialEq)]
pub struct LiveStage { pub name: SharedString, pub actor: SharedString, pub since_ms: Option<i64> }
#[derive(Clone, Debug, PartialEq)]
pub struct NoticeCopy { pub title: String, pub cause: String, pub effect: String, pub action: Option<(NoticeAction, String)> }
pub fn workers(run: &Run, p: &Projection, council: Option<&crate::desktop::ux::CouncilView>) -> Vec<Worker> {
    crate::desktop::live::activity::workers(run, p, council).into_iter().map(|w| Worker { kind: match w.kind { crate::desktop::live::activity::WorkerKind::Claude => WorkerKind::Claude, crate::desktop::live::activity::WorkerKind::Codex => WorkerKind::Codex, crate::desktop::live::activity::WorkerKind::Council => WorkerKind::Council }, who:w.who, what:w.what, since_ms:w.since_ms, unit:w.unit }).collect()
}
pub fn live_stage(run: &Run, p: &Projection, now_ms: i64, lang: Lang) -> Option<LiveStage> { crate::desktop::live::activity::live_stage(run,p,now_ms,lang).map(|s| LiveStage { name:s.name, actor:s.actor, since_ms:s.since_ms }) }
pub fn heartbeat_text(p: Pulse, now_ms:i64, lang:Lang) -> String { crate::desktop::live::activity::heartbeat_text(p,now_ms,lang) }
pub fn activity_text(a: &OpenActivity, lang:Lang) -> String { crate::desktop::live::activity::activity_text(a,lang) }
pub fn notice_copy(n:&Notice, lang:Lang) -> NoticeCopy { let n=crate::desktop::live::events::notice_copy(n,lang); NoticeCopy { title:n.title, cause:n.cause, effect:n.effect, action:n.action } }
pub fn board_sentence(text:&str, lang:Lang) -> Option<&'static str> { crate::desktop::demo::board_sentence(text,lang) }
pub fn sample_run() -> Run { crate::desktop::demo::sample_run() }
pub fn sample_state() -> AppState { crate::desktop::demo::state() }

// ---- Plain text engine
const BATCH: usize = 40;
const DEBOUNCE_MILLIS: u64 = 400;
const RUNNER_TIMEOUT_SECS: u64 = 90;
const DEBOUNCE: Duration = Duration::from_millis(DEBOUNCE_MILLIS);
const CLAUDE_ARGS: [&str; 10] = [
    "-p", "--model", "claude-haiku-5-5", "--tools", "", "--strict-mcp-config",
    "--setting-sources", "local", "--output-format", "text",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Plain {
    pub shown: SharedString,
    pub original: Option<SharedString>,
}

pub trait Runner: Send + Sync + 'static {
    fn run(&self, prompt: &str) -> Result<String, String>;
}

pub enum Source {
    Claude { cache: PathBuf },
    Fake(Arc<dyn Runner>),
}

struct Claude;
impl Runner for Claude {
    fn run(&self, prompt: &str) -> Result<String, String> {
        let mut command = base::proc::cmd("claude");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .args(CLAUDE_ARGS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = format!("Translate each string into plain Traditional Chinese for a non-programmer. Keep code identifiers, paths and numbers unchanged. Reply with only the JSON array.\n{prompt}");
        // Writing may block if the child does not read, so it must not prevent the deadline.
        let mut stdin = child.stdin.take().ok_or("missing stdin")?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
        let result = base::proc::capture(child, Duration::from_secs(RUNNER_TIMEOUT_SECS)).map_err(|e| e.message)?;
        if result.timed_out || result.exit_code != 0 {
            return Err("translation process failed or timed out".into());
        }
        String::from_utf8(result.stdout).map_err(|e| e.to_string())
    }
}

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn wants(text: &str) -> bool {
    let mut ascii = false;
    let mut count = 0;
    for c in text.chars() {
        count += 1;
        if count >= 2000
            || matches!(c as u32, 0x1100..=0x11ff | 0x2e80..=0x2fff | 0x3000..=0x30ff | 0x3130..=0x318f | 0x31f0..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xff65..=0xff9f | 0x20000..=0x323af)
        {
            return false;
        }
        ascii |= c.is_ascii_alphabetic();
    }
    ascii
}

#[derive(Default)]
struct Core {
    cache: HashMap<String, String>,
    queue: VecDeque<(String, String)>,
    pending: HashSet<String>,
    attempted: HashSet<String>,
}

impl Core {
    fn lookup(&mut self, text: &str, active: bool) -> Option<String> {
        if !active || !wants(text) {
            return None;
        }
        let key = hash(text);
        if let Some(value) = self.cache.get(&key) {
            return Some(value.clone());
        }
        if !self.attempted.contains(&key) && self.pending.insert(key.clone()) {
            self.queue.push_back((key, text.to_owned()));
        }
        None
    }
    fn clear_queue(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
    fn take_batch(&mut self) -> Vec<(String, String)> {
        let mut batch = Vec::new();
        while batch.len() < BATCH {
            let Some((key, text)) = self.queue.pop_front() else {
                break;
            };
            self.pending.remove(&key);
            if !self.cache.contains_key(&key) {
                self.attempted.insert(key.clone());
                batch.push((key, text));
            }
        }
        batch
    }
    fn accept(&mut self, batch: &[(String, String)], response: &str) -> bool {
        let Ok(values) = serde_json::from_str::<Vec<String>>(response) else {
            return false;
        };
        if values.len() != batch.len() {
            return false;
        }
        for ((key, _), value) in batch.iter().zip(values) {
            self.cache.insert(key.clone(), value);
        }
        true
    }
}

fn load(path: &Path) -> HashMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save(path: &Path, cache: &HashMap<String, String>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec(cache).map_err(std::io::Error::other)?;
    base::atomic_write(path, &json)
}

// Run a single batch on the background executor, including persistence on success.
fn process_batch(core: &mut Core, runner: &dyn Runner, path: Option<&Path>) -> bool {
    let batch = core.take_batch();
    if batch.is_empty() {
        return false;
    }
    let texts: Vec<_> = batch.iter().map(|(_, text)| text).collect();
    let prompt = serde_json::to_string(&texts).expect("strings serialize");
    let success = runner.run(&prompt).ok().is_some_and(|response| core.accept(&batch, &response));
    if success {
        if let Some(path) = path {
            let _ = save(path, &core.cache);
        }
    }
    success
}

struct State {
    on: bool,
    core: Core,
    runner: Arc<dyn Runner>,
    path: Option<PathBuf>,
    loaded: bool,
    running: bool,
    last_queued: Instant,
}
struct GlobalState(Rc<RefCell<State>>);
impl Global for GlobalState {}

pub fn install(cx: &mut App, on: bool, source: Source) {
    let (runner, path): (Arc<dyn Runner>, _) = match source {
        Source::Claude { cache } => (Arc::new(Claude), Some(cache)),
        Source::Fake(runner) => (runner, None),
    };
    cx.set_global(GlobalState(Rc::new(RefCell::new(State {
        on,
        core: Core::default(),
        runner,
        loaded: path.is_none(),
        path,
        running: false,
        last_queued: Instant::now(),
    }))));
}

// called by the screens
#[allow(dead_code)]
pub fn enabled(cx: &App) -> bool {
    cx.try_global::<GlobalState>()
        .is_some_and(|s| s.0.borrow().on)
}
fn active(on: bool, lang: Lang) -> bool {
    on && lang == Lang::ZhTw
}

// called by the screens
#[allow(dead_code)]
pub fn set_enabled(on: bool, cx: &mut App) {
    if let Some(state) = cx.try_global::<GlobalState>() {
        let mut state = state.0.borrow_mut();
        state.on = on;
        if !on {
            state.core.clear_queue();
        }
    }
    cx.refresh_windows();
}

fn plain_result(text: &SharedString, result: Option<String>) -> Plain {
    match result {
        Some(value) if value != text.as_ref() => Plain { shown: value.into(), original: Some(text.clone()) },
        _ => Plain { shown: text.clone(), original: None },
    }
}

// called by the screens
#[allow(dead_code)]
pub fn plain(cx: &App, text: &SharedString) -> Plain {
    let Some(global) = cx.try_global::<GlobalState>() else {
        return plain_result(text, None);
    };
    let lang = *cx.global::<Lang>();
    let mut state = global.0.borrow_mut();
    let on = active(state.on, lang);
    let before = state.core.queue.len();
    let result = state.core.lookup(text.as_ref(), on);
    if state.core.queue.len() != before {
        state.last_queued = Instant::now();
    }
    let start = on && !state.running && !state.core.queue.is_empty();
    if start {
        state.running = true;
    }
    drop(state);
    if start {
        start_worker(cx, global.0.clone());
    }
    plain_result(text, result)
}

fn start_worker(cx: &App, state: Rc<RefCell<State>>) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let path = {
            let s = state.borrow();
            if s.loaded {
                None
            } else {
                s.path.clone()
            }
        };
        if let Some(path) = path {
            let cache = cx.background_spawn(async move { load(&path) }).await;
            let mut s = state.borrow_mut();
            s.core.cache = cache;
            s.loaded = true;
            drop(s);
            let _ = cx.update(|cx| cx.refresh_windows());
        }
        loop {
            loop {
                let delay = DEBOUNCE.saturating_sub(state.borrow().last_queued.elapsed());
                if delay.is_zero() {
                    break;
                }
                cx.background_executor().timer(delay).await;
            }
            let active = cx.update(|cx| *cx.global::<Lang>() == Lang::ZhTw);
            let (mut work, runner, path) = {
                let mut s = state.borrow_mut();
                if !s.on || !active {
                    s.core.clear_queue();
                    s.running = false;
                    return;
                }
                let batch = s.core.take_batch();
                if batch.is_empty() {
                    s.running = false;
                    return;
                }
                let work = Core {
                    cache: s.core.cache.clone(),
                    queue: batch.into(),
                    ..Core::default()
                };
                (work, s.runner.clone(), s.path.clone())
            };
            let (success, cache) = cx
                .background_spawn(async move {
                    let success = process_batch(&mut work, runner.as_ref(), path.as_deref());
                    (success, work.cache)
                })
                .await;
            if success {
                state.borrow_mut().core.cache = cache;
                let _ = cx.update(|cx| cx.refresh_windows());
            }
            let mut s = state.borrow_mut();
            if s.core.queue.is_empty() {
                s.running = false;
                return;
            }
        }
    })
    .detach();
}

/// Deterministic translations for the demo, with no process or persistent cache.
pub struct DemoRunner;
impl Runner for DemoRunner {
    fn run(&self, prompt: &str) -> Result<String, String> {
        let input: Vec<String> = serde_json::from_str(prompt).map_err(|e| e.to_string())?;
        let output: Vec<String> = input
            .iter()
            .map(|text| {
                match text.as_str() {
                    "Screening: 1,467 included, 878 excluded, 5,258 pending" => {
                        return "篩選：納入 1,467，排除 878，待處理 5,258".into()
                    }
                    "Review failed: 1 major finding" => return "審查未通過：1 項重大問題".into(),
                    _ => {}
                }
                let (prefix, suffix) = text.split_once(": ").unwrap_or((text, ""));
                let translated = match prefix {
                    "Paths, hashing and shared I/O" => "路徑、雜湊與共用輸入輸出",
                    "Model calls with a cache" => "使用快取呼叫模型",
                    "Page text and page images" => "頁面文字與圖片",
                    "Find and dedupe PDFs" => "尋找並去除重複的 PDF",
                    "Record schema" => "紀錄格式",
                    "Check citations against pages" => "對照頁面檢查引用",
                    "Screen and read papers" => "篩選並閱讀論文",
                    "Compare reader models" => "比較閱讀模型",
                    "Build the dataset" => "建立資料集",
                    "Process features" => "處理特徵",
                    "Analysis protocol" => "分析流程",
                    "Causal models" => "因果模型",
                    "Six candidate recipes" => "六組候選方案",
                    "Fold the late findings into the summary" => "將後續發現納入摘要",
                    "Usage limit reset; resuming the dispatcher" => {
                        "用量限制已重設，繼續執行 dispatcher"
                    }
                    "VERIFY passed" => "VERIFY 通過",
                    "Reader decision still waiting for the benchmark" => {
                        "閱讀模型的決定仍在等待基準測試"
                    }
                    _ => return text.clone(),
                };
                if suffix.is_empty() {
                    translated.to_owned()
                } else {
                    format!("{translated}: {suffix}")
                }
            })
            .collect();
        serde_json::to_string(&output).map_err(|e| e.to_string())
    }
}


pub fn apply_plain_words(on: bool, cx: &mut App) { set_enabled(on, cx); }

#[cfg(test)]
#[path = "work_accept.rs"]
mod accept;
