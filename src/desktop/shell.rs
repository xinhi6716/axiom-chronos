//! The window frame: title bar, navigation rail and the active screen. It also owns what applies
//! across screens: the window-wide shortcuts (Ctrl+N, Ctrl+K and ?), the command palette's jumps,
//! Needs you notifications and the question asked before closing.
use super::control::{ask, Ask};
use super::i18n::{fill, tr, Lang, Table};
use super::launch::{self, Launch, Square};
use super::model::*;
use super::motion::{self, mix, Curve, Driver, Pace, Tween};
use super::ornament::Emblem;
use super::palette::{self, Command, NewWish, OpenPalette, Overlay, Palette, PaletteEvent, ShowShortcuts, Target};
use super::settings::{LanguageChoice, ThemeChoice};
use super::{ornament, settings};
use super::theme::{self, Tokened as _, Tokens};
use super::{composer::Composer, doctor::DoctorView, memory::MemoryView, run_view::RunView, usage::UsageView, wishes::WishesView};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Placement, Selectable as _, StyledExt as _, TitleBar, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

const T: Table = &[
    ("What the current wish is doing", "目前的願望進行到哪裡"),
    ("Every wish, newest first", "所有願望，最新的在前"),
    ("Every wish, newest first. {} need you.", "所有願望，最新的在前。其中 {} 個需要你。"),
    ("What Chronos remembers between wishes", "Chronos 在願望之間記住的事"),
    ("Cost and context by wish", "每個願望的花費與內容量"),
    ("Environment checks and preferences", "環境檢查與偏好設定"),
    ("{} not ready", "{} 尚未就緒"),
    ("You can't make a wish yet", "還不能許願"),
    ("{}: {}", "{}：{}"),
    ("Open Settings", "開啟設定"),
    ("Needs you: {}", "需要你：{}"),
    ("Waiting on batch: {}", "等待批次：{}"),
    ("Finished: {}", "已結束：{}"),
    ("{} of {} units verified", "已驗收 {} / {} 個單元"),
    ("Open", "開啟"),
    ("A wish is still running", "還有願望在進行"),
    (
        "\"{}\" is still running. Closing the window stops it; you can resume it later.",
        "「{}」還在進行。關閉視窗會停止它，之後可以接續。",
    ),
    (
        "{} wishes are still running. Closing the window stops them; you can resume them later.",
        "有 {} 個願望還在進行。關閉視窗會停止它們，之後可以接續。",
    ),
    ("Keep working", "繼續工作"),
    ("Stop and close", "停止並關閉"),
    ("Stopping…", "正在停止…"),
    ("The wish could not start", "願望無法開始"),
    ("Your message was not delivered", "你的訊息沒有送達"),
    ("The wish was not stopped", "願望沒有停止"),
    ("The wish did not resume", "願望沒有接續"),
    ("The memory was not deleted", "記憶沒有刪除"),
    ("The correction was not saved", "更正沒有儲存"),
    ("The runs could not be refreshed", "無法更新執行狀態"),
    ("The environment could not be checked", "無法檢查環境"),
    ("A wish stopped with an error", "有個願望因錯誤而停止"),
    ("Some saved data could not be read", "有些儲存的資料讀不到"),
    ("The connection to the engine stopped", "與引擎的連線已中斷"),
    ("Nothing was changed. Check Settings, then try again.", "沒有任何變更。請到設定檢查後再試一次。"),
    ("The run did not receive it. Check that it is still going, then send it again.", "這次執行沒有收到。請確認它還在進行，再送一次。"),
    ("It may still be working. Try again, or stop it where it was started.", "它可能還在工作。請再試一次，或到啟動它的地方停止。"),
    ("It is still stopped. Read the details, then try again.", "它仍然是停止的。請看詳細內容後再試一次。"),
    ("It is still stored. Try again.", "它還在，請再試一次。"),
    ("The old text is still in use. Try again.", "仍在使用舊的內容，請再試一次。"),
    ("What you see may be out of date.", "畫面上的內容可能不是最新的。"),
    ("The results shown may be out of date.", "顯示的結果可能不是最新的。"),
    ("Open it to see how far it got. It can be resumed.", "打開來看看進行到哪裡，之後可以接續。"),
    ("The rest of the app keeps working; the affected panel may be incomplete.", "其他功能不受影響，相關面板可能不完整。"),
    ("Nothing will update any more. Restart Chronos.", "畫面不會再更新，請重新啟動 Chronos。"),
    ("Copy details", "複製詳細內容"),
    ("Search", "搜尋"),
    ("Search wishes, goals, modules, units and memory, or run a command", "搜尋願望、目標、模組、單元與記憶，或執行指令"),
    ("Stop this wish?", "要停止這個願望嗎？"),
    ("It can be resumed later.", "之後可以接續。"),
    ("Keep running", "繼續執行"),
    ("Not connected to the engine, so nothing was sent.", "沒有連上引擎，所以沒有送出任何指令。"),
];

/// How long "Stop and close" waits for the runs to report that they stopped before closing anyway.
const STOP_DEADLINE: Duration = Duration::from_secs(10);
/// How long closing the window waits for the engine to drop its wishes before giving up.
const ENGINE_DEADLINE: Duration = Duration::from_secs(5);
const TOAST_MESSAGE_CHARS: usize = 120;
/// How far a new screen drifts up into place as it crossfades in.
const SCREEN_DRIFT: f32 = 10.0;

/// The rail's screens, in order.
const SCREENS: [Screen; 5] = [Screen::Run, Screen::Wishes, Screen::Memory, Screen::Usage, Screen::Doctor];

fn slot(screen: Screen) -> usize {
    SCREENS.iter().position(|s| *s == screen).unwrap_or(0)
}

