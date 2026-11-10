//! Auto-evolve preview and explicit confirmation.
use super::i18n::{self, tr};
use super::model::{AppState, EvolveOutcome, EvolveState, Verdict};
use super::theme::Tokened as _;
use super::{ornament, theme};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{v_flex, Disableable as _, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::{Path, PathBuf};

const TEXT: i18n::Table = &[
    ("Auto-evolve", "自動演進"),
    ("Check and improve this repository", "檢查並改善此儲存庫"),
    ("Checking for issues…", "正在檢查問題…"),
    ("Starting…", "正在啟動…"),
    ("Issues", "問題"),
    ("{title} — {count} occurrences", "{title} — 出現 {count} 次"),
    ("and {n} more", "另外還有 {n} 項"),
    ("No issues found.", "沒有發現問題。"),
    ("Estimated cost", "預估花費"),
    ("Cannot estimate", "無法估計"),
    ("Source gaps", "資料來源缺漏"),
    ("Repository to change", "將修改的儲存庫"),
    ("Choose a repository folder", "選擇儲存庫資料夾"),
    ("Choose folder", "選擇資料夾"),
    ("Change folder", "變更資料夾"),
    ("Past runs", "過去的執行"),
    ("No past runs.", "尚無過去的執行。"),
    ("Improved", "有改善"),
    ("Not improved", "沒有改善"),
    ("Unknown", "未知"),
    ("The working tree has uncommitted changes. Commit them first.", "工作目錄有未提交的變更，請先提交。"),
    ("No repository was found. Choose a repository folder.", "找不到儲存庫，請選擇儲存庫資料夾。"),
    ("This folder is not a Git repository. Choose a Git repository.", "這個資料夾不是 Git 儲存庫，請選擇 Git 儲存庫。"),
    ("This is not the repository root. Choose the root folder.", "這不是儲存庫根目錄，請選擇根目錄。"),
    ("This repository is not the expected package. Choose the correct repository.", "此儲存庫不是預期的套件，請選擇正確的儲存庫。"),
    ("The evolution branch already exists. Resolve the existing branch first.", "演進分支已存在，請先處理既有分支。"),
    ("Could not start auto-evolve. Check the details and try again.", "無法啟動自動演進，請查看詳細資訊後再試。"),
    ("Auto-evolve is already busy. Wait for it to finish.", "自動演進正在執行，請等待完成。"),
    ("Auto-evolve is unavailable. Check the service and try again.", "自動演進無法使用，請檢查服務後再試。"),
    ("Auto-evolve failed. Check the details and try again.", "自動演進失敗，請查看詳細資訊後再試。"),
    ("Copy details", "複製詳細資訊"),
    ("No changes were made: {reason}", "未進行任何變更：{reason}"),
    ("Auto-evolve finished with exit code {code}.", "自動演進已結束，結束代碼為 {code}。"),
    ("Cancel", "取消"),
    ("Start auto-evolve", "開始自動演進"),
];

fn failure_wording(kind: &str) -> &'static str {
    match kind {
        "dirty_tree" => "The working tree has uncommitted changes. Commit them first.",
        "no_repo" => "No repository was found. Choose a repository folder.",
        "not_git_repo" => "This folder is not a Git repository. Choose a Git repository.",
        "not_repo_root" => "This is not the repository root. Choose the root folder.",
        "wrong_package" => "This repository is not the expected package. Choose the correct repository.",
        "branch_exists" => "The evolution branch already exists. Resolve the existing branch first.",
        "launch_failed" => "Could not start auto-evolve. Check the details and try again.",
        "busy" => "Auto-evolve is already busy. Wait for it to finish.",
        "unavailable" => "Auto-evolve is unavailable. Check the service and try again.",
        _ => "Auto-evolve failed. Check the details and try again.",
    }
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Improved => "Improved",
        Verdict::NotImproved => "Not improved",
        Verdict::Unknown => "Unknown",
    }
}

fn cost_wording(cost: Option<f64>, cannot_estimate: &str) -> String {
    match cost.filter(|value| value.is_finite() && *value >= 0.0) {
        Some(value) if value > 0.0 && value < 0.01 => "<$0.01".to_owned(),
        Some(value) => format!("${value:.2}"),
        None => cannot_estimate.to_owned(),
    }
}

fn confirm_disabled(evolve: &EvolveState, open: bool, repo: Option<&Path>) -> bool {
    !open || evolve.checking || evolve.starting || evolve.outcome.is_some() || repo.is_none()
        || evolve.preview.as_ref().is_none_or(|p| p.issues.is_empty())
}

