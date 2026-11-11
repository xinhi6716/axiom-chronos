//! The Run screen: the wish, the stage rail, three resizable columns and the control bar,
//! plus what surrounds them: setup problems, no runs yet, and the summary when a run ends.
pub(super) mod board;
use super::i18n::{fill, tr, translate, Lang, Table};
use super::model::*;
use super::theme::{self, Density, Step, TypeSize, Tokened as _, Tokens};
use super::motion::{self, Pace};
use super::{
    control::{self, Control},
    detail::Detail,
    mesh::Mesh,
    ornament,
    plain_text,
    settings,
    todo::Todo,
    usage,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::resizable::{h_resizable, resizable_panel, ResizablePanelEvent, ResizableState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::component::ElementExt as _;
use std::collections::HashSet;
use std::path::PathBuf;

const BOARD_CARD_SHARE: f32 = 0.30;
const DETAIL_OVERVIEW_MIN: f32 = 315.0;

fn board_card_max_height(details_height: f32) -> f32 {
    if details_height.is_finite() && details_height >= 0.0 { details_height * BOARD_CARD_SHARE } else { 0.0 }
}

fn board_card_budget(details_height: f32) -> f32 {
    if details_height.is_finite() && details_height >= 0.0 {
        (details_height - DETAIL_OVERVIEW_MIN).max(0.0).min(board_card_max_height(details_height))
    } else { 0.0 }
}

#[derive(Debug, PartialEq)]
enum BoardMode { Card, Strip }

#[derive(Debug, PartialEq)]
struct BoardLayout {
    mode: BoardMode,
    visible: usize,
    max_height: f32,
    min_height: f32,
}

fn board_layout(details_height: f32, run_count: usize, needs_count: usize, padding: f32) -> Option<BoardLayout> {
    if run_count == 0 { return None; }
    let max_height = board_card_budget(details_height);
    let visible = board::compact_visible_count(run_count, max_height, padding);
    let min_height = 2.0 * padding + board::COMPACT_CARD_ESTIMATE
        + if visible < run_count { padding + board::MORE_ROW_HEIGHT } else { 0.0 };
    if max_height >= min_height {
        return Some(BoardLayout { mode: BoardMode::Card, visible, max_height, min_height });
    }
    if details_height < DETAIL_OVERVIEW_MIN + board::STRIP_ROW_HEIGHT || !details_height.is_finite() { return None; }
    if needs_count == 0 { return None; }
    Some(BoardLayout { mode: BoardMode::Strip, visible: 1, max_height: board::STRIP_ROW_HEIGHT, min_height: board::STRIP_ROW_HEIGHT })
}

const TEXT: Table = &[
    ("Exit focus", "離開專注模式"),
    ("Back to the whole window (Ctrl+Shift+F)", "返回完整視窗（Ctrl+Shift+F）"),
    ("Run", "執行"),
    ("Reading your wishes…", "正在讀取你的願望…"),
    ("Try again", "重試"),
    ("Build implements and checks the planned units.", "施工並檢查規劃好的單元。"),
    ("Full test checks the completed work together.", "完整測試會一起檢查已完成的工作。"),
    ("Report records the outcome of the wish.", "報告會記錄這個願望的結果。"),
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
    ("Say what you want done. Chronos plans it, builds it and checks every piece.", "說出你想完成的事，Chronos 會規劃、施工並逐項驗收。"),
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
    ("Say something about this", "對這項說句話"),
    ("Opens the message box with this item named at the start.", "開啟訊息框，開頭會註明是針對這一項。"),
    ("Your first day with Chronos", "初次與 Chronos 同行"),
    ("{} of {} done", "完成 {} / {}"),
    ("Tools found", "找到所需工具"),
    ("Signed in", "已登入"),
    ("A project is chosen", "選好專案資料夾"),
    ("First wish finished", "第一個願望已完成"),
    ("Claude Code and Codex are both installed.", "Claude Code 與 Codex 都已安裝。"),
    ("Neither tool reports a sign-in problem.", "兩個工具都沒有回報登入問題。"),
    ("A project folder has been used with Chronos.", "已經用 Chronos 選過專案資料夾。"),
    ("A wish has run to its end.", "已有一個願望執行到結束。"),
    ("Not yet.", "還沒有。"),
    ("Chronos is starting. Nothing needs you for now.", "Chronos 正在啟動，暫時不需要你動手。"),
    ("Chronos is working. Nothing needs you for now.", "Chronos 正在進行，暫時不需要你動手。"),
    ("Astra is planning. Nothing needs you for now; building starts once the plan is reviewed.", "Astra 正在規劃，暫時不需要你動手；計畫審過後才會開始施工。"),
    ("The plan is being reviewed. Nothing needs you for now; the work is split into modules next.", "計畫正在審查，暫時不需要你動手；接著會切成模組。"),
    ("The plan is being split into modules. Nothing needs you for now; building starts next.", "正在把計畫切成模組，暫時不需要你動手；接著開始施工。"),
    ("Getting the next units ready. Nothing needs you for now.", "正在準備下一批單元，暫時不需要你動手。"),
    ("Building 1 unit. Nothing needs you for now; it is checked once it is built.", "正在建造 1 個單元，暫時不需要你動手；建好後會驗收。"),
    ("Building {} units. Nothing needs you for now; each is checked once it is built.", "正在建造 {} 個單元，暫時不需要你動手；建好後會逐項驗收。"),
    ("Running the full test. Nothing needs you for now; it decides whether the wish counts as finished.", "正在跑完整測試，暫時不需要你動手；結果會決定這個願望算不算完成。"),
    ("Writing down what was learned. Nothing needs you for now; the report comes next.", "正在記下這次學到的事，暫時不需要你動手；接著會寫報告。"),
    ("Writing the report. Nothing needs you for now; it will be here when it is done.", "正在寫報告，暫時不需要你動手；寫好後會出現在這裡。"),
    ("A background batch is running. Nothing needs you for now; you will be told when it finishes, and waiting uses no model quota.", "背景批次正在執行，暫時不需要你動手；結束時會通知你，等待期間不消耗模型額度。"),
    ("The usage limit was reached. Nothing needs you; it continues by itself when the limit resets.", "用量額度用完了，不需要你動手；額度恢復後會自動繼續。"),
    ("The session stopped before it finished. Resume it to continue where it left off.", "工作階段在完成前就停止了，請按「接續」從中斷處繼續。"),
    ("Stopped and waiting for you: {}. Deal with it, then resume.", "已暫停，等你處理：{}。處理後請按「接續」。"),
    ("Stopped and waiting for you. Read the note below, then resume.", "已暫停，等你處理。請看下方說明，再按「接續」。"),
    ("Finished: all {} units are verified and the full test passed. Nothing is left for you to do.", "已完成：{} 個單元都已驗收，完整測試通過，沒有需要你做的事。"),
    ("Ended with 1 unit not verified. Read the summary, then decide what to do next.", "已結束，有 1 個單元尚未驗收。請看摘要，再決定下一步。"),
    ("Ended with {} units not verified. Read the summary, then decide what to do next.", "已結束，有 {} 個單元尚未驗收。請看摘要，再決定下一步。"),
    ("Ended: every unit is verified, but the full test failed. Read the summary before relying on it.", "已結束：單元都已驗收，但完整測試失敗。採用前請先看摘要。"),
    ("Ended: every unit is verified, but the full test has no passing result. Read the summary before relying on it.", "已結束：單元都已驗收，但完整測試沒有通過的結果。採用前請先看摘要。"),
    ("Ended with no units recorded, so nothing is confirmed as built.", "已結束，沒有記錄到任何單元，所以沒有任何東西確認已建好。"),
];

const STAGE_NAMES: [&str; 7] = ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report"];
#[derive(Clone, Copy, Debug)]
struct ChromeHeights { brief: Pixels, header: Pixels, rail: Pixels }

fn caption_line(d: Density) -> Pixels {
    px((f32::from(theme::font(d, TypeSize::Caption)) * 1.6).ceil())
}

fn chrome_heights(d: Density) -> ChromeHeights {
    ChromeHeights {
        brief: theme::row_height(d),
        header: theme::row_height(d) * 2.,
        rail: caption_line(d) * 2. + theme::space(d, Step::Sm) * 2.,
    }
}

#[derive(Debug, PartialEq)]
enum PanelView { Skeleton, Empty, Content, Error { message: SharedString, keep_content: bool } }

fn panel_view(p: &PanelState) -> PanelView {
    match p {
        PanelState::Loading => PanelView::Skeleton,
        PanelState::Empty => PanelView::Empty,
        PanelState::Ready => PanelView::Content,
        PanelState::Failed(message) => PanelView::Error { message: message.clone(), keep_content: false },
        PanelState::Stale(message) => PanelView::Error { message: message.clone(), keep_content: true },
    }
}

fn stage_tip(name: &str, lang: Lang) -> SharedString {
    let id = match name {
        "Astra" => Some("astra"), "Critic" => Some("critic"),
        "Architect" => Some("architect"), "Memory" => Some("memory-keeper"), _ => None,
    };
    if let Some(id) = id { return super::i18n::role_tooltip(lang, id); }
    let key = match name {
        "Build" => "Build implements and checks the planned units.",
        "Full test" => "Full test checks the completed work together.",
        "Report" => "Report records the outcome of the wish.",
        _ => return name.to_owned().into(),
    };
    translate(lang, TEXT, key).into()
}

const FULL_TEST: &str = "Full test";

const UNKNOWN: &str = "—";
/// The dim portrait behind the no-runs state.
const PORTRAIT_SIDE: f32 = 260.0;
const ONE_LINE_CHARS: usize = 400;
const TOOLTIP_CHARS: usize = 600;
const LONG_TEXT_CHARS: usize = 20_000;

const CHECKLIST_WIDTH: Pixels = px(300.);
const CHECKLIST_RANGE: std::ops::Range<Pixels> = px(240.)..px(480.);
const GRAPH_MIN: Pixels = px(320.);
const DETAILS_WIDTH: Pixels = px(380.);
const DETAILS_RANGE: std::ops::Range<Pixels> = px(300.)..px(560.);

fn restored_widths(saved: Option<settings::ColumnWidths>, available: f32) -> settings::ColumnWidths {
    saved.unwrap_or(settings::ColumnWidths { checklist: f32::from(CHECKLIST_WIDTH), details: f32::from(DETAILS_WIDTH) })
        .clamped(available, f32::from(CHECKLIST_RANGE.start)..f32::from(CHECKLIST_RANGE.end),
            f32::from(DETAILS_RANGE.start)..f32::from(DETAILS_RANGE.end), f32::from(GRAPH_MIN))
}

fn step_widths(mut w: settings::ColumnWidths, key: &str, available: f32) -> Option<settings::ColumnWidths> {
    match key {
        "alt-left" => w.checklist -= 16.,
        "alt-right" => w.checklist += 16.,
        "alt-shift-left" => w.details += 16.,
        "alt-shift-right" => w.details -= 16.,
        "alt-0" => return Some(restored_widths(None, available)),
        _ => return None,
    }
    Some(restored_widths(Some(w), available))
}

fn focus_target(mode: settings::WindowMode) -> Option<settings::FocusPanel> {
    match mode { settings::WindowMode::Focus(panel) => Some(panel), _ => None }
}

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
    OnLimit,
    Skipped,
    Unfinished,
}

impl StageState {
    fn of(phase: Phase, status: RunStatus) -> Self {
        match (phase, status) {
            (Phase::Done, _) => Self::Done,
            (Phase::Failed, _) => Self::Failed,
            (Phase::Skipped, _) => Self::Skipped,
            (_, RunStatus::Done) => Self::Unfinished,
            (Phase::Waiting, _) => Self::Waiting,
            (Phase::Running, RunStatus::Active) => Self::Running,
            (Phase::Running, RunStatus::Waiting) => Self::OnBatch,
            (Phase::Running, RunStatus::LimitWait) => Self::OnLimit,
            (Phase::Running, RunStatus::Blocked | RunStatus::Interrupted) => Self::NeedsYou,
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
            Self::OnLimit => RunStatus::LimitWait.label(),
            Self::Skipped => "Skipped",
            Self::Unfinished => "Not finished",
        }
    }

    fn color(self, t: &Tokens) -> Hsla {
        match self {
            Self::Done => t.ok,
            Self::Running => t.run,
            Self::Waiting | Self::Unfinished | Self::Skipped => t.idle,
            Self::Failed | Self::NeedsYou => t.fail,
            Self::OnBatch | Self::OnLimit => t.review,
        }
    }

    fn is_current(self) -> bool {
        matches!(self, Self::Running | Self::Failed | Self::NeedsYou | Self::OnBatch | Self::OnLimit)
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
    usage.unpriced()
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
        Tooltip::element(move |_, _| div().max_w(theme::READ_WIDTH).whitespace_normal().child(text.clone())).build(window, cx)
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

fn stage_icon(state: StageState, animate: bool, t: &Tokens, d: Density) -> AnyElement {
    let color = state.color(t);
    let icon = |name: IconName| Icon::new(name).size(theme::font(d, TypeSize::Title)).text_color(color).into_any_element();
    match state {
        StageState::Done => icon(IconName::CircleCheck),
        StageState::Running if animate => {
            super::motion::Spin::new(IconName::LoaderCircle).color(color).size(theme::font(d, TypeSize::Title)).into_any_element()
        }
        StageState::Running => icon(IconName::LoaderCircle),
        StageState::Failed => icon(IconName::CircleX),
        StageState::NeedsYou => icon(IconName::CircleAlert),
        StageState::OnBatch | StageState::OnLimit => icon(IconName::Pause),
        StageState::Unfinished | StageState::Skipped => icon(IconName::Dash),
        StageState::Waiting => div()
            .size(theme::font(d, TypeSize::Caption))
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
    let duty = stage_tip(&stage.name, *cx.global::<Lang>());
    let tip = match note {
        Some(note) => format!("{duty}\n{name}: {word}\n{note}"),
        None => format!("{duty}\n{name}: {word}"),
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
        .map(|this| if last { this.max_w(px(128.)) } else { this.flex_1() })
        .min_w_0()
        .items_center()
        .gap(theme::space(theme::density(cx), Step::Sm))
        .tooltip(tooltip(tip))
        .child(div().flex_none().child(stage_icon(state, animate, &t, theme::density(cx))))
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                        .font_weight(if state.is_current() { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                        .text_color(name_color)
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme::font(theme::density(cx), TypeSize::Caption))
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
                    .mx(theme::space(theme::density(cx), Step::Sm))
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
        .px(theme::space(theme::density(cx), Step::Xl))
        .py(theme::space(theme::density(cx), Step::Sm))
        .border_b_1()
        .border_color(t.border);
    if run.stages.is_empty() {
        return rail.text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(tr(cx, TEXT, "Stages have not been reported yet"));
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
        .child(div().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(tr(cx, TEXT, label)))
        .child(div().text_size(theme::font(theme::density(cx), TypeSize::Body)).font_weight(FontWeight::MEDIUM).text_color(t.text).child(value))
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
        RunStatus::Waiting | RunStatus::LimitWait => (run.status.label(), t.review, IconName::Pause),
        RunStatus::Interrupted | RunStatus::Blocked => (run.status.label(), t.fail, IconName::CircleAlert),
        RunStatus::Done if clean => (run.status.label(), t.ok, IconName::CircleCheck),
        RunStatus::Done => ("Ended, needs attention", t.review, IconName::TriangleAlert),
    };
    div().id("run-status").tooltip(tooltip(tr(cx, TEXT, word))).child(tinted_tag(color).rounded_full().flex_none().child(
        h_flex()
            .gap(theme::space(theme::density(cx), Step::Xs))
            .items_center()
            .child(Icon::new(icon).size(theme::font(theme::density(cx), TypeSize::Caption)))
            .child(tr(cx, TEXT, word)),
    ))
}

/// How the top line should look: calm when Chronos is simply working, urgent when it needs a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Working,
    Waiting,
    NeedsYou,
    Finished,
    Unsettled,
}

/// The one sentence at the top of the Run screen: what is happening, whether the person must act,
/// and what comes next. `key` is an English template in `TEXT`, `args` fill its `{}` slots.
#[derive(Clone, Debug, PartialEq)]
struct Brief {
    key: &'static str,
    args: Vec<String>,
    tone: Tone,
}

impl Brief {
    fn plain(key: &'static str, tone: Tone) -> Self {
        Self { key, args: vec![], tone }
    }

    fn text(&self, lang: Lang) -> String {
        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        fill(translate(lang, TEXT, self.key), &args)
    }
}

fn clipped_title(text: &str) -> String {
    const LIMIT: usize = 80;
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}…", line[..end].trim_end()),
        None => line.to_owned(),
    }
}

/// Reads only what the run reports. A state it cannot tell apart gets the plainest true sentence.
fn blocked_brief_title(title: plain_text::Plain) -> (String, Option<SharedString>) {
    (clipped_title(&title.shown), title.original)
}

fn attention_title(title: plain_text::Plain) -> (SharedString, SharedString) {
    let tooltip = title.original.unwrap_or_else(|| title.shown.clone());
    (title.shown, tooltip)
}

fn brief_of(run: &Run) -> Brief {
    let working = |key| Brief::plain(key, Tone::Working);
    match run.status {
        RunStatus::Active => {
            let building = run.count(UnitState::Running);
            if building == 1 {
                return working("Building 1 unit. Nothing needs you for now; it is checked once it is built.");
            }
            if building > 1 {
                return Brief {
                    key: "Building {} units. Nothing needs you for now; each is checked once it is built.",
                    args: vec![building.to_string()],
                    tone: Tone::Working,
                };
            }
            match run.stages.iter().find(|s| s.phase == Phase::Running).map(|s| s.name.as_ref()) {
                Some("Astra") => working("Astra is planning. Nothing needs you for now; building starts once the plan is reviewed."),
                Some("Critic") => working("The plan is being reviewed. Nothing needs you for now; the work is split into modules next."),
                Some("Architect") => working("The plan is being split into modules. Nothing needs you for now; building starts next."),
                Some("Build") => working("Getting the next units ready. Nothing needs you for now."),
                Some("Full test") => working("Running the full test. Nothing needs you for now; it decides whether the wish counts as finished."),
                Some("Memory") => working("Writing down what was learned. Nothing needs you for now; the report comes next."),
                Some("Report") => working("Writing the report. Nothing needs you for now; it will be here when it is done."),
                _ if run.units.is_empty() => working("Chronos is starting. Nothing needs you for now."),
                _ => working("Chronos is working. Nothing needs you for now."),
            }
        }
        RunStatus::Waiting => Brief::plain(
            "A background batch is running. Nothing needs you for now; you will be told when it finishes, and waiting uses no model quota.",
            Tone::Waiting,
        ),
        RunStatus::LimitWait => Brief::plain(
            "The usage limit was reached. Nothing needs you; it continues by itself when the limit resets.",
            Tone::Waiting,
        ),
        RunStatus::Interrupted => Brief::plain(
            "The session stopped before it finished. Resume it to continue where it left off.",
            Tone::NeedsYou,
        ),
        RunStatus::Blocked => match run.needs.iter().find(|n| n.kind != NeedKind::Interjection) {
            Some(need) if !clipped_title(&need.title).is_empty() => Brief {
                key: "Stopped and waiting for you: {}. Deal with it, then resume.",
                args: vec![clipped_title(&need.title)],
                tone: Tone::NeedsYou,
            },
            _ => Brief::plain("Stopped and waiting for you. Read the note below, then resume.", Tone::NeedsYou),
        },
        RunStatus::Done => {
            let summary = Summary::of(run);
            if run.verified_end() {
                return Brief {
                    key: "Finished: all {} units are verified and the full test passed. Nothing is left for you to do.",
                    args: vec![summary.total.to_string()],
                    tone: Tone::Finished,
                };
            }
            let unsettled = |key| Brief::plain(key, Tone::Unsettled);
            match summary.verdict {
                Verdict::NotVerified if summary.unverified.len() == 1 => {
                    unsettled("Ended with 1 unit not verified. Read the summary, then decide what to do next.")
                }
                Verdict::NotVerified => Brief {
                    key: "Ended with {} units not verified. Read the summary, then decide what to do next.",
                    args: vec![summary.unverified.len().to_string()],
                    tone: Tone::Unsettled,
                },
                Verdict::TestFailed => {
                    unsettled("Ended: every unit is verified, but the full test failed. Read the summary before relying on it.")
                }
                Verdict::NoUnits => unsettled("Ended with no units recorded, so nothing is confirmed as built."),
                Verdict::TestNotRun | Verdict::Clean => unsettled(
                    "Ended: every unit is verified, but the full test has no passing result. Read the summary before relying on it.",
                ),
            }
        }
    }
}

/// The first-day checklist. Every item is read from state the app already holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FirstDay {
    tools: bool,
    signed_in: bool,
    project: bool,
    wish: bool,
}

impl FirstDay {
    /// `has_project` is whether a project folder has been chosen before, outside any run.
    fn of(checks: &[Check], runs: &[Run], has_project: bool) -> Self {
        let tool = |name: &str| checks.iter().find(|c| c.name.eq_ignore_ascii_case(name));
        let pair = [tool("claude"), tool("codex")];
        // A tool the doctor could not find says so with "not on PATH".
        let found = |c: Option<&Check>| c.is_some_and(|c| !c.detail.contains("not on PATH"));
        let tools = pair.iter().all(|c| found(*c));
        // The doctor has no sign-in probe of its own: a tool that is found and not failing is
        // taken as signed in, and a failure such as "not logged in" is what unticks it.
        let signed_in = tools && pair.iter().all(|c| c.is_some_and(|c| c.status != CheckStatus::Fail));
        Self {
            tools,
            signed_in,
            project: has_project || runs.iter().any(|r| r.cwd.is_some()),
            wish: runs.iter().any(|r| r.status == RunStatus::Done),
        }
    }

    fn complete(self) -> bool {
        self.items().iter().all(|(_, done, _)| *done)
    }

    fn done_count(self) -> usize {
        self.items().iter().filter(|(_, done, _)| *done).count()
    }

    /// Label, ticked, and what ticking it means.
    fn items(self) -> [(&'static str, bool, &'static str); 4] {
        [
            ("Tools found", self.tools, "Claude Code and Codex are both installed."),
            ("Signed in", self.signed_in, "Neither tool reports a sign-in problem."),
            ("A project is chosen", self.project, "A project folder has been used with Chronos."),
            ("First wish finished", self.wish, "A wish has run to its end."),
        ]
    }
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
        "run-select-unit" => state.selection = Selection::Unit(1),
        // A first day: no wish has ended yet.
        "run-first-day" => {
            state.runs.retain(|r| r.status != RunStatus::Done);
            state.current = state.runs.iter().position(|r| r.status == RunStatus::Active).or(Some(0));
            state.checks.iter_mut().filter(|c| c.name.as_ref() == "codex").for_each(|c| {
                c.status = CheckStatus::Fail;
                c.detail = "codex failed: not logged in".into();
            });
        }
        "run-first-day-empty" => {
            state.runs.clear();
            state.current = None;
        }
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
    columns_focus: FocusHandle,
    columns_shown: bool,
    restored: bool,
    /// The run the transient state below belongs to.
    viewing: Option<SharedString>,
    wish_open: bool,
    report_open: bool,
    hidden_summaries: HashSet<SharedString>,
    glows: motion::Glows<SharedString>,
    hovered: Option<SharedString>,
    details_height: std::rc::Rc<std::cell::Cell<f32>>,
}

impl RunView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, _, cx| {
            if this.state.read(cx).view != Screen::Run { this.columns_shown = false; }
            this.settle_first_day(cx);
            cx.notify();
        })
        .detach();
        let scene = state.read(cx).scene.clone().unwrap_or_default();
        state.update(cx, |s, _| apply_scene(s, &scene));
        match scene.as_ref() {
            "run-focus-graph" => { settings::set_mode(settings::WindowMode::Focus(settings::FocusPanel::Graph), cx); }
            "run-focus-checklist" => { settings::set_mode(settings::WindowMode::Focus(settings::FocusPanel::Checklist), cx); }
            _ => {}
        }
        cx.observe_global::<settings::Store>(|_, cx| cx.notify()).detach();
        let columns = cx.new(|_| ResizableState::default());
        cx.subscribe_in(&columns, window, |_, columns, _: &ResizablePanelEvent, _, cx| {
            let sizes = columns.read(cx).sizes();
            if sizes.len() == 3 {
                let widths = settings::ColumnWidths { checklist: f32::from(sizes[0]), details: f32::from(sizes[2]) };
                settings::remember_columns(widths, cx);
            }
        }).detach();
        let viewing = state.read(cx).run().map(|r| r.session.clone());
        let this = Self {
            todo: cx.new(|cx| Todo::new(state.clone(), window, cx)),
            mesh: cx.new(|cx| Mesh::new(state.clone(), window, cx)),
            detail: cx.new(|cx| Detail::new(state.clone(), window, cx)),
            control: cx.new(|cx| Control::new(state.clone(), window, cx)),
            columns,
            columns_focus: cx.focus_handle().tab_stop(true),
            columns_shown: false,
            restored: false,
            hidden_summaries: viewing.iter().filter(|_| scene.ends_with("-graph")).cloned().collect(),
            viewing,
            wish_open: scene.ends_with("-open"),
            report_open: false,
            glows: motion::Glows::new(Pace::Glow),
            hovered: None,
            details_height: std::rc::Rc::new(std::cell::Cell::new(0.0)),
            state,
        };
        this.settle_first_day(cx);
        this
    }

    fn first_day(&self, cx: &App) -> FirstDay {
        let state = self.state.read(cx);
        FirstDay::of(&state.checks, &state.runs, settings::has_recent_project(cx))
    }

    /// Once every item is ticked the list is retired for good, through the settings store.
    fn settle_first_day(&self, cx: &mut App) {
        if !settings::first_day_done(cx) && self.first_day(cx).complete() {
            settings::finish_first_day(cx);
        }
    }

    /// The checklist, or nothing once it has been completed.
    fn first_day_card(&self, cx: &App) -> Option<AnyElement> {
        if settings::first_day_done(cx) {
            return None;
        }
        let t = cx.tokens();
        let day = self.first_day(cx);
        let count = fill(tr(cx, TEXT, "{} of {} done"), &[&day.done_count().to_string(), "4"]);
        Some(
            h_flex()
                .id("first-day")
                .w_full()
                .flex_wrap()
                .items_center()
                .gap_x(theme::space(theme::density(cx), Step::Xl))
                .gap_y(theme::space(theme::density(cx), Step::Xs))
                .px(theme::space(theme::density(cx), Step::Lg))
                .py(theme::space(theme::density(cx), Step::Sm))
                .rounded(theme::RADIUS)
                .bg(t.surface)
                .border_1()
                .border_color(t.gold.opacity(0.3))
                .child(
                    h_flex()
                        .gap(theme::space(theme::density(cx), Step::Sm))
                        .items_baseline()
                        .child(ornament::section_label(&t, tr(cx, TEXT, "Your first day with Chronos")))
                        .child(div().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(count)),
                )
                .children(day.items().into_iter().enumerate().map(|(i, (label, done, meaning))| {
                    let tip = if done { meaning } else { "Not yet." };
                    h_flex()
                        .id(("first-day-item", i))
                        .gap(theme::space(theme::density(cx), Step::Sm))
                        .items_center()
                        .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                        .text_color(if done { t.muted } else { t.text })
                        .child(if done {
                            Icon::new(IconName::CircleCheck).size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.ok).into_any_element()
                        } else {
                            div().size(px(12.)).m(px(1.)).rounded_full().border_1().border_color(t.muted).into_any_element()
                        })
                        .child(tr(cx, TEXT, label))
                        .tooltip(tooltip(tr(cx, TEXT, tip)))
                }))
                .into_any_element(),
        )
    }

    /// The top line: what is happening, whether anything is needed, and what follows.
    fn brief(&self, run: &Run, cx: &App) -> impl IntoElement {
        let t = cx.tokens();
        let mut brief = brief_of(run);
        let mut full_brief = brief.clone();
        let original = if run.status == RunStatus::Blocked && !brief.args.is_empty() {
            run.needs.iter().find(|n| n.kind != NeedKind::Interjection).and_then(|need| {
                let plain = plain_text::plain(cx, &need.title);
                full_brief.args[0] = plain.shown.to_string();
                let (arg, original) = blocked_brief_title(plain);
                brief.args[0] = arg;
                original
            })
        } else {
            None
        };
        let (icon, color) = match brief.tone {
            Tone::Working => (IconName::LoaderCircle, t.run),
            Tone::Waiting => (IconName::Pause, t.review),
            Tone::NeedsYou => (IconName::CircleAlert, t.fail),
            Tone::Finished => (IconName::CircleCheck, t.ok),
            Tone::Unsettled => (IconName::TriangleAlert, t.review),
        };
        let text = brief.text(*cx.global::<Lang>());
        let full_text = full_brief.text(*cx.global::<Lang>());
        let tip = original.map_or_else(|| full_text.clone(), |original| format!("{full_text}\n{original}"));
        h_flex()
            .id("run-brief")
            .flex_none()
            .w_full()
            .items_center()
            .gap(theme::space(theme::density(cx), Step::Sm))
            .px(theme::space(theme::density(cx), Step::Xl))
            .py(theme::space(theme::density(cx), Step::Sm))
            .bg(color.opacity(0.08))
            .border_b_1()
            .border_color(t.border)
            .child(Icon::new(icon).size(theme::font(theme::density(cx), TypeSize::Title)).text_color(color))
            .child(
                div().id("run-brief-text").min_w_0().max_w(theme::READ_WIDTH).truncate().text_size(theme::font(theme::density(cx), TypeSize::Body)).font_weight(FontWeight::MEDIUM).text_color(t.text)
                    .tooltip(tooltip(tip)).child(text),
            )
    }

    /// The details panel, with its "say something about this" action when a message can reach the run.
    fn details_column(&self, run: &Run, selection: &Selection, driver: &mut motion::Driver, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let can_say = control::offer(run.status, run.hosted, run.cwd.is_some(), run.report.is_some()).entry
            == control::Entry::Interjection;
        let subject = control::subject_of(run, selection).filter(|_| can_say);
        let state = self.state.clone();
        let app = self.state.read(cx);
        let runs = board_runs(app);
        let d = theme::density(cx);
        let padding = f32::from(theme::space(d, Step::Sm));
        let layout = board_layout(self.details_height.get(), runs.len(), runs.iter().filter(|&&i| needs_you(&app.runs[i])).count(), padding)
            .filter(|_| *selection == Selection::None);
        let measured_height = self.details_height.clone();
        let board = layout.map(|layout| {
            let levels: std::collections::HashMap<SharedString, f32> = board_runs(app).into_iter()
                .map(|i| { let key = app.runs[i].session.clone(); let value = self.glows.value(&key, driver); (key, value) }).collect();
            let lift = |key: &SharedString| levels.get(key).copied().unwrap_or(0.0);
            let weak = cx.weak_entity();
            let hover = std::rc::Rc::new(move |key: SharedString, inside: bool, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    let reduce = this.state.read(cx).reduce_motion;
                    this.hovered = if inside { Some(key.clone()) } else if this.hovered.as_ref() == Some(&key) { None } else { this.hovered.clone() };
                    this.glows.set(&key, if inside { 1.0 } else { 0.0 }, motion::now(cx), reduce);
                    cx.notify();
                });
            });
            let board = match layout.mode {
                BoardMode::Strip => board::board_strip(app, cx),
                BoardMode::Card => board::board_with_hover(app, true, Some(layout.visible), &lift, hover, cx),
            };
            (layout, board)
        });
        column(
            v_flex()
                .size_full()
                .on_prepaint(move |bounds, window, _| {
                    let height = f32::from(bounds.size.height);
                    if measured_height.replace(height) != height { window.request_animation_frame(); }
                })
                .when_some(board, |this, (layout, board)| this.child(div().id("overview-board").flex_none().w_full().max_h(px(layout.max_height)).min_h(px(layout.min_height)).when(layout.mode == BoardMode::Card, |this| this.p(theme::space(d, Step::Sm))).child(board)))
                .child(div().flex_1().min_h_0().w_full().child(self.detail.clone()))
                .when_some(subject, |this, subject| {
                    this.child(
                        h_flex()
                            .flex_none()
                            .w_full()
                            .px(theme::space(theme::density(cx), Step::Lg))
                            .py(theme::space(theme::density(cx), Step::Sm))
                            .border_t_1()
                            .border_color(t.border)
                            .child(
                                Button::new("say-about-this")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ArrowUp)
                                    .label(tr(cx, TEXT, "Say something about this"))
                                    .tooltip(tr(cx, TEXT, "Opens the message box with this item named at the start."))
                                    .on_click(move |_, _, cx| {
                                        let text = control::address_text(&subject, cx);
                                        state.update(cx, |s, cx| s.address_interjection(text, cx));
                                    }),
                            ),
                    )
                }),
        )
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
        let row = h_flex().w_full().max_w(theme::READ_WIDTH).min_w_0().items_start().gap(theme::space(theme::density(cx), Step::Xs)).text_size(theme::font(theme::density(cx), TypeSize::Title)).font_weight(FontWeight::SEMIBOLD);
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
                .max_h(theme::row_height(theme::density(cx)) - theme::space(theme::density(cx), Step::Sm))
                .gap(theme::space(theme::density(cx), Step::Xs))
                .overflow_y_scrollbar()
                .child(
                    div()
                        .w_full()
                        .whitespace_normal()
                        .text_size(theme::font(theme::density(cx), TypeSize::Body))
                        .line_height(relative(1.6))
                        .font_weight(FontWeight::NORMAL)
                        .child(SharedString::from(shown.to_owned())),
                )
                .when(cut, |this| {
                    this.child(
                        div()
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
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
                .accessibility_label(tr(cx, TEXT, if open { "Show one line" } else { "Show the whole wish" }))
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
                .gap(theme::space(theme::density(cx), Step::Xs))
                .px(theme::space(theme::density(cx), Step::Xs))
                .child(Icon::new(IconName::Folder).size(theme::font(theme::density(cx), TypeSize::Caption)))
                .child(div().min_w_0().truncate().child(name))
                .tooltip(tooltip(tr(cx, TEXT, "The folder of this wish is not known")))
                .into_any_element(),
        }
    }

    fn header(&self, run: &Run, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let elapsed = run.elapsed.clone().unwrap_or_else(|| UNKNOWN.into());
        let warnings = usage::warning_texts(run, *cx.global::<Lang>());
        let warning_count = warnings.len();
        h_flex()
            .flex_none()
            .w_full()
            .items_center()
            .gap(theme::space(theme::density(cx), Step::Lg))
            .px(theme::space(theme::density(cx), Step::Lg))
            .py(theme::space(theme::density(cx), Step::Xs))
            .bg(t.surface)
            .border_b_1()
            .border_color(t.border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(theme::space(theme::density(cx), Step::Xs))
                    .child(self.wish(run, cx))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
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
                                    Tag::custom(t.raised, t.muted, t.border).flex_none().py_0().child(
                                        h_flex()
                                            .gap(theme::space(theme::density(cx), Step::Xs))
                                            .items_center()
                                            .child(Icon::new(IconName::Eye).size(theme::font(theme::density(cx), TypeSize::Caption)))
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
                                let tip = warnings.iter().map(|w| w.as_ref()).collect::<Vec<_>>().join("\n");
                                this.child(
                                    h_flex()
                                        .id("run-warnings")
                                        .flex_none()
                                        .gap(theme::space(theme::density(cx), Step::Xs))
                                        .items_center()
                                        .text_color(t.review)
                                        .child(Icon::new(IconName::TriangleAlert).size(theme::font(theme::density(cx), TypeSize::Caption)))
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
                    .gap(theme::space(theme::density(cx), Step::Xl))
                    .child(stat("stat-elapsed", "Elapsed", elapsed.clone(), Some(elapsed), cx))
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
            .gap(theme::space(theme::density(cx), Step::Sm))
            .px(theme::space(theme::density(cx), Step::Lg))
            .py(theme::space(theme::density(cx), Step::Md))
            .rounded(theme::RADIUS)
            .bg(t.surface)
            .border_1()
            .border_color(t.fail.opacity(0.4))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap(theme::space(theme::density(cx), Step::Md))
                    .child(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .text_size(theme::font(theme::density(cx), TypeSize::Body))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(Icon::new(IconName::TriangleAlert).size(theme::font(theme::density(cx), TypeSize::Title)).text_color(t.fail))
                            .child(div().min_w_0().truncate().child(tr(cx, TEXT, "Chronos cannot start a wish yet"))),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(theme::space(theme::density(cx), Step::Xs))
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
                                        state.update(cx, |s, cx| s.recheck(cx))
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
                    .gap(theme::space(theme::density(cx), Step::Md))
                    .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                    .child(
                        h_flex()
                            .w(px(104.))
                            .flex_none()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .items_center()
                            .text_color(t.text)
                            .font_weight(FontWeight::MEDIUM)
                            .child(Icon::new(IconName::CircleX).size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.fail))
                            .child(div().min_w_0().truncate().child(check.name.clone())),
                    )
                    .child(
                        v_flex()
                            .id(("setup-check", i))
                            .flex_1()
                            .min_w_0()
                            .gap(theme::space(theme::density(cx), Step::Xs))
                            .tooltip(tooltip(check.detail.clone()))
                            .child(div().w_full().text_color(t.text).line_clamp(2).child(detail))
                            .when_some(fix, |this, fix| {
                                this.child(div().w_full().text_color(t.muted).child(tr(cx, TEXT, fix)))
                            }),
                    )
            }))
    }

    fn retry_note(&self, message: SharedString, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let state = self.state.clone();
        h_flex().w_full().items_center().gap(theme::space(theme::density(cx), Step::Sm))
            .px(theme::space(theme::density(cx), Step::Lg))
            .child(ornament::stale_note(&t, message))
            .child(Button::new("retry-runs").ghost().small().label(tr(cx, TEXT, "Try again"))
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.retry(FeedKind::Runs, cx))))
    }

    fn no_run(&self, no_runs: bool, failures: &[&Check], cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let blocked = !failures.is_empty();
        let compose = self.state.clone();
        let wishes = self.state.clone();
        let (title, description) = if no_runs {
            (
                "Start with a wish",
                "Say what you want done. Chronos plans it, builds it and checks every piece.",
            )
        } else {
            ("No wish is open", "Open one from Wishes, or make a new one.")
        };
        v_flex()
            .id("run-empty")
            .size_full()
            .items_center()
            .justify_center()
            .gap(theme::space(theme::density(cx), Step::Xl))
            .p(theme::space(theme::density(cx), Step::Xxl))
            .overflow_y_scrollbar()
            // The portrait sits in the flow above everything, and its lower half fades out, so no
            // text is ever drawn over it.
            .child(ornament::portrait_backdrop(&t, PORTRAIT_SIDE, cx.theme().is_dark()))
            .when_some(self.first_day_card(cx), |this, card| {
                this.child(div().w_full().max_w(px(640.)).child(card))
            })
            .when(blocked, |this| {
                this.child(div().w_full().max_w(px(640.)).child(self.setup_card(failures, cx)))
            })
            .child(
                Empty::new()
                    .flex_none()
                    .gap(theme::space(theme::density(cx), Step::Xl))
                    .header(
                        EmptyHeader::new()
                            .max_w(px(560.))
                            .title(EmptyTitle::new().child(ornament::page_title(&t, tr(cx, TEXT, title))))
                            .description(EmptyDescription::new().text_size(theme::font(theme::density(cx), TypeSize::Body)).child(tr(cx, TEXT, description))),
                    )
                    .content(
                        EmptyContent::new()
                            .child(
                                theme::gold_leaf(
                                    Button::new("make-a-wish")
                                        .large()
                                        .h(px(44.))
                                        .px(theme::space(theme::density(cx), Step::Xl))
                                        .disabled(blocked)
                                        .child(
                                            h_flex()
                                                .gap(theme::space(theme::density(cx), Step::Sm))
                                                .items_center()
                                                .text_size(theme::font(theme::density(cx), TypeSize::Title))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(Icon::new(IconName::Star).size(theme::font(theme::density(cx), TypeSize::Title)))
                                                .child(tr(cx, TEXT, "Make a wish")),
                                        )
                                        .on_click(move |_, _, cx| compose.update(cx, |s, cx| s.set_composing(true, cx))),
                                    &t,
                                ),
                            )
                            .when(blocked, |this| {
                                this.child(
                                    div().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(tr(cx, TEXT, "Fix the items above to make a wish")),
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
            .gap(theme::space(theme::density(cx), Step::Xs))
            .px(theme::space(theme::density(cx), Step::Md))
            .py(theme::space(theme::density(cx), Step::Md))
            .rounded(theme::RADIUS)
            .bg(t.raised)
            .child(div().truncate().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(tr(cx, TEXT, label)))
    }

    fn attention_list(&self, run: &Run, summary: &Summary, driver: &mut motion::Driver, cx: &Context<Self>) -> Vec<AnyElement> {
        let t = cx.tokens();
        let mut rows: Vec<AnyElement> = summary
            .unverified
            .iter()
            .map(|&index| {
                let unit = &run.units[index];
                let (title, tooltip) = attention_title(plain_text::plain(cx, &unit.title));
                let color = t.unit(unit.state);
                let state = self.state.clone();
                let hover_key: SharedString = format!("hover:{}", unit.id).into();
                let selection_key: SharedString = format!("sel:{}", unit.id).into();
                let lift = self.glows.value(&hover_key, driver);
                let selected = self.glows.value(&selection_key, driver);
                ornament::raised_card(&t, lift, selected)
                    .id(("attention-card", index))
                    .on_hover(cx.listener(move |this, inside: &bool, _, cx| {
                        let reduce = this.state.read(cx).reduce_motion;
                        this.glows.set(&hover_key, if *inside { 1.0 } else { 0.0 }, motion::now(cx), reduce);
                        cx.notify();
                    }))
                    .child(Button::new(("attention-unit", index))
                    .ghost()
                    .small()
                    .w_full()
                    .tooltip(tooltip)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .items_center()
                            .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                            .child(Icon::new(unit_icon(unit.state)).size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(color))
                            .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(title))
                            .child(div().flex_none().text_color(color).child(tr(cx, TEXT, unit.state.label()))),
                    )
                    .focus_visible(move |style| style.shadow(theme::focus_ring(&t)))
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.select(Selection::Unit(index), cx))))
                    .into_any_element()
            })
            .collect();
        let line = |id: (&'static str, usize), icon: IconName, color: Hsla, text: SharedString| {
            h_flex()
                .id(id)
                .w_full()
                .min_w_0()
                .gap(theme::space(theme::density(cx), Step::Sm))
                .px(theme::space(theme::density(cx), Step::Sm))
                .py(theme::space(theme::density(cx), Step::Xs))
                .items_start()
                .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                .tooltip(tooltip(text.clone()))
                .child(div().flex_none().mt(theme::space(theme::density(cx), Step::Xs)).child(Icon::new(icon).size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(color)))
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
            usage::warning_texts(run, *cx.global::<Lang>())
                .into_iter()
                .enumerate()
                .map(|(i, w)| line(("attention-warning", i), IconName::TriangleAlert, t.review, w)),
        );
        rows
    }

    fn summary(&self, run: &Run, summary: &Summary, driver: &mut motion::Driver, cx: &Context<Self>) -> impl IntoElement {
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
        let attention = self.attention_list(run, summary, driver, cx);
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
            .p(theme::space(theme::density(cx), Step::Lg))
            .bg(t.canvas.opacity(0.72))
            .child(
                ornament::frame(&t, accent.opacity(0.6), t.surface)
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(560.))
                    .max_h_full()
                    .child(
                        h_flex()
                            .flex_none()
                            .w_full()
                            .items_center()
                            .gap(theme::space(theme::density(cx), Step::Md))
                            .px(theme::space(theme::density(cx), Step::Lg))
                            .pt(theme::space(theme::density(cx), Step::Lg))
                            .pb(theme::space(theme::density(cx), Step::Md))
                            .child(Icon::new(icon).size(theme::font(theme::density(cx), TypeSize::Heading)).text_color(accent))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(theme::font(theme::density(cx), TypeSize::Title))
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
                            .gap(theme::space(theme::density(cx), Step::Md))
                            .px(theme::space(theme::density(cx), Step::Lg))
                            .pb(theme::space(theme::density(cx), Step::Md))
                            .overflow_y_scrollbar()
                            .child(
                                h_flex()
                                    .flex_none()
                                    .w_full()
                                    .items_stretch()
                                    .gap(theme::space(theme::density(cx), Step::Sm))
                                    .child(
                                        Self::summary_tile("Units verified", cx)
                                            .child(
                                                div()
                                                    .text_size(theme::font(theme::density(cx), TypeSize::Heading))
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
                                                    .gap(theme::space(theme::density(cx), Step::Sm))
                                                    .items_center()
                                                    .text_size(theme::font(theme::density(cx), TypeSize::Heading))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(test_color)
                                                    .when_some(test_icon, |this, icon| {
                                                        this.child(Icon::new(icon).size(theme::font(theme::density(cx), TypeSize::Title)))
                                                    })
                                                    .child(tr(cx, TEXT, test_word)),
                                            )
                                            .when_some(summary.full_test_note.clone(), |this, note| {
                                                this.child(div().truncate().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(note))
                                            }),
                                    )
                                    .child(
                                        Self::summary_tile("Cost", cx)
                                            .child(
                                                div()
                                                    .text_size(theme::font(theme::density(cx), TypeSize::Heading))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(t.text)
                                                    .child(cost_text(run.usage.total_usd)),
                                            )
                                            .child(div().truncate().text_size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.muted).child(tr(
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
                                    .gap(theme::space(theme::density(cx), Step::Xs))
                                    .when(attention_count > 0, |this| {
                                        this.child(
                                            ornament::section_label(&t, tr(cx, TEXT, "What needs attention")),
                                        )
                                    })
                                    .when(summary.verdict == Verdict::Clean && attention_count == 0, |this| {
                                        this.child(
                                            h_flex()
                                                .gap(theme::space(theme::density(cx), Step::Sm))
                                                .px(theme::space(theme::density(cx), Step::Sm))
                                                .py(theme::space(theme::density(cx), Step::Xs))
                                                .items_center()
                                                .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                                                .text_color(t.text)
                                                .child(Icon::new(IconName::Check).size(theme::font(theme::density(cx), TypeSize::Caption)).text_color(t.ok))
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
                                        .gap(theme::space(theme::density(cx), Step::Xs))
                                        .p(theme::space(theme::density(cx), Step::Md))
                                        .rounded(theme::RADIUS)
                                        .bg(t.raised)
                                        .text_size(theme::font(theme::density(cx), TypeSize::Caption))
                                        .text_color(t.text)
                                        .child(div().w_full().max_w(theme::READ_WIDTH).whitespace_normal().child(SharedString::from(text.to_owned())))
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
                            .gap(theme::space(theme::density(cx), Step::Sm))
                            .px(theme::space(theme::density(cx), Step::Lg))
                            .py(theme::space(theme::density(cx), Step::Md))
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
                .text_size(theme::font(theme::density(cx), TypeSize::Caption))
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

    fn graph_column(&self, run: &Run, driver: &mut motion::Driver, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let summary = (run.status == RunStatus::Done).then(|| Summary::of(run));
        let hidden = self.hidden_summaries.contains(&run.session);
        let session = run.session.clone();
        column(self.mesh.clone())
            .relative()
            .map(|this| match &summary {
                Some(summary) if !hidden => this.child(self.summary(run, summary, driver, cx)).into_any_element(),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let entity = self.state.clone();
        let state = entity.read(cx);
        let failures = required_failures(&state.checks);
        let panel = panel_view(&panel_state(&state.feeds.runs, state.runs.is_empty()));
        let mut driver = motion::Driver::new(state.reduce_motion, cx);
        let Some(run) = state.run() else {
            let result = match panel {
                PanelView::Error { message, keep_content: true } => v_flex().size_full().child(self.retry_note(message, cx)).child(self.no_run(state.runs.is_empty(), &failures, cx)).into_any_element(),
                PanelView::Skeleton => ornament::empty_state(&t, tr(cx, TEXT, "Reading your wishes…").into(), "".into()).size_full().into_any_element(),
                PanelView::Error { message, .. } => self.retry_note(message, cx).into_any_element(),
                _ => self.no_run(state.runs.is_empty(), &failures, cx).into_any_element(),
            };
            driver.finish(window);
            return result;
        };
        if let PanelView::Error { message, keep_content: false } = &panel {
            let result = self.retry_note(message.clone(), cx).into_any_element();
            driver.finish(window);
            return result;
        }
        if self.viewing.as_ref() != Some(&run.session) {
            self.viewing = Some(run.session.clone());
            self.wish_open = false;
            self.report_open = false;
        }
        let animate = run.status == RunStatus::Active && !state.reduce_motion;
        let d = theme::density(cx);
        let heights = chrome_heights(d);
        for (index, unit) in run.units.iter().enumerate() {
            let key: SharedString = format!("sel:{}", unit.id).into();
            self.glows.set(&key, if state.selection == Selection::Unit(index) { 1.0 } else { 0.0 }, motion::now(cx), state.reduce_motion);
        }
        if let Some(target) = focus_target(settings::Store::current(cx).mode) {
            let focused = match target {
                settings::FocusPanel::Graph => self.graph_column(run, &mut driver, cx).into_any_element(),
                settings::FocusPanel::Checklist => column(self.todo.clone()).bg(t.surface).into_any_element(),
            };
            let result = v_flex().size_full()
                .when(needs_you(run), |this| this.child(div().flex_none().h(heights.brief).overflow_hidden().child(self.brief(run, cx))))
                .child(h_flex().w_full().flex_none().justify_end()
                    .px(theme::space(d, Step::Lg)).py(theme::space(d, Step::Sm))
                    .child(theme::gold_leaf(Button::new("exit-focus"), &t)
                        .icon(IconName::ArrowLeft)
                        .label(tr(cx, TEXT, "Exit focus"))
                        .tooltip(tr(cx, TEXT, "Back to the whole window (Ctrl+Shift+F)"))
                        .on_click(|_, _, cx| { settings::set_mode(settings::WindowMode::Full, cx); })))
                .child(div().flex_1().min_h_0().w_full().child(focused))
                .into_any_element();
            driver.finish(window);
            return result;
        }
        let available = f32::from(window.viewport_size().width);
        if !self.restored && (!available.is_finite() || available <= 0.) {
            driver.finish(window);
            return div().size_full().into_any_element();
        }
        let widths = restored_widths(settings::columns(cx), available);
        self.restored = true;
        let graph = self.graph_column(run, &mut driver, cx).into_any_element();
        let details = self.details_column(run, &state.selection, &mut driver, cx).into_any_element();
        let result = v_flex()
            .size_full()
            .child(div().flex_none().h(heights.brief).overflow_hidden().child(self.brief(run, cx)))
            .when_some(match panel { PanelView::Error { message, keep_content: true } => Some(message), _ => None }, |this, message| this.child(self.retry_note(message, cx)))
            .when_some(self.first_day_card(cx), |this, card| {
                this.child(div().flex_none().w_full().px(theme::space(theme::density(cx), Step::Xl)).pt(theme::space(theme::density(cx), Step::Md)).child(card))
            })
            .when(!failures.is_empty(), |this| {
                this.child(div().flex_none().w_full().px(theme::space(theme::density(cx), Step::Xl)).py(theme::space(theme::density(cx), Step::Md)).child(self.setup_card(&failures, cx)))
            })
            .child(div().flex_none().h(heights.header).overflow_hidden().child(self.header(run, cx)))
            .child(div().flex_none().h(heights.rail).overflow_hidden().child(stage_rail(run, animate, cx)))
            .child(
                div().flex_1().min_h_0().w_full()
                    .key_context("RunColumns")
                    .track_focus(&self.columns_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let m = event.keystroke.modifiers;
                        if !m.alt || m.control || m.platform { return; }
                        let key = format!("alt-{}{}", if m.shift { "shift-" } else { "" }, event.keystroke.key);
                        match key.as_str() {
                            "alt-left" | "alt-right" | "alt-shift-left" | "alt-shift-right" | "alt-0" => {
                                let state = this.columns.read(cx);
                                let sizes = state.sizes();
                                if sizes.len() != 3 { return; }
                                let current = settings::ColumnWidths { checklist: f32::from(sizes[0]), details: f32::from(sizes[2]) };
                                let available = f32::from(state.container_size());
                                let Some(next) = step_widths(current, &key, available) else { return };
                                this.columns.update(cx, |state, cx| {
                                    // Free space before growing either side when resetting both.
                                    if next.checklist < current.checklist { state.resize_panel(0, px(next.checklist), window, cx); }
                                    if next.details < current.details { state.resize_panel(2, px(next.details), window, cx); }
                                    if next.checklist > current.checklist { state.resize_panel(0, px(next.checklist), window, cx); }
                                    if next.details > current.details { state.resize_panel(2, px(next.details), window, cx); }
                                });
                                let sizes = this.columns.read(cx).sizes();
                                let actual = settings::ColumnWidths { checklist: f32::from(sizes[0]), details: f32::from(sizes[2]) };
                                settings::remember_columns(actual, cx);
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(
                    h_resizable("run-columns")
                        .with_state(&self.columns)
                        .child(
                            resizable_panel()
                                .size(px(widths.checklist)).fixed()
                                .size_range(CHECKLIST_RANGE)
                                .child(column(self.todo.clone()).bg(t.surface)),
                        )
                        .child(
                            resizable_panel()
                                .size_range(GRAPH_MIN..Pixels::MAX)
                                .child(graph),
                        )
                        .child(
                            resizable_panel()
                                .size(px(widths.details)).fixed()
                                .size_range(DETAILS_RANGE)
                                .child(details),
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
            .into_any_element();
        if state.view == Screen::Run && !self.columns_shown {
            self.columns_shown = true;
            // Wait for the focus tree, once per entry; inputs and overlays keep the keyboard.
            cx.defer_in(window, |this, window, cx| {
                if this.state.read(cx).view != Screen::Run
                    || this.state.read(cx).composing
                    || focus_target(settings::Store::current(cx).mode).is_some()
                    || window.has_active_dialog(cx) || window.has_active_sheet(cx) { return; }
                let idle_focus = window.context_stack().iter().all(|context| {
                    context.primary().is_none_or(|entry| matches!(entry.key.as_ref(), "Root" | "ChronosShell" | "RunColumns"))
                });
                if idle_focus { this.columns_focus.focus(window, cx); }
            });
        }
        driver.finish(window);
        result
    }
}

#[cfg(test)]
mod accept;

#[cfg(test)]
mod tests {
    use super::super::demo::sample_run;
    use super::super::i18n::COMMON;
    use super::super::model::*;
    use super::{
        beginning, cost_text, fix_hint, one_line, required_failures, stage_key, unpriced_models, without_fix, FullTest,
        StageState, Summary, Verdict, FULL_TEST, STAGE_NAMES, TEXT,
    };
    use super::super::i18n::Lang;
    use super::{brief_of, Brief, FirstDay, Tone};
    use super::{attention_title, blocked_brief_title, plain_text::Plain};
    use gpui_kit::SharedString;

    #[test]
    fn production_code_has_no_uncapped_spinner() {
        let source = include_str!("run_view.rs").split_once("#[cfg(test)]").unwrap().0;
        assert!(source.contains("fn details_column"));
        assert!(!source.contains("Spinner::new"));
        assert!(!source.contains("component::spinner"));
    }

    #[test]
    fn unpriced_list_uses_model_price_not_missing_usage() {
        let mut usage = crate::desktop::demo::state().runs[0].usage.clone();
        let terra = usage.models.iter_mut().find(|m| m.model.as_ref() == "gpt-6.1-terra").unwrap();
        terra.usd = None;
        terra.cached = None;
        assert_eq!(unpriced_models(&usage), usage.unpriced());
        assert_eq!(unpriced_models(&usage), vec![SharedString::from("gpt-unknown")]);
    }

    #[test]
    fn plain_titles_supply_attention_and_blocked_text_with_original_tooltips() {
        let title = Plain { shown: "Plain\n title".into(), original: Some("Technical title".into()) };
        let (shown, tooltip) = attention_title(title.clone());
        assert_eq!(shown.as_ref(), "Plain\n title");
        assert_eq!(tooltip.as_ref(), "Technical title");
        let (arg, original) = blocked_brief_title(title);
        assert_eq!(arg, "Plain");
        assert_eq!(original.as_deref(), Some("Technical title"));
    }

    #[test]
    fn untranslated_titles_keep_attention_and_blocked_fallbacks() {
        let title = Plain { shown: "Untranslated title".into(), original: None };
        let (shown, tooltip) = attention_title(title.clone());
        assert_eq!(shown.as_ref(), "Untranslated title");
        assert_eq!(tooltip.as_ref(), "Untranslated title");
        let (arg, original) = blocked_brief_title(title);
        assert_eq!(arg, "Untranslated title");
        assert_eq!(original, None);
    }

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
        assert_eq!(unpriced_models(&usage), vec![SharedString::from("gpt-unknown")]);
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


    fn running_run(running_units: usize) -> Run {
        let mut run = sample_run();
        run.status = RunStatus::Active;
        for unit in &mut run.units {
            unit.state = UnitState::Pending;
        }
        for unit in run.units.iter_mut().take(running_units) {
            unit.state = UnitState::Running;
        }
        run
    }

    #[test]
    fn building_units_is_said_with_the_real_count_and_no_action_asked() {
        let brief = brief_of(&running_run(2));
        assert_eq!(brief.tone, Tone::Working);
        assert_eq!(brief.text(Lang::ZhTw), "正在建造 2 個單元，暫時不需要你動手；建好後會逐項驗收。");
        assert!(brief.text(Lang::En).starts_with("Building 2 units."));
        let one = brief_of(&running_run(1));
        assert!(one.text(Lang::En).starts_with("Building 1 unit."), "{}", one.text(Lang::En));
    }

    #[test]
    fn with_no_unit_running_the_running_stage_decides_the_sentence() {
        let mut run = running_run(0);
        for stage in &mut run.stages {
            stage.phase = Phase::Waiting;
        }
        run.stages.iter_mut().find(|s| s.name == "Astra").unwrap().phase = Phase::Running;
        assert!(brief_of(&run).text(Lang::En).starts_with("Astra is planning."));
        run.stages.iter_mut().find(|s| s.name == "Astra").unwrap().phase = Phase::Done;
        run.stages.iter_mut().find(|s| s.name == "Full test").unwrap().phase = Phase::Running;
        assert!(brief_of(&run).text(Lang::En).starts_with("Running the full test."));
        for stage in &mut run.stages {
            stage.phase = Phase::Waiting;
        }
        assert!(brief_of(&run).text(Lang::En).starts_with("Chronos is working."), "unknown stage says no more than that");
        run.units.clear();
        assert!(brief_of(&run).text(Lang::En).starts_with("Chronos is starting."));
    }

    #[test]
    fn a_blocked_run_says_it_needs_the_person_and_quotes_its_own_note() {
        let mut run = sample_run();
        run.status = RunStatus::Blocked;
        run.needs = vec![Need { kind: NeedKind::Blocked, title: "A credential is needed".into(), note: "x".into() }];
        let brief = brief_of(&run);
        assert_eq!(brief.tone, Tone::NeedsYou);
        assert_eq!(brief.args, vec!["A credential is needed".to_owned()]);
        assert!(!brief.text(Lang::En).contains("Nothing needs you"));
        run.needs.clear();
        let bare = brief_of(&run);
        assert_eq!(bare.tone, Tone::NeedsYou);
        assert!(bare.args.is_empty());
    }

    #[test]
    fn waiting_and_interrupted_runs_are_told_apart() {
        let mut run = sample_run();
        run.status = RunStatus::Waiting;
        assert_eq!(brief_of(&run).tone, Tone::Waiting);
        run.status = RunStatus::LimitWait;
        assert!(brief_of(&run).text(Lang::En).contains("usage limit"));
        run.status = RunStatus::Interrupted;
        assert_eq!(brief_of(&run).tone, Tone::NeedsYou);
    }

    #[test]
    fn finished_is_claimed_only_when_everything_is_verified_and_the_test_passed() {
        let mut run = run_with(&[UnitState::Verified, UnitState::Verified]);
        let brief = brief_of(&run);
        assert_eq!(brief.tone, Tone::Finished);
        assert_eq!(brief.args, vec!["2".to_owned()]);
        set_full_test(&mut run, Phase::Failed);
        assert_eq!(brief_of(&run).tone, Tone::Unsettled);
        assert!(brief_of(&run).text(Lang::En).contains("full test failed"));
        let mut partial = run_with(&[UnitState::Verified, UnitState::Failed, UnitState::Built]);
        let brief = brief_of(&partial);
        assert_eq!(brief.tone, Tone::Unsettled);
        assert_eq!(brief.args, vec!["2".to_owned()]);
        partial.units.clear();
        assert_eq!(brief_of(&partial).tone, Tone::Unsettled, "no units is never finished");
    }

    #[test]
    fn every_sentence_the_brief_can_say_is_translated() {
        let mut briefs: Vec<Brief> = vec![];
        let mut run = running_run(1);
        briefs.push(brief_of(&run));
        run.units[1].state = UnitState::Running;
        briefs.push(brief_of(&run));
        for name in ["Astra", "Critic", "Architect", "Build", "Full test", "Memory", "Report", "none"] {
            let mut r = running_run(0);
            for stage in &mut r.stages {
                stage.phase = if stage.name == name { Phase::Running } else { Phase::Waiting };
            }
            briefs.push(brief_of(&r));
            r.units.clear();
            briefs.push(brief_of(&r));
        }
        for status in [RunStatus::Waiting, RunStatus::LimitWait, RunStatus::Interrupted, RunStatus::Blocked] {
            let mut r = sample_run();
            r.status = status;
            r.needs.clear();
            briefs.push(brief_of(&r));
            r.needs = vec![Need { kind: NeedKind::Blocked, title: "t".into(), note: String::new().into() }];
            briefs.push(brief_of(&r));
        }
        let verified = run_with(&[UnitState::Verified]);
        briefs.push(brief_of(&verified));
        let mut failed_test = verified.clone();
        set_full_test(&mut failed_test, Phase::Failed);
        briefs.push(brief_of(&failed_test));
        let mut not_run = verified.clone();
        set_full_test(&mut not_run, Phase::Waiting);
        briefs.push(brief_of(&not_run));
        briefs.push(brief_of(&run_with(&[UnitState::Failed])));
        briefs.push(brief_of(&run_with(&[UnitState::Failed, UnitState::Built])));
        briefs.push(brief_of(&run_with(&[])));
        for brief in briefs {
            let zh = TEXT.iter().find(|(en, _)| *en == brief.key);
            assert!(zh.is_some(), "no translation for {:?}", brief.key);
            assert_eq!(brief.key.matches("{}").count(), brief.args.len(), "{:?}", brief.key);
            assert_eq!(zh.unwrap().1.matches("{}").count(), brief.args.len(), "{:?}", brief.key);
            assert!(!brief.text(Lang::ZhTw).contains("{}"));
        }
    }

    fn ok_check(name: &str) -> Check {
        Check { name: name.to_owned().into(), status: CheckStatus::Ok, detail: "1.0".into() }
    }

    #[test]
    fn the_first_day_ticks_only_from_what_is_true() {
        let day = FirstDay::of(&[], &[], false);
        assert_eq!(day, FirstDay { tools: false, signed_in: false, project: false, wish: false });
        assert!(!day.complete());
        assert_eq!(day.done_count(), 0);

        let checks = [ok_check("claude"), ok_check("codex")];
        let day = FirstDay::of(&checks, &[], false);
        assert!(day.tools && day.signed_in && !day.project && !day.wish);
        assert_eq!(day.done_count(), 2);
    }

    #[test]
    fn a_missing_tool_and_a_signed_out_tool_untick_different_items() {
        let missing = Check {
            name: "codex".into(),
            status: CheckStatus::Fail,
            detail: "not on PATH. Install the Codex CLI and sign in.".into(),
        };
        let day = FirstDay::of(&[ok_check("claude"), missing], &[], false);
        assert!(!day.tools && !day.signed_in);

        let signed_out =
            Check { name: "codex".into(), status: CheckStatus::Fail, detail: "codex failed: not logged in".into() };
        let day = FirstDay::of(&[ok_check("claude"), signed_out], &[], false);
        assert!(day.tools, "found on PATH");
        assert!(!day.signed_in, "but failing");
        let day = FirstDay::of(&[ok_check("claude")], &[], false);
        assert!(!day.tools, "codex is not reported at all");
    }

    #[test]
    fn a_project_counts_from_the_recent_list_or_from_a_run_with_a_folder() {
        assert!(FirstDay::of(&[], &[], true).project);
        let mut run = sample_run();
        run.cwd = Some("E:/work".into());
        assert!(FirstDay::of(&[], &[run.clone()], false).project);
        run.cwd = None;
        assert!(!FirstDay::of(&[], &[run], false).project);
    }

    #[test]
    fn only_a_wish_that_ended_ticks_the_last_item_and_all_four_complete_the_list() {
        let checks = [ok_check("claude"), ok_check("codex")];
        let mut run = sample_run();
        run.cwd = Some("E:/work".into());
        for status in [RunStatus::Active, RunStatus::Waiting, RunStatus::Blocked, RunStatus::Interrupted] {
            run.status = status;
            assert!(!FirstDay::of(&checks, &[run.clone()], true).wish, "{status:?}");
        }
        run.status = RunStatus::Done;
        let day = FirstDay::of(&checks, &[run], false);
        assert!(day.wish && day.complete());
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
