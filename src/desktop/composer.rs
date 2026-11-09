//! The New wish dialog: choose a project, say what you want, start.
use super::control::{ask, Ask};
use super::i18n::{tr, Table};
use super::model::*;
use super::theme::Tokened as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::input::{Enter, IndentInline, Input, InputEvent, InputState, OutdentInline, Textarea, TextareaState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const T: Table = &[
    ("Choose folder…", "選擇資料夾…"),
    ("Choose the project folder", "選擇專案資料夾"),
    ("Project folder", "專案資料夾"),
    ("No folder chosen yet", "尚未選擇資料夾"),
    ("Recent", "最近"),
    ("Checking this folder…", "正在檢查這個資料夾…"),
    ("branch", "分支"),
    ("detached at {head}", "分離於 {head}"),
    ("no commits yet", "還沒有提交"),
    ("No uncommitted changes", "沒有未提交的變更"),
    ("{n} uncommitted changes. Commit first so this can be undone.", "有 {n} 個未提交的變更。請先提交，之後才能復原。"),
    ("No commits yet. Commit first so this can be undone.", "還沒有任何提交。請先提交，之後才能復原。"),
    ("Not a git repository. You can still start, but changes cannot be undone with git.", "這不是 git 儲存庫。仍可開始，但無法用 git 復原變更。"),
    ("Could not read the git state: {error}", "讀不到 git 狀態：{error}"),
    ("This folder does not exist.", "這個資料夾不存在。"),
    ("Wish", "願望"),
    ("Say what you want done and how you will know it is done.", "說明你想完成什麼，以及怎樣算完成。"),
    ("Creative", "創意模式"),
    ("Let Chronos find one small, evidence-backed improvement.", "讓 Chronos 自己找一個有證據支持的小改進。"),
    ("Advanced", "進階"),
    ("Maximum turns", "最多回合數"),
    ("Maximum turns: {n}", "最多回合數：{n}"),
    ("No limit", "不限"),
    ("Leave blank for no limit.", "留空代表不限。"),
    ("Start", "開始"),
    ("Choose a project folder to start.", "請先選擇專案資料夾。"),
    ("Pick a folder that exists to start.", "請選擇存在的資料夾。"),
    ("Checking the folder first…", "正在檢查資料夾…"),
    ("Write your wish to start.", "請先寫下願望。"),
    ("Maximum turns must be a whole number above 0.", "最多回合數必須是大於 0 的整數。"),
    ("Not connected to a session, so this wish cannot start.", "目前沒有連上工作階段，無法開始這個願望。"),
    ("Discard this wish?", "要捨棄這個願望嗎？"),
    ("What you wrote will be lost.", "你寫的內容不會保留。"),
    ("Discard", "捨棄"),
    ("Keep writing", "繼續寫"),
];

const GIT_TIMEOUT: Duration = Duration::from_secs(6);
const RECENT_LIMIT: usize = 5;

/// What a git probe found about the chosen folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FolderCheck {
    Idle,
    Checking,
    Repo { head: Head, changes: usize },
    NotRepo,
    Missing,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Head {
    Branch(String),
    Detached(String),
    Unborn(String),
}

/// Why Start is disabled. The text is shown beside the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Blocked {
    NoFolder,
    FolderMissing,
    Checking,
    NoWish,
    BadTurns,
}

impl Blocked {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::NoFolder => "Choose a project folder to start.",
            Self::FolderMissing => "Pick a folder that exists to start.",
            Self::Checking => "Checking the folder first…",
            Self::NoWish => "Write your wish to start.",
            Self::BadTurns => "Maximum turns must be a whole number above 0.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Ready {
    pub wish: String,
    pub max_turns: Option<u32>,
}

/// Decides whether Start is available. A folder that merely warns does not block.
pub(super) fn validate(has_folder: bool, check: &FolderCheck, wish: &str, turns: &str) -> Result<Ready, Blocked> {
    if !has_folder {
        return Err(Blocked::NoFolder);
    }
    match check {
        FolderCheck::Missing => return Err(Blocked::FolderMissing),
        FolderCheck::Checking => return Err(Blocked::Checking),
        _ => {}
    }
    let wish = wish.trim();
    if wish.is_empty() {
        return Err(Blocked::NoWish);
    }
    let max_turns = parse_turns(turns).ok_or(Blocked::BadTurns)?;
    Ok(Ready { wish: wish.to_owned(), max_turns })
}

