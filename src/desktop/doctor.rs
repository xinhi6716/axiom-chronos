//! The Settings screen: environment checks, versions and the preferences that persist.
use super::i18n::{fill, tr, Table};
use super::model::*;
use super::ornament;
use super::settings::{self, LanguageChoice, Store, ThemeChoice};
use super::theme::{Tokened as _, Tokens};
use crate::base;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::component::{Icon, IconName, Selectable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::time::Duration;

const T: Table = &[
    ("Check the environment and choose how Chronos looks.", "檢查環境，調整 Chronos 的語言與外觀。"),
    ("Environment", "環境檢查"),
    ("Re-check", "重新檢查"),
    ("Checking…", "檢查中…"),
    ("OK", "正常"),
    ("Warning", "注意"),
    ("Needs fixing", "需要修復"),
    ("Everything is ready", "一切就緒"),
    ("Ready. {} to look at when you have time.", "可以使用，另有 {} 項提醒。"),
    ("{} must be fixed before you can make a wish.", "有 {} 項必須先修復，才能許願。"),
    ("Checks have not run yet", "尚未執行檢查"),
    ("Results will appear here after the first check.", "完成第一次檢查後，結果會顯示在這裡。"),
    ("Install Claude Code, then run claude once to sign in.", "安裝 Claude Code，然後執行一次 claude 完成登入。"),
    ("Install the Codex CLI, then run codex once to sign in.", "安裝 Codex CLI，然後執行一次 codex 完成登入。"),
    (
        "Memory search needs SQLite 3.34 or later with FTS5. Reinstall Chronos.",
        "記憶搜尋需要 SQLite 3.34 以上版本並支援 FTS5，請重新安裝 Chronos。",
    ),
    (
        "Chronos could not write to its data folder. Check the folder's permissions, or set AXIOM_HOME to another folder.",
        "Chronos 無法寫入資料夾。請檢查資料夾權限，或用 AXIOM_HOME 指定其他資料夾。",
    ),
    (
        "Install PowerShell with: winget install --id Microsoft.PowerShell --exact --installer-type wix",
        "請用這個指令安裝 PowerShell：winget install --id Microsoft.PowerShell --exact --installer-type wix",
    ),
    ("Optional. Set up WSL so full test suites also run on Linux.", "非必要。設定 WSL 後，完整測試也會在 Linux 上跑。"),
    ("Optional. Chronos works without it.", "非必要，沒有它 Chronos 也能正常運作。"),
    ("Versions", "版本"),
    ("App", "應用程式"),
    ("Plugin", "外掛"),
    ("Does not match the app", "與應用程式不一致"),
    (
        "The plugin is {} but the app is {}. Install the same version of both.",
        "外掛是 {}，應用程式是 {}。請安裝相同版本。",
    ),
    ("Preferences", "偏好設定"),
    ("Language", "語言"),
    ("System", "跟隨系統"),
    ("Theme", "主題"),
    ("Light", "淺色"),
    ("Dark", "深色"),
    ("Reduce motion", "減少動態"),
    ("Turns off the particles and transitions.", "關閉粒子與過場動畫。"),
    ("Plain words", "白話翻譯"),
    ("Turns technical text into plain Traditional Chinese using Claude Haiku.", "使用 Claude Haiku 將技術文字轉成白話繁體中文。"),
    ("Data folder", "資料存放位置"),
    ("Open", "開啟"),
    ("Settings could not be saved: {}", "無法儲存設定：{}"),
    ("Could not open the folder: {}", "無法開啟資料夾：{}"),
];

const UNKNOWN: &str = "—";
/// How long Re-check stays busy when the check list does not change.
const RECHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// With no backend nothing will answer, so the button only acknowledges the press.
const RECHECK_ACKNOWLEDGE: Duration = Duration::from_millis(900);

fn check_glyph(status: CheckStatus) -> UnitState {
    match status {
        CheckStatus::Ok => UnitState::Verified,
        CheckStatus::Warn => UnitState::Built,
        CheckStatus::Fail => UnitState::Failed,
    }
}

fn status_word(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "OK",
        CheckStatus::Warn => "Warning",
        CheckStatus::Fail => "Needs fixing",
    }
}

fn fix_hint(name: &str) -> Option<&'static str> {
    Some(match name {
        "claude" => "Install Claude Code, then run claude once to sign in.",
        "codex" => "Install the Codex CLI, then run codex once to sign in.",
        "sqlite fts5" => "Memory search needs SQLite 3.34 or later with FTS5. Reinstall Chronos.",
        "state" => "Chronos could not write to its data folder. Check the folder's permissions, or set AXIOM_HOME to another folder.",
        "pwsh" => "Install PowerShell with: winget install --id Microsoft.PowerShell --exact --installer-type wix",
        "linux" => "Optional. Set up WSL so full test suites also run on Linux.",
        "codegraph" => "Optional. Chronos works without it.",
        _ => return None,
    })
}