/// Demo only: splits a scene such as `switch-at-120` into what to do and when to hold it.
pub fn held_scene(scene: &str) -> Option<(&str, u64)> {
    let (what, ms) = scene.rsplit_once("-at-")?;
    Some((what, ms.parse().ok()?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Blocked,
    Waiting,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAlert {
    pub session: SharedString,
    pub kind: AlertKind,
}

/// Records each run's status and returns the runs that just entered a state worth telling the
/// user about. A run seen for the first time is a baseline, so opening the app announces nothing.
pub fn run_alerts(seen: &mut HashMap<SharedString, RunStatus>, runs: &[Run]) -> Vec<RunAlert> {
    let mut alerts = vec![];
    for run in runs {
        let before = seen.insert(run.session.clone(), run.status);
        if before.is_none_or(|status| status == run.status) {
            continue;
        }
        let kind = match run.status {
            RunStatus::Active | RunStatus::LimitWait => continue,
            RunStatus::Blocked | RunStatus::Interrupted => AlertKind::Blocked,
            RunStatus::Waiting => AlertKind::Waiting,
            RunStatus::Done => AlertKind::Finished,
        };
        alerts.push(RunAlert { session: run.session.clone(), kind });
    }
    alerts
}

/// What the Wishes badge counts.
pub fn needs_user(run: &Run) -> bool {
    match run.status {
        RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted => true,
        RunStatus::Active | RunStatus::LimitWait => !run.needs.is_empty(),
        RunStatus::Done => false,
    }
}

/// The first required check that failed. Optional tools report a warning and never block.
pub fn blocking_check(checks: &[Check]) -> Option<&Check> {
    checks.iter().find(|check| check.status == CheckStatus::Fail)
}

/// Wishes this app started that are still working: the ones closing the window would stop.
pub fn running_here(runs: &[Run]) -> Vec<&Run> {
    runs.iter().filter(|run| run.hosted && matches!(run.status, RunStatus::Active | RunStatus::LimitWait)).collect()
}

fn clipped(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => format!("{}…", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

/// A count that stays legible in both themes, which the kit's white-on-color badge does not.
fn count_badge(count: usize, t: &Tokens) -> impl IntoElement {
    h_flex()
        .absolute()
        .top(px(-6.))
        .right(px(-9.))
        .h(px(15.))
        .min_w(px(15.))
        .px(px(3.))
        .justify_center()
        .rounded_full()
        .bg(t.fail)
        .border_1()
        .border_color(t.surface)
        .text_size(px(10.))
        .font_semibold()
        .text_color(t.canvas)
        .child(if count > 9 { "9+".to_string() } else { count.to_string() })
}

/// Sends a stop or resume for the run on screen; without an engine, says that nothing was sent.
fn command_or_report(state: &mut AppState, action: super::model::Action, offline: &'static str, cx: &mut Context<AppState>) {
    let session = state.run().map(|run| run.session.clone());
    if state.backend.is_none() {
        state.report(action, session, offline);
        cx.notify();
        return;
    }
    match action {
        super::model::Action::Stop => state.stop_current(cx),
        _ => state.resume_current(None, cx),
    }
}

fn open_session(state: &Entity<AppState>, session: &str, cx: &mut App) {
    state.update(cx, |state, cx| {
        if let Some(index) = state.runs.iter().position(|run| run.session.as_ref() == session) {
            state.open_run(index, cx);
        }
    });
}

struct NeedsYouToast;
struct SetupToast;
struct ErrorToast;

pub struct Shell {
    state: Entity<AppState>,
    run: Entity<RunView>,
    wishes: Entity<WishesView>,
    memory: Entity<MemoryView>,
    usage: Entity<UsageView>,
    doctor: Entity<DoctorView>,
    composer: Entity<Composer>,
    palette: Entity<Palette>,
    may_flash: bool,
    seen: HashMap<SharedString, RunStatus>,
    toasts: HashSet<SharedString>,
    viewing: Option<SharedString>,
    asking_close: bool,
    stopping: bool,
    stop_deadline: Option<Task<()>>,
    /// The newest entry of `AppState::errors` already shown.
    errors_shown: u64,
    /// Holds the keyboard when nothing else does, so window-wide keys see a key context; `?`
    /// is bound outside text boxes, which needs one.
    focus: FocusHandle,
    /// The opening ritual, while it plays.
    launch: Launch,
    /// Where the rail's portrait was last drawn, so the launch's keeper can land on it.
    rail_portrait: Rc<Cell<Option<Square>>>,
    /// The screen last drawn, the one a switch is leaving, and how far the switch has come.
    shown: Screen,
    leaving: Option<Screen>,
    switch: Tween,
    /// How lit and how hovered each rail item is, so selection and hover glows fade.
    lit: [Tween; 5],
    hover: [Tween; 5],
    hovered: Option<Screen>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    /// Plays the launch over this window.
    pub fn with_launch(mut self, launch: Launch) -> Self {
        self.launch = launch;
        self
    }

    /// Any key or click during the launch ends it at once. Returns whether it was playing.
    fn skip_launch(&mut self, cx: &mut Context<Self>) -> bool {
        // A held demo frame is a still picture: stray input must not end it.
        if !self.launch.playing(motion::now(cx)) || motion::is_frozen(cx) {
            return false;
        }
        self.launch.skip();
        cx.notify();
        true
    }

    fn screen(&self, screen: Screen) -> AnyElement {
        match screen {
            Screen::Run => self.run.clone().into_any_element(),
            Screen::Wishes => self.wishes.clone().into_any_element(),
            Screen::Memory => self.memory.clone().into_any_element(),
            Screen::Usage => self.usage.clone().into_any_element(),
            Screen::Doctor => self.doctor.clone().into_any_element(),
        }
    }

    pub fn new(state: Entity<AppState>, may_flash: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &state.read(cx).runs);
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.state_changed(window, cx)),
            cx.observe_window_bounds(window, |_, window, cx| {
                settings::remember_window(window.window_bounds(), cx);
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                settings::system_appearance_changed(window, cx);
            }),
        ];
        // The window-wide keys come from the shortcut table, so the overlay can never disagree with them.
        cx.bind_keys(palette::global_bindings());
        Self::on_global::<NewWish>(window, cx, Self::new_wish);
        Self::on_global::<OpenPalette>(window, cx, Self::toggle_palette);
        Self::on_global::<ShowShortcuts>(window, cx, Self::toggle_shortcuts);
        let palette = cx.new(|cx| Palette::new(state.clone(), window, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut subscriptions = subscriptions;
        subscriptions.extend(motion::watch_window(window, cx));
        subscriptions.push(cx.subscribe_in(&palette, window, |this, _, event: &PaletteEvent, window, cx| {
            let PaletteEvent::Chose(target) = event;
            this.go_to(target.clone(), window, cx);
        }));
        let shell = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            shell.update(cx, |shell, cx| shell.should_close(window, cx)).unwrap_or(true)
        });
        let scene = state.read(cx).scene.clone();
        let shown = state.read(cx).view;
        let glow = |on: bool| Tween::rest(if on { 1.0 } else { 0.0 }, Pace::Glow, Curve::EaseOut);
        if let Some(scene) = scene {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(400)).await;
                let _ = this.update_in(cx, |this, window, cx| this.play_scene(&scene, window, cx));
            })
            .detach();
        }
        Self {
            run: cx.new(|cx| RunView::new(state.clone(), window, cx)),
            wishes: cx.new(|cx| WishesView::new(state.clone(), window, cx)),
            memory: cx.new(|cx| MemoryView::new(state.clone(), window, cx)),
            usage: cx.new(|cx| UsageView::new(state.clone(), window, cx)),
            doctor: cx.new(|cx| DoctorView::new(state.clone(), window, cx)),
            composer: cx.new(|cx| Composer::new(state.clone(), window, cx)),
            palette,
            state,
            may_flash,
            seen,
            toasts: HashSet::new(),
            viewing: None,
            asking_close: false,
            stopping: false,
            stop_deadline: None,
            errors_shown: 0,
            focus,
            launch: Launch::none(),
            rail_portrait: Rc::new(Cell::new(None)),
            shown,
            leaving: None,
            switch: Tween::rest(1.0, Pace::Screen, Curve::EaseOut),
            lit: SCREENS.map(|screen| glow(screen == shown)),
            hover: SCREENS.map(|_| glow(false)),
            hovered: None,
            _subscriptions: subscriptions,
        }
    }

    fn state_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let screen = state.view;
        let started: Vec<_> = state
            .runs
            .iter()
            .filter(|run| run.hosted && !self.seen.contains_key(&run.session))
            .filter_map(|run| run.cwd.clone())
            .collect();
        let alerts = run_alerts(&mut self.seen, &state.runs);
        let viewing = (screen == Screen::Run).then(|| state.run().map(|run| run.session.clone())).flatten();
        let newly_viewed = viewing.clone().filter(|_| viewing != self.viewing);
        let stale: Vec<SharedString> = self
            .toasts
            .iter()
            .filter(|session| {
                newly_viewed.as_ref() == Some(*session)
                    || state.runs.iter().any(|run| run.session == **session && run.status == RunStatus::Active)
            })
            .cloned()
            .collect();
        let all_stopped = running_here(&state.runs).is_empty();
        let errors: Vec<AppError> = state.errors.iter().filter(|error| error.id > self.errors_shown).cloned().collect();

        self.viewing = viewing;
        settings::remember_screen(screen, cx);
        for path in started {
            settings::remember_project(path, cx);
        }
        for session in stale {
            self.toasts.remove(&session);
            window.remove_notification1::<NeedsYouToast>(session, cx);
        }
        self.show_alerts(alerts, window, cx);
        self.show_errors(errors, window, cx);
        if self.stopping && all_stopped {
            self.finish_close(window, cx);
        }
        cx.notify();
    }

    fn show_alerts(&mut self, alerts: Vec<RunAlert>, window: &mut Window, cx: &mut Context<Self>) {
        if alerts.is_empty() {
            return;
        }
        for alert in alerts {
            if let Some(toast) = self.toast_for(&alert, cx) {
                self.toasts.insert(alert.session);
                window.push_notification(toast, cx);
            }
        }
        if self.may_flash && !window.is_window_active() {
            window.request_attention();
        }
    }

    fn show_errors(&mut self, errors: Vec<AppError>, window: &mut Window, cx: &mut Context<Self>) {
        for error in errors {
            self.errors_shown = error.id;
            let advice = tr(cx, T, error.action.advice());
            let raw = error.message.clone();
            let copy = tr(cx, T, "Copy details");
            window.push_notification(
                Notification::new()
                    .id1::<ErrorToast>(error.id as usize)
                    .with_type(NotificationType::Error)
                    .title(tr(cx, T, error.action.headline()))
                    .message(clipped(&format!("{advice} {}", error.message), TOAST_MESSAGE_CHARS))
                    .action(move |_, _, cx| {
                        let raw = raw.clone();
                        Button::new("copy-error").label(copy).on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(raw.to_string()));
                        }))
                    }),
                cx,
            );
        }
    }

    fn toast_for(&self, alert: &RunAlert, cx: &App) -> Option<Notification> {
        let run = self.state.read(cx).runs.iter().find(|run| run.session == alert.session)?;
        let need = || run.needs.first().map(|need| if need.title.is_empty() { need.note.clone() } else { need.title.clone() });
        let (kind, title, message) = match alert.kind {
            AlertKind::Blocked => (NotificationType::Warning, tr(cx, T, "Needs you: {}"), need().unwrap_or_else(|| run.wish.clone())),
            AlertKind::Waiting => (NotificationType::Info, tr(cx, T, "Waiting on batch: {}"), need().unwrap_or_else(|| run.wish.clone())),
            AlertKind::Finished => {
                let verified = run.count(UnitState::Verified).to_string();
                let total = run.units.len().to_string();
                let summary: SharedString = if run.units.is_empty() {
                    run.wish.clone()
                } else {
                    fill(tr(cx, T, "{} of {} units verified"), &[&verified, &total]).into()
                };
                let kind = if run.verified_end() { NotificationType::Success } else { NotificationType::Warning };
                (kind, tr(cx, T, "Finished: {}"), summary)
            }
        };
        let (state, session, open) = (self.state.clone(), alert.session.clone(), tr(cx, T, "Open"));
        let id = session.clone();
        Some(
            Notification::new()
                .id1::<NeedsYouToast>(id)
                .with_type(kind)
                .title(fill(title, &[run.project.as_ref()]))
                .content(move |_, _, _| div().w_full().text_sm().truncate().child(message.clone()).into_any_element())
                .action(move |_, _, cx| {
                    let (state, session) = (state.clone(), session.clone());
                    Button::new("open-run").label(open).on_click(cx.listener(move |note, _, window, cx| {
                        open_session(&state, &session, cx);
                        note.dismiss(window, cx);
                    }))
                }),
        )
    }

    /// Runs `act` when the window-wide action `A` fires.
    fn on_global<A: gpui_kit::Action>(window: &mut Window, cx: &mut Context<Self>, act: fn(&mut Self, &mut Window, &mut Context<Self>)) {
        let (shell, handle) = (cx.weak_entity(), window.window_handle());
        App::on_action(cx, move |_: &A, cx: &mut App| {
            // A key press runs inside the window's own update, which cannot be re-entered.
            let shell = shell.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| act(shell, window, cx));
                });
            });
        });
    }

    /// A dialog or the New wish form has the keyboard; window-wide overlays wait for it.
    fn dialog_open(&self, window: &mut Window, cx: &mut App) -> bool {
        window.has_active_dialog(cx) || window.has_active_sheet(cx) || self.state.read(cx).composing
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_open(window, cx) {
            return;
        }
        self.palette.update(cx, |palette, cx| match palette.overlay() {
            Overlay::Search => palette.close(window, cx),
            _ => palette.open(window, cx),
        });
    }

    fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_open(window, cx) {
            return;
        }
        self.palette.update(cx, |palette, cx| match palette.overlay() {
            Overlay::Shortcuts => palette.close(window, cx),
            _ => palette.show_shortcuts(window, cx),
        });
    }

    /// Opens a run and selects something in it, found by id so a refresh in between does not
    /// change what is selected.
    fn open_at(&mut self, session: &SharedString, select: impl FnOnce(&Run) -> Selection, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            let Some(index) = state.runs.iter().position(|run| run.session == *session) else { return };
            let selection = select(&state.runs[index]);
            state.open_run(index, cx);
            state.select(selection, cx);
        });
    }

    /// Where a palette result leads.
    fn go_to(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Command(command) => self.run_command(command, window, cx),
            Target::Run(session) => open_session(&self.state, &session, cx),
            Target::Goal(session, id) => self.open_at(
                &session,
                |run| {
                    let lead = run.goals.iter().find(|goal| goal.id == id).and_then(|goal| palette::goal_lead(run, goal));
                    lead.map_or(Selection::None, Selection::Module)
                },
                cx,
            ),
            Target::Module(session, id) => self.open_at(
                &session,
                |run| run.modules.iter().position(|m| m.id == id).map_or(Selection::None, Selection::Module),
                cx,
            ),
            Target::Unit(session, id) => self.open_at(
                &session,
                |run| run.units.iter().position(|u| u.id == id).map_or(Selection::None, Selection::Unit),
                cx,
            ),
            Target::Memory(id) => {
                self.state.update(cx, |state, cx| state.set_view(Screen::Memory, cx));
                self.memory.update(cx, |memory, cx| memory.reveal(id, window, cx));
            }
        }
    }

    fn run_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::NewWish => self.new_wish(window, cx),
            Command::Open(screen) => self.state.update(cx, |state, cx| state.set_view(screen, cx)),
            Command::ToggleTheme => {
                let choice = if cx.theme().is_dark() { ThemeChoice::Light } else { ThemeChoice::Dark };
                settings::set_theme(choice, window, cx);
            }
            Command::ToggleLanguage => {
                let choice = if *cx.global::<Lang>() == Lang::En { LanguageChoice::ZhTw } else { LanguageChoice::English };
                settings::set_language(choice, window, cx);
            }
            Command::Stop => {
                let state = self.state.clone();
                let offline = tr(cx, T, "Not connected to the engine, so nothing was sent.");
                ask(
                    Ask {
                        title: tr(cx, T, "Stop this wish?"),
                        detail: tr(cx, T, "It can be resumed later."),
                        confirm: tr(cx, &[], "Stop"),
                        back: tr(cx, T, "Keep running"),
                        danger: true,
                        focus_confirm: false,
                    },
                    window,
                    cx,
                    move |_, cx| state.update(cx, |state, cx| command_or_report(state, super::model::Action::Stop, offline, cx)),
                );
            }
            Command::Resume => {
                let offline = tr(cx, T, "Not connected to the engine, so nothing was sent.");
                self.state.update(cx, |state, cx| command_or_report(state, super::model::Action::Resume, offline, cx));
            }
            Command::Shortcuts => self.palette.update(cx, |palette, cx| palette.show_shortcuts(window, cx)),
        }
    }

    fn new_wish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.update(cx, |palette, cx| palette.close(window, cx));
        if self.dialog_open(window, cx) {
            return;
        }
        let blocked = blocking_check(&self.state.read(cx).checks).map(|check| (check.name.clone(), check.detail.clone()));
        let Some((name, detail)) = blocked else {
            self.state.update(cx, |state, cx| state.set_composing(true, cx));
            return;
        };
        let (state, open_settings) = (self.state.clone(), tr(cx, T, "Open Settings"));
        window.push_notification(
            Notification::new()
                .id::<SetupToast>()
                .with_type(NotificationType::Warning)
                .title(tr(cx, T, "You can't make a wish yet"))
                .message(clipped(&fill(tr(cx, T, "{}: {}"), &[&name, &detail]), TOAST_MESSAGE_CHARS))
                .action(move |_, _, cx| {
                    let state = state.clone();
                    Button::new("open-settings").label(open_settings).on_click(cx.listener(move |note, _, window, cx| {
                        state.update(cx, |state, cx| state.set_view(Screen::Doctor, cx));
                        note.dismiss(window, cx);
                    }))
                }),
            cx,
        );
    }

    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.stopping {
            return false;
        }
        if running_here(&self.state.read(cx).runs).is_empty() {
            self.end_engine(cx);
            settings::remember_window(window.window_bounds(), cx);
            return true;
        }
        self.ask_before_close(window, cx);
        false
    }

    fn ask_before_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.asking_close && window.has_active_dialog(cx) {
            return;
        }
        self.asking_close = true;
        let shell = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let Some(view) = shell.upgrade() else { return alert };
            let t = cx.tokens();
            let this = view.read(cx);
            let stopping = this.stopping;
            let state = this.state.read(cx);
            let running = running_here(&state.runs);
            let description = match running.as_slice() {
                [run] => fill(
                    tr(cx, T, "\"{}\" is still running. Closing the window stops it; you can resume it later."),
                    &[&clipped(&run.wish, 60)],
                ),
                runs => fill(
                    tr(cx, T, "{} wishes are still running. Closing the window stops them; you can resume them later."),
                    &[&runs.len().to_string()],
                ),
            };
            let stop_label = if stopping { tr(cx, T, "Stopping…") } else { tr(cx, T, "Stop and close") };
            let (keep, stop, closed) = (shell.clone(), shell.clone(), shell.clone());
            alert
                .entrance(motion::DIALOG_ENTRANCE)
                .icon(Icon::new(IconName::TriangleAlert).size(px(20.)).text_color(t.review))
                .title(tr(cx, T, "A wish is still running"))
                .description(description)
                .centered()
                .on_ok(move |_, _, _| !stopping)
                .on_cancel(move |_, _, _| !stopping)
                .on_close(move |_, _, cx| {
                    let _ = closed.update(cx, |shell, _| shell.asking_close = false);
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("stop-and-close")
                                .danger()
                                .outline()
                                .label(stop_label)
                                .loading(stopping)
                                .on_click(move |_, window, cx| {
                                    let _ = stop.update(cx, |shell, cx| shell.stop_and_close(window, cx));
                                }),
                        )
                        .child(
                            theme::gold_leaf(Button::new("keep-working"), &t)
                                .label(tr(cx, T, "Keep working"))
                                .disabled(stopping)
                                .on_click(move |_, window, cx| {
                                    let _ = keep.update(cx, |shell, _| shell.asking_close = false);
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    fn stop_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let running: Vec<SharedString> = running_here(&state.runs).iter().map(|run| run.session.clone()).collect();
        if running.is_empty() || state.backend.is_none() {
            self.finish_close(window, cx);
            return;
        }
        self.state.update(cx, |state, _| running.iter().for_each(|session| state.stop(session)));
        self.stopping = true;
        self.stop_deadline = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(STOP_DEADLINE).await;
            let _ = this.update_in(cx, |this, window, cx| this.finish_close(window, cx));
        }));
        window.refresh();
    }

    /// The engine owns every wish started here; ending it is what stops their `claude` processes.
    fn end_engine(&self, cx: &mut Context<Self>) {
        if let Some(backend) = self.state.read(cx).backend.as_deref() {
            backend.shutdown(ENGINE_DEADLINE);
        }
    }

    fn finish_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_engine(cx);
        settings::remember_window(window.window_bounds(), cx);
        window.remove_window();
    }

    /// Demo only: starts a transition on a frozen clock, then holds it `ms` in, so a capture shows
    /// the frame mid-way. The hold waits a moment of real time first, so every view has drawn the
    /// transition's first frame and taken its start from the frozen clock.
    fn hold_transition(&mut self, what: &str, ms: u64, window: &mut Window, cx: &mut Context<Self>) {
        let start = Instant::now();
        motion::freeze(start, cx);
        match what {
            "switch" => self.state.update(cx, |state, cx| state.set_view(Screen::Wishes, cx)),
            "palette" => self.palette.update(cx, |palette, cx| {
                palette.open(window, cx);
                palette.type_query("read", window, cx);
            }),
            "palette-close" => {
                self.palette.update(cx, |palette, cx| {
                    palette.open(window, cx);
                    palette.type_query("read", window, cx);
                });
                // Opened long enough ago to be settled, closing now.
                motion::freeze(start + Pace::Open.duration() * 2, cx);
                self.palette.update(cx, |palette, cx| palette.close(window, cx));
            }
            "progress" => self.state.update(cx, |state, cx| {
                if let Some(run) = state.current.and_then(|i| state.runs.get_mut(i)) {
                    super::demo::advance(run);
                }
                cx.notify();
            }),
            _ => return,
        }
        let held = motion::now(cx) + Duration::from_millis(ms);
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = cx.update(|_, cx| motion::freeze(held, cx));
        })
        .detach();
    }

    fn play_scene(&mut self, scene: &str, window: &mut Window, cx: &mut Context<Self>) {
        // `launch-at-<ms>` is held from the very start, before this runs.
        if let Some((what, ms)) = held_scene(scene).filter(|(what, _)| *what != "launch") {
            self.hold_transition(what, ms, window, cx);
            return;
        }
        match scene {
            "close-confirm" => self.ask_before_close(window, cx),
            "close-stop" => self.stop_and_close(window, cx),
            "rail-focus" => window.focus_next(cx),
            "needs-you" => self.state.update(cx, |state, cx| {
                state.runs[0].status = RunStatus::Blocked;
                state.runs[0].needs = vec![Need {
                    kind: NeedKind::Blocked,
                    title: "causal 模組的審查有重大問題，需要你決定".into(),
                    note: "Review failed twice. Decide whether to drop the module.".into(),
                }];
                state.runs[1].status = RunStatus::Done;
                state.runs[2].status = RunStatus::Waiting;
                state.runs[2].needs = vec![Need {
                    kind: NeedKind::Waiting,
                    title: "Background batch running".into(),
                    note: "Progress in reports/progress.json.".into(),
                }];
                cx.notify();
            }),
            "checks-failed" => self.state.update(cx, |state, cx| {
                state.checks[0].status = CheckStatus::Fail;
                state.checks[0].detail = "not on PATH".into();
                state.checks[1].status = CheckStatus::Fail;
                state.checks[1].detail = "not signed in: run `codex login`".into();
                cx.notify();
            }),
            "palette" => self.palette.update(cx, |palette, cx| {
                palette.open(window, cx);
                palette.type_query("read", window, cx);
            }),
            "palette-empty" => self.palette.update(cx, |palette, cx| palette.open(window, cx)),
            "palette-none" => self.palette.update(cx, |palette, cx| {
                palette.open(window, cx);
                palette.type_query("qqzz", window, cx);
            }),
            "shortcuts" => self.toggle_shortcuts(window, cx),
            // Types a query and presses Enter, to show where a result leads.
            jump if jump.starts_with("palette-go-") => {
                let query = jump["palette-go-".len()..].replace('_', " ");
                let query = if query == "d10 scorer" { "d10_scorer".to_owned() } else { query };
                self.palette.update(cx, |palette, cx| {
                    palette.open(window, cx);
                    palette.type_query(&query, window, cx);
                    palette.choose_selected(window, cx);
                });
            }
            "version-mismatch" => self.state.update(cx, |state, cx| {
                state.checks.push(Check { name: "plugin".into(), status: CheckStatus::Ok, detail: "0.9.4".into() });
                cx.notify();
            }),
            _ => {}
        }
    }

    fn nav_item(
        &self,
        id: &'static str,
        kind: Emblem,
        label: &'static str,
        hint: SharedString,
        view: Screen,
        count: usize,
        (lit, hover): (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.tokens();
        let active = self.state.read(cx).view == view;
        let state = self.state.clone();
        // Glow and underline follow selection (lit); hover only tints the face.
        let face = mix(t.raised.opacity(0.0), t.raised, lit.max(hover));
        let colors = ButtonCustomVariant::new(cx).color(face).foreground(t.rail_ink(lit)).hover(face).active(face);
        div()
            .id(("nav-glow", slot(view)))
            .relative()
            .rounded(px(8.))
            .when(lit > 0.01, |this| this.shadow(ornament::gold_glow(&t, lit)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(view);
                } else if this.hovered == Some(view) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .child(
                Button::new(id)
                    .custom(colors)
                    .selected(active)
                    .w(px(64.))
                    .h(px(52.))
                    .rounded(px(8.))
                    .accessibility_label(tr(cx, &[], label))
                    .tooltip(hint)
                    .tooltip_placement(Placement::Right)
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_view(view, cx)))
                    .child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.))
                            .text_color(t.rail_ink(lit))
                            .child(
                                div()
                                    .relative()
                                    .child(ornament::emblem(kind, 20., t.rail_ink(lit)))
                                    .when(count > 0, |this| this.child(count_badge(count, &t))),
                            )
                            .child(tr(cx, &[], label)),
                    ),
            )
            .when(shows_underline(lit), |this| this.child(div().absolute().left_0().bottom_0().w_full().child(ornament::orbit_underline(t.gold, lit))))
    }

    fn new_wish_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let blocked = blocking_check(&self.state.read(cx).checks).map(|check| (check.name.clone(), check.detail.clone()));
        let label = tr(cx, &[], "New wish");
        let button = theme::gold_leaf(Button::new("new-wish"), &t)
            .w(px(64.))
            .h(px(52.))
            .rounded(px(8.))
            .disabled(blocked.is_some())
            .accessibility_label(label)
            .tooltip_placement(Placement::Right)
            .on_click(cx.listener(|this, _, window, cx| this.new_wish(window, cx)))
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .font_medium()
                    .child(Icon::new(IconName::Plus).size(px(18.)))
                    .child(label),
            );
        let Some((name, detail)) = blocked else {
            return v_flex().child(button.tooltip_with_action(label, &NewWish, None));
        };
        let reason: SharedString = fill(tr(cx, T, "{}: {}"), &[&name, &detail]).into();
        let state = self.state.clone();
        v_flex()
            .items_center()
            .gap_1()
            .child(button.tooltip(reason.clone()))
            .child(
                Button::new("setup-reason")
                    .text()
                    .w(px(68.))
                    .h_auto()
                    .tooltip(reason)
                    .tooltip_placement(Placement::Right)
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_view(Screen::Doctor, cx)))
                    .child(
                        div()
                            .w_full()
                            .whitespace_normal()
                            .text_center()
                            .text_size(px(10.))
                            .line_height(px(13.))
                            .text_color(t.review)
                            .child(fill(tr(cx, T, "{} not ready"), &[&name])),
                    ),
            )
    }

    /// The palette's door in the title bar, for people who have not learned Ctrl+K yet.
    fn search_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let label = tr(cx, T, "Search");
        h_flex()
            .id("title-search")
            .flex_none()
            .h(px(26.))
            .px_2()
            .gap_2()
            .rounded(px(4.))
            .border_1()
            .border_color(t.border)
            .text_xs()
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|this| this.bg(t.raised))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tr(cx, T, "Search wishes, goals, modules, units and memory, or run a command")).build(window, cx)
            })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)))
            .child(Icon::new(IconName::Search).size(px(13.)))
            .child(label)
            .child(palette::key_cap(palette::keys_of(palette::Global::Palette)))
    }

    fn title_status(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let t = cx.tokens();
        let state = self.state.read(cx);
        let run = state.run()?;
        let color = t.outcome(run.status, run.verified_end());
        let (project, label) = (run.project.clone(), tr(cx, &[], run.status_label()));
        Some(
            h_flex()
                .gap_2()
                .min_w_0()
                .child(div().w(px(1.)).h(px(12.)).bg(t.border))
                .child(div().text_sm().text_color(t.muted).truncate().child(project))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color)
                        .child(div().size(px(6.)).rounded_full().bg(color))
                        .child(label),
                ),
        )
    }
}

