//! The Run screen: the wish, the stage rail, three resizable columns and the control bar,
//! plus what surrounds them: setup problems, no runs yet, and the summary when a run ends.
use super::i18n::{tr, Table};
use super::model::*;
use super::theme::{Tokened as _, Tokens};
use super::{control::Control, detail::Detail, mesh::Mesh, todo::Todo};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::resizable::{h_resizable, resizable_panel, ResizableState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;

const TEXT: Table = &[
    ("Started in terminal: view only", "在終端機啟動：僅供檢視"),
    ("Elapsed", "經過時間"),
    ("Cost", "費用"),
    ("Started", "開始於"),
    ("Show the whole wish", "展開完整願望"),
    ("Show one line", "收合成一行"),
    ("No wish text was recorded", "沒有記錄到願望內容"),
    ("The wish is longer than this. Only its beginning is shown.", "願望比這裡顯示的更長，這裡只列出開頭。"),
    ("The folder of this wish is not known", "不知道這個願望的資料夾在哪裡"),
    ("That folder no longer exists", "找不到這個資料夾，可能已被移動或刪除"),
    ("That report file no longer exists", "找不到這份報告檔案，可能已被移動或刪除"),
    ("API-price equivalent. Subscription plans do not bill this.", "這是換算成 API 價格的金額，訂閱方案不會照此收費。"),
    ("Not included, no price reported:", "未計入，因為沒有回報價格："),
    ("API-price equivalent", "以 API 價格換算"),
    ("Not reported", "尚未回報"),
    ("This run has not reported a cost.", "這次執行還沒有回報費用。"),
    ("Waiting", "等待中"),
    ("Not finished", "未完成"),
    ("Stages have not been reported yet", "還沒有階段資訊"),
    ("Ended, needs attention", "已結束，需要留意"),
    ("1 warning", "1 則警告"),
    ("{n} warnings", "{n} 則警告"),
    ("Chronos cannot start a wish yet", "還不能開始許願"),
    ("Check again", "重新檢查"),
    ("Open Settings", "開啟設定"),
    ("Fix the items above to make a wish", "先修好上面的項目，才能許願"),
    ("Install Claude Code and sign in with `claude`.", "安裝 Claude Code，再執行 `claude` 登入。"),
    ("Install the Codex CLI and sign in.", "安裝 Codex CLI，並登入帳號。"),
    ("Memory needs SQLite 3.34 or later with FTS5.", "記憶功能需要 SQLite 3.34 以上版本，並支援 FTS5。"),
    ("Make this folder writable, or set AXIOM_HOME to one that is.", "讓這個資料夾可以寫入，或把 AXIOM_HOME 設成可寫入的資料夾。"),
    ("Remove the Store build of PowerShell and install it with winget.", "移除 Microsoft Store 版的 PowerShell，改用 winget 安裝。"),
    ("Start with a wish", "從一個願望開始"),
    ("Say what you want done. Chronos plans it, builds it and checks each piece while you watch.", "說出你想完成的事，Chronos 會規劃、施工並逐項驗收，過程都看得到。"),
    ("Make a wish", "許個願"),
    ("No wish is open", "目前沒有開啟的願望"),
    ("Open one from Wishes, or make a new one.", "從「願望」開啟一個，或許一個新的。"),
    ("Open Wishes", "開啟願望清單"),
    ("Every unit is verified", "所有單元都已驗收"),
    ("1 unit is not verified", "有 1 個單元尚未驗收"),
    ("{n} units are not verified", "有 {n} 個單元尚未驗收"),
    ("Every unit is verified, but the full test failed", "單元都已驗收，但完整測試失敗"),
    ("Every unit is verified, but the full test did not run", "單元都已驗收，但完整測試沒有執行"),
    ("No units were recorded for this wish", "這個願望沒有記錄到任何單元"),
    ("Units verified", "已驗收單元"),
    ("Passed", "通過"),
    ("Did not run", "沒有執行"),
    ("What needs attention", "需要留意"),
    ("Nothing needs attention", "沒有需要留意的項目"),
    ("{stage} failed", "{stage}失敗"),
    ("Open the report", "查看報告"),
    ("Hide the report", "收起報告"),
    ("No report was written", "這次沒有產生報告"),
    ("The report is longer than this. Only its beginning is shown.", "報告比這裡顯示的更長，這裡只列出開頭。"),
    ("See the graph", "查看執行圖"),
    ("Show the summary", "顯示摘要"),
];

const STAGE_NAMES: [&str; 7] = ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report"];
const FULL_TEST: &str = "Full test";

const UNKNOWN: &str = "—";
const ONE_LINE_CHARS: usize = 400;
const TOOLTIP_CHARS: usize = 600;
const LONG_TEXT_CHARS: usize = 20_000;

const CHECKLIST_WIDTH: Pixels = px(300.);
const CHECKLIST_RANGE: std::ops::Range<Pixels> = px(240.)..px(480.);
const GRAPH_MIN: Pixels = px(320.);
const DETAILS_WIDTH: Pixels = px(380.);
const DETAILS_RANGE: std::ops::Range<Pixels> = px(300.)..px(560.);

/// A stage as it should read given what its run is doing now. A stage still marked running
/// in a paused run is not running, and one still waiting in a finished run never will.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageState {
    Done,
    Running,
    Waiting,
    Failed,
    NeedsYou,
    OnBatch,
    Unfinished,
}

impl StageState {
    fn of(phase: Phase, status: RunStatus) -> Self {
        match (phase, status) {
            (Phase::Done, _) => Self::Done,
            (Phase::Failed, _) => Self::Failed,
            (_, RunStatus::Done) => Self::Unfinished,
            (Phase::Waiting, _) => Self::Waiting,
            (Phase::Running, RunStatus::Active) => Self::Running,
            (Phase::Running, RunStatus::Waiting) => Self::OnBatch,
            (Phase::Running, RunStatus::Blocked) => Self::NeedsYou,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Failed => "Failed",
            Self::NeedsYou => RunStatus::Blocked.label(),
            Self::OnBatch => RunStatus::Waiting.label(),
            Self::Unfinished => "Not finished",
        }
    }

    fn color(self, t: &Tokens) -> Hsla {
        match self {
            Self::Done => t.ok,
            Self::Running => t.run,
            Self::Waiting | Self::Unfinished => t.idle,
            Self::Failed | Self::NeedsYou => t.fail,
            Self::OnBatch => t.review,
        }
    }