/// Most urgent first, keeping the order within each level.
fn by_urgency(checks: &[Check]) -> Vec<Check> {
    let rank = |check: &Check| match check.status {
        CheckStatus::Fail => 0,
        CheckStatus::Warn => 1,
        CheckStatus::Ok => 2,
    };
    let mut sorted = checks.to_vec();
    sorted.sort_by_key(rank);
    sorted
}

fn count(checks: &[Check], status: CheckStatus) -> usize {
    checks.iter().filter(|check| check.status == status).count()
}

/// The version inside a tool's output, such as `0.159.3` in `codex-cli 0.159.3`.
fn version_in(detail: &str) -> Option<&str> {
    detail
        .split_whitespace()
        .map(|word| word.trim_start_matches('v').trim_end_matches(|c: char| !c.is_ascii_alphanumeric()))
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()) && word.contains('.'))
}

/// A failed check's text is an error message, so it says nothing about a version.
fn version_of(checks: &[Check], name: &str) -> Option<String> {
    let check = checks.iter().find(|check| check.name.eq_ignore_ascii_case(name))?;
    if check.status == CheckStatus::Fail {
        return None;
    }
    version_in(&check.detail).map(str::to_owned)
}

#[derive(Debug, PartialEq, Eq)]
struct Versions {
    app: String,
    plugin: Option<String>,
    claude: Option<String>,
    codex: Option<String>,
}

impl Versions {
    fn from_checks(checks: &[Check]) -> Self {
        Self {
            app: env!("CARGO_PKG_VERSION").to_owned(),
            plugin: version_of(checks, "plugin"),
            claude: version_of(checks, "claude"),
            codex: version_of(checks, "codex"),
        }
    }

    /// Only a version that is known can disagree.
    fn plugin_mismatch(&self) -> bool {
        self.plugin.as_ref().is_some_and(|plugin| *plugin != self.app)
    }
}

fn snapshot(checks: &[Check]) -> Vec<(SharedString, CheckStatus, SharedString)> {
    checks.iter().map(|check| (check.name.clone(), check.status, check.detail.clone())).collect()
}

struct Recheck {
    before: Vec<(SharedString, CheckStatus, SharedString)>,
    _timeout: Task<()>,
}

pub struct DoctorView {
    state: Entity<AppState>,
    data_folder: PathBuf,
    recheck: Option<Recheck>,
}