fn start_repo(evolve: &EvolveState, open: bool, picked: Option<&PathBuf>) -> Option<PathBuf> {
    let repo = picked.or_else(|| evolve.preview.as_ref().and_then(|p| p.repo.as_ref()));
    if confirm_disabled(evolve, open, repo.map(PathBuf::as_path)) { None } else { repo.cloned() }
}

pub struct EvolvePanel {
    state: Entity<AppState>,
    open: bool,
    picked: Option<PathBuf>,
    generation: u64,
    cancel_focus: FocusHandle,
    restore_focus: Option<FocusHandle>,
}

impl EvolvePanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let open = state.read(cx).scene.as_deref() == Some("evolve");
        let cancel_focus = cx.focus_handle();
        let restore_focus = if open { window.focused(cx) } else { None };
        if open { state.update(cx, |state, cx| state.evolve_check(cx)); }
        if open {
            let focus = cancel_focus.clone();
            cx.defer_in(window, move |_, window, cx| focus.focus(window, cx));
        }
        Self { state, open, picked: None, generation: 0, cancel_focus, restore_focus }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.picked = None;
        self.open = true;
        self.restore_focus = window.focused(cx);
        self.cancel_focus.focus(window, cx);
        self.state.update(cx, |state, cx| state.evolve_check(cx));
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.picked = None;
        self.generation = self.generation.wrapping_add(1);
        if let Some(focus) = self.restore_focus.take() { focus.focus(window, cx); }
        self.state.update(cx, |state, cx| state.evolve_dismiss(cx));
        cx.notify();
    }

    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false, directories: true, multiple: false,
            prompt: Some(tr(cx, TEXT, "Choose a repository folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(folder) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, _, cx| {
                if this.open && this.generation == generation {
                    this.picked = Some(folder);
                    cx.notify();
                }
            });
        }).detach();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let evolve = &self.state.read(cx).evolve;
        let Some(repo) = start_repo(evolve, self.open, self.picked.as_ref()) else { return };
        self.state.update(cx, |state, cx| { state.evolve_start(repo, cx); });
    }

    fn dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let evolve = self.state.read(cx).evolve.clone();
        let repo = self.picked.as_ref().or_else(|| evolve.preview.as_ref().and_then(|p| p.repo.as_ref()));
        let disabled = confirm_disabled(&evolve, true, repo.map(PathBuf::as_path));
        let mut body = v_flex().gap_3().w_full().text_sm().text_color(t.text);
        if evolve.checking { body = body.child(tr(cx, TEXT, "Checking for issues…")); }
        if evolve.starting { body = body.child(tr(cx, TEXT, "Starting…")); }
        if let Some(preview) = &evolve.preview {
            let mut issues = v_flex().gap_1().child(div().font_medium().child(tr(cx, TEXT, "Issues")));
            for (ix, issue) in preview.issues.iter().enumerate() {
                let line: SharedString = tr(cx, TEXT, "{title} — {count} occurrences")
                    .replace("{title}", &issue.title).replace("{count}", &issue.count.to_string()).into();
                issues = issues.child(div().id(("evolve-issue", ix)).role(Role::Label).aria_label(line.clone())
                    .child(line.clone().test_text(line)).test_support());
            }
            if preview.issues.is_empty() { issues = issues.child(tr(cx, TEXT, "No issues found.")); }
            if preview.more_issues > 0 {
                let line: SharedString = tr(cx, TEXT, "and {n} more").replace("{n}", &preview.more_issues.to_string()).into();
                issues = issues.child(div().id("evolve-more").role(Role::Label).aria_label(line.clone())
                    .child(line.clone().test_text(line)).test_support());
            }
            let cost: SharedString = cost_wording(preview.estimate_usd, tr(cx, TEXT, "Cannot estimate")).into();
            body = body.child(issues)
                .child(v_flex().gap_1().child(div().font_medium().child(tr(cx, TEXT, "Estimated cost")))
                    .child(div().id("evolve-cost").role(Role::Label).aria_label(cost.clone())
                        .child(cost.clone().test_text(cost)).test_support()));
            if !preview.gaps.is_empty() {
                let mut gaps = v_flex().gap_1().child(div().font_medium().child(tr(cx, TEXT, "Source gaps")));
                for (ix, gap) in preview.gaps.iter().enumerate() {
                    let line: SharedString = format!("{} — {} — {}", gap.family, gap.location, gap.reason).into();
                    gaps = gaps.child(div().id(("evolve-gap", ix)).role(Role::Label).aria_label(line.clone())
                        .child(line.clone().test_text(line)).test_support());
                }
                body = body.child(gaps);
            }
        }
        if evolve.preview.is_some() || self.picked.is_some() {
            let mut repository = v_flex().gap_1().child(div().font_medium().child(tr(cx, TEXT, "Repository to change")));
            if let Some(repo) = repo {
                let path: SharedString = repo.to_string_lossy().into_owned().into();
                repository = repository.child(div().id("evolve-repo").role(Role::Label).aria_label(path.clone())
                    .child(path.clone().test_text(path)).test_support());
            }
            let pick_label = if repo.is_some() { tr(cx, TEXT, "Change folder") } else { tr(cx, TEXT, "Choose folder") };
            body = body.child(repository.child(Button::new("evolve-pick").label(pick_label)
                .on_click(cx.listener(|this, _, window, cx| this.pick(window, cx)))));
        }
        if let Some(error) = &evolve.error {
            let message = error.message.clone();
            let wording: SharedString = tr(cx, TEXT, failure_wording(&error.kind)).into();
            body = body.child(v_flex().gap_1().text_color(t.fail)
                .child(div().id("evolve-error").role(Role::Label).aria_label(wording.clone())
                    .child(wording.clone().test_text(wording)).test_support())
                .child(Button::new("evolve-copy").label(tr(cx, TEXT, "Copy details"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(message.to_string())))));
        }
        if let Some(outcome) = &evolve.outcome {
            let result: SharedString = match outcome {
                EvolveOutcome::NoOp { reason } => tr(cx, TEXT, "No changes were made: {reason}").replace("{reason}", reason),
                EvolveOutcome::Ran { exit_code } => tr(cx, TEXT, "Auto-evolve finished with exit code {code}.").replace("{code}", &exit_code.to_string()),
            }.into();
            body = body.child(div().id("evolve-outcome").role(Role::Label).aria_label(result.clone())
                .child(result.clone().test_text(result)).test_support());
        }
        let mut runs = v_flex().gap_1().child(div().font_medium().child(tr(cx, TEXT, "Past runs")));
        if evolve.runs.is_empty() { runs = runs.child(tr(cx, TEXT, "No past runs.")); }
        for (ix, run) in evolve.runs.iter().enumerate() {
            let summary: SharedString = format!("{} — {}", tr(cx, TEXT, verdict_label(run.verdict)), run.repo).into();
            let detail: SharedString = format!("{} · {}", run.branch, run.state).into();
            runs = runs.child(div().id(("evolve-run", ix)).py_1().border_b_1().border_color(t.border)
                .role(Role::Label).aria_label(format!("{} — {}", summary, detail))
                .child(summary.clone().test_text(summary))
                .child(div().text_color(t.muted).child(detail.clone().test_text(detail)))
                .test_support());
        }
        body = body.child(runs);
        let confirm_label: SharedString = tr(cx, TEXT, "Start auto-evolve").into();
        let footer = DialogFooter::new()
            .child(Button::new("evolve-cancel").label(tr(cx, TEXT, "Cancel"))
                .track_focus(&self.cancel_focus)
                .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))))
            .child(Button::new("evolve-confirm").map(|b| theme::gold_leaf(b, &t))
                .accessibility_label(confirm_label.clone())
                .child(div().min_w_0().whitespace_nowrap().text_ellipsis()
                    .child(confirm_label.clone().test_text(confirm_label)))
                .disabled(disabled)
                .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))));
        let viewport = window.viewport_size();
        let mut backdrop = t.canvas;
        backdrop.a = 0.8;
        anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.)))
            .child(div().w(viewport.width).h(viewport.height).bg(backdrop).flex().items_center().justify_center().occlude()
                .child(ornament::frame(&t, t.gold, t.surface).w(px(620.)).max_w(viewport.width - px(32.))
                    .p_5().shadow_lg().child(v_flex().gap_4().w_full()
                        .child(div().text_lg().font_medium().text_color(t.text).child(tr(cx, TEXT, "Check and improve this repository")))
                        .child(div().id("evolve-content").max_h(viewport.height - px(200.)).overflow_y_scrollbar().child(body))
                        .child(footer))))
    }
}

