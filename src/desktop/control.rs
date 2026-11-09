//! The control bar at the foot of the Run screen: what this run offers right now, and its event log.
use super::i18n::{tr, Table};
use super::model::*;
use super::theme::{Tokened as _, Tokens};
use crate::session::local_offset_secs;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::input::{IndentInline, InputEvent, OutdentInline, Textarea, TextareaState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const T: Table = &[
    ("Send", "送出"),
    ("send", "送出"),
    ("new line", "換行"),
    ("Add to the work, ask a question, or say stop", "補充要求、提問，或叫它停下來"),
    ("Reply to Chronos (optional)", "回覆 Chronos（可不填）"),
    ("Resume now", "立即接續"),
    ("Resuming…", "正在接續…"),
    ("Stopping…", "正在停止…"),
    ("Stop this wish?", "要停止這個願望嗎？"),
    ("It can be resumed later.", "之後可以接續。"),
    ("Keep running", "繼續執行"),
    ("Make another wish", "再許一個願望"),
    ("View report", "查看報告"),
    ("This run did not write a report", "這次執行沒有產生報告"),
    ("Started in terminal: view only", "在終端機啟動：僅供檢視"),
    ("To interject or stop it, use the terminal it runs in.", "要插話或停止，請到執行它的終端機。"),
    ("Chronos needs you before it can continue.", "Chronos 需要你才能繼續。"),
    ("Chronos continues when the batch finishes.", "批次完成後 Chronos 會繼續。"),
    ("Verified {done} of {total}", "已驗收 {done} / {total}"),
    ("{n} not verified", "{n} 個尚未驗收"),
    ("No wish selected. Press Ctrl+N to make one.", "尚未選擇願望。按 Ctrl+N 許願。"),
    ("Not connected to a session, so nothing was sent. Your text is still here.", "目前沒有連上工作階段，沒有送出任何內容。文字還留著。"),
    ("Not connected to a session, so it was not stopped.", "目前沒有連上工作階段，沒有停止。"),
    ("Not connected to a session, so it was not resumed.", "目前沒有連上工作階段，沒有接續。"),
    ("No answer from the session yet. You can try again.", "工作階段還沒有回應，可以再試一次。"),
    ("You", "你"),
    ("Event log", "事件紀錄"),
    ("Show the event log", "顯示事件紀錄"),
    ("Hide the event log", "隱藏事件紀錄"),
    ("All", "全部"),
    ("Info", "資訊"),
    ("OK", "成功"),
    ("Warnings", "警告"),
    ("Errors", "錯誤"),
    ("Jump to latest", "跳到最新"),
    ("{n} new", "{n} 則新紀錄"),
    ("No events yet.", "還沒有事件。"),
    ("No events at this level.", "這個等級沒有事件。"),
    ("Close", "關閉"),
    ("Reading the report…", "正在讀取報告…"),
    ("Could not read the report: {error}", "讀不到報告：{error}"),
    ("Only the beginning is shown: the report is longer.", "報告太長，只顯示開頭。"),
];

const LOG_HEIGHT: Pixels = px(320.);
const PENDING_TIMEOUT: Duration = Duration::from_secs(20);
const REPORT_LIMIT: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Primary {
    Send,
    Resume,
    ResumeNow,
    AnotherWish,
}

/// What the text box is for in the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Hidden,
    Interjection,
    Reply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReportOffer {
    Hidden,
    Ready,
    NotWritten,
}

/// Everything the bar offers for one state of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Offer {
    pub primary: Option<Primary>,
    pub entry: Entry,
    pub stop: bool,
    pub open_folder: bool,
    pub report: ReportOffer,
    pub batch: bool,
    pub view_only: bool,
}