fn shows_underline(lit: f32) -> bool {
    lit > 0.01
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let state = self.state.read(cx);
        let reduce = state.reduce_motion;
        let view = state.view;
        let composing = state.composing;
        let waiting = state.runs.iter().filter(|run| needs_user(run)).count();
        let mut motion = Driver::new(reduce, cx);
        let now = motion.now();
        motion.track(&self.launch);
        let launching = self.launch.seconds(now).filter(|_| !reduce);

        if view != self.shown {
            // Behind the launch the screen simply changes; after it, a switch crossfades.
            if launching.is_none() {
                self.leaving = Some(self.shown);
                self.switch.jump(0.0);
                self.switch.set(1.0, now, reduce);
            }
            self.shown = view;
        }
        let switch = motion.value(&self.switch);
        if switch >= 1.0 {
            self.leaving = None;
        }
        let mut glows = [(0.0, 0.0); 5];
        for (i, screen) in SCREENS.into_iter().enumerate() {
            self.lit[i].set(if screen == view { 1.0 } else { 0.0 }, now, reduce);
            self.hover[i].set(if self.hovered == Some(screen) { 1.0 } else { 0.0 }, now, reduce);
            glows[i] = (motion.value(&self.lit[i]), motion.value(&self.hover[i]));
        }
        let content = self.screen(view);
        // The screen being left stays underneath while the new one fades in over it.
        let leaving = self.leaving.map(|screen| self.screen(screen));
        let wishes_hint: SharedString = if waiting == 0 {
            tr(cx, T, "Every wish, newest first").into()
        } else {
            fill(tr(cx, T, "Every wish, newest first. {} need you."), &[&waiting.to_string()]).into()
        };
        let status = self.title_status(cx);
        let search = self.search_button(cx);
        let new_wish = self.new_wish_button(cx);
        let g = |screen: Screen| glows[slot(screen)];
        let run_item = self.nav_item("nav-run", Emblem::Hourglass, "Run", tr(cx, T, "What the current wish is doing").into(), Screen::Run, 0, g(Screen::Run), cx);
        let wishes_item = self.nav_item("nav-wishes", Emblem::Scroll, "Wishes", wishes_hint, Screen::Wishes, waiting, g(Screen::Wishes), cx);
        let memory_item = self.nav_item("nav-memory", Emblem::StarBook, "Memory", tr(cx, T, "What Chronos remembers between wishes").into(), Screen::Memory, 0, g(Screen::Memory), cx);
        let usage_item = self.nav_item("nav-usage", Emblem::Astrolabe, "Usage", tr(cx, T, "Cost and context by wish").into(), Screen::Usage, 0, g(Screen::Usage), cx);
        let settings_item = self.nav_item("nav-doctor", Emblem::Armillary, "Settings", tr(cx, T, "Environment checks and preferences").into(), Screen::Doctor, 0, g(Screen::Doctor), cx);
        let rail_cell = self.rail_portrait.clone();
        // Hidden while the launch's keeper is on its way to this spot.
        let rail_portrait = div()
            .relative()
            .mt_1()
            .mb_2()
            .child(ornament::portrait(44.).opacity(if launching.is_some() { 0.0 } else { 1.0 }))
            .child(
                canvas(
                    move |bounds, _, _| {
                        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        rail_cell.set(Some(Square { x, y, side: f32::from(bounds.size.width) }));
                    },
                    |_, _, _, _| (),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        let viewport = window.viewport_size();
        let overlay = launching.map(|s| {
            let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
            launch::overlay(s, w, h, self.rail_portrait.get(), super::theme::night())
                .id("launch")
                .occlude()
                .on_any_mouse_down(cx.listener(|this, _, _, cx| {
                    this.skip_launch(cx);
                }))
        });
        motion.finish(window);
        v_flex()
            .size_full()
            .relative()
            .key_context("ChronosShell")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, _: &KeyDownEvent, _, cx| {
                if this.skip_launch(cx) {
                    cx.stop_propagation();
                }
            }))
            .bg(tokens.canvas)
            .text_color(tokens.text)
            .child(
                TitleBar::new()
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        if this.should_close(window, cx) {
                            window.remove_window();
                        }
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .pr_2()
                            .gap_2()
                            .child(div().flex_shrink_0().text_sm().font_medium().text_color(tokens.gold).child("Axiom Chronos"))
                            .children(status)
                            .child(div().flex_1())
                            .child(search),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .id("rail")
                            .h_full()
                            .w(px(80.))
                            .flex_shrink_0()
                            .p_2()
                            .gap_1()
                            .items_center()
                            .bg(tokens.surface)
                            .border_r_1()
                            .border_color(tokens.border)
                            .child(rail_portrait)
                            .child(new_wish)
                            .child(div().w(px(60.)).my_1().child(ornament::divider(&tokens, 0.0)))
                            .child(run_item)
                            .child(wishes_item)
                            .child(memory_item)
                            .child(usage_item)
                            .child(div().flex_1())
                            .child(settings_item),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(ornament::backdrop(&tokens, cx.theme().is_dark()))
                            .children(leaving.map(|screen| div().absolute().top_0().left_0().size_full().child(screen)))
                            .child(
                                div()
                                    .relative()
                                    .size_full()
                                    .top(px(SCREEN_DRIFT * (1.0 - switch)))
                                    .when(switch < 1.0, |this| this.opacity(switch))
                                    .child(content),
                            ),
                    ),
            )
            .when(composing, |this| this.child(self.composer.clone()))
            .child(self.palette.clone())
            .children(overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use crate::desktop::demo::sample_run;

    #[test]
    fn backdrop_stays_behind_the_crossfade() {
        let source = include_str!("shell.rs").split("#[cfg(test)]").next().unwrap();
        let render = source.split("impl Render for Shell {").nth(1).expect("shell render");
        assert_eq!(render.matches("ornament::backdrop(").count(), 1);
        let backdrop = render.find("ornament::backdrop(").expect("backdrop");
        let content_end = backdrop + render[backdrop..].find(".child(content)").expect("content") + ".child(content)".len();
        let content_area = &render[backdrop..content_end];
        let backdrop = content_area.find("ornament::backdrop(").unwrap();
        assert!(backdrop < content_area.find("leaving.map(").expect("leaving screen"));
        assert!(backdrop < content_area.find(".child(content)").expect("content"));
        assert!(!content_area.contains(".bg(tokens.canvas)"));
        assert!(!content_area.contains(".bg("));

        let ornament = include_str!("ornament.rs");
        let backdrop_fn = ornament.split("pub fn backdrop(").nth(1).expect("backdrop function").split("\npub ").next().unwrap();
        assert!(backdrop_fn.contains(".absolute().top_0().left_0().size_full()"));
    }

    #[test]
    fn underline_visibility_follows_lit_threshold() {
        assert!(!shows_underline(0.0));
        assert!(!shows_underline(0.01));
        assert!(shows_underline(0.02));
        assert!(shows_underline(1.0));
    }

    #[test]
    fn orbit_underline_uses_visibility_rule() {
        let source = include_str!("shell.rs").split("#[cfg(test)]").next().unwrap();
        let rail_item = source.split("    fn nav_item(").nth(1).expect("rail item").split("    fn new_wish_button(").next().unwrap();
        let underline = rail_item.lines().find(|line| line.contains("ornament::orbit_underline(")).expect("orbit underline");
        assert!(underline.contains(".when(shows_underline(lit)"));
        assert!(!rail_item.contains(".when(lit > 0.01, |this| this.child(div().absolute()"));
    }

    #[test]
    fn wish_and_keep_working_buttons_use_gold_leaf() {
        let source = include_str!("shell.rs").split("#[cfg(test)]").next().unwrap();
        for (id, end) in [("new-wish", "let Some((name, detail))"), ("keep-working", "fn stop_and_close(")] {
            let marker = format!("Button::new(\"{id}\")");
            assert_eq!(source.matches(&marker).count(), 1);
            let position = source.find(&marker).unwrap();
            let start = source[..position].rfind('\n').unwrap() + 1;
            let builder = source[start..].split(end).next().unwrap();
            assert!(builder.contains("gold_leaf("), "{id} must use gold_leaf");
            assert!(!builder.contains(".primary()"), "{id} must not use primary");
        }
    }

    fn run_with(session: &'static str, status: RunStatus) -> Run {
        let mut run = sample_run();
        run.session = session.into();
        run.status = status;
        run
    }

    fn check(name: &'static str, status: CheckStatus) -> Check {
        Check { name: name.into(), status, detail: "detail".into() }
    }

    fn alert(session: &'static str, kind: AlertKind) -> RunAlert {
        RunAlert { session: session.into(), kind }
    }

    #[test]
    fn nothing_is_announced_for_runs_already_in_their_state_at_launch() {
        let mut seen = HashMap::new();
        let runs = [run_with("a", RunStatus::Blocked), run_with("b", RunStatus::Done), run_with("c", RunStatus::Waiting)];
        assert_eq!(run_alerts(&mut seen, &runs), vec![]);
    }

    #[test]
    fn a_run_is_announced_when_it_blocks_waits_or_finishes() {
        let cases = [
            (RunStatus::Blocked, AlertKind::Blocked),
            (RunStatus::Waiting, AlertKind::Waiting),
            (RunStatus::Done, AlertKind::Finished),
        ];
        for (status, kind) in cases {
            let mut seen = HashMap::new();
            run_alerts(&mut seen, &[run_with("a", RunStatus::Active)]);
            assert_eq!(run_alerts(&mut seen, &[run_with("a", status)]), vec![alert("a", kind)]);
        }
    }

    #[test]
    fn the_same_state_is_never_announced_twice() {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &[run_with("a", RunStatus::Active)]);
        assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]).len(), 1);
        for _ in 0..3 {
            assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]), vec![]);
        }
    }

    #[test]
    fn returning_to_work_is_silent_and_blocking_again_is_news() {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]);
        assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Active)]), vec![]);
        assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]), vec![alert("a", AlertKind::Blocked)]);
    }

    #[test]
    fn moving_between_states_that_matter_is_announced_for_the_new_one() {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &[run_with("a", RunStatus::Waiting)]);
        assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Done)]), vec![alert("a", AlertKind::Finished)]);
    }

    #[test]
    fn a_new_run_is_a_baseline_while_others_still_change() {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &[run_with("a", RunStatus::Active)]);
        let runs = [run_with("a", RunStatus::Done), run_with("b", RunStatus::Blocked)];
        assert_eq!(run_alerts(&mut seen, &runs), vec![alert("a", AlertKind::Finished)]);
    }

    #[test]
    fn a_run_that_left_the_list_is_remembered_when_it_returns() {
        let mut seen = HashMap::new();
        run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]);
        run_alerts(&mut seen, &[]);
        assert_eq!(run_alerts(&mut seen, &[run_with("a", RunStatus::Blocked)]), vec![]);
    }

    #[test]
    fn the_badge_counts_runs_waiting_on_the_user() {
        assert!(needs_user(&run_with("a", RunStatus::Blocked)));
        assert!(needs_user(&run_with("a", RunStatus::Waiting)));
        assert!(!needs_user(&run_with("a", RunStatus::Active)));
        assert!(!needs_user(&run_with("a", RunStatus::Done)));
        let mut asking = run_with("a", RunStatus::Active);
        asking.needs = vec![Need { kind: NeedKind::Interjection, title: "?".into(), note: "?".into() }];
        assert!(needs_user(&asking));
        let mut finished = asking;
        finished.status = RunStatus::Done;
        assert!(!needs_user(&finished));
    }

    #[test]
    fn only_a_failed_check_blocks_new_wishes() {
        assert!(blocking_check(&[]).is_none());
        assert!(blocking_check(&[check("codegraph", CheckStatus::Warn), check("claude", CheckStatus::Ok)]).is_none());
        let checks = [check("claude", CheckStatus::Ok), check("codex", CheckStatus::Fail), check("state", CheckStatus::Fail)];
        assert_eq!(blocking_check(&checks).map(|c| c.name.as_ref()), Some("codex"));
    }

    #[test]
    fn closing_asks_only_about_wishes_started_here_that_are_still_working() {
        let mut here = run_with("here", RunStatus::Active);
        here.hosted = true;
        let mut terminal = run_with("terminal", RunStatus::Active);
        terminal.hosted = false;
        let mut paused = run_with("paused", RunStatus::Blocked);
        paused.hosted = true;
        let mut waiting = run_with("waiting", RunStatus::Waiting);
        waiting.hosted = true;
        let runs = [terminal, paused, waiting, here];
        let running: Vec<_> = running_here(&runs).iter().map(|run| run.session.to_string()).collect();
        assert_eq!(running, vec!["here"]);
        assert!(running_here(&runs[..3]).is_empty());
    }

    #[test]
    fn a_finished_run_counts_as_verified_only_when_every_unit_is_and_the_full_test_passed() {
        let mut run = run_with("a", RunStatus::Done);
        assert!(!run.verified_end());
        for unit in &mut run.units {
            unit.state = UnitState::Verified;
        }
        assert!(!run.verified_end(), "the full test has not passed yet");
        assert_eq!(run.status_label(), "Ended, needs attention");
        run.stages.iter_mut().filter(|stage| stage.name == "Full test").for_each(|stage| stage.phase = Phase::Done);
        assert!(run.verified_end());
        assert_eq!(run.status_label(), "Done");
        run.units.clear();
        assert!(!run.verified_end());
    }

    #[test]
    fn a_held_scene_names_its_transition_and_moment() {
        assert_eq!(held_scene("switch-at-120"), Some(("switch", 120)));
        assert_eq!(held_scene("palette-close-at-60"), Some(("palette-close", 60)));
        assert_eq!(held_scene("launch-at-0"), Some(("launch", 0)));
        assert_eq!(held_scene("switch-at-"), None);
        assert_eq!(held_scene("palette"), None);
    }

    #[test]
    fn the_rail_lists_every_screen_once() {
        for (i, screen) in SCREENS.into_iter().enumerate() {
            assert_eq!(slot(screen), i);
        }
    }

    #[test]
    fn long_text_is_clipped_on_a_character_boundary() {
        assert_eq!(clipped("short", 10), "short");
        assert_eq!(clipped("全面盤點電阻式記憶體", 4), "全面盤點…");
    }

    #[test]
    fn every_translation_key_used_here_exists() {
        let source = include_str!("shell.rs");
        let used = regex::Regex::new(r#"tr\(cx, T, "((?:[^"\\]|\\.)*)"\)"#).unwrap();
        for capture in used.captures_iter(source) {
            let key = capture[1].replace("\\\"", "\"");
            assert!(T.iter().any(|(en, _)| *en == key), "missing translation: {key}");
        }
        let labels = regex::Regex::new(r#"Emblem::\w+, "(\w+)""#).unwrap();
        for capture in labels.captures_iter(source) {
            let key = &capture[1];
            assert!(super::super::i18n::COMMON.iter().any(|(en, _)| *en == key), "missing translation: {key}");
        }
    }

    #[test]
    fn every_nav_call_site_has_an_emblem() {
        let source = include_str!("shell.rs");
        let labels = regex::Regex::new(r#"self\.nav_item\("nav-[\w-]+", Emblem::\w+, "(\w+)""#).unwrap();
        assert_eq!(labels.captures_iter(source).count(), 5);
    }
}