    fn is_current(self) -> bool {
        matches!(self, Self::Running | Self::Failed | Self::NeedsYou | Self::OnBatch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FullTest {
    Passed,
    Failed,
    NotRun,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Clean,
    NotVerified,
    TestFailed,
    TestNotRun,
    NoUnits,
}

/// What a finished run actually achieved. Only a verified unit counts.
#[derive(Clone, Debug, PartialEq)]
struct Summary {
    verified: usize,
    total: usize,
    /// Indices into `Run::units`, the most urgent first.
    unverified: Vec<usize>,
    full_test: FullTest,
    full_test_note: Option<SharedString>,
    verdict: Verdict,
}

impl Summary {
    fn of(run: &Run) -> Self {
        let mut unverified: Vec<usize> = (0..run.units.len())
            .filter(|&i| run.units[i].state != UnitState::Verified)
            .collect();
        unverified.sort_by_key(|&i| urgency(run.units[i].state));
        let total = run.units.len();
        let test_stage = run.stages.iter().find(|s| s.name.eq_ignore_ascii_case(FULL_TEST));
        let full_test = match test_stage.map(|s| s.phase) {
            Some(Phase::Done) => FullTest::Passed,
            Some(Phase::Failed) => FullTest::Failed,
            Some(_) => FullTest::NotRun,
            None => FullTest::Unknown,
        };
        let verdict = if total == 0 {
            Verdict::NoUnits
        } else if !unverified.is_empty() {
            Verdict::NotVerified
        } else {
            match full_test {
                FullTest::Failed => Verdict::TestFailed,
                FullTest::NotRun => Verdict::TestNotRun,
                FullTest::Passed | FullTest::Unknown => Verdict::Clean,
            }
        };
        Self {
            verified: total - unverified.len(),
            total,
            unverified,
            full_test,
            full_test_note: test_stage.and_then(|s| s.note.clone()),
            verdict,
        }
    }

    fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            self.verified as f32 / self.total as f32 * 100.
        }
    }

    fn headline(&self) -> (&'static str, Option<usize>) {
        match self.verdict {
            Verdict::Clean => ("Every unit is verified", None),
            Verdict::NotVerified if self.unverified.len() == 1 => ("1 unit is not verified", None),
            Verdict::NotVerified => ("{n} units are not verified", Some(self.unverified.len())),
            Verdict::TestFailed => ("Every unit is verified, but the full test failed", None),
            Verdict::TestNotRun => ("Every unit is verified, but the full test did not run", None),
            Verdict::NoUnits => ("No units were recorded for this wish", None),
        }
    }
}

fn urgency(state: UnitState) -> u8 {
    match state {
        UnitState::Failed => 0,
        UnitState::Built => 1,
        UnitState::Running => 2,
        UnitState::Pending => 3,
        UnitState::Verified => 4,
    }
}

/// The wish on one line. Bounded first, so a pasted document cannot stall text layout.
fn one_line(text: &str, limit: usize) -> SharedString {
    let start: String = text.trim_start().chars().take(limit).collect();
    start.split_whitespace().collect::<Vec<_>>().join(" ").into()
}

/// The start of `text` and whether anything was cut off.
fn beginning(text: &str, limit: usize) -> (&str, bool) {
    match text.char_indices().nth(limit) {
        Some((end, _)) => (&text[..end], true),
        None => (text, false),
    }
}

fn cost_text(usd: Option<f64>) -> SharedString {
    match usd {
        Some(value) if value.is_finite() && value >= 0. => format!("${value:.2}").into(),
        _ => UNKNOWN.into(),
    }
}

fn unpriced_models(usage: &Usage) -> Vec<SharedString> {
    usage.models.iter().filter(|m| m.usd.is_none()).map(|m| m.model.clone()).collect()
}

fn required_failures(checks: &[Check]) -> Vec<&Check> {
    checks.iter().filter(|c| c.status == CheckStatus::Fail).collect()
}

fn fix_hint(check: &str) -> Option<&'static str> {
    match check {
        "claude" => Some("Install Claude Code and sign in with `claude`."),
        "codex" => Some("Install the Codex CLI and sign in."),
        "sqlite fts5" => Some("Memory needs SQLite 3.34 or later with FTS5."),
        "state" => Some("Make this folder writable, or set AXIOM_HOME to one that is."),
        "pwsh" => Some("Remove the Store build of PowerShell and install it with winget."),
        _ => None,
    }
}

/// The doctor appends its fix to the detail in English. The card shows the fix on its own line,
/// translated, so it is removed here rather than said twice.
fn without_fix<'a>(detail: &'a str, fix: Option<&str>) -> &'a str {
    fix.and_then(|fix| detail.strip_suffix(fix)).map_or(detail, str::trim_end)
}

fn stage_key(name: &str) -> Option<&'static str> {
    STAGE_NAMES.iter().copied().find(|key| *key == name)
}

fn stage_name(name: &SharedString, cx: &App) -> SharedString {
    stage_key(name).map_or_else(|| name.clone(), |key| tr(cx, TEXT, key).into())
}

fn counted(template: &'static str, count: usize, cx: &App) -> SharedString {
    tr(cx, TEXT, template).replace("{n}", &count.to_string()).into()
}

fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| {
        let text = text.clone();
        Tooltip::element(move |_, _| div().max_w(px(440.)).whitespace_normal().child(text.clone())).build(window, cx)
    }
}

fn cost_tooltip(usage: &Usage, cx: &App) -> SharedString {
    if usage.total_usd.is_none() {
        return tr(cx, TEXT, "This run has not reported a cost.").into();
    }
    let mut text = tr(cx, TEXT, "API-price equivalent. Subscription plans do not bill this.").to_string();
    let missing = unpriced_models(usage);
    if !missing.is_empty() {
        let names = missing.iter().map(|m| m.as_ref()).collect::<Vec<_>>().join(", ");
        text.push_str(&format!("\n{} {names}", tr(cx, TEXT, "Not included, no price reported:")));
    }
    text.into()
}

fn unit_icon(state: UnitState) -> IconName {
    match state {
        UnitState::Failed => IconName::CircleX,
        UnitState::Built => IconName::Eye,
        UnitState::Running => IconName::LoaderCircle,
        UnitState::Pending => IconName::Dash,
        UnitState::Verified => IconName::CircleCheck,
    }
}