/// The design's Control bar table: one primary action per state, the rest quiet.
pub(super) fn offer(status: RunStatus, hosted: bool, has_folder: bool, has_report: bool) -> Offer {
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
        RunStatus::Blocked => Offer {
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

/// The trimmed text to send, or `None` when it is blank.
pub(super) fn sendable(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

fn you_line(text: &str, time: SharedString, you: &'static str) -> LogLine {
    LogLine {
        time,
        source: you.into(),
        text: text.to_owned().into(),
        level: LogLevel::Info,
    }
}

fn clock(local_secs: i64) -> String {
    let day = local_secs.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

fn now_clock() -> SharedString {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    clock(secs + i64::from(local_offset_secs(secs))).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    All,
    Info,
    Ok,
    Warn,
    Error,
}

impl Level {
    fn admits(self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::Info => level == LogLevel::Info,
            Self::Ok => level == LogLevel::Ok,
            Self::Warn => level == LogLevel::Warn,
            Self::Error => level == LogLevel::Error,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Info => "Info",
            Self::Ok => "OK",
            Self::Warn => "Warnings",
            Self::Error => "Errors",
        }
    }
}

const LEVELS: [Level; 5] = [Level::All, Level::Info, Level::Ok, Level::Warn, Level::Error];

/// What a change in the run's log means for the rows already on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogChange {
    Same,
    Append,
    Rebuild,
}

fn log_change(same_session: bool, seen: usize, now: usize) -> LogChange {
    match (same_session, now.cmp(&seen)) {
        (false, _) | (true, std::cmp::Ordering::Less) => LogChange::Rebuild,
        (true, std::cmp::Ordering::Equal) => LogChange::Same,
        (true, std::cmp::Ordering::Greater) => LogChange::Append,
    }
}

struct LogPanel {
    open: bool,
    level: Level,
    rows: Vec<LogLine>,
    list: ListState,
    seen: usize,
    session: Option<SharedString>,
    unread: usize,
    focus: FocusHandle,
}

impl LogPanel {
    fn new(cx: &mut App) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(240.));
        list.set_follow_mode(FollowMode::Tail);
        Self {
            open: false,
            level: Level::All,
            rows: vec![],
            list,
            seen: 0,
            session: None,
            unread: 0,
            focus: cx.focus_handle(),
        }
    }

    fn rebuild(&mut self, session: Option<SharedString>, lines: &[LogLine]) {
        self.session = session;
        self.seen = lines.len();
        self.unread = 0;
        let level = self.level;
        self.rows = lines.iter().filter(|l| level.admits(l.level)).cloned().collect();
        self.list.reset(self.rows.len());
        self.list.set_follow_mode(FollowMode::Tail);
    }

    fn append(&mut self, lines: &[LogLine]) {
        let before = self.rows.len();
        let level = self.level;
        self.rows.extend(lines[self.seen..].iter().filter(|l| level.admits(l.level)).cloned());
        self.seen = lines.len();
        let added = self.rows.len() - before;
        if added > 0 {
            self.list.splice(before..before, added);
            if !self.list.is_following_tail() {
                self.unread += added;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    NotSent,
    NotStopped,
    NotResumed,
    NoAnswer,
}

impl Notice {
    fn text(self) -> &'static str {
        match self {
            Self::NotSent => "Not connected to a session, so nothing was sent. Your text is still here.",
            Self::NotStopped => "Not connected to a session, so it was not stopped.",
            Self::NotResumed => "Not connected to a session, so it was not resumed.",
            Self::NoAnswer => "No answer from the session yet. You can try again.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingKind {
    Stopping,
    Resuming,
}

struct Pending {
    kind: PendingKind,
    status: RunStatus,
}

enum ReportText {
    Loading,
    Ready { text: SharedString, cut: bool },
    Failed(String),
}

struct ReportView {
    path: Option<PathBuf>,
    text: ReportText,
}

fn cut_report(mut text: String) -> (String, bool) {
    if text.len() <= REPORT_LIMIT {
        return (text, false);
    }
    let mut end = REPORT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

fn read_report(path: &Path) -> Result<(String, bool), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(cut_report(String::from_utf8_lossy(&bytes).into_owned()))
}

/// A question asked before something that cannot be taken back or would be a surprise.
pub(super) struct Ask {
    pub title: &'static str,
    pub detail: &'static str,
    pub confirm: &'static str,
    pub back: &'static str,
    pub danger: bool,
    /// Which button Enter presses when the question appears.
    pub focus_confirm: bool,
}

/// Opens the question as a dialog. Escape and the back button leave without doing anything.
pub(super) fn ask(question: Ask, window: &mut Window, cx: &mut App, confirmed: impl Fn(&mut Window, &mut App) + 'static) {
    let confirmed = Rc::new(confirmed);
    let confirm_focus = cx.focus_handle();
    let back_focus = cx.focus_handle();
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let handle = if question.focus_confirm { &confirm_focus } else { &back_focus };
            handle.focus(window, cx);
        }
        let confirmed = confirmed.clone();
        let muted = cx.tokens().muted;
        dialog
            .w(px(420.))
            .close_button(false)
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .title(question.title)
            .child(div().text_sm().text_color(muted).child(question.detail))
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
                            .map(|b| if question.danger { b.danger() } else { b.primary() })
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

struct Facts {
    offer: Offer,
    status: RunStatus,
    session: SharedString,
    need: Option<(SharedString, SharedString)>,
    verified: usize,
    total: usize,
    cost: Option<f64>,
    log_lines: usize,
}

pub struct Control {
    state: Entity<AppState>,
    input: Entity<TextareaState>,
    session: Option<SharedString>,
    entry: Entry,
    drafts: HashMap<SharedString, String>,
    notice: Option<Notice>,
    pending: Option<Pending>,
    pending_timeout: Option<Task<()>>,
    log: LogPanel,
    report: Option<ReportView>,
    _subscriptions: Vec<Subscription>,
}

impl Control {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx);
            input.set_rows(2, cx);
            input.set_submit_on_enter(true, cx);
            input
        });
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.notice = None;
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => this.submit(window, cx),
                _ => {}
            }),
        ];
        cx.defer_in(window, |this, window, cx| {
            this.sync(window, cx);
            this.apply_scene(window, cx);
        });
        Self {
            log: LogPanel::new(cx),
            state,
            input,
            session: None,
            entry: Entry::Hidden,
            drafts: HashMap::new(),
            notice: None,
            pending: None,
            pending_timeout: None,
            report: None,
            _subscriptions: subscriptions,
        }
    }

    fn facts(&self, cx: &App) -> Option<Facts> {
        let run = self.state.read(cx).run()?;
        Some(Facts {
            offer: offer(run.status, run.hosted, run.cwd.is_some(), run.report.is_some()),
            status: run.status,
            session: run.session.clone(),
            need: run
                .needs
                .iter()
                .find(|n| n.kind != NeedKind::Interjection)
                .map(|n| (n.title.clone(), n.note.clone())),
            verified: run.count(UnitState::Verified),
            total: run.units.len(),
            cost: run.usage.total_usd,
            log_lines: run.log.len(),
        })
    }

    /// Follows the model: keeps drafts per run, the placeholder per state, and clears finished requests.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let facts = self.facts(cx);
        let session = facts.as_ref().map(|f| f.session.clone());
        if session != self.session {
            let draft = self.input.read(cx).value();
            if let Some(old) = self.session.take() {
                if draft.trim().is_empty() {
                    self.drafts.remove(&old);
                } else {
                    self.drafts.insert(old, draft.to_string());
                }
            }
            let restored = session.as_ref().and_then(|s| self.drafts.get(s)).cloned().unwrap_or_default();
            self.input.update(cx, |s, cx| s.set_value(restored, window, cx));
            self.session = session;
            self.notice = None;
            self.pending = None;
            self.pending_timeout = None;
        }
        let entry = facts.as_ref().map_or(Entry::Hidden, |f| f.offer.entry);
        if entry != self.entry {
            self.entry = entry;
            let placeholder = match entry {
                Entry::Reply => tr(cx, T, "Reply to Chronos (optional)"),
                _ => tr(cx, T, "Add to the work, ask a question, or say stop"),
            };
            self.input.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        if let (Some(pending), Some(facts)) = (&self.pending, &facts) {
            if pending.status != facts.status {
                self.pending = None;
                self.pending_timeout = None;
            }
        }
        if self.log.open {
            self.sync_log(cx);
        }
        cx.notify();
    }

    fn sync_log(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(run) = state.run() else {
            self.log.rebuild(None, &[]);
            return;
        };
        let same = self.log.session.as_ref() == Some(&run.session);
        match log_change(same, self.log.seen, run.log.len()) {
            LogChange::Same => {}
            LogChange::Append => self.log.append(&run.log),
            LogChange::Rebuild => self.log.rebuild(Some(run.session.clone()), &run.log),
        }
    }

    fn toggle_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.log.open = !self.log.open;
        if self.log.open {
            self.log.session = None;
            self.sync_log(cx);
            self.log.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn set_level(&mut self, level: Level, cx: &mut Context<Self>) {
        self.log.level = level;
        self.log.session = None;
        self.sync_log(cx);
        cx.notify();
    }

    fn connected(&self, cx: &App) -> bool {
        self.state.read(cx).backend.is_some()
    }

    fn append_you(&mut self, text: &str, cx: &mut Context<Self>) {
        let line = you_line(text, now_clock(), tr(cx, T, "You"));
        self.state.update(cx, |s, _| {
            if let Some(run) = s.current.and_then(|i| s.runs.get_mut(i)) {
                run.log.push(line);
            }
        });
    }

    fn clear_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        if let Some(session) = &self.session {
            self.drafts.remove(session);
        }
    }

    /// Enter in the text box: sends an interjection, or resumes with the reply that was typed.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.facts(cx).map(|f| f.offer.entry) {
            Some(Entry::Interjection) => self.send(window, cx),
            Some(Entry::Reply) if sendable(&self.input.read(cx).value()).is_some() => self.resume(window, cx),
            _ => {}
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value();
        let Some(message) = sendable(&text) else { return };
        if self.pending.is_some() {
            return;
        }
        if !self.connected(cx) {
            self.notice = Some(Notice::NotSent);
            cx.notify();
            return;
        }
        self.append_you(message, cx);
        self.state.update(cx, |s, cx| s.interject(message, cx));
        self.clear_input(window, cx);
        cx.notify();
    }

    fn wait_for(&mut self, kind: PendingKind, status: RunStatus, cx: &mut Context<Self>) {
        self.pending = Some(Pending { kind, status });
        self.pending_timeout = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PENDING_TIMEOUT).await;
            let _ = this.update(cx, |this, cx| {
                this.pending = None;
                this.notice = Some(Notice::NoAnswer);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn ask_stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let view = cx.entity().downgrade();
        ask(
            Ask {
                title: tr(cx, T, "Stop this wish?"),
                detail: tr(cx, T, "It can be resumed later."),
                confirm: tr(cx, T, "Stop"),
                back: tr(cx, T, "Keep running"),
                danger: true,
                focus_confirm: true,
            },
            window,
            cx,
            move |_, cx| {
                let _ = view.update(cx, |this, cx| this.stop(cx));
            },
        );
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.connected(cx) {
            self.notice = Some(Notice::NotStopped);
            cx.notify();
            return;
        }
        self.wait_for(PendingKind::Stopping, RunStatus::Active, cx);
        self.state.update(cx, |s, cx| s.stop_current(cx));
    }

    fn resume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(facts) = self.facts(cx) else { return };
        if self.pending.is_some() {
            return;
        }
        if !self.connected(cx) {
            self.notice = Some(Notice::NotResumed);
            cx.notify();
            return;
        }
        let text = self.input.read(cx).value();
        let reply = (facts.offer.entry == Entry::Reply).then(|| sendable(&text)).flatten();
        if let Some(reply) = reply {
            self.append_you(reply, cx);
        }
        self.wait_for(PendingKind::Resuming, facts.status, cx);
        self.state.update(cx, |s, cx| s.resume_current(reply, cx));
        if facts.offer.entry == Entry::Reply {
            self.clear_input(window, cx);
        }
    }

    fn open_folder(&self, cx: &mut App) {
        let cwd = self.state.read(cx).run().and_then(|r| r.cwd.clone());
        if let Some(cwd) = cwd {
            cx.open_with_system(&cwd);
        }
    }

    fn open_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.state.read(cx).run().and_then(|r| r.report.clone()) else { return };
        match source {
            TextSource::Inline(text) => {
                self.report = Some(ReportView {
                    path: None,
                    text: ReportText::Ready { text, cut: false },
                });
            }
            TextSource::Path(path) => {
                self.report = Some(ReportView {
                    path: Some(path.clone()),
                    text: ReportText::Loading,
                });
                let read = cx.background_spawn(async move { read_report(&path) });
                cx.spawn(async move |this, cx| {
                    let result = read.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(report) = &mut this.report {
                            report.text = match result {
                                Ok((text, cut)) => ReportText::Ready { text: text.into(), cut },
                                Err(error) => ReportText::Failed(error),
                            };
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            view.update(cx, |this, cx| this.fill_report(dialog, window, cx))
        });
    }

    fn fill_report(&mut self, dialog: Dialog, _window: &mut Window, cx: &mut Context<Self>) -> Dialog {
        let t = cx.tokens();
        let Some(report) = &self.report else { return dialog };
        let path = report.path.clone();
        let copied = match &report.text {
            ReportText::Ready { text, .. } => Some(text.clone()),
            _ => None,
        };
        let body: AnyElement = match &report.text {
            ReportText::Loading => div().text_sm().text_color(t.muted).child(tr(cx, T, "Reading the report…")).into_any_element(),
            ReportText::Failed(error) => div()
                .text_sm()
                .text_color(t.fail)
                .child(tr(cx, T, "Could not read the report: {error}").replace("{error}", error))
                .into_any_element(),
            ReportText::Ready { text, cut } => v_flex()
                .gap_2()
                .text_sm()
                .child(div().child(text.clone()))
                .when(*cut, |this| {
                    this.child(div().text_xs().text_color(t.review).child(tr(cx, T, "Only the beginning is shown: the report is longer.")))
                })
                .into_any_element(),
        };
        dialog
            .w(px(720.))
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .title(tr(cx, T, "Report"))
            .child(body)
            .footer(
                DialogFooter::new()
                    .when_some(path, |this, path| {
                        this.child(
                            Button::new("report-open")
                                .icon(IconName::ExternalLink)
                                .label(tr(cx, T, "Open in editor"))
                                .on_click(move |_, _, cx| cx.open_with_system(&path)),
                        )
                    })
                    .child(
                        Button::new("report-copy")
                            .icon(IconName::Copy)
                            .label(tr(cx, T, "Copy"))
                            .disabled(copied.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(text) = &copied {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                                }
                            }),
                    )
                    .child(
                        Button::new("report-close")
                            .primary()
                            .label(tr(cx, T, "Close"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }

    /// Demo only: puts the bar into a named state so it can be captured.
    fn apply_scene(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scene) = self.state.read(cx).scene.clone() else { return };
        let scene = scene.strip_suffix("-min").map_or(scene.as_str(), |base| {
            window.resize(size(px(1100.), px(700.)));
            base
        });
        let select = |status: RunStatus, this: &mut Self, cx: &mut Context<Self>| {
            this.state.update(cx, |s, cx| {
                if let Some(i) = s.runs.iter().position(|r| r.status == status) {
                    s.open_run(i, cx);
                }
            });
        };
        match scene {
            "blocked" => select(RunStatus::Blocked, self, cx),
            "waiting" => select(RunStatus::Waiting, self, cx),
            "done" => select(RunStatus::Done, self, cx),
            "report" => {
                select(RunStatus::Done, self, cx);
                self.sync(window, cx);
                self.open_report(window, cx);
            }
            "view-only" => self.state.update(cx, |s, cx| {
                s.runs[0].hosted = false;
                cx.notify();
            }),
            "stop" => self.ask_stop(window, cx),
            "typing" => self.input.update(cx, |s, cx| s.set_value("先把 causal 的那個失敗修好，再繼續後面的單元。", window, cx)),
            "send-failed" => {
                self.input.update(cx, |s, cx| s.set_value("先把 causal 的那個失敗修好，再繼續後面的單元。", window, cx));
                self.send(window, cx);
            }
            "sent" => {
                self.state.update(cx, |s, _| s.backend = Some(Box::new(SceneBackend)));
                self.input.update(cx, |s, cx| s.set_value("先把 causal 的那個失敗修好。", window, cx));
                self.send(window, cx);
                self.toggle_log(window, cx);
            }
            "stopping" => {
                self.state.update(cx, |s, _| s.backend = Some(Box::new(SceneBackend)));
                self.stop(cx);
            }
            "log" => self.toggle_log(window, cx),
            "log-follow" | "log-paused" => {
                self.grow_log(120, cx);
                self.toggle_log(window, cx);
                let paused = scene == "log-paused";
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(800)).await;
                    let _ = this.update(cx, |this, cx| {
                        if paused {
                            this.log.list.scroll_by(px(-400.));
                        }
                        cx.notify();
                    });
                    for _ in 0..4 {
                        cx.background_executor().timer(Duration::from_millis(500)).await;
                        let _ = this.update(cx, |this, cx| this.grow_log(3, cx));
                    }
                })
                .detach();
            }
            _ => {}
        }
    }

    fn grow_log(&mut self, count: usize, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            let Some(run) = s.current.and_then(|i| s.runs.get_mut(i)) else { return };
            for _ in 0..count {
                let n = run.log.len();
                run.log.push(LogLine {
                    time: format!("10:{:02}:{:02}", n / 60 % 60, n % 60).into(),
                    source: "read_batch".into(),
                    text: format!("Line {n}: screened another batch of papers and kept the page references").into(),
                    level: if n % 11 == 0 { LogLevel::Warn } else if n % 17 == 0 { LogLevel::Error } else { LogLevel::Info },
                });
            }
            cx.notify();
        });
    }

    fn log_toggle(&self, log_lines: usize, cx: &mut Context<Self>) -> Button {
        let open = self.log.open;
        Button::new("control-log")
            .ghost()
            .small()
            .icon(if open { IconName::ChevronDown } else { IconName::ChevronUp })
            .label(format!("{} {log_lines}", tr(cx, T, "Event log")))
            .tooltip(tr(cx, T, if open { "Hide the event log" } else { "Show the event log" }))
            .selected(open)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_log(window, cx)))
    }

    fn folder_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("control-folder")
            .ghost()
            .small()
            .icon(IconName::FolderOpen)
            .label(tr(cx, T, "Open folder"))
            .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx)))
    }

    fn hints(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        h_flex()
            .gap_1p5()
            .text_xs()
            .text_color(t.muted)
            .child(Kbd::new(Keystroke::parse("enter").expect("valid keystroke")))
            .child(tr(cx, T, "send"))
            .child(div().w_2())
            .child(Kbd::new(Keystroke::parse("shift-enter").expect("valid keystroke")))
            .child(tr(cx, T, "new line"))
    }

    fn notice_line(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = cx.tokens();
        let notice = self.notice?;
        Some(
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.fail)
                .child(Icon::new(IconName::CircleAlert).size(px(13.)))
                .child(tr(cx, T, notice.text())),
        )
    }

    fn entry_box(&self, height: Pixels) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .capture_action(|_: &IndentInline, window, cx| {
                cx.stop_propagation();
                window.focus_next(cx);
            })
            .capture_action(|_: &OutdentInline, window, cx| {
                cx.stop_propagation();
                window.focus_prev(cx);
            })
            .child(Textarea::new(&self.input).small().h(height))
    }

    fn resume_button(&self, id: &'static str, label: &'static str, window: &Window, cx: &mut Context<Self>) -> Button {
        let _ = window;
        let resuming = matches!(&self.pending, Some(p) if p.kind == PendingKind::Resuming);
        Button::new(id)
            .primary()
            .small()
            .icon(IconName::Play)
            .label(if resuming { tr(cx, T, "Resuming…") } else { tr(cx, T, label) })
            .loading(resuming)
            .disabled(self.pending.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.resume(window, cx)))
    }

    fn render_running(&mut self, facts: &Facts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let blank = sendable(&self.input.read(cx).value()).is_none();
        let stopping = matches!(&self.pending, Some(p) if p.kind == PendingKind::Stopping);
        let t = cx.tokens();
        let _ = window;
        v_flex()
            .size_full()
            .gap_1p5()
            .child(
                h_flex()
                    .justify_between()
                    .child(match self.notice_line(cx) {
                        Some(notice) => notice.into_any_element(),
                        None => self.hints(cx).into_any_element(),
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(self.log_toggle(facts.log_lines, cx))
                            .child(
                                Button::new("control-stop")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Square).text_color(t.fail))
                                    .label(if stopping { tr(cx, T, "Stopping…") } else { tr(cx, T, "Stop") })
                                    .loading(stopping)
                                    .disabled(self.pending.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| this.ask_stop(window, cx))),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_end()
                    .gap_2()
                    .child(self.entry_box(px(56.)))
                    .child(
                        Button::new("control-send")
                            .primary()
                            .icon(IconName::ArrowUp)
                            .label(tr(cx, T, "Send"))
                            .disabled(blank || self.pending.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_view_only(&mut self, facts: &Facts, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        h_flex()
            .size_full()
            .justify_between()
            .gap_4()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(Icon::new(IconName::SquareTerminal).size(px(20.)).text_color(t.muted))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().child(tr(cx, T, "Started in terminal: view only")))
                            .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "To interject or stop it, use the terminal it runs in."))),
                    ),
            )
            .child(self.log_toggle(facts.log_lines, cx))
            .into_any_element()
    }

    fn need_header(&mut self, facts: &Facts, icon: IconName, fallback_title: &'static str, fallback_note: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let color = t.status(facts.status);
        let (title, note) = facts.need.clone().unwrap_or_else(|| (tr(cx, T, fallback_title).into(), tr(cx, T, fallback_note).into()));
        let tip = note.clone();
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(Icon::new(icon).size(px(18.)).text_color(color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(title))
                    .child(
                        div()
                            .id("control-note")
                            .text_xs()
                            .text_color(t.muted)
                            .truncate()
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .child(note),
                    ),
            )
    }

    fn render_blocked(&mut self, facts: &Facts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let header = self.need_header(facts, IconName::TriangleAlert, "Needs you", "Chronos needs you before it can continue.", cx);
        v_flex()
            .size_full()
            .gap_1p5()
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(header)
                    .child(
                        h_flex()
                            .gap_1()
                            .when(facts.offer.open_folder, |this| this.child(self.folder_button(cx)))
                            .child(self.log_toggle(facts.log_lines, cx)),
                    ),
            )
            .when_some(self.notice_line(cx), |this, notice| this.child(notice))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_end()
                    .gap_2()
                    .child(self.entry_box(px(34.)))
                    .child(self.resume_button("control-resume", "Resume", window, cx)),
            )
            .into_any_element()
    }

    fn render_waiting(&mut self, facts: &Facts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let header = self.need_header(facts, IconName::LoaderCircle, "Waiting on batch", "Chronos continues when the batch finishes.", cx);
        v_flex()
            .size_full()
            .gap_1p5()
            .child(
                h_flex()
                    .flex_1()
                    .justify_between()
                    .gap_3()
                    .child(header)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.log_toggle(facts.log_lines, cx))
                            .child(self.resume_button("control-resume-now", "Resume now", window, cx)),
                    ),
            )
            .when_some(self.notice_line(cx), |this, notice| this.child(notice))
            .into_any_element()
    }

    fn render_done(&mut self, facts: &Facts, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let unverified = facts.total.saturating_sub(facts.verified);
        let summary = tr(cx, T, "Verified {done} of {total}")
            .replace("{done}", &facts.verified.to_string())
            .replace("{total}", &facts.total.to_string());
        let cost = facts.cost.map_or("—".to_owned(), |c| format!("${c:.2}"));
        h_flex()
            .size_full()
            .justify_between()
            .gap_4()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(Icon::new(IconName::CircleCheck).size(px(20.)).text_color(t.ok))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_sm().child(tr(cx, T, "Done")))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(summary)
                                    .when(unverified > 0, |this| {
                                        this.child(div().text_color(t.review).child(tr(cx, T, "{n} not verified").replace("{n}", &unverified.to_string())))
                                    })
                                    .child(cost),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .when(facts.offer.open_folder, |this| this.child(self.folder_button(cx)))
                    .when(facts.offer.report != ReportOffer::Hidden, |this| {
                        let ready = facts.offer.report == ReportOffer::Ready;
                        this.child(
                            Button::new("control-report")
                                .ghost()
                                .small()
                                .icon(IconName::FileText)
                                .label(tr(cx, T, "View report"))
                                .disabled(!ready)
                                .when(!ready, |b| b.tooltip(tr(cx, T, "This run did not write a report")))
                                .on_click(cx.listener(|this, _, window, cx| this.open_report(window, cx))),
                        )
                    })
                    .child(self.log_toggle(facts.log_lines, cx))
                    .child(div().w_1())
                    .child(
                        Button::new("control-another")
                            .primary()
                            .small()
                            .icon(IconName::Plus)
                            .label(tr(cx, T, "Make another wish"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.state.update(cx, |s, cx| s.set_composing(true, cx));
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_log_row(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(line) = self.log.rows.get(ix).cloned() else { return div().into_any_element() };
        let t = cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let source = line.source.clone();
        h_flex()
            .id(("log-row", ix))
            .w_full()
            .items_start()
            .gap_2()
            .px_3()
            .py(px(3.))
            .text_xs()
            .hover(|this| this.bg(t.raised))
            .child(div().w(px(56.)).flex_none().text_color(t.muted).font_family(mono).child(line.time.clone()))
            .child(div().pt(px(1.)).child(Icon::new(level_icon(line.level)).size(px(13.)).text_color(t.log(line.level))))
            .child(
                div()
                    .id(("log-source", ix))
                    .w(px(112.))
                    .flex_none()
                    .truncate()
                    .text_color(t.muted)
                    .tooltip(move |window, cx| Tooltip::new(source.clone()).build(window, cx))
                    .child(line.source.clone()),
            )
            .child(div().flex_1().min_w_0().text_color(t.text).child(line.text.clone()))
            .into_any_element()
    }

    fn render_log(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        if self.log.list.is_following_tail() {
            self.log.unread = 0;
        }
        let run = self.state.read(cx).run();
        let counts: [usize; 5] = std::array::from_fn(|i| {
            run.map_or(0, |r| r.log.iter().filter(|l| LEVELS[i].admits(l.level)).count())
        });
        let empty = self.log.rows.is_empty();
        let empty_text = if counts[0] == 0 { "No events yet." } else { "No events at this level." };
        let focused = self.log.focus.is_focused(window);
        let unread = self.log.unread;
        let paused = !self.log.list.is_following_tail() && !empty;
        let levels = LEVELS.iter().enumerate().map(|(i, &level)| {
            Button::new(("log-level", i))
                .ghost()
                .xsmall()
                .selected(self.log.level == level)
                .label(format!("{} {}", tr(cx, T, level.label()), counts[i]))
                .on_click(cx.listener(move |this, _, _, cx| this.set_level(level, cx)))
        });
        let list = list(self.log.list.clone(), cx.processor(Self::render_log_row)).size_full();
        v_flex()
            .id("control-log-panel")
            .track_focus(&self.log.focus.clone().tab_stop(true))
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .h(LOG_HEIGHT)
            .bg(t.surface)
            .border_1()
            .border_color(if focused { t.gold } else { t.border })
            .rounded_t(px(8.))
            .shadow_lg()
            .occlude()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.log_key(event, cx)))
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().text_sm().child(tr(cx, T, "Event log")))
                            .child(h_flex().gap_0p5().children(levels)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .when(paused, |this| {
                                this.child(
                                    Button::new("log-latest")
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::ArrowDown)
                                        .label(if unread > 0 {
                                            format!("{} · {}", tr(cx, T, "Jump to latest"), tr(cx, T, "{n} new").replace("{n}", &unread.to_string()))
                                        } else {
                                            tr(cx, T, "Jump to latest").to_owned()
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.log.list.set_follow_mode(FollowMode::Tail);
                                            this.log.unread = 0;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("log-close")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(tr(cx, T, "Hide the event log"))
                                    .on_click(cx.listener(|this, _, window, cx| this.toggle_log(window, cx))),
                            ),
                    ),
            )
            .child(if empty {
                h_flex()
                    .flex_1()
                    .justify_center()
                    .text_sm()
                    .text_color(t.muted)
                    .child(tr(cx, T, empty_text))
                    .into_any_element()
            } else {
                div()
                    .id("control-log-list")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .py_1()
                    .child(list)
                    .vertical_scrollbar(&self.log.list)
                    .into_any_element()
            })
            .into_any_element()
    }

    fn log_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        const LINE: f32 = 24.;
        const PAGE: f32 = 240.;
        match event.keystroke.key.as_str() {
            "up" => self.log.list.scroll_by(px(-LINE)),
            "down" => self.log.list.scroll_by(px(LINE)),
            "pageup" => self.log.list.scroll_by(px(-PAGE)),
            "pagedown" => self.log.list.scroll_by(px(PAGE)),
            "home" => self.log.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) }),
            "end" => {
                self.log.list.set_follow_mode(FollowMode::Tail);
                self.log.unread = 0;
            }
            "escape" => self.log.open = false,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
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

