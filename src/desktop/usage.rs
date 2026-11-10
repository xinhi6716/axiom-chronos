//! The Usage screen: what the work cost, which agent carried too much, and what to do about it.
use super::i18n::{self, tr};
use super::model::*;
use super::theme::{Tokened as _, Tokens};
use gpui_kit::base::{
    Table as BaseTable, TableBody as BaseBody, TableCell as BaseCell, TableHead as BaseHead,
    TableHeader as BaseHeader, TableRow as BaseRow,
};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, Icon, IconName, Sizable as _, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::BTreeMap;

const TEXT: i18n::Table = &[
    ("This run", "這次執行"),
    ("All runs", "全部執行"),
    ("1 run", "共 1 次執行"),
    ("{n} runs", "共 {n} 次執行"),
    (
        "Dollar figures are API-price equivalents. Subscription plans are not billed these amounts.",
        "金額是按 API 定價換算的等值金額，訂閱方案不會被收這筆錢。",
    ),
    ("Cost at API prices", "API 定價換算花費"),
    ("Input", "輸入"),
    ("Cached", "快取讀取"),
    ("Output", "輸出"),
    ("Peak context", "上下文高峰"),
    ("{missing} of {runs} runs not reported", "{runs} 次中有 {missing} 次沒有金額"),
    ("Excludes Codex", "不含 Codex"),
    ("{ratio}× the {limit} line", "警戒線 {limit} 的 {ratio} 倍"),
    ("Under the {limit} line", "在警戒線 {limit} 以下"),
    ("Peak context per agent", "各 agent 的上下文高峰"),
    ("Warning line {limit}", "警戒線 {limit}"),
    (
        "Each bar is the largest context that agent carried. Past the line, every step re-reads too much.",
        "每條長度是該 agent 累積過的最大上下文；超過警戒線，代表每一步都要重讀太多內容。",
    ),
    ("No agent has reported its context size yet.", "還沒有 agent 回報上下文大小。"),
    ("{tokens} tokens at its largest", "最大時 {tokens} token"),
    ("Highest in {project}", "最高出現在 {project}"),
    ("Over the line in {over} of {seen} runs", "{seen} 次中有 {over} 次超過警戒線"),
    ("Warnings", "警告"),
    ("Nothing to fix. Every agent stayed under the line.", "沒有需要處理的事，每個 agent 都在警戒線內。"),
    ("Nothing to check yet.", "還沒有可檢查的資料。"),
    ("{agent}: context too large", "{agent}：上下文過大"),
    (
        "It reached {peak} tokens, {ratio} times the {limit} line. Every step re-reads all of it, so each step costs more than the last.",
        "累積到 {peak} token，是警戒線 {limit} 的 {ratio} 倍。每一步都會重讀整份上下文，所以越往後越貴。",
    ),
    ("Over the line in {over} of {seen} runs, so this is a pattern.", "{seen} 次中有 {over} 次超過，不是偶發。"),
    (
        "**What to do:** split the wish into smaller parts so a new agent takes over each one, and have it search large files instead of reading them whole.",
        "**建議：**把願望拆成幾個較小的部分，讓每個部分由新的 agent 接手；大檔案用搜尋的，不要整份讀進來。",
    ),
    ("Cost by model", "各模型花費"),
    ("Cost by role", "各角色花費"),
    ("Model", "模型"),
    ("Role", "角色"),
    ("API cost", "API 花費"),
    ("Tokens only", "只有 token"),
    (
        "Codex reports tokens but no dollars, so its rows show “—” and are not in the total.",
        "Codex 只回報 token、沒有金額，所以這些列顯示「—」，也不計入總額。",
    ),
    ("No model has reported usage yet.", "還沒有模型回報用量。"),
    ("Runs", "各次執行"),
    ("Newest first. Click a run to open it.", "最新的在最上面，點一列可開啟該次執行。"),
    ("Status", "狀態"),
    ("Project", "專案"),
    ("Wish", "願望"),
    ("Tokens", "Token"),
    ("Started", "開始時間"),
    ("Over the warning line", "超過警戒線"),
    ("No usage yet", "還沒有用量資料"),
    (
        "Cost and token figures appear here once a wish has run.",
        "許下願望並開始執行後，花費和 token 用量就會出現在這裡。",
    ),
    ("Make a wish", "許願"),
];

/// An agent past this many tokens of context is flagged: every step re-reads all of it.
const CONTEXT_WARNING_TOKENS: u64 = crate::usage::CONTEXT_WARN;

const UNKNOWN: &str = "—";

fn over_line(peak: u64) -> bool {
    peak > CONTEXT_WARNING_TOKENS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    ThisRun,
    AllRuns,
}

/// Dollars added up from figures that may be missing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Dollars {
    /// `None` when no part reported a figure, so the sum is unknown rather than zero.
    known: Option<f64>,
    /// Parts that reported nothing. With some known, the sum is a lower bound.
    missing: usize,
}