fn stage_icon(state: StageState, animate: bool, t: &Tokens) -> AnyElement {
    let color = state.color(t);
    let icon = |name: IconName| Icon::new(name).size(px(16.)).text_color(color).into_any_element();
    match state {
        StageState::Done => icon(IconName::CircleCheck),
        StageState::Running if animate => {
            Spinner::new().icon(IconName::LoaderCircle).color(color).with_size(px(16.)).into_any_element()
        }
        StageState::Running => icon(IconName::LoaderCircle),
        StageState::Failed => icon(IconName::CircleX),
        StageState::NeedsYou => icon(IconName::CircleAlert),
        StageState::OnBatch => icon(IconName::Pause),
        StageState::Unfinished => icon(IconName::Dash),
        StageState::Waiting => div()
            .size(px(14.))
            .m(px(1.))
            .rounded_full()
            .border_2()
            .border_color(color.opacity(0.6))
            .into_any_element(),
    }
}

fn stage_cell(index: usize, stage: &Stage, status: RunStatus, last: bool, animate: bool, cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    let state = StageState::of(stage.phase, status);
    let name = stage_name(&stage.name, cx);
    let word = tr(cx, TEXT, state.word());
    let note = stage.note.as_ref().filter(|n| !n.is_empty());
    let tip = match note {
        Some(note) => format!("{name}: {word}\n{note}"),
        None => format!("{name}: {word}"),
    };
    let caption: SharedString = note.cloned().unwrap_or_else(|| word.into());
    let name_color = if state.is_current() {
        t.text
    } else if state == StageState::Done {
        t.text.opacity(0.78)
    } else {
        t.muted
    };
    h_flex()
        .id(("stage", index))
        .when(!last, |this| this.flex_1())
        .min_w_0()
        .items_center()
        .gap_2()
        .tooltip(tooltip(tip))
        .child(div().flex_none().child(stage_icon(state, animate, &t)))
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .font_weight(if state.is_current() { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                        .text_color(name_color)
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(if state == StageState::Failed { t.fail } else { t.muted })
                        .child(caption),
                ),
        )
        .when(!last, |this| {
            this.child(
                div()
                    .flex_1()
                    .min_w(px(10.))
                    .h(px(1.))
                    .mx_2()
                    .bg(if state == StageState::Done { t.ok.opacity(0.55) } else { t.border }),
            )
        })
}

fn stage_rail(run: &Run, animate: bool, cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    let rail = h_flex()
        .flex_none()
        .w_full()
        .items_center()
        .px_5()
        .py_2p5()
        .border_b_1()
        .border_color(t.border);
    if run.stages.is_empty() {
        return rail.text_xs().text_color(t.muted).child(tr(cx, TEXT, "Stages have not been reported yet"));
    }
    let last = run.stages.len() - 1;
    rail.children(
        run.stages
            .iter()
            .enumerate()
            .map(|(i, stage)| stage_cell(i, stage, run.status, i == last, animate, cx)),
    )
}

fn stat(id: &'static str, label: &'static str, value: SharedString, tip: Option<SharedString>, cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    v_flex()
        .id(id)
        .flex_none()
        .items_end()
        .child(div().text_size(px(11.)).text_color(t.muted).child(tr(cx, TEXT, label)))
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(t.text).child(value))
        .when_some(tip, |this, tip| this.tooltip(tooltip(tip)))
}

fn tinted_tag(color: Hsla) -> Tag {
    Tag::custom(color.opacity(0.13), color, color.opacity(0.38))
}

fn status_pill(run: &Run, cx: &App) -> impl IntoElement {
    let t = cx.tokens();
    let clean = Summary::of(run).verdict == Verdict::Clean;
    let (word, color, icon) = match run.status {
        RunStatus::Active => (run.status.label(), t.run, IconName::LoaderCircle),
        RunStatus::Waiting => (run.status.label(), t.review, IconName::Pause),
        RunStatus::Blocked => (run.status.label(), t.fail, IconName::CircleAlert),
        RunStatus::Done if clean => (run.status.label(), t.ok, IconName::CircleCheck),
        RunStatus::Done => ("Ended, needs attention", t.review, IconName::TriangleAlert),
    };
    tinted_tag(color).rounded_full().flex_none().child(
        h_flex()
            .gap_1()
            .items_center()
            .child(Icon::new(icon).size(px(12.)))
            .child(tr(cx, TEXT, word)),
    )
}

fn column(content: impl IntoElement) -> Div {
    div().size_full().min_w_0().overflow_hidden().child(content)
}

fn scene_setup_checks() -> Vec<Check> {
    vec![
        Check {
            name: "claude".into(),
            status: CheckStatus::Fail,
            detail: "not on PATH. Install Claude Code and sign in with `claude`.".into(),
        },
        Check {
            name: "codex".into(),
            status: CheckStatus::Fail,
            detail: "codex failed: not logged in".into(),
        },
        Check {
            name: "codegraph".into(),
            status: CheckStatus::Warn,
            detail: "Not installed (optional)".into(),
        },
    ]
}

fn current_run(state: &mut AppState) -> Option<&mut Run> {
    state.current.and_then(|i| state.runs.get_mut(i))
}