/// `Some(None)` is blank (no limit), `Some(Some(n))` a limit, `None` something that is not a turn count.
pub(super) fn parse_turns(text: &str) -> Option<Option<u32>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    match text.parse::<u32>() {
        Ok(n) if n > 0 => Some(Some(n)),
        _ => None,
    }
}

/// Text in the wish box is the only thing Cancel can lose.
pub(super) fn asks_before_discarding(wish: &str) -> bool {
    !wish.trim().is_empty()
}

/// Reads `git status --porcelain=v2 --branch`.
pub(super) fn parse_status(out: &str) -> FolderCheck {
    let (mut oid, mut branch) = ("", "");
    let mut changes = 0;
    for line in out.lines().filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            oid = rest;
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest;
        } else if !line.starts_with('#') {
            changes += 1;
        }
    }
    let head = if branch == "(detached)" {
        Head::Detached(oid.chars().take(7).collect())
    } else if oid == "(initial)" {
        Head::Unborn(branch.to_owned())
    } else {
        Head::Branch(branch.to_owned())
    };
    FolderCheck::Repo { head, changes }
}

fn classify_git(success: bool, stdout: &str, stderr: &str) -> FolderCheck {
    if success {
        return parse_status(stdout);
    }
    if stderr.to_ascii_lowercase().contains("not a git repository") {
        return FolderCheck::NotRepo;
    }
    FolderCheck::Unknown(stderr.lines().next().unwrap_or("git failed").trim().to_owned())
}

/// Runs on a background thread. Git is given a deadline so a hung drive cannot hold the dialog.
fn check_folder(folder: &Path) -> FolderCheck {
    if !folder.is_dir() {
        return FolderCheck::Missing;
    }
    let mut git = Command::new("git");
    git.args(["status", "--porcelain=v2", "--branch"])
        .current_dir(folder)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        git.creation_flags(CREATE_NO_WINDOW);
    }
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(git.output());
    });
    match receive.recv_timeout(GIT_TIMEOUT) {
        Ok(Ok(output)) => classify_git(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => FolderCheck::Unknown("git is not installed".to_owned()),
        Ok(Err(error)) => FolderCheck::Unknown(error.to_string()),
        Err(_) => FolderCheck::Unknown("git did not answer in time".to_owned()),
    }
}

/// The folders of past runs, one entry each, in the order the runs are listed.
pub(super) fn recent_projects(runs: &[Run]) -> Vec<(SharedString, PathBuf)> {
    let mut seen: Vec<(SharedString, PathBuf)> = Vec::new();
    for run in runs {
        if let Some(cwd) = &run.cwd {
            if !seen.iter().any(|(_, known)| known == cwd) {
                seen.push((run.project.clone(), cwd.clone()));
            }
        }
    }
    seen.truncate(RECENT_LIMIT);
    seen
}

pub struct Composer {
    state: Entity<AppState>,
    wish: Entity<TextareaState>,
    turns: Entity<InputState>,
    choose_focus: FocusHandle,
    open: bool,
    folder: Option<PathBuf>,
    check: FolderCheck,
    probe: Option<Task<()>>,
    creative: bool,
    advanced: bool,
    not_connected: bool,
    _subscriptions: Vec<Subscription>,
}