impl Dollars {
    fn add(&mut self, value: Option<f64>) {
        match value {
            Some(value) => self.known = Some(self.known.unwrap_or(0.0) + value),
            None => self.missing += 1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CostRow {
    name: SharedString,
    usd: Dollars,
    input: u64,
    cached: u64,
    output: u64,
    /// Some part did not report its tokens, so the counts are lower bounds.
    tokens_missing: bool,
}

impl CostRow {
    fn tokens(&self) -> u64 {
        self.input + self.cached + self.output
    }

    fn tokens_only(&self) -> bool {
        self.usd.known.is_none() && self.tokens() > 0
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ContextRow {
    agent: SharedString,
    peak: u64,
    /// The project of the run that reached the peak.
    project: SharedString,
    runs_seen: usize,
    runs_over: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Warning {
    ContextTooLarge { agent: SharedString, peak: u64, runs_seen: usize, runs_over: usize },
    /// A warning the run itself carries, shown as written.
    Reported { project: SharedString, text: SharedString },
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Summary {
    runs: usize,
    cost: Dollars,
    input: u64,
    cached: u64,
    output: u64,
    /// Some model did not report its tokens, so the counts are lower bounds.
    tokens_missing: bool,
    models: Vec<CostRow>,
    roles: Vec<CostRow>,
    contexts: Vec<ContextRow>,
    warnings: Vec<Warning>,
}

impl Summary {
    fn peak(&self) -> u64 {
        self.contexts.first().map_or(0, |row| row.peak)
    }
}

/// The run's reported total, or the dollars its models reported when it has none.
fn run_cost(usage: &Usage) -> Option<f64> {
    usage.total_usd.or_else(|| {
        let mut sum = Dollars::default();
        usage.models.iter().filter(|m| m.usd.is_some()).for_each(|m| sum.add(m.usd));
        sum.known
    })
}

fn add_cost(rows: &mut Vec<CostRow>, name: &SharedString, usd: Option<f64>, tokens: [Option<u64>; 3]) {
    let at = rows.iter().position(|row| row.name == *name).unwrap_or_else(|| {
        rows.push(CostRow { name: name.clone(), ..Default::default() });
        rows.len() - 1
    });
    let row = &mut rows[at];
    row.usd.add(usd);
    row.input += tokens[0].unwrap_or(0);
    row.cached += tokens[1].unwrap_or(0);
    row.output += tokens[2].unwrap_or(0);
    row.tokens_missing |= tokens.iter().any(Option::is_none);
}

/// Largest spend first, then rows with tokens but no dollars, largest first.
fn sort_costs(rows: &mut [CostRow]) {
    rows.sort_by(|a, b| {
        let by_spend = match (a.usd.known, b.usd.known) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.tokens().cmp(&a.tokens()),
        };
        by_spend.then_with(|| a.name.cmp(&b.name))
    });
}

/// Adds up the given runs. Callers pass one run for "This run" and all of them for "All runs".
fn summarize(runs: &[Run]) -> Summary {
    let mut summary = Summary { runs: runs.len(), ..Default::default() };
    for run in runs {
        let usage = &run.usage;
        summary.cost.add(run_cost(usage));
        for model in &usage.models {
            add_cost(&mut summary.models, &model.model, model.usd, [model.input, model.cached, model.output]);
            summary.input += model.input.unwrap_or(0);
            summary.cached += model.cached.unwrap_or(0);
            summary.output += model.output.unwrap_or(0);
            summary.tokens_missing |= [model.input, model.cached, model.output].iter().any(Option::is_none);
        }
        for role in &usage.roles {
            add_cost(&mut summary.roles, &role.role, role.usd, [Some(role.input), Some(role.cached), Some(role.output)]);
        }
        let mut peaks: BTreeMap<&SharedString, u64> = BTreeMap::new();
        for context in usage.contexts.iter().filter(|context| context.peak_tokens > 0) {
            let peak = peaks.entry(&context.agent).or_default();
            *peak = (*peak).max(context.peak_tokens);
        }
        for (agent, peak) in peaks {
            let at = summary.contexts.iter().position(|row| row.agent == *agent).unwrap_or_else(|| {
                summary.contexts.push(ContextRow {
                    agent: agent.clone(),
                    peak: 0,
                    project: run.project.clone(),
                    runs_seen: 0,
                    runs_over: 0,
                });
                summary.contexts.len() - 1
            });
            let row = &mut summary.contexts[at];
            row.runs_seen += 1;
            row.runs_over += usize::from(over_line(peak));
            if peak > row.peak {
                row.peak = peak;
                row.project = run.project.clone();
            }
        }
    }
    sort_costs(&mut summary.models);
    sort_costs(&mut summary.roles);
    summary.contexts.sort_by(|a, b| b.peak.cmp(&a.peak).then_with(|| a.agent.cmp(&b.agent)));
    summary.warnings = summary
        .contexts
        .iter()
        .filter(|row| over_line(row.peak))
        .map(|row| Warning::ContextTooLarge {
            agent: row.agent.clone(),
            peak: row.peak,
            runs_seen: row.runs_seen,
            runs_over: row.runs_over,
        })
        .collect();
    for run in runs {
        summary.warnings.extend(
            run.warnings
                .iter()
                .map(|text| Warning::Reported { project: run.project.clone(), text: text.clone() }),
        );
    }
    summary
}

#[derive(Clone, Debug, PartialEq)]
struct RunRow {
    /// Position in `AppState::runs`, for opening the run.
    index: usize,
    status: RunStatus,
    label: &'static str,
    verified_end: bool,
    project: SharedString,
    wish: SharedString,
    started: SharedString,
    started_at: Option<u64>,
    cost: Option<f64>,
    tokens: u64,
    peak: u64,
}

/// One row per run, newest first. Runs with no time keep their order after the rest.
fn run_rows(runs: &[Run]) -> Vec<RunRow> {
    let mut rows: Vec<RunRow> = runs
        .iter()
        .enumerate()
        .map(|(index, run)| RunRow {
            index,
            status: run.status,
            label: run.status_label(),
            verified_end: run.verified_end(),
            project: run.project.clone(),
            wish: run.wish.clone(),
            started: run.started.clone(),
            started_at: run.started_at,
            cost: run_cost(&run.usage),
            tokens: run.usage.models.iter().flat_map(|m| [m.input, m.cached, m.output]).flatten().sum(),
            peak: run.usage.contexts.iter().map(|c| c.peak_tokens).max().unwrap_or(0),
        })
        .collect();
    rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    rows
}

/// The value a full bar stands for, always leaving the warning line inside the chart.
fn chart_scale(rows: &[ContextRow]) -> u64 {
    rows.iter()
        .map(|row| row.peak)
        .max()
        .unwrap_or(0)
        .max(CONTEXT_WARNING_TOKENS + CONTEXT_WARNING_TOKENS / 4)
}

fn with_commas(value: u64) -> String {
    let mut text = value.to_string();
    let mut comma = text.len();
    while comma > 3 {
        comma -= 3;
        text.insert(comma, ',');
    }
    text
}

fn fmt_usd(value: f64) -> String {
    if value > 0.0 && value < 0.005 {
        return "<$0.01".into();
    }
    let cents = (value * 100.0).round() as u64;
    format!("${}.{:02}", with_commas(cents / 100), cents % 100)
}

fn fmt_cost(cost: Option<f64>) -> String {
    cost.map_or_else(|| UNKNOWN.into(), fmt_usd)
}

fn fmt_dollars(dollars: Dollars) -> String {
    match dollars.known {
        Some(known) if dollars.missing > 0 => format!("{}+", fmt_usd(known)),
        known => fmt_cost(known),
    }
}

/// A sum that is missing some parts is a lower bound, marked the way dollars are.
fn fmt_sum(tokens: u64, missing: bool) -> String {
    if missing && tokens > 0 {
        format!("{}+", fmt_tokens(tokens))
    } else {
        fmt_tokens(tokens)
    }
}

/// Token counts in three significant digits. Zero means nothing was reported, so it shows as unknown.
fn fmt_tokens(tokens: u64) -> String {
    if tokens == 0 {
        return UNKNOWN.into();
    }
    let value = tokens as f64;
    let (scaled, unit) = if value >= 999_500_000.0 {
        (value / 1e9, "B")
    } else if value >= 999_500.0 {
        (value / 1e6, "M")
    } else if value >= 1_000.0 {
        (value / 1e3, "k")
    } else {
        return tokens.to_string();
    };
    if scaled < 99.95 {
        format!("{scaled:.1}{unit}")
    } else {
        format!("{scaled:.0}{unit}")
    }
}

fn fmt_ratio(peak: u64) -> String {
    format!("{:.1}", peak as f64 / CONTEXT_WARNING_TOKENS as f64)
}

fn fmt_limit() -> String {
    fmt_tokens(CONTEXT_WARNING_TOKENS)
}

/// A table column: a fixed width, or the room left over.
#[derive(Clone, Copy)]
struct Col {
    width: Option<f32>,
    right: bool,
}

const fn grow() -> Col {
    Col { width: None, right: false }
}

const fn left(width: f32) -> Col {
    Col { width: Some(width), right: false }
}

const fn right(width: f32) -> Col {
    Col { width: Some(width), right: true }
}

const NUMBER_COLS: [Col; 4] = [right(80.), right(60.), right(60.), right(60.)];
const RUN_COLS: [Col; 7] = [left(144.), left(120.), grow(), right(88.), right(72.), right(104.), right(104.)];

fn sized<E: Styled>(element: E, col: Col) -> E {
    let element = element.flex().items_center().overflow_hidden().px_2().text_size(px(13.));
    let element = match col.width {
        Some(width) => element.w(px(width)).flex_none(),
        None => element.flex_1().min_w_0(),
    };
    if col.right {
        element.justify_end()
    } else {
        element
    }
}

fn header_row(t: Tokens, cols: &[Col], labels: &[&'static str], cx: &App) -> BaseHeader {
    BaseHeader::new("head").child(
        BaseRow::new("row", 1)
            .flex()
            .flex_row()
            .h(px(30.))
            .border_b_1()
            .border_color(t.border)
            .text_color(t.muted)
            .children(cols.iter().zip(labels).enumerate().map(|(ix, (col, label))| {
                sized(BaseHead::new(("th", ix), ix + 1), *col).text_xs().child(tr(cx, TEXT, label))
            })),
    )
}

fn data_cell(ix: usize, col: Col, child: impl IntoElement) -> BaseCell {
    sized(BaseCell::new(("td", ix), ix + 1), col).child(child)
}

fn tooltip_text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

fn panel(t: Tokens) -> Div {
    v_flex()
        .min_w_0()
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
}

fn panel_title(t: Tokens, text: &str) -> Div {
    div().text_sm().font_semibold().text_color(t.text).child(text.to_owned())
}

fn tile(t: Tokens, label: &str, value: String, color: Hsla, note: Option<(String, Hsla)>) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .px_4()
        .py_3()
        .gap_0p5()
        .child(div().text_xs().text_color(t.muted).child(label.to_owned()))
        .child(div().text_size(px(22.)).font_semibold().text_color(color).child(value))
        .child(
            div()
                .h(px(16.))
                .truncate()
                .text_xs()
                .text_color(note.as_ref().map_or(t.muted, |(_, color)| *color))
                .child(note.map_or(String::new(), |(text, _)| text)),
        )
}

fn totals(summary: &Summary, cx: &App) -> Div {
    let t = cx.tokens();
    let limit = fmt_limit();
    let peak = summary.peak();
    let cost_note = if summary.cost.known.is_some() && summary.cost.missing > 0 {
        Some(
            tr(cx, TEXT, "{missing} of {runs} runs not reported")
                .replace("{missing}", &summary.cost.missing.to_string())
                .replace("{runs}", &summary.runs.to_string()),
        )
    } else if summary.models.iter().any(CostRow::tokens_only) {
        Some(tr(cx, TEXT, "Excludes Codex").to_owned())
    } else {
        None
    };
    let peak_note = (peak > 0).then(|| {
        if over_line(peak) {
            (
                tr(cx, TEXT, "{ratio}× the {limit} line")
                    .replace("{ratio}", &fmt_ratio(peak))
                    .replace("{limit}", &limit),
                t.fail,
            )
        } else {
            (tr(cx, TEXT, "Under the {limit} line").replace("{limit}", &limit), t.muted)
        }
    });
    let tiles = [
        tile(t, tr(cx, TEXT, "Cost at API prices"), fmt_dollars(summary.cost), t.gold, cost_note.map(|note| (note, t.muted))),
        tile(t, tr(cx, TEXT, "Input"), fmt_sum(summary.input, summary.tokens_missing), t.text, None),
        tile(t, tr(cx, TEXT, "Cached"), fmt_sum(summary.cached, summary.tokens_missing), t.text, None),
        tile(t, tr(cx, TEXT, "Output"), fmt_sum(summary.output, summary.tokens_missing), t.text, None),
        tile(
            t,
            tr(cx, TEXT, "Peak context"),
            fmt_tokens(peak),
            if over_line(peak) { t.fail } else { t.text },
            peak_note,
        ),
    ];
    v_flex()
        .rounded(px(10.))
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .child(h_flex().children(tiles.into_iter().enumerate().map(|(ix, tile)| {
            tile.when(ix > 0, |tile| tile.border_l_1().border_color(t.border))
        })))
        .child(
            h_flex()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(t.border)
                .text_xs()
                .text_color(t.muted)
                .child(Icon::new(IconName::Info).size(px(14.)).flex_shrink_0())
                .child(tr(
                    cx,
                    TEXT,
                    "Dollar figures are API-price equivalents. Subscription plans are not billed these amounts.",
                )),
        )
}

const CHART_NAME: f32 = 152.;
const CHART_VALUE: f32 = 92.;
const CHART_GAP: f32 = 12.;
const CHART_ROW: f32 = 30.;
const CHART_LABEL: f32 = 20.;

fn context_tooltip(row: &ContextRow, all_runs: bool, cx: &App) -> String {
    let mut lines = vec![
        format!("{}: {}", row.agent, tr(cx, TEXT, "{tokens} tokens at its largest").replace("{tokens}", &with_commas(row.peak))),
    ];
    if all_runs && row.runs_seen > 1 {
        lines.push(tr(cx, TEXT, "Highest in {project}").replace("{project}", &row.project));
        lines.push(
            tr(cx, TEXT, "Over the line in {over} of {seen} runs")
                .replace("{over}", &row.runs_over.to_string())
                .replace("{seen}", &row.runs_seen.to_string()),
        );
    }
    lines.join("\n")
}

fn context_row(ix: usize, row: &ContextRow, scale: f32, all_runs: bool, cx: &App) -> Stateful<Div> {
    let t = cx.tokens();
    let over = over_line(row.peak);
    let share = |tokens: u64| relative(tokens as f32 / scale);
    h_flex()
        .id(("usage-context", ix))
        .h(px(CHART_ROW))
        .gap(px(CHART_GAP))
        .tooltip(tooltip_text(context_tooltip(row, all_runs, cx)))
        .child(
            div()
                .w(px(CHART_NAME))
                .flex_shrink_0()
                .truncate()
                .text_size(px(13.))
                .text_color(if over { t.text } else { t.muted })
                .child(row.agent.clone()),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(14.))
                .rounded(px(3.))
                .overflow_hidden()
                .bg(t.border.opacity(0.45))
                .child(div().absolute().top_0().bottom_0().left_0().w(share(row.peak.min(CONTEXT_WARNING_TOKENS))).bg(t.run))
                .when(over, |track| {
                    track.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(share(CONTEXT_WARNING_TOKENS))
                            .w(share(row.peak - CONTEXT_WARNING_TOKENS))
                            .bg(t.fail),
                    )
                }),
        )
        .child(
            h_flex()
                .w(px(CHART_VALUE))
                .flex_shrink_0()
                .justify_end()
                .gap_1()
                .text_size(px(13.))
                .text_color(if over { t.fail } else { t.muted })
                .when(over, |value| value.child(Icon::new(IconName::TriangleAlert).size(px(13.))))
                .child(fmt_tokens(row.peak))
                .when(over, |value| {
                    value.child(div().text_xs().child(format!("{}×", fmt_ratio(row.peak))))
                }),
        )
}

fn context_panel(summary: &Summary, all_runs: bool, cx: &App) -> Div {
    let t = cx.tokens();
    let title = tr(cx, TEXT, "Peak context per agent");
    if summary.contexts.is_empty() {
        return panel(t)
            .child(panel_title(t, title))
            .child(div().text_sm().text_color(t.muted).child(tr(cx, TEXT, "No agent has reported its context size yet.")));
    }
    let scale = chart_scale(&summary.contexts) as f32;
    let line = CONTEXT_WARNING_TOKENS as f32 / scale;
    let label = tr(cx, TEXT, "Warning line {limit}").replace("{limit}", &fmt_limit());
    let caption = tr(
        cx,
        TEXT,
        "Each bar is the largest context that agent carried. Past the line, every step re-reads too much.",
    );
    panel(t)
        .child(
            h_flex()
                .justify_between()
                .gap_4()
                .child(panel_title(t, title).flex_none())
                .child(div().min_w_0().truncate().text_xs().text_color(t.muted).child(caption)),
        )
        .child(
        div()
            .relative()
            .pt(px(CHART_LABEL))
            .children(summary.contexts.iter().enumerate().map(|(ix, row)| context_row(ix, row, scale, all_runs, cx)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(CHART_NAME + CHART_GAP))
                    .right(px(CHART_VALUE + CHART_GAP))
                    .child(div().absolute().top(px(CHART_LABEL)).bottom_0().left(relative(line)).w(px(2.)).ml(px(-1.)).bg(t.review))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .h(px(CHART_LABEL))
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(t.review)
                            .map(|label| if line > 0.6 { label.right(relative(1. - line)).pr_1() } else { label.left(relative(line)).pl_1() })
                            .child(label),
                    ),
            ),
    )
}

fn warning_alert(ix: usize, warning: &Warning, all_runs: bool, cx: &App) -> Alert {
    let t = cx.tokens();
    let icon = Icon::new(IconName::TriangleAlert).text_color(t.review);
    match warning {
        Warning::ContextTooLarge { agent, peak, runs_seen, runs_over } => {
            let mut why = tr(
                cx,
                TEXT,
                "It reached {peak} tokens, {ratio} times the {limit} line. Every step re-reads all of it, so each step costs more than the last.",
            )
            .replace("{peak}", &fmt_tokens(*peak))
            .replace("{ratio}", &fmt_ratio(*peak))
            .replace("{limit}", &fmt_limit());
            if all_runs && *runs_over > 1 {
                why.push(' ');
                why.push_str(
                    &tr(cx, TEXT, "Over the line in {over} of {seen} runs, so this is a pattern.")
                        .replace("{over}", &runs_over.to_string())
                        .replace("{seen}", &runs_seen.to_string()),
                );
            }
            let body = format!(
                "{why}\n\n{}",
                tr(
                    cx,
                    TEXT,
                    "**What to do:** split the wish into smaller parts so a new agent takes over each one, and have it search large files instead of reading them whole.",
                ),
            );
            Alert::warning(("usage-warning", ix), TextView::markdown(("usage-warning-text", ix), body))
                .title(tr(cx, TEXT, "{agent}: context too large").replace("{agent}", agent))
                .text_color(t.text)
                .icon(icon)
        }
        Warning::Reported { project, text } => {
            let message = if all_runs { format!("{project}: {text}") } else { text.to_string() };
            Alert::warning(("usage-warning", ix), message).text_color(t.text).icon(icon)
        }
    }
}

fn warnings_panel(summary: &Summary, all_runs: bool, cx: &App) -> Div {
    let t = cx.tokens();
    panel(t)
        .child(panel_title(t, tr(cx, TEXT, "Warnings")))
        .when(summary.warnings.is_empty(), |panel| {
            panel.child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .text_sm()
                    .text_color(t.muted)
                    .when(!summary.contexts.is_empty(), |line| {
                        line.child(Icon::new(IconName::CircleCheck).size(px(16.)).text_color(t.ok).mt(px(2.)).flex_shrink_0())
                    })
                    .child(tr(
                        cx,
                        TEXT,
                        if summary.contexts.is_empty() {
                            "Nothing to check yet."
                        } else {
                            "Nothing to fix. Every agent stayed under the line."
                        },
                    )),
            )
        })
        .child(div().flex().flex_wrap().gap_3().children(summary.warnings.iter().enumerate().map(|(ix, warning)| {
            div().flex().flex_1().min_w(px(360.)).child(warning_alert(ix, warning, all_runs, cx))
        })))
}

fn cost_table(id: &'static str, name_label: &'static str, rows: &[CostRow], cx: &App) -> BaseTable {
    let t = cx.tokens();
    let mut cols = vec![grow()];
    cols.extend(NUMBER_COLS);
    BaseTable::new(id)
        .row_count(rows.len() + 1)
        .column_count(cols.len())
        .child(header_row(t, &cols, &[name_label, "API cost", "Input", "Cached", "Output"], cx))
        .child(BaseBody::new("body").children(rows.iter().enumerate().map(|(ix, row)| {
            BaseRow::new(("tr", ix), ix + 2)
                .flex()
                .flex_row()
                .h(px(34.))
                .when(ix > 0, |this| this.border_t_1().border_color(t.border.opacity(0.6)))
                .child(data_cell(
                    0,
                    cols[0],
                    h_flex()
                        .id(("name", ix))
                        .gap_2()
                        .min_w_0()
                        .tooltip(tooltip_text(row.name.clone()))
                        .child(div().min_w_0().truncate().child(row.name.clone()))
                        .when(row.tokens_only(), |name| {
                            name.child(Tag::secondary().outline().small().flex_none().child(tr(cx, TEXT, "Tokens only")))
                        }),
                ))
                .child(data_cell(
                    1,
                    cols[1],
                    div()
                        .text_color(if row.usd.known.is_some() { t.text } else { t.muted })
                        .child(fmt_dollars(row.usd)),
                ))
                .child(data_cell(2, cols[2], div().text_color(t.muted).child(fmt_sum(row.input, row.tokens_missing))))
                .child(data_cell(3, cols[3], div().text_color(t.muted).child(fmt_sum(row.cached, row.tokens_missing))))
                .child(data_cell(4, cols[4], div().text_color(t.muted).child(fmt_tokens(row.output))))
        })))
}

fn cost_panel(id: &'static str, title: &'static str, name_label: &'static str, rows: &[CostRow], cx: &App) -> Div {
    let t = cx.tokens();
    panel(t)
        .flex_1()
        .child(panel_title(t, tr(cx, TEXT, title)))
        .map(|panel| {
            if rows.is_empty() {
                panel.child(div().text_sm().text_color(t.muted).child(tr(cx, TEXT, "No model has reported usage yet.")))
            } else {
                panel.child(cost_table(id, name_label, rows, cx))
            }
        })
        .when(rows.iter().any(CostRow::tokens_only), |panel| {
            panel.child(
                div().text_xs().text_color(t.muted).child(tr(
                    cx,
                    TEXT,
                    "Codex reports tokens but no dollars, so its rows show “—” and are not in the total.",
                )),
            )
        })
}

pub struct UsageView {
    state: Entity<AppState>,
    scope: Scope,
    row_focus: Vec<FocusHandle>,
}

impl UsageView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let scene = state.read(cx).scene.clone().unwrap_or_default();
        if scene.contains("usage-min") {
            window.resize(size(px(1100.), px(700.)));
        } else if scene.contains("usage-tall") {
            window.resize(size(px(1100.), px(1300.)));
        }
        if scene.contains("usage-blank") {
            state.update(cx, |state, _| {
                if let Some(run) = state.current.and_then(|index| state.runs.get_mut(index)) {
                    run.usage = Usage::default();
                }
            });
        }
        Self {
            state,
            scope: if scene.contains("usage-all") { Scope::AllRuns } else { Scope::ThisRun },
            row_focus: Vec::new(),
        }
    }