impl DoctorView {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, state, cx| {
            let done = this.recheck.as_ref().is_some_and(|recheck| recheck.before != snapshot(&state.read(cx).checks));
            if done {
                this.recheck = None;
            }
            cx.notify();
        })
        .detach();
        Self { state, data_folder: base::axiom_home(), recheck: None }
    }

    fn start_recheck(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let before = snapshot(&state.checks);
        let wait = if state.backend.is_some() { RECHECK_TIMEOUT } else { RECHECK_ACKNOWLEDGE };
        self.state.update(cx, |state, cx| state.recheck(cx));
        let timeout = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.recheck = None;
                cx.notify();
            });
        });
        self.recheck = Some(Recheck { before, _timeout: timeout });
        cx.notify();
    }

    fn open_data_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.data_folder.clone();
        cx.spawn_in(window, async move |_, cx| {
            let target = folder.clone();
            let made = cx.background_spawn(async move { std::fs::create_dir_all(&target) }).await;
            let _ = cx.update(|window, cx| match made {
                Ok(()) => cx.open_with_system(&folder),
                Err(error) => {
                    let message = fill(tr(cx, T, "Could not open the folder: {}"), &[&error.to_string()]);
                    window.push_notification(Notification::warning(message), cx);
                }
            });
        })
        .detach();
    }

    fn environment(&self, checks: &[Check], cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let checking = self.recheck.is_some();
        let failed = count(checks, CheckStatus::Fail);
        let warned = count(checks, CheckStatus::Warn);
        let (summary, color): (String, Hsla) = if checks.is_empty() {
            (tr(cx, T, "Checks have not run yet").into(), t.muted)
        } else if failed > 0 {
            (fill(tr(cx, T, "{} must be fixed before you can make a wish."), &[&failed.to_string()]), t.fail)
        } else if warned > 0 {
            (fill(tr(cx, T, "Ready. {} to look at when you have time."), &[&warned.to_string()]), t.muted)
        } else {
            (tr(cx, T, "Everything is ready").into(), t.ok)
        };
        let recheck = super::theme::gold_leaf(
            Button::new("recheck")
                .small()
                .icon(IconName::RefreshCw)
                .label(if checking { tr(cx, T, "Checking…") } else { tr(cx, T, "Re-check") })
                .loading(checking)
                .on_click(cx.listener(|this, _, _, cx| this.start_recheck(cx))),
            &t,
        );
        let body = if checks.is_empty() {
            div()
                .py_4()
                .text_sm()
                .text_color(t.muted)
                .child(tr(cx, T, "Results will appear here after the first check."))
                .into_any_element()
        } else {
            v_flex()
                .children(by_urgency(checks).iter().enumerate().map(|(index, check)| check_row(index, check, cx)))
                .into_any_element()
        };
        card(&t, tr(cx, T, "Environment"), Some(recheck.into_any_element()))
            .child(div().text_sm().text_color(color).pb_1().child(summary))
            .child(body)
    }

    fn versions(&self, checks: &[Check], cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let versions = Versions::from_checks(checks);
        let mismatch = versions.plugin_mismatch().then(|| {
            fill(
                tr(cx, T, "The plugin is {} but the app is {}. Install the same version of both."),
                &[versions.plugin.as_deref().unwrap_or(UNKNOWN), &versions.app],
            )
        });
        card(&t, tr(cx, T, "Versions"), None)
            .child(version_row(&t, tr(cx, T, "App"), Some(versions.app.clone()), None))
            .child(version_row(
                &t,
                tr(cx, T, "Plugin"),
                versions.plugin.clone(),
                mismatch.is_some().then(|| tr(cx, T, "Does not match the app")),
            ))
            .child(version_row(&t, "Claude Code", versions.claude.clone(), None))
            .child(version_row(&t, "Codex", versions.codex.clone(), None))
            .when_some(mismatch, |this, message| this.child(div().pt_2().child(Alert::warning("version-mismatch", message).small())))
    }

    fn preferences(&self, reduce_motion: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let prefs = Store::current(cx);
        let (language, theme_choice) = (prefs.language, prefs.theme);
        let languages = [
            ("lang-system", tr(cx, T, "System"), LanguageChoice::System),
            ("lang-zh", "繁體中文", LanguageChoice::ZhTw),
            ("lang-en", "English", LanguageChoice::English),
        ];
        let themes = [
            ("theme-system", tr(cx, T, "System"), ThemeChoice::System),
            ("theme-light", tr(cx, T, "Light"), ThemeChoice::Light),
            ("theme-dark", tr(cx, T, "Dark"), ThemeChoice::Dark),
        ];
        let state = self.state.clone();
        let folder: SharedString = self.data_folder.display().to_string().into();
        let error = Store::last_error(cx).map(|error| fill(tr(cx, T, "Settings could not be saved: {}"), &[&error]));
        card(&t, tr(cx, T, "Preferences"), None)
            .child(preference_row(
                &t,
                tr(cx, T, "Language"),
                None,
                ButtonGroup::new("language").small().outline().children(languages.map(|(id, label, choice)| {
                    segment(id, label, choice == language, cx)
                        .on_click(move |_, window, cx| settings::set_language(choice, window, cx))
                })),
            ))
            .child(preference_row(
                &t,
                tr(cx, T, "Theme"),
                None,
                ButtonGroup::new("theme").small().outline().children(themes.map(|(id, label, choice)| {
                    segment(id, label, choice == theme_choice, cx)
                        .on_click(move |_, window, cx| settings::set_theme(choice, window, cx))
                })),
            ))
            .child(preference_row(
                &t,
                tr(cx, T, "Reduce motion"),
                Some(tr(cx, T, "Turns off the particles and transitions.")),
                Switch::new("reduce-motion")
                    .checked(reduce_motion)
                    .accessibility_label(tr(cx, T, "Reduce motion"))
                    .on_click(move |on, _, cx| settings::set_reduce_motion(*on, &state, cx)),
            ))
            .child(preference_row(
                &t,
                tr(cx, T, "Plain words"),
                Some(tr(cx, T, "Turns technical text into plain Traditional Chinese using Claude Haiku.")),
                Switch::new("plain-words")
                    .checked(super::plain_text::enabled(cx))
                    .accessibility_label(tr(cx, T, "Plain words"))
                    .on_click(|on, _, cx| super::plain_text::set_enabled(*on, cx)),
            ))
            .child(
                v_flex()
                    .gap_1p5()
                    .pt_2()
                    .child(div().text_sm().child(tr(cx, T, "Data folder")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("data-folder")
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(t.muted)
                                    .truncate()
                                    .tooltip({
                                        let folder = folder.clone();
                                        move |window, cx| Tooltip::new(folder.clone()).build(window, cx)
                                    })
                                    .child(folder),
                            )
                            .child(
                                Button::new("open-data-folder")
                                    .small()
                                    .icon(IconName::FolderOpen)
                                    .label(tr(cx, T, "Open"))
                                    .on_click(cx.listener(|this, _, window, cx| this.open_data_folder(window, cx))),
                            ),
                    ),
            )
            .when_some(error, |this, message| this.child(div().pt_2().child(Alert::warning("save-failed", message).small())))
    }
}