struct SceneBackend;

impl Backend for SceneBackend {
    fn start_wish(&self, _request: WishRequest) {}
    fn interject(&self, _session: &str, _text: &str) {}
    fn stop(&self, _session: &str) {}
    fn resume(&self, _session: &str, _note: Option<&str>) {}
    fn delete_memory(&self, _id: i64) {}
    fn refresh(&self) {}
}

impl Render for Control {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Tokens = cx.tokens();
        let facts = self.facts(cx);
        let body = match &facts {
            None => h_flex()
                .size_full()
                .text_sm()
                .text_color(t.muted)
                .child(tr(cx, T, "No wish selected. Press Ctrl+N to make one."))
                .into_any_element(),
            Some(f) => match f.offer.primary {
                Some(Primary::Send) => self.render_running(f, window, cx),
                Some(Primary::Resume) => self.render_blocked(f, window, cx),
                Some(Primary::ResumeNow) => self.render_waiting(f, window, cx),
                Some(Primary::AnotherWish) => self.render_done(f, cx),
                None => self.render_view_only(f, cx),
            },
        };
        let log = (self.log.open && facts.is_some()).then(|| self.render_log(window, cx));
        div()
            .size_full()
            .relative()
            .px_4()
            .py_2p5()
            .bg(t.surface)
            .border_t_1()
            .border_color(t.border)
            .child(body)
            .children(log.map(deferred))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_here_offers_send_and_stop() {
        let o = offer(RunStatus::Active, true, true, false);
        assert_eq!(o.primary, Some(Primary::Send));
        assert_eq!(o.entry, Entry::Interjection);
        assert!(o.stop);
        assert!(!o.view_only);
        assert!(!o.open_folder);
    }