    fn scope_tabs(&self, scope: Scope, can_show_run: bool, cx: &mut Context<Self>) -> TabBar {
        TabBar::new("usage-scope")
            .segmented()
            .small()
            .selected_index(if scope == Scope::ThisRun { 0 } else { 1 })
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.scope = if *ix == 0 { Scope::ThisRun } else { Scope::AllRuns };
                cx.notify();
            }))
            .child(Tab::new().label(tr(cx, TEXT, "This run")).disabled(!can_show_run))
            .child(Tab::new().label(tr(cx, TEXT, "All runs")))
    }

    fn run_table(&self, rows: &[RunRow], current: Option<usize>, cx: &App) -> BaseTable {
        let t = cx.tokens();
        BaseTable::new("usage-runs")
            .row_count(rows.len() + 1)
            .column_count(RUN_COLS.len())
            .child(header_row(t, &RUN_COLS, &["Status", "Project", "Wish", "API cost", "Tokens", "Peak context", "Started"], cx))
            .child(BaseBody::new("body").children(rows.iter().zip(&self.row_focus).enumerate().map(|(ix, (row, focus))| {
                let state = self.state.clone();
                let index = row.index;
                let over = over_line(row.peak);
                let is_current = current == Some(index);
                BaseRow::new(("tr", ix), ix + 2)
                    .flex()
                    .flex_row()
                    .h(px(36.))
                    .cursor_pointer()
                    .track_focus(&focus.clone().tab_stop(true))
                    .border_l_2()
                    .border_color(if is_current { t.gold } else { transparent_black() })
                    .when(is_current, |this| this.bg(t.raised.opacity(0.6)))
                    .hover(|this| this.bg(t.raised))
                    .focus_visible(|this| this.bg(t.raised).border_color(t.gold))
                    .on_click(move |_, _, cx| state.update(cx, |state, cx| state.open_run(index, cx)))
                    .child(data_cell(
                        0,
                        RUN_COLS[0],
                        h_flex()
                            .min_w_0()
                            .gap_1p5()
                            .child(div().size_2().rounded_full().flex_shrink_0().bg(t.outcome(row.status, row.verified_end)))
                            .child(div().min_w_0().truncate().text_color(t.text).child(tr(cx, TEXT, row.label))),
                    ))
                    .child(data_cell(1, RUN_COLS[1], div().min_w_0().truncate().text_color(t.text).child(row.project.clone())))
                    .child(data_cell(
                        2,
                        RUN_COLS[2],
                        div()
                            .id(("wish", ix))
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted)
                            .tooltip(tooltip_text(row.wish.clone()))
                            .child(row.wish.clone()),
                    ))
                    .child(data_cell(3, RUN_COLS[3], div().text_color(t.text).child(fmt_cost(row.cost))))
                    .child(data_cell(4, RUN_COLS[4], div().text_color(t.muted).child(fmt_tokens(row.tokens))))
                    .child(data_cell(
                        5,
                        RUN_COLS[5],
                        h_flex()
                            .gap_1()
                            .text_color(if over { t.fail } else { t.muted })
                            .when(over, |peak| peak.child(Icon::new(IconName::TriangleAlert).size(px(13.))))
                            .child(fmt_tokens(row.peak)),
                    ))
                    .child(data_cell(6, RUN_COLS[6], div().min_w_0().truncate().text_color(t.muted).child(row.started.clone())))
            })))
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.clone();
        v_flex().size_full().items_center().justify_center().p_6().child(
            Empty::new()
                .header(
                    EmptyHeader::new()
                        .media(
                            EmptyMedia::new()
                                .with_variant(EmptyMediaVariant::Icon)
                                .child(Icon::new(IconName::ChartPie)),
                        )
                        .title(EmptyTitle::new().child(tr(cx, TEXT, "No usage yet")))
                        .description(EmptyDescription::new().child(tr(cx, TEXT, "Cost and token figures appear here once a wish has run."))),
                )
                .content(
                    EmptyContent::new().child(
                        Button::new("usage-make-wish")
                            .primary()
                            .label(tr(cx, TEXT, "Make a wish"))
                            .on_click(move |_, _, cx| state.update(cx, |state, cx| state.set_composing(true, cx))),
                    ),
                ),
        )
    }
}