/// Puts the demo data into a state worth capturing. Scenes for this screen start with `run-`.
fn apply_scene(state: &mut AppState, scene: &str) {
    let finished = state.runs.iter().position(|r| r.status == RunStatus::Done);
    let with_status = |state: &AppState, status| state.runs.iter().position(|r| r.status == status);
    match scene {
        "run-done" => state.current = finished,
        "run-waiting" => state.current = with_status(state, RunStatus::Waiting),
        "run-blocked" => state.current = with_status(state, RunStatus::Blocked),
        "run-setup" => state.checks.extend(scene_setup_checks()),
        "run-setup-empty" => {
            state.runs.clear();
            state.current = None;
            state.checks.extend(scene_setup_checks());
        }
        "run-none" => state.current = None,
        "run-partial" | "run-partial-graph" => {
            state.current = finished;
            let Some(run) = current_run(state) else { return };
            for (unit, new) in run.units.iter_mut().zip([
                UnitState::Failed,
                UnitState::Built,
                UnitState::Pending,
                UnitState::Running,
                UnitState::Verified,
            ]) {
                unit.state = new;
            }
            for stage in run.stages.iter_mut().filter(|s| s.name == FULL_TEST) {
                stage.phase = Phase::Failed;
                stage.note = Some("3 failed, 409 passed".into());
            }
            run.usage.total_usd = None;
            run.report = Some(TextSource::Path("Z:\\no-such-folder\\report.md".into()));
            run.warnings = vec![
                "Could not read reports/review_causal.md".into(),
                "A background worker was stopped before it wrote its result, so its unit stays unverified.".into(),
            ];
        }
        "run-long" | "run-long-open" => {
            let Some(run) = current_run(state) else { return };
            let paragraph = "把整個資料管線重新整理：先盤點所有來源檔案與它們的欄位定義，標出缺值與單位不一致的地方，\
                再把清理步驟寫成可以重跑的腳本，最後為每一步補上測試與說明文件，確保任何人都能從頭重建同樣的結果。";
            run.wish = (0..18).map(|_| paragraph).collect::<Vec<_>>().join("\n\n").into();
            run.project = "a-project-with-a-remarkably-long-folder-name-that-keeps-going-and-going-past-any-reasonable-width".into();
            run.cwd = Some("E:\\codex\\a-very-long-path\\that-goes-on\\and-on\\for-many-levels\\of-nesting\\project".into());
            run.hosted = false;
            for stage in &mut run.stages {
                stage.note = Some("a rather long note that cannot possibly fit".into());
            }
        }
        "run-sparse" => {
            let Some(run) = current_run(state) else { return };
            run.wish = "".into();
            run.project = "".into();
            run.cwd = None;
            run.started = "".into();
            run.elapsed = None;
            run.usage = Usage::default();
            run.stages.clear();
            run.goals.clear();
            run.modules.clear();
            run.units.clear();
            run.report = None;
            run.status = RunStatus::Done;
        }
        _ => {}
    }
}

pub struct RunView {
    state: Entity<AppState>,
    todo: Entity<Todo>,
    mesh: Entity<Mesh>,
    detail: Entity<Detail>,
    control: Entity<Control>,
    columns: Entity<ResizableState>,
    /// The run the transient state below belongs to.
    viewing: Option<SharedString>,
    wish_open: bool,
    report_open: bool,
    hidden_summaries: HashSet<SharedString>,
}