impl Render for EvolvePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let dialog = self.open.then(|| self.dialog(window, cx).into_any_element());
        div().child(Button::new("evolve-open").map(|b| theme::gold_leaf(b, &t))
            .label(tr(cx, TEXT, "Auto-evolve"))
            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))))
            .children(dialog.map(|dialog| deferred(dialog).with_priority(gpui_kit::base::POPUP_PRIORITY)))
    }
}

#[cfg(test)]
mod tests {
    use super::{confirm_disabled, cost_wording, failure_wording, start_repo, verdict_label, EvolveOutcome, EvolveState, Verdict, TEXT};
    use super::super::model::{EvolveIssue, EvolvePreview};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn ready_evolve() -> EvolveState {
        EvolveState {
            preview: Some(EvolvePreview {
                issues: vec![EvolveIssue { id: "issue".into(), title: "Issue".into(), count: 1, cost_usd: None }],
                more_issues: 0, estimate_usd: None, estimate_samples: 0, gaps: vec![],
                repo: Some(PathBuf::from("preview-repo")), text: "".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn confirmation_selects_picked_repo_before_preview_repo() {
        let mut evolve = ready_evolve();
        let preview_repo = PathBuf::from("preview-repo");
        let picked = PathBuf::from("picked-repo");
        assert!(!confirm_disabled(&evolve, true, Some(preview_repo.as_path())));
        assert_eq!(start_repo(&evolve, true, None), Some(preview_repo));
        assert_eq!(start_repo(&evolve, true, Some(&picked)), Some(picked.clone()));
        evolve.preview.as_mut().unwrap().repo = None;
        assert!(!confirm_disabled(&evolve, true, Some(picked.as_path())));
        assert_eq!(start_repo(&evolve, true, Some(&picked)), Some(picked));
    }

    #[test]
    fn confirmation_rejects_each_blocking_condition() {
        let ready = ready_evolve();
        let picked = PathBuf::from("picked-repo");
        let mut checking = ready.clone();
        checking.checking = true;
        let mut starting = ready.clone();
        starting.starting = true;
        let mut finished = ready.clone();
        finished.outcome = Some(EvolveOutcome::NoOp { reason: "done".into() });
        let mut missing_preview = ready.clone();
        missing_preview.preview = None;
        let mut no_issues = ready.clone();
        no_issues.preview.as_mut().unwrap().issues.clear();
        for (name, evolve, open) in [
            ("closed", &ready, false), ("checking", &checking, true), ("starting", &starting, true),
            ("finished", &finished, true), ("missing preview", &missing_preview, true),
            ("no issues", &no_issues, true),
        ] {
            assert!(confirm_disabled(evolve, open, Some(picked.as_path())), "{name}");
            assert_eq!(start_repo(evolve, open, Some(&picked)), None, "{name}");
        }
        let mut no_repo = ready;
        no_repo.preview.as_mut().unwrap().repo = None;
        assert!(confirm_disabled(&no_repo, true, None));
        assert_eq!(start_repo(&no_repo, true, None), None);
    }

    #[test]
    fn translations_cover_drawn_strings_and_placeholders() {
        let source = include_str!("evolve.rs");
        let calls = regex::Regex::new(r#"tr\(\s*cx,\s*TEXT,\s*"([^"]+)""#).unwrap();
        for call in calls.captures_iter(source) {
            let key = &call[1];
            assert!(TEXT.iter().any(|(en, _)| *en == key), "missing translation: {key}");
        }
        let placeholders = regex::Regex::new(r"\{[a-z_]+\}").unwrap();
        for (en, zh) in TEXT {
            assert!(!zh.is_empty(), "{en}");
            let keys = |s: &str| placeholders.find_iter(s).map(|m| m.as_str().to_owned()).collect::<BTreeSet<_>>();
            assert_eq!(keys(en), keys(zh), "{en}");
        }
        for kind in ["dirty_tree", "no_repo", "not_git_repo", "not_repo_root", "wrong_package", "branch_exists", "launch_failed", "busy", "unavailable", "other"] {
            assert!(TEXT.iter().any(|(en, _)| *en == failure_wording(kind)), "{kind}");
        }
        for verdict in [Verdict::Improved, Verdict::NotImproved, Verdict::Unknown] {
            assert!(TEXT.iter().any(|(en, _)| *en == verdict_label(verdict)));
        }
    }

    #[test]
    fn helpers_explain_failures_verdicts_and_cost() {
        assert_ne!(failure_wording("dirty_tree"), failure_wording("no_repo"));
        assert_eq!(failure_wording("other"), failure_wording("unexpected"));
        assert_eq!(verdict_label(Verdict::Improved), "Improved");
        assert_eq!(verdict_label(Verdict::NotImproved), "Not improved");
        assert_eq!(verdict_label(Verdict::Unknown), "Unknown");
        assert_eq!(cost_wording(Some(2.5), "unknown"), "$2.50");
        assert_eq!(cost_wording(None, "unknown"), "unknown");
        assert_eq!(cost_wording(Some(f64::NAN), "unknown"), "unknown");
    }

    #[test]
    fn small_positive_cost_does_not_look_free() {
        assert_eq!(cost_wording(Some(0.001), "unknown"), "<$0.01");
        assert_eq!(cost_wording(Some(0.009), "unknown"), "<$0.01");
        assert_eq!(cost_wording(Some(0.0), "unknown"), "$0.00");
        assert_eq!(cost_wording(Some(0.01), "unknown"), "$0.01");
    }
}