fn card(t: &Tokens, title: &'static str, action: Option<AnyElement>) -> Stateful<Div> {
    ornament::frame(t, t.border, t.surface)
        .id(title)
        .flex()
        .flex_col()
        .p_4()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .gap_2()
                .child(ornament::section_label(t, title).flex_1().min_w_0())
                .children(action),
        )
}

/// One option of a segmented choice. The chosen one is filled, so it reads without relying on hue.
fn segment(id: &'static str, label: &'static str, chosen: bool, cx: &App) -> Button {
    let button = Button::new(id).label(label).selected(chosen);
    if !chosen {
        return button;
    }
    let t = cx.tokens();
    button.custom(ButtonCustomVariant::new(cx).color(t.gold).foreground(t.gold).hover(t.gold.opacity(0.22)).active(t.gold.opacity(0.18)))
}

fn check_row(index: usize, check: &Check, cx: &App) -> AnyElement {
    let t = cx.tokens();
    let color = t.check(check.status);
    let hint = (check.status != CheckStatus::Ok).then(|| fix_hint(&check.name)).flatten();
    let detail = check.detail.clone();
    v_flex()
        .py_2()
        .gap_0p5()
        .border_t_1()
        .border_color(if index == 0 { transparent_black() } else { t.border.opacity(0.6) })
        .child(
            h_flex()
                .gap_2()
                .child(ornament::glyph(check_glyph(check.status), 15.0, color, 0.0))
                .child(div().w(px(92.)).flex_shrink_0().text_sm().truncate().child(check.name.clone()))
                .child(
                    div()
                        .id(("check-detail", index))
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(t.muted)
                        .truncate()
                        .tooltip({
                            let detail = detail.clone();
                            move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
                        })
                        .child(detail),
                )
                .child(div().flex_shrink_0().text_xs().text_color(color).child(tr(cx, T, status_word(check.status)))),
        )
        .when_some(hint, |this, hint| {
            this.child(div().pl(px(23.)).text_xs().text_color(t.muted).child(tr(cx, T, hint)))
        })
        .into_any_element()
}

fn version_row(t: &Tokens, name: &'static str, version: Option<String>, problem: Option<&'static str>) -> impl IntoElement {
    h_flex()
        .h(px(28.))
        .justify_between()
        .text_sm()
        .child(div().text_color(t.muted).child(name))
        .child(
            h_flex()
                .gap_1p5()
                .when_some(problem, |this, problem| {
                    this.text_color(t.review)
                        .child(Icon::new(IconName::TriangleAlert).size(px(13.)))
                        .child(div().text_xs().child(problem))
                })
                .child(version.unwrap_or_else(|| UNKNOWN.to_owned())),
        )
}

fn preference_row(t: &Tokens, name: &'static str, description: Option<&'static str>, control: impl IntoElement) -> impl IntoElement {
    h_flex()
        .py_1p5()
        .gap_4()
        .justify_between()
        .child(
            v_flex()
                .min_w_0()
                .child(div().text_sm().child(name))
                .when_some(description, |this, text| this.child(div().text_xs().text_color(t.muted).child(text))),
        )
        .child(div().flex_shrink_0().child(control))
}