impl RunView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let scene = state.read(cx).scene.clone().unwrap_or_default();
        state.update(cx, |s, _| apply_scene(s, &scene));
        let viewing = state.read(cx).run().map(|r| r.session.clone());
        Self {
            todo: cx.new(|cx| Todo::new(state.clone(), window, cx)),
            mesh: cx.new(|cx| Mesh::new(state.clone(), window, cx)),
            detail: cx.new(|cx| Detail::new(state.clone(), window, cx)),
            control: cx.new(|cx| Control::new(state.clone(), window, cx)),
            columns: cx.new(|_| ResizableState::default()),
            hidden_summaries: viewing.iter().filter(|_| scene.ends_with("-graph")).cloned().collect(),
            viewing,
            wish_open: scene.ends_with("-open"),
            report_open: false,
            state,
        }
    }

    /// Opens a file or folder through the OS, on a background thread, and says so if it is gone.
    fn open_path(&self, path: PathBuf, missing: &'static str, window: &Window, cx: &Context<Self>) {
        let message = tr(cx, TEXT, missing);
        cx.spawn_in(window, async move |_, cx| {
            let probe = path.clone();
            let exists = cx.background_spawn(async move { probe.exists() }).await;
            let _ = cx.update(|window, cx| {
                if exists {
                    cx.open_with_system(&path);
                } else {
                    window.push_notification(Notification::warning(message), cx);
                }
            });
        })
        .detach();
    }

    fn wish(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let row = h_flex().w_full().min_w_0().items_start().gap_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD);
        if run.wish.trim().is_empty() {
            return row.text_color(t.muted).child(tr(cx, TEXT, "No wish text was recorded"));
        }
        let open = self.wish_open;
        let text: AnyElement = if open {
            let (shown, cut) = beginning(&run.wish, LONG_TEXT_CHARS);
            v_flex()
                .id("wish-full")
                .flex_1()
                .min_w_0()
                .max_h(px(132.))
                .gap_1()
                .overflow_y_scrollbar()
                .child(div().w_full().whitespace_normal().child(SharedString::from(shown.to_owned())))
                .when(cut, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(t.muted)
                            .child(tr(cx, TEXT, "The wish is longer than this. Only its beginning is shown.")),
                    )
                })
                .into_any_element()
        } else {
            let (preview, cut) = beginning(&run.wish, TOOLTIP_CHARS);
            let tip = if cut { format!("{preview}…") } else { preview.to_owned() };
            div()
                .id("wish-line")
                .min_w_0()
                .truncate()
                .cursor_pointer()
                .child(one_line(&run.wish, ONE_LINE_CHARS))
                .tooltip(tooltip(tip))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.wish_open = true;
                    cx.notify();
                }))
                .into_any_element()
        };
        row.child(text).child(
            Button::new("wish-toggle")
                .ghost()
                .xsmall()
                .flex_none()
                .icon(if open { IconName::ChevronUp } else { IconName::ChevronDown })
                .tooltip(tr(cx, TEXT, if open { "Show one line" } else { "Show the whole wish" }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.wish_open = !this.wish_open;
                    cx.notify();
                })),
        )
    }

    fn folder(&self, run: &Run, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let name: SharedString = if run.project.is_empty() { UNKNOWN.into() } else { run.project.clone() };
        match run.cwd.clone() {
            Some(path) => Button::new("open-folder")
                .ghost()
                .xsmall()
                .icon(IconName::FolderOpen)
                .label(name)
                .tooltip(path.display().to_string())
                .flex_shrink(1.)
                .min_w_0()
                .max_w(px(320.))
                .text_color(t.muted)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_path(path.clone(), "That folder no longer exists", window, cx)
                }))
                .into_any_element(),
            None => h_flex()
                .id("folder-unknown")
                .min_w_0()
                .max_w(px(320.))
                .gap_1()
                .px_1()
                .child(Icon::new(IconName::Folder).size(px(12.)))
                .child(div().min_w_0().truncate().child(name))
                .tooltip(tooltip(tr(cx, TEXT, "The folder of this wish is not known")))
                .into_any_element(),
        }
    }

    fn header(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let elapsed = run.elapsed.clone().unwrap_or_else(|| UNKNOWN.into());
        let warning_count = run.warnings.len();
        h_flex()
            .flex_none()
            .w_full()
            .items_start()
            .gap_5()
            .px_5()
            .py_3()
            .bg(t.surface)
            .border_b_1()
            .border_color(t.border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(self.wish(run, cx))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(t.muted)
                            .child(self.folder(run, cx))
                            .when(!run.started.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .child(format!("{} {}", tr(cx, TEXT, "Started"), run.started)),
                                )
                            })
                            .when(!run.hosted, |this| {
                                this.child(
                                    Tag::custom(t.raised, t.muted, t.border).flex_none().h(px(20.)).py_0().child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(Icon::new(IconName::Eye).size(px(11.)))
                                            .child(tr(cx, TEXT, "Started in terminal: view only")),
                                    ),
                                )
                            })
                            .when(warning_count > 0, |this| {
                                let label = if warning_count == 1 {
                                    tr(cx, TEXT, "1 warning").into()
                                } else {
                                    counted("{n} warnings", warning_count, cx)
                                };
                                let tip = run.warnings.iter().map(|w| w.as_ref()).collect::<Vec<_>>().join("\n");
                                this.child(
                                    h_flex()
                                        .id("run-warnings")
                                        .flex_none()
                                        .gap_1()
                                        .items_center()
                                        .text_color(t.review)
                                        .child(Icon::new(IconName::TriangleAlert).size(px(12.)))
                                        .child(label)
                                        .tooltip(tooltip(tip)),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap_5()
                    .child(stat("stat-elapsed", "Elapsed", elapsed, None, cx))
                    .child(stat(
                        "stat-cost",
                        "Cost",
                        cost_text(run.usage.total_usd),
                        Some(cost_tooltip(&run.usage, cx)),
                        cx,
                    ))
                    .child(status_pill(run, cx)),
            )
    }

    fn setup_card(&self, failures: &[&Check], cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let state = self.state.clone();
        let settings = self.state.clone();
        v_flex()
            .w_full()
            .gap_2()
            .px_4()
            .py_3()
            .rounded(px(10.))
            .bg(t.fail.opacity(0.07))
            .border_1()
            .border_color(t.fail.opacity(0.4))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(Icon::new(IconName::TriangleAlert).size(px(16.)).text_color(t.fail))
                            .child(div().min_w_0().truncate().child(tr(cx, TEXT, "Chronos cannot start a wish yet"))),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .child(
                                Button::new("setup-settings")
                                    .ghost()
                                    .small()
                                    .label(tr(cx, TEXT, "Open Settings"))
                                    .on_click(move |_, _, cx| settings.update(cx, |s, cx| s.set_view(Screen::Doctor, cx))),
                            )
                            .child(
                                Button::new("setup-recheck")
                                    .small()
                                    .outline()
                                    .icon(IconName::RefreshCw)
                                    .label(tr(cx, TEXT, "Check again"))
                                    .on_click(move |_, _, cx| {
                                        state.update(cx, |s, cx| {
                                            if let Some(backend) = &s.backend {
                                                backend.refresh();
                                            }
                                            cx.notify();
                                        })
                                    }),
                            ),
                    ),
            )
            .children(failures.iter().enumerate().map(|(i, check)| {
                let fix = fix_hint(&check.name);
                let detail: SharedString = without_fix(&check.detail, fix).to_owned().into();
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_3()
                    .text_xs()
                    .child(
                        h_flex()
                            .w(px(104.))
                            .flex_none()
                            .gap_1p5()
                            .items_center()
                            .text_color(t.text)
                            .font_weight(FontWeight::MEDIUM)
                            .child(Icon::new(IconName::CircleX).size(px(12.)).text_color(t.fail))
                            .child(div().min_w_0().truncate().child(check.name.clone())),
                    )
                    .child(
                        v_flex()
                            .id(("setup-check", i))
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .tooltip(tooltip(check.detail.clone()))
                            .child(div().w_full().text_color(t.text).line_clamp(2).child(detail))
                            .when_some(fix, |this, fix| {
                                this.child(div().w_full().text_color(t.muted).child(tr(cx, TEXT, fix)))
                            }),
                    )
            }))
    }

    fn no_run(&self, no_runs: bool, failures: &[&Check], cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let blocked = !failures.is_empty();
        let compose = self.state.clone();
        let wishes = self.state.clone();
        let (title, description) = if no_runs {
            (
                "Start with a wish",
                "Say what you want done. Chronos plans it, builds it and checks each piece while you watch.",
            )
        } else {
            ("No wish is open", "Open one from Wishes, or make a new one.")
        };
        v_flex()
            .id("run-empty")
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .p_8()
            .overflow_y_scrollbar()
            .when(blocked, |this| {
                this.child(div().w_full().max_w(px(640.)).child(self.setup_card(failures, cx)))
            })
            .child(
                Empty::new()
                    .flex_none()
                    .gap_5()
                    .header(
                        EmptyHeader::new()
                            .max_w(px(560.))
                            .media(
                                EmptyMedia::new().child(
                                    div()
                                        .size(px(72.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(t.gold.opacity(0.12))
                                        .border_1()
                                        .border_color(t.gold.opacity(0.35))
                                        .child(Icon::new(IconName::Star).size(px(30.)).text_color(t.gold)),
                                ),
                            )
                            .title(EmptyTitle::new().text_xl().text_color(t.text).child(tr(cx, TEXT, title)))
                            .description(EmptyDescription::new().text_sm().child(tr(cx, TEXT, description))),
                    )
                    .content(
                        EmptyContent::new()
                            .child(
                                Button::new("make-a-wish")
                                    .custom(
                                        ButtonCustomVariant::new(cx)
                                            .color(t.gold)
                                            .foreground(t.canvas)
                                            .hover(t.gold.opacity(0.88))
                                            .active(t.gold.opacity(0.76)),
                                    )
                                    .large()
                                    .h(px(44.))
                                    .px_6()
                                    .disabled(blocked)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(Icon::new(IconName::Star).size(px(16.)))
                                            .child(tr(cx, TEXT, "Make a wish")),
                                    )
                                    .on_click(move |_, _, cx| compose.update(cx, |s, cx| s.set_composing(true, cx))),
                            )
                            .when(blocked, |this| {
                                this.child(
                                    div().text_xs().text_color(t.muted).child(tr(cx, TEXT, "Fix the items above to make a wish")),
                                )
                            })
                            .when(!no_runs, |this| {
                                this.child(
                                    Button::new("open-wishes")
                                        .ghost()
                                        .small()
                                        .label(tr(cx, TEXT, "Open Wishes"))
                                        .on_click(move |_, _, cx| wishes.update(cx, |s, cx| s.set_view(Screen::Wishes, cx))),
                                )
                            }),
                    ),
            )
    }

    fn summary_tile(label: &'static str, cx: &App) -> Div {
        let t = cx.tokens();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .px_3()
            .py_2p5()
            .rounded(px(8.))
            .bg(t.raised)
            .child(div().truncate().text_size(px(11.)).text_color(t.muted).child(tr(cx, TEXT, label)))
    }

    fn attention_list(&self, run: &Run, summary: &Summary, cx: &Context<Self>) -> Vec<AnyElement> {
        let t = cx.tokens();
        let mut rows: Vec<AnyElement> = summary
            .unverified
            .iter()
            .map(|&index| {
                let unit = &run.units[index];
                let color = t.unit(unit.state);
                let state = self.state.clone();
                Button::new(("attention-unit", index))
                    .ghost()
                    .small()
                    .w_full()
                    .tooltip(unit.title.clone())
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .text_xs()
                            .child(Icon::new(unit_icon(unit.state)).size(px(13.)).text_color(color))
                            .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(unit.title.clone()))
                            .child(div().flex_none().text_color(color).child(tr(cx, TEXT, unit.state.label()))),
                    )
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.select(Selection::Unit(index), cx)))
                    .into_any_element()
            })
            .collect();
        let line = |id: (&'static str, usize), icon: IconName, color: Hsla, text: SharedString| {
            h_flex()
                .id(id)
                .w_full()
                .min_w_0()
                .gap_2()
                .px_2()
                .py_1()
                .items_start()
                .text_xs()
                .tooltip(tooltip(text.clone()))
                .child(div().flex_none().mt_0p5().child(Icon::new(icon).size(px(13.)).text_color(color)))
                .child(div().flex_1().min_w_0().line_clamp(2).text_color(t.text).child(text))
                .into_any_element()
        };
        rows.extend(
            run.stages
                .iter()
                .enumerate()
                .filter(|(_, s)| s.phase == Phase::Failed && !s.name.eq_ignore_ascii_case(FULL_TEST))
                .map(|(i, s)| {
                    let name = stage_name(&s.name, cx);
                    let mut text = tr(cx, TEXT, "{stage} failed").replace("{stage}", &name);
                    if let Some(note) = s.note.as_ref().filter(|n| !n.is_empty()) {
                        text.push_str(&format!(" · {note}"));
                    }
                    line(("attention-stage", i), IconName::CircleX, t.fail, text.into())
                }),
        );
        rows.extend(
            run.warnings
                .iter()
                .enumerate()
                .map(|(i, w)| line(("attention-warning", i), IconName::TriangleAlert, t.review, w.clone())),
        );
        rows
    }

    fn summary(&self, run: &Run, summary: &Summary, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let (headline, count) = summary.headline();
        let headline: SharedString = match count {
            Some(count) => counted(headline, count, cx),
            None => tr(cx, TEXT, headline).into(),
        };
        let (accent, icon) = match summary.verdict {
            Verdict::Clean => (t.ok, IconName::CircleCheck),
            Verdict::TestFailed => (t.fail, IconName::CircleX),
            Verdict::NotVerified if run.count(UnitState::Failed) > 0 => (t.fail, IconName::TriangleAlert),
            Verdict::NotVerified | Verdict::TestNotRun | Verdict::NoUnits => (t.review, IconName::TriangleAlert),
        };
        let units_value: SharedString = if summary.total == 0 {
            UNKNOWN.into()
        } else {
            format!("{} / {}", summary.verified, summary.total).into()
        };
        let (test_word, test_color, test_icon): (&'static str, Hsla, Option<IconName>) = match summary.full_test {
            FullTest::Passed => ("Passed", t.ok, Some(IconName::CircleCheck)),
            FullTest::Failed => ("Failed", t.fail, Some(IconName::CircleX)),
            FullTest::NotRun => ("Did not run", t.review, Some(IconName::Dash)),
            FullTest::Unknown => (UNKNOWN, t.muted, None),
        };
        let attention = self.attention_list(run, summary, cx);
        let attention_count = attention.len();
        let report = self.report_link(run, cx);
        let inline_report = match (&run.report, self.report_open) {
            (Some(TextSource::Inline(text)), true) => Some(beginning(text, LONG_TEXT_CHARS)),
            _ => None,
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(t.canvas.opacity(0.72))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(560.))
                    .max_h_full()
                    .rounded(px(12.))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .flex_none()
                            .w_full()
                            .items_center()
                            .gap_2p5()
                            .px_4()
                            .pt_4()
                            .pb_3()
                            .child(Icon::new(icon).size(px(20.)).text_color(accent))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(headline),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("summary-body")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .gap_3()
                            .px_4()
                            .pb_3()
                            .overflow_y_scrollbar()
                            .child(
                                h_flex()
                                    .flex_none()
                                    .w_full()
                                    .items_stretch()
                                    .gap_2()
                                    .child(
                                        Self::summary_tile("Units verified", cx)
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(t.text)
                                                    .child(units_value),
                                            )
                                            .child(Progress::new("summary-units").xsmall().color(t.ok).value(summary.fraction())),
                                    )
                                    .child(
                                        Self::summary_tile(FULL_TEST, cx)
                                            .child(
                                                h_flex()
                                                    .gap_1p5()
                                                    .items_center()
                                                    .text_lg()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(test_color)
                                                    .when_some(test_icon, |this, icon| {
                                                        this.child(Icon::new(icon).size(px(16.)))
                                                    })
                                                    .child(tr(cx, TEXT, test_word)),
                                            )
                                            .when_some(summary.full_test_note.clone(), |this, note| {
                                                this.child(div().truncate().text_size(px(11.)).text_color(t.muted).child(note))
                                            }),
                                    )
                                    .child(
                                        Self::summary_tile("Cost", cx)
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(t.text)
                                                    .child(cost_text(run.usage.total_usd)),
                                            )
                                            .child(div().truncate().text_size(px(11.)).text_color(t.muted).child(tr(
                                                cx,
                                                TEXT,
                                                if run.usage.total_usd.is_some() { "API-price equivalent" } else { "Not reported" },
                                            ))),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_none()
                                    .w_full()
                                    .gap_1()
                                    .when(attention_count > 0, |this| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(t.muted)
                                                .child(tr(cx, TEXT, "What needs attention")),
                                        )
                                    })
                                    .when(attention_count == 0, |this| {
                                        this.child(
                                            h_flex()
                                                .gap_2()
                                                .px_2()
                                                .py_1()
                                                .items_center()
                                                .text_xs()
                                                .text_color(t.text)
                                                .child(Icon::new(IconName::Check).size(px(13.)).text_color(t.ok))
                                                .child(tr(cx, TEXT, "Nothing needs attention")),
                                        )
                                    })
                                    .children(attention),
                            )
                            .when_some(inline_report, |this, (text, cut)| {
                                this.child(
                                    v_flex()
                                        .flex_none()
                                        .w_full()
                                        .gap_1()
                                        .p_3()
                                        .rounded(px(8.))
                                        .bg(t.raised)
                                        .text_xs()
                                        .text_color(t.text)
                                        .child(div().w_full().whitespace_normal().child(SharedString::from(text.to_owned())))
                                        .when(cut, |this| {
                                            this.child(div().text_color(t.muted).child(tr(
                                                cx,
                                                TEXT,
                                                "The report is longer than this. Only its beginning is shown.",
                                            )))
                                        }),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(t.border)
                            .child(report)
                            .child(
                                Button::new("see-graph")
                                    .small()
                                    .outline()
                                    .label(tr(cx, TEXT, "See the graph"))
                                    .on_click({
                                        let session = run.session.clone();
                                        cx.listener(move |this, _, _, cx| {
                                            this.hidden_summaries.insert(session.clone());
                                            cx.notify();
                                        })
                                    }),
                            ),
                    ),
            )
    }

    fn report_link(&self, run: &Run, cx: &Context<Self>) -> AnyElement {
        let t = cx.tokens();
        match run.report.clone() {
            None => div()
                .text_xs()
                .text_color(t.muted)
                .child(tr(cx, TEXT, "No report was written"))
                .into_any_element(),
            Some(TextSource::Path(path)) => Button::new("open-report")
                .link()
                .small()
                .icon(IconName::FileText)
                .label(tr(cx, TEXT, "Open the report"))
                .tooltip(path.display().to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_path(path.clone(), "That report file no longer exists", window, cx)
                }))
                .into_any_element(),
            Some(TextSource::Inline(_)) => Button::new("open-report")
                .link()
                .small()
                .icon(IconName::FileText)
                .label(tr(cx, TEXT, if self.report_open { "Hide the report" } else { "Open the report" }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.report_open = !this.report_open;
                    cx.notify();
                }))
                .into_any_element(),
        }
    }

    fn graph_column(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let summary = (run.status == RunStatus::Done).then(|| Summary::of(run));
        let hidden = self.hidden_summaries.contains(&run.session);
        let session = run.session.clone();
        column(self.mesh.clone())
            .relative()
            .bg(t.canvas)
            .map(|this| match &summary {
                Some(summary) if !hidden => this.child(self.summary(run, summary, cx)).into_any_element(),
                Some(_) => this
                    .child(
                        h_flex().absolute().top_3().left_0().right_0().justify_center().child(
                            Button::new("show-summary")
                                .small()
                                .outline()
                                .bg(t.surface)
                                .icon(IconName::FileText)
                                .label(tr(cx, TEXT, "Show the summary"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.hidden_summaries.remove(&session);
                                    cx.notify();
                                })),
                        ),
                    )
                    .into_any_element(),
                None => this.into_any_element(),
            })
    }
}