impl Render for UsageView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let state = self.state.read(cx);
        if state.runs.is_empty() {
            return self.render_empty(cx).into_any_element();
        }
        let current = state.current.filter(|index| *index < state.runs.len());
        let scope = if current.is_some() { self.scope } else { Scope::AllRuns };
        let selected = match (scope, current) {
            (Scope::ThisRun, Some(index)) => &state.runs[index..=index],
            _ => &state.runs[..],
        };
        let summary = summarize(selected);
        let rows = run_rows(&state.runs);
        let subtitle: SharedString = match (scope, current) {
            (Scope::ThisRun, Some(index)) => format!("{} · {}", state.runs[index].project, state.runs[index].wish).into(),
            _ if state.runs.len() == 1 => tr(cx, TEXT, "1 run").into(),
            _ => tr(cx, TEXT, "{n} runs").replace("{n}", &state.runs.len().to_string()).into(),
        };
        while self.row_focus.len() < rows.len() {
            self.row_focus.push(cx.focus_handle());
        }
        let all_runs = scope == Scope::AllRuns;
        let has_roles = !summary.roles.is_empty();
        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(1400.))
                    .mx_auto()
                    .p_6()
                    .gap_4()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(div().text_lg().font_semibold().text_color(t.text).child(tr(cx, TEXT, "Usage")))
                                    .child(div().truncate().text_xs().text_color(t.muted).child(subtitle)),
                            )
                            .child(div().flex_none().child(self.scope_tabs(scope, current.is_some(), cx))),
                    )
                    .child(totals(&summary, cx))
                    .child(context_panel(&summary, all_runs, cx))
                    .child(warnings_panel(&summary, all_runs, cx))
                    .child(
                        h_flex()
                            .items_stretch()
                            .gap_4()
                            .child(cost_panel("usage-models", "Cost by model", "Model", &summary.models, cx))
                            .when(has_roles, |this| this.child(cost_panel("usage-roles", "Cost by role", "Role", &summary.roles, cx))),
                    )
                    .child(
                        panel(t)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(panel_title(t, tr(cx, TEXT, "Runs")))
                                    .child(div().text_xs().text_color(t.muted).child(tr(cx, TEXT, "Newest first. Click a run to open it."))),
                            )
                            .child(self.run_table(&rows, current, cx)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::*;
    use super::{
        chart_scale, fmt_cost, fmt_dollars, fmt_ratio, fmt_tokens, fmt_usd, run_cost, run_rows, summarize, ContextRow,
        Dollars, Warning, CONTEXT_WARNING_TOKENS, TEXT, UNKNOWN,
    };
    use crate::desktop::i18n::COMMON;

    fn run(project: &'static str) -> Run {
        Run {
            session: "s".into(),
            project: project.into(),
            cwd: None,
            wish: "w".into(),
            status: RunStatus::Done,
            hosted: false,
            started: "".into(),
            started_at: None,
            stages: vec![],
            goals: vec![],
            modules: vec![],
            units: vec![],
            usage: Usage::default(),
            needs: vec![],
            log: vec![],
            decisions: vec![],
            report: None,
            elapsed: None,
            warnings: vec![],
        }
    }

    fn model(name: &'static str, usd: Option<f64>, tokens: [u64; 3]) -> ModelCost {
        ModelCost { model: name.into(), usd, input: Some(tokens[0]), cached: Some(tokens[1]), output: Some(tokens[2]) }
    }

    #[test]
    fn an_agent_with_no_reported_peak_is_not_charted() {
        let usage = Usage { contexts: vec![agent("silent", 0), agent("loud", 42)], ..Default::default() };
        let summary = summarize(&[with_usage("a", usage)]);
        let names: Vec<&str> = summary.contexts.iter().map(|c| c.agent.as_ref()).collect();
        assert_eq!(names, ["loud"]);
    }

    fn agent(name: &'static str, peak_tokens: u64) -> AgentContext {
        AgentContext { agent: name.into(), peak_tokens }
    }

    fn with_usage(project: &'static str, usage: Usage) -> Run {
        Run { usage, ..run(project) }
    }

    #[test]
    fn an_unknown_figure_stays_unknown_and_never_becomes_zero() {
        let summary = summarize(&[run("a")]);
        assert_eq!(summary.cost.known, None);
        assert_eq!(fmt_dollars(summary.cost), UNKNOWN);
        assert_eq!(fmt_tokens(summary.input), UNKNOWN);
        assert_eq!(fmt_tokens(summary.peak()), UNKNOWN);
    }

    #[test]
    fn a_reported_zero_is_a_figure() {
        let mut dollars = Dollars::default();
        dollars.add(Some(0.0));
        assert_eq!(fmt_dollars(dollars), "$0.00");
    }

    #[test]
    fn a_sum_with_missing_parts_is_marked_as_a_lower_bound() {
        let runs = [
            with_usage("a", Usage { total_usd: Some(2.5), ..Default::default() }),
            run("b"),
        ];
        let summary = summarize(&runs);
        assert_eq!(summary.cost, Dollars { known: Some(2.5), missing: 1 });
        assert_eq!(fmt_dollars(summary.cost), "$2.50+");
    }

    #[test]
    fn a_run_without_a_total_falls_back_to_its_model_dollars() {
        let usage = Usage {
            models: vec![model("a", Some(1.25), [0; 3]), model("b", None, [1, 2, 3]), model("c", Some(0.75), [0; 3])],
            ..Default::default()
        };
        assert_eq!(run_cost(&usage), Some(2.0));
        assert_eq!(run_cost(&Usage { models: vec![model("b", None, [1, 2, 3])], ..Default::default() }), None);
    }

    #[test]
    fn a_reported_total_wins_over_the_model_sum() {
        let usage = Usage { total_usd: Some(9.0), models: vec![model("a", Some(1.0), [0; 3])], ..Default::default() };
        assert_eq!(run_cost(&usage), Some(9.0));
    }

    #[test]
    fn codex_rows_keep_their_tokens_with_no_dollars_and_sort_after_priced_rows() {
        let usage = Usage {
            total_usd: Some(3.0),
            models: vec![
                model("gpt-terra", None, [100, 50, 20]),
                model("claude-haiku", Some(1.0), [1, 2, 3]),
                model("claude-sonnet", Some(2.0), [4, 5, 6]),
            ],
            ..Default::default()
        };
        let summary = summarize(&[with_usage("a", usage)]);
        let names: Vec<&str> = summary.models.iter().map(|m| m.name.as_ref()).collect();
        assert_eq!(names, ["claude-sonnet", "claude-haiku", "gpt-terra"]);
        let codex = &summary.models[2];
        assert_eq!(codex.usd.known, None);
        assert!(codex.tokens_only());
        assert_eq!(fmt_dollars(codex.usd), UNKNOWN);
        assert_eq!((codex.input, codex.cached, codex.output), (100, 50, 20));
        assert_eq!((summary.input, summary.cached, summary.output), (105, 57, 29));
    }

    #[test]
    fn the_same_model_adds_up_across_runs_and_notes_runs_without_dollars() {
        let runs = [
            with_usage("a", Usage { models: vec![model("m", Some(1.5), [10, 20, 30])], ..Default::default() }),
            with_usage("b", Usage { models: vec![model("m", None, [1, 2, 3])], ..Default::default() }),
            with_usage("c", Usage { models: vec![model("m", Some(0.5), [100, 200, 300])], ..Default::default() }),
        ];
        let summary = summarize(&runs);
        assert_eq!(summary.models.len(), 1);
        let row = &summary.models[0];
        assert_eq!(row.usd, Dollars { known: Some(2.0), missing: 1 });
        assert_eq!((row.input, row.cached, row.output), (111, 222, 333));
    }

    #[test]
    fn roles_add_up_like_models_and_are_empty_when_none_are_recorded() {
        let role = |usd| RoleCost { role: "reviewer".into(), usd, input: 1, cached: 2, output: 3 };
        let runs = [
            with_usage("a", Usage { roles: vec![role(Some(1.0))], ..Default::default() }),
            with_usage("b", Usage { roles: vec![role(Some(2.0))], ..Default::default() }),
        ];
        let summary = summarize(&runs);
        assert_eq!(summary.roles.len(), 1);
        assert_eq!(summary.roles[0].usd.known, Some(3.0));
        assert_eq!(summary.roles[0].cached, 4);
        assert!(summarize(&[run("c")]).roles.is_empty());
    }

    #[test]
    fn peaks_keep_the_worst_run_for_each_agent_largest_first() {
        let runs = [
            with_usage("small", Usage { contexts: vec![agent("dispatcher", 600_000), agent("main", 90_000)], ..Default::default() }),
            with_usage("big", Usage { contexts: vec![agent("dispatcher", 965_312), agent("reviewer", 44_120)], ..Default::default() }),
        ];
        let summary = summarize(&runs);
        let peaks: Vec<(&str, u64)> = summary.contexts.iter().map(|c| (c.agent.as_ref(), c.peak)).collect();
        assert_eq!(peaks, [("dispatcher", 965_312), ("main", 90_000), ("reviewer", 44_120)]);
        let worst = &summary.contexts[0];
        assert_eq!(worst.project.as_ref(), "big");
        assert_eq!((worst.runs_seen, worst.runs_over), (2, 2));
    }

    #[test]
    fn two_agents_with_one_name_in_a_run_count_as_one_run() {
        let usage = Usage { contexts: vec![agent("dispatcher", 600_000), agent("dispatcher", 100_000)], ..Default::default() };
        let summary = summarize(&[with_usage("a", usage)]);
        assert_eq!(summary.contexts.len(), 1);
        assert_eq!(summary.contexts[0].peak, 600_000);
        assert_eq!((summary.contexts[0].runs_seen, summary.contexts[0].runs_over), (1, 1));
    }

    #[test]
    fn only_agents_past_the_line_warn() {
        let usage = Usage {
            contexts: vec![
                agent("under", CONTEXT_WARNING_TOKENS - 1),
                agent("on", CONTEXT_WARNING_TOKENS),
                agent("over", CONTEXT_WARNING_TOKENS + 1),
            ],
            ..Default::default()
        };
        let summary = summarize(&[with_usage("a", usage)]);
        assert_eq!(
            summary.warnings,
            [Warning::ContextTooLarge { agent: "over".into(), peak: CONTEXT_WARNING_TOKENS + 1, runs_seen: 1, runs_over: 1 }]
        );
    }

    #[test]
    fn the_965k_dispatcher_is_flagged_first_and_warnings_the_run_carries_follow() {
        let usage = Usage { contexts: vec![agent("reviewer", 510_000), agent("dispatcher", 965_312)], ..Default::default() };
        let mut flagged = with_usage("a", usage);
        flagged.warnings = vec!["Log was cut short".into()];
        let summary = summarize(&[flagged]);
        assert!(matches!(&summary.warnings[0], Warning::ContextTooLarge { agent, peak: 965_312, .. } if agent.as_ref() == "dispatcher"));
        assert!(matches!(&summary.warnings[1], Warning::ContextTooLarge { agent, .. } if agent.as_ref() == "reviewer"));
        assert_eq!(summary.warnings[2], Warning::Reported { project: "a".into(), text: "Log was cut short".into() });
        assert_eq!(fmt_ratio(965_312), "1.9");
    }

    #[test]
    fn run_rows_are_newest_first_and_keep_their_place_in_the_list() {
        let at = |project, time| Run { started_at: time, ..run(project) };
        let rows = run_rows(&[at("old", Some(100)), at("none-1", None), at("new", Some(300)), at("none-2", None), at("mid", Some(200))]);
        let order: Vec<(&str, usize)> = rows.iter().map(|r| (r.project.as_ref(), r.index)).collect();
        assert_eq!(order, [("new", 2), ("mid", 4), ("old", 0), ("none-1", 1), ("none-2", 3)]);
    }

    #[test]
    fn a_run_row_shows_unknown_for_what_was_not_reported() {
        let row = &run_rows(&[run("a")])[0];
        assert_eq!(fmt_cost(row.cost), UNKNOWN);
        assert_eq!(fmt_tokens(row.tokens), UNKNOWN);
        assert_eq!(fmt_tokens(row.peak), UNKNOWN);
    }

    #[test]
    fn a_run_row_sums_every_model_and_takes_the_largest_context() {
        let usage = Usage {
            total_usd: Some(4.0),
            models: vec![model("a", Some(4.0), [1, 2, 3]), model("b", None, [10, 20, 30])],
            contexts: vec![agent("x", 5), agent("y", 50)],
            ..Default::default()
        };
        let row = &run_rows(&[with_usage("a", usage)])[0];
        assert_eq!((row.cost, row.tokens, row.peak), (Some(4.0), 66, 50));
    }

    #[test]
    fn the_warning_line_is_always_inside_the_chart() {
        assert_eq!(chart_scale(&[]), 625_000);
        let row = |peak| ContextRow { agent: "a".into(), peak, project: "p".into(), runs_seen: 1, runs_over: 0 };
        assert_eq!(chart_scale(&[row(10)]), 625_000);
        assert_eq!(chart_scale(&[row(965_312), row(10)]), 965_312);
    }

    #[test]
    fn dollars_format_with_cents_and_thousands() {
        assert_eq!(fmt_usd(0.0), "$0.00");
        assert_eq!(fmt_usd(0.004), "<$0.01");
        assert_eq!(fmt_usd(0.005), "$0.01");
        assert_eq!(fmt_usd(116.12), "$116.12");
        assert_eq!(fmt_usd(1_234.5), "$1,234.50");
        assert_eq!(fmt_usd(1_234_567.891), "$1,234,567.89");
    }

    #[test]
    fn tokens_format_in_three_significant_digits() {
        assert_eq!(fmt_tokens(999), "999");
        assert_eq!(fmt_tokens(1_234), "1.2k");
        assert_eq!(fmt_tokens(44_120), "44.1k");
        assert_eq!(fmt_tokens(99_960), "100k");
        assert_eq!(fmt_tokens(965_312), "965k");
        assert_eq!(fmt_tokens(999_499), "999k");
        assert_eq!(fmt_tokens(999_500), "1.0M");
        assert_eq!(fmt_tokens(11_643_991), "11.6M");
        assert_eq!(fmt_tokens(226_026_522), "226M");
        assert_eq!(fmt_tokens(1_500_000_000), "1.5B");
    }

    #[test]
    fn every_string_the_screen_draws_is_in_a_table() {
        let source = include_str!("usage.rs");
        let calls = regex::Regex::new(r#"tr\(\s*cx,\s*TEXT,\s*"([^"]+)""#).unwrap();
        let drawn = calls.captures_iter(source).map(|call| call.get(1).unwrap().as_str());
        let passed = [
            "API cost", "Input", "Cached", "Output", "Status", "Project", "Wish", "Tokens", "Peak context", "Started",
            "Model", "Role", "Cost by model", "Cost by role",
        ];
        for text in drawn.chain(passed) {
            assert!(TEXT.iter().chain(COMMON).any(|(key, _)| *key == text), "no translation for {text}");
        }
    }

    #[test]
    fn every_interface_string_has_a_translation_that_keeps_its_placeholders() {
        for (en, zh) in TEXT {
            assert!(!zh.is_empty(), "{en}");
            for placeholder in ["{n}", "{runs}", "{missing}", "{ratio}", "{limit}", "{tokens}", "{project}", "{over}", "{seen}", "{agent}", "{peak}"] {
                assert_eq!(en.contains(placeholder), zh.contains(placeholder), "{placeholder} in {en}");
            }
        }
    }
}