    #[test]
    fn running_elsewhere_is_view_only_and_offers_nothing() {
        let o = offer(RunStatus::Active, false, true, true);
        assert_eq!(o.primary, None);
        assert_eq!(o.entry, Entry::Hidden);
        assert!(!o.stop);
        assert!(o.view_only);
    }

    #[test]
    fn blocked_offers_resume_with_a_reply_whoever_started_it() {
        for hosted in [true, false] {
            let o = offer(RunStatus::Blocked, hosted, true, false);
            assert_eq!(o.primary, Some(Primary::Resume));
            assert_eq!(o.entry, Entry::Reply);
            assert!(o.open_folder);
            assert!(!o.stop);
        }
    }

    #[test]
    fn waiting_offers_resume_now_and_the_batch() {
        for hosted in [true, false] {
            let o = offer(RunStatus::Waiting, hosted, true, false);
            assert_eq!(o.primary, Some(Primary::ResumeNow));
            assert_eq!(o.entry, Entry::Hidden);
            assert!(o.batch);
        }
    }

    #[test]
    fn done_offers_another_wish_the_folder_and_the_report() {
        for hosted in [true, false] {
            let o = offer(RunStatus::Done, hosted, true, true);
            assert_eq!(o.primary, Some(Primary::AnotherWish));
            assert!(o.open_folder);
            assert_eq!(o.report, ReportOffer::Ready);
        }
    }