impl Composer {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let wish = cx.new(|cx| {
            let mut wish = TextareaState::new(window, cx);
            wish.set_rows(6, cx);
            wish
        });
        let turns = cx.new(|cx| InputState::new(window, cx).validate(|text, _| text.chars().all(|c| c.is_ascii_digit())));
        let changed = |this: &mut Self, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.not_connected = false;
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if this.open && window.is_window_active() {
                    this.recheck(cx);
                }
            }),
            cx.subscribe_in(&wish, window, move |this, _, event, _, cx| changed(this, event, cx)),
            cx.subscribe_in(&turns, window, move |this, _, event, _, cx| changed(this, event, cx)),
        ];
        cx.defer_in(window, |this, window, cx| this.apply_scene(window, cx));
        Self {
            choose_focus: cx.focus_handle(),
            state,
            wish,
            turns,
            open: false,
            folder: None,
            check: FolderCheck::Idle,
            probe: None,
            creative: false,
            advanced: false,
            not_connected: false,
            _subscriptions: subscriptions,
        }
    }

    /// `AppState::composing` is the only truth about whether the dialog is up.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.state.read(cx).composing;
        if composing == self.open {
            return;
        }
        self.open = composing;
        if composing {
            self.reset(window, cx);
            self.show(window, cx);
        } else {
            window.close_all_dialogs(cx);
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creative = false;
        self.advanced = false;
        self.not_connected = false;
        let (wish_hint, turns_hint) = (
            tr(cx, T, "Say what you want done and how you will know it is done."),
            tr(cx, T, "No limit"),
        );
        self.wish.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(wish_hint, window, cx);
        });
        self.turns.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(turns_hint, window, cx);
        });
        let folder = self.state.read(cx).run().and_then(|r| r.cwd.clone());
        self.set_folder(folder, cx);
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            view.update(cx, |this, cx| {
                if first.replace(false) {
                    this.focus_first(window, cx);
                }
                this.fill(dialog, cx)
            })
        });
    }

    fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.folder.is_some() {
            self.wish.update(cx, |s, cx| s.focus(window, cx));
        } else {
            self.choose_focus.focus(window, cx);
        }
    }

    fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.folder = folder;
        self.recheck(cx);
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            self.check = FolderCheck::Idle;
            self.probe = None;
            cx.notify();
            return;
        };
        self.check = FolderCheck::Checking;
        let probe = cx.background_spawn(async move { check_folder(&folder) });
        self.probe = Some(cx.spawn(async move |this, cx| {
            let found = probe.await;
            let _ = this.update(cx, |this, cx| {
                this.check = found;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr(cx, T, "Choose the project folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(folder) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_folder(Some(folder), cx);
                this.focus_first(window, cx);
            });
        })
        .detach();
    }

    fn verdict(&self, cx: &App) -> Result<Ready, Blocked> {
        validate(self.folder.is_some(), &self.check, &self.wish.read(cx).value(), &self.turns.read(cx).value())
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Ok(ready) = self.verdict(cx) else { return };
        let Some(cwd) = self.folder.clone() else { return };
        if self.state.read(cx).backend.is_none() {
            self.not_connected = true;
            cx.notify();
            return;
        }
        let request = WishRequest {
            cwd,
            wish: ready.wish,
            creative: self.creative,
            max_turns: ready.max_turns,
        };
        self.state.update(cx, |s, cx| {
            s.set_view(Screen::Run, cx);
            s.start_wish(request, cx);
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if asks_before_discarding(&self.wish.read(cx).value()) {
            let state = self.state.clone();
            ask(
                Ask {
                    title: tr(cx, T, "Discard this wish?"),
                    detail: tr(cx, T, "What you wrote will be lost."),
                    confirm: tr(cx, T, "Discard"),
                    back: tr(cx, T, "Keep writing"),
                    danger: true,
                    focus_confirm: false,
                },
                window,
                cx,
                move |_, cx| state.update(cx, |s, cx| s.set_composing(false, cx)),
            );
        } else {
            self.state.update(cx, |s, cx| s.set_composing(false, cx));
        }
    }

    fn fill(&mut self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let view = cx.entity().downgrade();
        dialog
            .w(px(640.))
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.cancel(window, cx));
                false
            })
            .title(tr(cx, T, "New wish"))
            .child(self.form(cx))
            .footer(self.footer(cx))
    }

    fn label(&self, text: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, T, text))
    }

    fn form(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .capture_action(cx.listener(|this, action: &Enter, _, cx| {
                if action.secondary {
                    cx.stop_propagation();
                    this.start(cx);
                }
            }))
            .capture_action(|_: &IndentInline, window, cx| {
                cx.stop_propagation();
                window.focus_next(cx);
            })
            .capture_action(|_: &OutdentInline, window, cx| {
                cx.stop_propagation();
                window.focus_prev(cx);
            })
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" && event.keystroke.modifiers.control {
                    cx.stop_propagation();
                    this.start(cx);
                }
            }))
            .child(self.folder_section(cx))
            .child(self.wish_section(cx))
            .child(self.creative_row(cx))
            .child(self.advanced_section(cx))
    }

    fn folder_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let tip = self.folder.as_ref().map(|f| f.display().to_string());
        let current = self.folder.clone();
        let recents: Vec<_> = recent_projects(&self.state.read(cx).runs);
        v_flex()
            .gap_2()
            .child(self.label("Project folder", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .id("composer-path")
                            .flex_1()
                            .min_w_0()
                            .h_8()
                            .px_2()
                            .gap_1p5()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.canvas)
                            .text_sm()
                            .when_some(tip.clone(), |this, tip| {
                                let tip: SharedString = tip.into();
                                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            })
                            .child(Icon::new(IconName::Folder).size(px(14.)).text_color(t.muted))
                            .child(match tip {
                                Some(path) => div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis_start()
                                    .child(path)
                                    .into_any_element(),
                                None => div().text_color(t.muted).child(tr(cx, T, "No folder chosen yet")).into_any_element(),
                            }),
                    )
                    .child(
                        Button::new("composer-choose")
                            .outline()
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(tr(cx, T, "Choose folder…"))
                            .track_focus(&self.choose_focus)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_folder(window, cx))),
                    ),
            )
            .when(!recents.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap_1p5()
                        .flex_wrap()
                        .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Recent")))
                        .children(recents.into_iter().enumerate().map(|(i, (name, path))| {
                            let tip = path.display().to_string();
                            let selected = current.as_ref() == Some(&path);
                            Button::new(("composer-recent", i))
                                .outline()
                                .xsmall()
                                .label(name)
                                .selected(selected)
                                .tooltip(tip)
                                .on_click(cx.listener(move |this, _, _, cx| this.set_folder(Some(path.clone()), cx)))
                        })),
                )
            })
            .child(self.git_line(cx))
    }

    fn git_line(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let warn = |text: String, cx: &mut Context<Self>| -> AnyElement {
            let _ = &cx;
            h_flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap_1p5()
                .px_2()
                .py_1()
                .rounded(px(6.))
                .bg(t.review.opacity(0.12))
                .text_xs()
                .text_color(t.review)
                .child(div().pt(px(1.)).child(Icon::new(IconName::TriangleAlert).size(px(13.))))
                .child(div().min_w_0().child(text))
                .into_any_element()
        };
        let branch = |head: &Head, cx: &mut Context<Self>| -> AnyElement {
            let name = match head {
                Head::Branch(name) => name.clone(),
                Head::Unborn(name) => format!("{name} ({})", tr(cx, T, "no commits yet")),
                Head::Detached(sha) => tr(cx, T, "detached at {head}").replace("{head}", sha),
            };
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(tr(cx, T, "branch"))
                .child(
                    div()
                        .px_1p5()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.canvas)
                        .text_color(t.text)
                        .font_family(mono.clone())
                        .child(name),
                )
                .into_any_element()
        };
        let line: AnyElement = match &self.check {
            FolderCheck::Idle => div().into_any_element(),
            FolderCheck::Checking => div().text_xs().text_color(t.muted).child(tr(cx, T, "Checking this folder…")).into_any_element(),
            FolderCheck::Repo { head, changes } => {
                let warning = match (head, changes) {
                    (Head::Unborn(_), _) => Some(tr(cx, T, "No commits yet. Commit first so this can be undone.").to_owned()),
                    (_, 0) => None,
                    (_, n) => Some(tr(cx, T, "{n} uncommitted changes. Commit first so this can be undone.").replace("{n}", &n.to_string())),
                };
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().pt_1().child(branch(head, cx)))
                    .child(match warning {
                        Some(text) => warn(text, cx),
                        None => h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(t.muted)
                            .child(Icon::new(IconName::Check).size(px(12.)).text_color(t.ok))
                            .child(tr(cx, T, "No uncommitted changes"))
                            .into_any_element(),
                    })
                    .into_any_element()
            }
            FolderCheck::NotRepo => warn(tr(cx, T, "Not a git repository. You can still start, but changes cannot be undone with git.").to_owned(), cx),
            FolderCheck::Unknown(error) => warn(tr(cx, T, "Could not read the git state: {error}").replace("{error}", error), cx),
            FolderCheck::Missing => h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.fail)
                .child(Icon::new(IconName::CircleX).size(px(13.)))
                .child(tr(cx, T, "This folder does not exist."))
                .into_any_element(),
        };
        div().min_h_6().child(line)
    }

    fn wish_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_2().child(self.label("Wish", cx)).child(Textarea::new(&self.wish).h(px(132.)))
    }

    fn creative_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        h_flex()
            .gap_2()
            .child(
                Switch::new("composer-creative")
                    .checked(self.creative)
                    .accessibility_label(tr(cx, T, "Creative"))
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.creative = *checked;
                        cx.notify();
                    })),
            )
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, T, "Creative")))
            .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Let Chronos find one small, evidence-backed improvement.")))
    }

    fn advanced_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let turns = self.turns.read(cx).value();
        let summary = (!self.advanced && !turns.trim().is_empty()).then(|| tr(cx, T, "Maximum turns: {n}").replace("{n}", turns.trim()));
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("composer-advanced")
                            .ghost()
                            .xsmall()
                            .icon(if self.advanced { IconName::ChevronDown } else { IconName::ChevronRight })
                            .label(tr(cx, T, "Advanced"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.advanced = !this.advanced;
                                cx.notify();
                            })),
                    )
                    .children(summary.map(|text| div().text_xs().text_color(t.muted).child(text))),
            )
            .when(self.advanced, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .pl_5()
                        .child(div().text_sm().child(tr(cx, T, "Maximum turns")))
                        .child(div().w(px(96.)).child(Input::new(&self.turns).small()))
                        .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Leave blank for no limit."))),
                )
            })
    }

    fn footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let verdict = self.verdict(cx);
        let blocked = verdict.as_ref().err().copied();
        let reason: AnyElement = if self.not_connected {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.fail)
                .child(Icon::new(IconName::CircleAlert).size(px(13.)))
                .child(tr(cx, T, "Not connected to a session, so this wish cannot start."))
                .into_any_element()
        } else if let Some(blocked) = blocked {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(Icon::new(IconName::Info).size(px(13.)))
                .child(tr(cx, T, blocked.reason()))
                .into_any_element()
        } else {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(Kbd::new(Keystroke::parse("ctrl-enter").expect("valid keystroke")))
                .child(tr(cx, T, "Start"))
                .into_any_element()
        };
        DialogFooter::new()
            .justify_between()
            .gap_3()
            .child(div().flex_1().min_w_0().child(reason))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("composer-cancel")
                            .label(tr(cx, T, "Cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        Button::new("composer-start")
                            .primary()
                            .label(tr(cx, T, "Start"))
                            .disabled(blocked.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
            )
    }

    /// Demo only: opens the dialog in a named state so it can be captured.
    fn apply_scene(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scene) = self.state.read(cx).scene.clone() else { return };
        let Some(scene) = scene.strip_prefix("compose") else { return };
        let scene = scene.strip_suffix("-min").map_or(scene, |base| {
            window.resize(size(px(1100.), px(700.)));
            base
        });
        self.state.update(cx, |s, _| s.composing = true);
        self.sync(window, cx);
        let wish = "把 RRAM 讀取流程改成背景批次，保留每一頁的引用。";
        let fill = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.wish.update(cx, |s, cx| s.set_value(wish, window, cx));
        };
        match scene {
            "-no-folder" => self.set_folder(None, cx),
            "-ready" => fill(self, window, cx),
            "-dirty" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("r6/ui-control".into()), changes: 3 };
            }
            "-clean" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 };
            }
            "-not-git" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::NotRepo;
            }
            "-missing" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Missing;
            }
            "-advanced" => {
                fill(self, window, cx);
                self.advanced = true;
                self.turns.update(cx, |s, cx| s.set_value("40", window, cx));
            }
            "-bad-turns" => {
                fill(self, window, cx);
                self.advanced = true;
                self.turns.update(cx, |s, cx| s.set_value("0", window, cx));
            }
            "-discard" => {
                fill(self, window, cx);
                self.cancel(window, cx);
            }
            "-not-connected" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 };
                self.start(cx);
            }
            "" => {}
            _ => {}
        }
        cx.notify();
    }
}

