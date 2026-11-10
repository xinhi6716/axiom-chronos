//! The window frame: title bar, navigation rail and the active screen. It also owns what applies
//! across screens: Ctrl+N, Needs you notifications and the question asked before closing.
use super::i18n::{fill, tr, Table};
use super::model::*;
use super::{ornament, settings};
use super::theme::{Tokened as _, Tokens};
use super::{composer::Composer, doctor::DoctorView, memory::MemoryView, run_view::RunView, usage::UsageView, wishes::WishesView};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Placement, Selectable as _, StyledExt as _, TitleBar, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

gpui_kit::actions!(chronos, [NewWish]);

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
];

/// How long "Stop and close" waits for the runs to report that they stopped before closing anyway.
const STOP_DEADLINE: Duration = Duration::from_secs(10);
const TOAST_MESSAGE_CHARS: usize = 120;

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
    may_flash: bool,
    seen: HashMap<SharedString, RunStatus>,
    toasts: HashSet<SharedString>,
    viewing: Option<SharedString>,
    asking_close: bool,
    stopping: bool,
    stop_deadline: Option<Task<()>>,
    /// The newest entry of `AppState::errors` already shown.
    errors_shown: u64,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
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
        cx.bind_keys([KeyBinding::new(if cfg!(target_os = "macos") { "cmd-n" } else { "ctrl-n" }, NewWish, None)]);
        let (shell, handle) = (cx.weak_entity(), window.window_handle());
        App::on_action(cx, move |_: &NewWish, cx: &mut App| {
            // A key press runs inside the window's own update, which cannot be re-entered.
            let shell = shell.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.new_wish(window, cx));
                });
            });
        });
        let shell = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            shell.update(cx, |shell, cx| shell.should_close(window, cx)).unwrap_or(true)
        });
        let scene = state.read(cx).scene.clone();
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
            state,
            may_flash,
            seen,
            toasts: HashSet::new(),
            viewing: None,
            asking_close: false,
            stopping: false,
            stop_deadline: None,
            errors_shown: 0,
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

    fn new_wish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) || window.has_active_sheet(cx) || self.state.read(cx).composing {
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
                            Button::new("keep-working")
                                .primary()
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

    fn finish_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        settings::remember_window(window.window_bounds(), cx);
        window.remove_window();
    }

    fn play_scene(&mut self, scene: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        icon: IconName,
        label: &'static str,
        hint: SharedString,
        view: Screen,
        count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.tokens();
        let active = self.state.read(cx).view == view;
        let state = self.state.clone();
        let colors = ButtonCustomVariant::new(cx)
            .color(if active { t.raised } else { cx.theme().transparent })
            .foreground(if active { t.gold } else { t.muted })
            .hover(t.raised)
            .active(t.raised);
        div()
            .relative()
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
                            .child(
                                div()
                                    .relative()
                                    .child(Icon::new(icon).size(px(18.)))
                                    .when(count > 0, |this| this.child(count_badge(count, &t))),
                            )
                            .child(tr(cx, &[], label)),
                    ),
            )
            .when(active, |this| {
                this.child(
                    div()
                        .absolute()
                        .left(px(-8.))
                        .top(px(14.))
                        .w(px(3.))
                        .h(px(24.))
                        .rounded_r(px(3.))
                        .bg(t.gold),
                )
            })
    }

    fn new_wish_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let blocked = blocking_check(&self.state.read(cx).checks).map(|check| (check.name.clone(), check.detail.clone()));
        let label = tr(cx, &[], "New wish");
        let button = Button::new("new-wish")
            .primary()
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

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let state = self.state.read(cx);
        let view = state.view;
        let composing = state.composing;
        let waiting = state.runs.iter().filter(|run| needs_user(run)).count();
        let content: AnyElement = match view {
            Screen::Run => self.run.clone().into_any_element(),
            Screen::Wishes => self.wishes.clone().into_any_element(),
            Screen::Memory => self.memory.clone().into_any_element(),
            Screen::Usage => self.usage.clone().into_any_element(),
            Screen::Doctor => self.doctor.clone().into_any_element(),
        };
        let wishes_hint: SharedString = if waiting == 0 {
            tr(cx, T, "Every wish, newest first").into()
        } else {
            fill(tr(cx, T, "Every wish, newest first. {} need you."), &[&waiting.to_string()]).into()
        };
        let status = self.title_status(cx);
        let new_wish = self.new_wish_button(cx);
        let run_item = self.nav_item("nav-run", IconName::LayoutDashboard, "Run", tr(cx, T, "What the current wish is doing").into(), Screen::Run, 0, cx);
        let wishes_item = self.nav_item("nav-wishes", IconName::Inbox, "Wishes", wishes_hint, Screen::Wishes, waiting, cx);
        let memory_item = self.nav_item("nav-memory", IconName::BookOpen, "Memory", tr(cx, T, "What Chronos remembers between wishes").into(), Screen::Memory, 0, cx);
        let usage_item = self.nav_item("nav-usage", IconName::ChartPie, "Usage", tr(cx, T, "Cost and context by wish").into(), Screen::Usage, 0, cx);
        let settings_item = self.nav_item("nav-doctor", IconName::Settings, "Settings", tr(cx, T, "Environment checks and preferences").into(), Screen::Doctor, 0, cx);
        v_flex()
            .size_full()
            .relative()
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
                            .children(status),
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
                            .child(ornament::portrait(44.).mt_1().mb_2())
                            .child(new_wish)
                            .child(div().w(px(60.)).my_1().child(ornament::divider(&tokens, 0.0)))
                            .child(run_item)
                            .child(wishes_item)
                            .child(memory_item)
                            .child(usage_item)
                            .child(div().flex_1())
                            .child(settings_item),
                    )
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
            .when(composing, |this| this.child(self.composer.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use crate::desktop::demo::sample_run;

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
        let labels = regex::Regex::new(r#"IconName::\w+, "(\w+)""#).unwrap();
        for capture in labels.captures_iter(source) {
            let key = &capture[1];
            assert!(super::super::i18n::COMMON.iter().any(|(en, _)| *en == key), "missing translation: {key}");
        }
    }
}