    #[test]
    fn a_missing_folder_or_report_is_not_offered_as_working() {
        let blocked = offer(RunStatus::Blocked, false, false, false);
        assert!(!blocked.open_folder);
        let done = offer(RunStatus::Done, false, false, false);
        assert!(!done.open_folder);
        assert_eq!(done.report, ReportOffer::NotWritten);
    }

    #[test]
    fn only_a_run_started_here_and_running_can_be_stopped() {
        let cases = [
            (RunStatus::Active, true, true),
            (RunStatus::Active, false, false),
            (RunStatus::Blocked, true, false),
            (RunStatus::Waiting, true, false),
            (RunStatus::Done, true, false),
        ];
        for (status, hosted, can_stop) in cases {
            assert_eq!(offer(status, hosted, true, true).stop, can_stop, "{status:?} {hosted}");
        }
    }

    #[test]
    fn blank_text_cannot_be_sent() {
        assert_eq!(sendable(""), None);
        assert_eq!(sendable("   \n\t "), None);
        assert_eq!(sendable("  hello\n"), Some("hello"));
        assert_eq!(sendable("第一行\n第二行"), Some("第一行\n第二行"));
    }

    #[test]
    fn a_sent_message_is_a_log_line_from_you() {
        let line = you_line("先修這個", "09:05:03".into(), "你");
        assert_eq!(line.text.as_ref(), "先修這個");
        assert_eq!(line.time.as_ref(), "09:05:03");
        assert_eq!(line.source.as_ref(), "你");
        assert_eq!(line.level, LogLevel::Info);
    }