impl Render for Composer {
    /// The dialog lives in the window's dialog layer; the shell only mounts this view while composing.
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use crate::desktop::demo;

    fn repo() -> FolderCheck {
        FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 }
    }

    #[test]
    fn start_needs_a_folder_then_a_wish() {
        assert_eq!(validate(false, &FolderCheck::Idle, "wish", ""), Err(Blocked::NoFolder));
        assert_eq!(validate(true, &repo(), "", ""), Err(Blocked::NoWish));
        assert_eq!(validate(true, &repo(), "   \n ", ""), Err(Blocked::NoWish));
    }

    #[test]
    fn no_folder_is_reported_before_no_wish() {
        assert_eq!(validate(false, &FolderCheck::Idle, "", ""), Err(Blocked::NoFolder));
    }

    #[test]
    fn a_folder_that_is_gone_blocks_start() {
        assert_eq!(validate(true, &FolderCheck::Missing, "wish", ""), Err(Blocked::FolderMissing));
    }

    #[test]
    fn start_waits_while_the_folder_is_being_checked() {
        assert_eq!(validate(true, &FolderCheck::Checking, "wish", ""), Err(Blocked::Checking));
    }

    #[test]
    fn warnings_do_not_block_start() {
        let dirty = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 4 };
        for check in [dirty, FolderCheck::NotRepo, FolderCheck::Unknown("x".into())] {
            assert!(validate(true, &check, "wish", "").is_ok(), "{check:?}");
        }
    }

    #[test]
    fn a_ready_wish_is_trimmed_and_carries_its_limit() {
        assert_eq!(validate(true, &repo(), "  wish\n", "40"), Ok(Ready { wish: "wish".into(), max_turns: Some(40) }));
        assert_eq!(validate(true, &repo(), "wish", ""), Ok(Ready { wish: "wish".into(), max_turns: None }));
    }

    #[test]
    fn maximum_turns_must_be_a_positive_whole_number() {
        assert_eq!(parse_turns(""), Some(None));
        assert_eq!(parse_turns("  "), Some(None));
        assert_eq!(parse_turns("12"), Some(Some(12)));
        assert_eq!(parse_turns("0"), None);
        assert_eq!(parse_turns("-3"), None);
        assert_eq!(parse_turns("1.5"), None);
        assert_eq!(parse_turns("99999999999"), None);
        assert_eq!(validate(true, &repo(), "wish", "0"), Err(Blocked::BadTurns));
    }

    #[test]
    fn cancelling_asks_only_when_there_is_text_to_lose() {
        assert!(!asks_before_discarding(""));
        assert!(!asks_before_discarding(" \n\t"));
        assert!(asks_before_discarding("x"));
    }

    #[test]
    fn git_status_gives_the_branch_and_the_number_of_changes() {
        let out = "# branch.oid 1a2b3c4d5e\n# branch.head main\n# branch.upstream origin/main\n1 .M N... 100644 100644 100644 a b src/a.rs\n? new.txt\n";
        assert_eq!(parse_status(out), FolderCheck::Repo { head: Head::Branch("main".into()), changes: 2 });
    }

    #[test]
    fn a_clean_tree_has_no_changes() {
        let out = "# branch.oid 1a2b3c4d5e\n# branch.head feature/x\n";
        assert_eq!(parse_status(out), FolderCheck::Repo { head: Head::Branch("feature/x".into()), changes: 0 });
    }

    #[test]
    fn a_detached_head_and_an_empty_repository_are_named() {
        let detached = "# branch.oid 1a2b3c4d5e6f\n# branch.head (detached)\n";
        assert_eq!(parse_status(detached), FolderCheck::Repo { head: Head::Detached("1a2b3c4".into()), changes: 0 });
        let unborn = "# branch.oid (initial)\n# branch.head main\n";
        assert_eq!(parse_status(unborn), FolderCheck::Repo { head: Head::Unborn("main".into()), changes: 0 });
    }

    #[test]
    fn git_failures_are_told_apart() {
        assert_eq!(classify_git(false, "", "fatal: not a git repository (or any of the parent directories): .git"), FolderCheck::NotRepo);
        assert_eq!(classify_git(false, "", "fatal: detected dubious ownership\nmore"), FolderCheck::Unknown("fatal: detected dubious ownership".into()));
        assert_eq!(classify_git(true, "# branch.oid 1\n# branch.head main\n", ""), FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 });
    }

    #[test]
    fn a_folder_that_cannot_be_found_is_missing() {
        assert_eq!(check_folder(Path::new("Z:\\no\\such\\folder\\anywhere")), FolderCheck::Missing);
    }

    #[test]
    fn recent_projects_are_distinct_folders_in_run_order() {
        let mut runs = demo::state().runs;
        runs[3].cwd = None;
        let recent = recent_projects(&runs);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].0.as_ref(), "ml_hfo2");
        let mut many = demo::state().runs;
        for (i, run) in many.iter_mut().enumerate() {
            run.cwd = Some(PathBuf::from(format!("C:\\p{i}")));
        }
        assert_eq!(recent_projects(&many).len(), 4);
    }
}