impl Render for DoctorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let state = self.state.read(cx);
        let checks = state.checks.clone();
        let reduce_motion = state.reduce_motion;
        let environment = self.environment(&checks, cx);
        let versions = self.versions(&checks, cx);
        let preferences = self.preferences(reduce_motion, cx);
        div().id("settings").size_full().overflow_y_scrollbar().child(
            v_flex()
                .w_full()
                .max_w(px(1100.))
                .mx_auto()
                .p_6()
                .gap_5()
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(ornament::page_title(&t, tr(cx, &[], "Settings")))
                        .child(div().text_sm().text_color(t.muted).child(tr(cx, T, "Check the environment and choose how Chronos looks."))),
                )
                .child(
                    h_flex()
                        .items_start()
                        .gap_5()
                        .child(v_flex().flex_1().min_w_0().gap_5().child(environment).child(versions))
                        .child(v_flex().w(px(400.)).flex_shrink_0().gap_5().child(preferences)),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;

    fn check(name: &'static str, status: CheckStatus, detail: &'static str) -> Check {
        Check { name: name.into(), status, detail: detail.into() }
    }

    #[test]
    fn check_glyphs_match_statuses_and_are_distinct() {
        let ok = check_glyph(CheckStatus::Ok);
        let warn = check_glyph(CheckStatus::Warn);
        let fail = check_glyph(CheckStatus::Fail);
        assert_eq!(ok, UnitState::Verified);
        assert_eq!(warn, UnitState::Built);
        assert_eq!(fail, UnitState::Failed);
        assert_ne!(ok, warn);
        assert_ne!(ok, fail);
        assert_ne!(warn, fail);
    }

    #[test]
    fn versions_are_read_out_of_tool_output() {
        assert_eq!(version_in("2.1.317 (Claude Code)"), Some("2.1.317"));
        assert_eq!(version_in("codex-cli 0.159.3"), Some("0.159.3"));
        assert_eq!(version_in("v1.0.0"), Some("1.0.0"));
        assert_eq!(version_in("1.2.3-beta.1 (build)"), Some("1.2.3-beta.1"));
        assert_eq!(version_in("0.9.4, installed"), Some("0.9.4"));
        assert_eq!(version_in("not on PATH"), None);
        assert_eq!(version_in(""), None);
    }

    #[test]
    fn a_version_comes_only_from_a_check_that_did_not_fail() {
        let checks = [
            check("Claude", CheckStatus::Ok, "2.1.317 (Claude Code)"),
            check("codex", CheckStatus::Fail, "codex 0.1.0 crashed"),
        ];
        assert_eq!(version_of(&checks, "claude").as_deref(), Some("2.1.317"));
        assert_eq!(version_of(&checks, "codex"), None);
        assert_eq!(version_of(&checks, "plugin"), None);
    }

    #[test]
    fn unknown_versions_never_count_as_a_mismatch() {
        let unknown = Versions { app: "1.0.0".into(), plugin: None, claude: None, codex: None };
        assert!(!unknown.plugin_mismatch());
        let same = Versions { plugin: Some("1.0.0".into()), ..unknown };
        assert!(!same.plugin_mismatch());
        let different = Versions { plugin: Some("0.9.4".into()), ..same };
        assert!(different.plugin_mismatch());
    }

    #[test]
    fn the_app_version_is_always_known() {
        let versions = Versions::from_checks(&[]);
        assert_eq!(versions.app, env!("CARGO_PKG_VERSION"));
        assert_eq!((versions.plugin, versions.claude, versions.codex), (None, None, None));
    }

    #[test]
    fn problems_come_first_and_keep_their_order() {
        let checks = [
            check("a", CheckStatus::Ok, ""),
            check("b", CheckStatus::Warn, ""),
            check("c", CheckStatus::Fail, ""),
            check("d", CheckStatus::Ok, ""),
            check("e", CheckStatus::Fail, ""),
        ];
        let names: Vec<_> = by_urgency(&checks).iter().map(|c| c.name.to_string()).collect();
        assert_eq!(names, ["c", "e", "b", "a", "d"]);
    }

    #[test]
    fn a_changed_check_is_visible_in_the_snapshot() {
        let before = snapshot(&[check("claude", CheckStatus::Fail, "missing")]);
        assert_eq!(before, snapshot(&[check("claude", CheckStatus::Fail, "missing")]));
        assert_ne!(before, snapshot(&[check("claude", CheckStatus::Ok, "2.1.317")]));
    }

    #[test]
    fn every_translation_key_used_here_exists() {
        let source = include_str!("doctor.rs");
        let used = regex::Regex::new(r#"tr\(cx, T, "((?:[^"\\]|\\.)*)"\)"#).unwrap();
        for capture in used.captures_iter(source) {
            assert!(T.iter().any(|(en, _)| *en == &capture[1]), "missing translation: {}", &capture[1]);
        }
        for status in [CheckStatus::Ok, CheckStatus::Warn, CheckStatus::Fail] {
            assert!(T.iter().any(|(en, _)| *en == status_word(status)));
        }
        for name in ["claude", "codex", "sqlite fts5", "state", "pwsh", "linux", "codegraph"] {
            let hint = fix_hint(name).unwrap();
            assert!(T.iter().any(|(en, _)| *en == hint), "missing translation: {hint}");
        }
    }
}