impl Render for RunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let entity = self.state.clone();
        let state = entity.read(cx);
        let failures = required_failures(&state.checks);
        let Some(run) = state.run() else {
            return self.no_run(state.runs.is_empty(), &failures, cx).into_any_element();
        };
        if self.viewing.as_ref() != Some(&run.session) {
            self.viewing = Some(run.session.clone());
            self.wish_open = false;
            self.report_open = false;
        }
        let animate = run.status == RunStatus::Active && !state.reduce_motion;
        v_flex()
            .size_full()
            .when(!failures.is_empty(), |this| {
                this.child(div().flex_none().w_full().px_5().py_3().child(self.setup_card(&failures, cx)))
            })
            .child(self.header(run, cx))
            .child(stage_rail(run, animate, cx))
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("run-columns")
                        .with_state(&self.columns)
                        .child(
                            resizable_panel()
                                .size(CHECKLIST_WIDTH)
                                .size_range(CHECKLIST_RANGE)
                                .child(column(self.todo.clone()).bg(t.surface)),
                        )
                        .child(
                            resizable_panel()
                                .size_range(GRAPH_MIN..Pixels::MAX)
                                .child(self.graph_column(run, cx)),
                        )
                        .child(
                            resizable_panel()
                                .size(DETAILS_WIDTH)
                                .size_range(DETAILS_RANGE)
                                .child(column(self.detail.clone()).bg(t.surface)),
                        ),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .max_h(relative(0.5))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.surface)
                    .child(self.control.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::demo::sample_run;
    use super::super::i18n::COMMON;
    use super::super::model::*;
    use super::{
        beginning, cost_text, fix_hint, one_line, required_failures, stage_key, unpriced_models, without_fix, FullTest,
        StageState, Summary, Verdict, FULL_TEST, STAGE_NAMES, TEXT,
    };
    use gpui_kit::SharedString;

    fn run_with(states: &[UnitState]) -> Run {
        let mut run = sample_run();
        run.units.truncate(states.len());
        for (unit, state) in run.units.iter_mut().zip(states) {
            unit.state = *state;
        }
        run.status = RunStatus::Done;
        for stage in &mut run.stages {
            stage.phase = Phase::Done;
        }
        run
    }

    fn set_full_test(run: &mut Run, phase: Phase) {
        run.stages.iter_mut().find(|s| s.name == FULL_TEST).unwrap().phase = phase;
    }

    #[test]
    fn one_line_collapses_whitespace_and_stays_bounded() {
        assert_eq!(one_line("  first\n\n second\u{3000}third\t", 400).as_ref(), "first second third");
        let long = "長".repeat(10_000);
        assert_eq!(one_line(&long, 400).chars().count(), 400);
    }

    #[test]
    fn beginning_cuts_only_between_characters() {
        assert_eq!(beginning("願望清單", 2), ("願望", true));
        assert_eq!(beginning("願望", 2), ("願望", false));
        assert_eq!(beginning("", 2), ("", false));
    }

    #[test]
    fn cost_is_a_dash_when_unknown_and_never_a_guess() {
        assert_eq!(cost_text(None).as_ref(), "—");
        assert_eq!(cost_text(Some(f64::NAN)).as_ref(), "—");
        assert_eq!(cost_text(Some(-1.)).as_ref(), "—");
        assert_eq!(cost_text(Some(0.)).as_ref(), "$0.00");
        assert_eq!(cost_text(Some(116.123)).as_ref(), "$116.12");
    }

    #[test]
    fn models_without_a_price_are_named() {
        let usage = sample_run().usage;
        assert_eq!(unpriced_models(&usage), vec![SharedString::from("gpt-6.1-terra")]);
        assert!(unpriced_models(&Usage::default()).is_empty());
    }

    #[test]
    fn a_running_stage_reads_as_what_its_run_is_doing() {
        assert_eq!(StageState::of(Phase::Running, RunStatus::Active), StageState::Running);
        assert_eq!(StageState::of(Phase::Running, RunStatus::Waiting), StageState::OnBatch);
        assert_eq!(StageState::of(Phase::Running, RunStatus::Blocked), StageState::NeedsYou);
        assert_eq!(StageState::of(Phase::Running, RunStatus::Done), StageState::Unfinished);
    }

    #[test]
    fn a_finished_run_never_says_a_later_stage_is_still_waiting() {
        assert_eq!(StageState::of(Phase::Waiting, RunStatus::Done), StageState::Unfinished);
        assert_eq!(StageState::of(Phase::Waiting, RunStatus::Active), StageState::Waiting);
        assert_eq!(StageState::of(Phase::Done, RunStatus::Blocked), StageState::Done);
        assert_eq!(StageState::of(Phase::Failed, RunStatus::Done), StageState::Failed);
    }

    #[test]
    fn summary_counts_only_verified_units_as_verified() {
        let run = run_with(&[UnitState::Verified, UnitState::Built, UnitState::Verified, UnitState::Failed]);
        let summary = Summary::of(&run);
        assert_eq!((summary.verified, summary.total), (2, 4));
        assert_eq!(summary.verdict, Verdict::NotVerified);
    }

    #[test]
    fn every_unverified_unit_is_listed_with_the_worst_first() {
        let run = run_with(&[
            UnitState::Pending,
            UnitState::Running,
            UnitState::Verified,
            UnitState::Built,
            UnitState::Failed,
        ]);
        assert_eq!(Summary::of(&run).unverified, vec![4, 3, 1, 0]);
    }

    #[test]
    fn verdict_requires_the_full_test_to_have_passed() {
        let mut run = run_with(&[UnitState::Verified]);
        assert_eq!(Summary::of(&run).verdict, Verdict::Clean);
        assert_eq!(Summary::of(&run).full_test, FullTest::Passed);

        set_full_test(&mut run, Phase::Failed);
        assert_eq!(Summary::of(&run).verdict, Verdict::TestFailed);
        assert_eq!(Summary::of(&run).full_test, FullTest::Failed);

        set_full_test(&mut run, Phase::Waiting);
        assert_eq!(Summary::of(&run).verdict, Verdict::TestNotRun);
        assert_eq!(Summary::of(&run).full_test, FullTest::NotRun);
    }

    #[test]
    fn missing_stage_data_reads_as_unknown_not_passed() {
        let mut run = run_with(&[UnitState::Verified]);
        run.stages.clear();
        assert_eq!(Summary::of(&run).full_test, FullTest::Unknown);
    }

    #[test]
    fn a_run_without_units_is_not_called_verified() {
        let summary = Summary::of(&run_with(&[]));
        assert_eq!(summary.verdict, Verdict::NoUnits);
        assert_eq!(summary.fraction(), 0.);
    }

    #[test]
    fn headline_counts_the_units_it_names() {
        let one = Summary::of(&run_with(&[UnitState::Failed, UnitState::Verified]));
        assert_eq!(one.headline(), ("1 unit is not verified", None));
        let many = Summary::of(&run_with(&[UnitState::Failed, UnitState::Built]));
        assert_eq!(many.headline(), ("{n} units are not verified", Some(2)));
    }

    #[test]
    fn only_failed_checks_block_a_wish() {
        let check = |name: &'static str, status| Check { name: name.into(), status, detail: "".into() };
        let checks = [
            check("claude", CheckStatus::Ok),
            check("codegraph", CheckStatus::Warn),
            check("codex", CheckStatus::Fail),
        ];
        let failures = required_failures(&checks);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].name.as_ref(), "codex");
    }

    #[test]
    fn fix_hints_cover_every_required_check_the_doctor_reports() {
        for name in ["claude", "codex", "sqlite fts5", "state", "pwsh"] {
            let hint = fix_hint(name).unwrap_or_else(|| panic!("no hint for {name}"));
            assert!(TEXT.iter().any(|(key, _)| *key == hint), "untranslated hint for {name}");
        }
        assert_eq!(fix_hint("codegraph"), None);
    }

    #[test]
    fn the_doctors_fix_is_not_said_twice() {
        let fix = fix_hint("claude");
        assert_eq!(without_fix("not on PATH. Install Claude Code and sign in with `claude`.", fix), "not on PATH.");
        assert_eq!(without_fix("claude failed: exit 1", fix), "claude failed: exit 1");
        assert_eq!(without_fix("anything", None), "anything");
    }

    #[test]
    fn stage_names_translate_only_when_known() {
        assert_eq!(stage_key("Full test"), Some("Full test"));
        assert_eq!(stage_key("Something new"), None);
        for key in STAGE_NAMES {
            assert!(COMMON.iter().chain(TEXT.iter()).any(|(k, _)| *k == key), "no translation for {key}");
        }
    }

    #[test]
    fn state_words_have_translations() {
        let unit_words = [
            UnitState::Pending,
            UnitState::Running,
            UnitState::Built,
            UnitState::Verified,
            UnitState::Failed,
        ]
        .map(UnitState::label);
        let stage_words = [
            StageState::Done,
            StageState::Running,
            StageState::Waiting,
            StageState::Failed,
            StageState::NeedsYou,
            StageState::OnBatch,
            StageState::Unfinished,
        ]
        .map(StageState::word);
        for key in unit_words.iter().chain(stage_words.iter()) {
            assert!(COMMON.iter().chain(TEXT.iter()).any(|(k, _)| k == key), "no translation for {key}");
        }
    }

    #[test]
    fn the_translation_table_has_no_duplicate_keys() {
        let mut keys: Vec<_> = TEXT.iter().map(|(key, _)| *key).collect();
        keys.sort_unstable();
        let total = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), total);
    }
}