    #[test]
    fn clock_shows_local_hours_minutes_seconds() {
        assert_eq!(clock(0), "00:00:00");
        assert_eq!(clock(3 * 3600 + 7 * 60 + 9), "03:07:09");
        assert_eq!(clock(86_400 + 61), "00:01:01");
        assert_eq!(clock(-1), "23:59:59");
    }

    #[test]
    fn log_changes_are_classified() {
        assert_eq!(log_change(true, 5, 5), LogChange::Same);
        assert_eq!(log_change(true, 5, 8), LogChange::Append);
        assert_eq!(log_change(true, 5, 3), LogChange::Rebuild);
        assert_eq!(log_change(false, 5, 5), LogChange::Rebuild);
    }

    #[test]
    fn log_levels_filter_as_labelled() {
        let levels = [LogLevel::Info, LogLevel::Ok, LogLevel::Warn, LogLevel::Error];
        for level in levels {
            assert!(Level::All.admits(level));
        }
        assert!(Level::Warn.admits(LogLevel::Warn));
        assert!(!Level::Warn.admits(LogLevel::Error));
        assert!(!Level::Error.admits(LogLevel::Warn));
        assert!(Level::Ok.admits(LogLevel::Ok));
        assert!(!Level::Info.admits(LogLevel::Ok));
    }

    #[test]
    fn long_reports_are_cut_at_a_character_boundary() {
        let (text, cut) = cut_report("短".to_owned());
        assert_eq!((text.as_str(), cut), ("短", false));
        let long = "一".repeat(REPORT_LIMIT);
        let (text, cut) = cut_report(long);
        assert!(cut);
        assert!(text.len() <= REPORT_LIMIT);
        assert!(text.chars().all(|c| c == '一'));
    }
}
