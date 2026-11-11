//! A factual overview and the latest model-written summary for each open run.
use super::super::i18n::{fill, tr, translate, Lang, Table};
use super::super::model::{board_facts, board_runs, AppState, BoardEntry, BoardFacts, BoardStatus, Run};
use super::super::ornament;
use super::super::plain_text;
use super::super::theme::{self, Step, Tokened as _, TypeSize};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const TEXT: Table = &[
    ("{} of {} verified", "已驗收 {} / {}"),
    ("no plan yet", "尚無計畫"),
    ("{} running", "{} 個進行中"),
    ("{} failed", "{} 個失敗"),
    ("needs you", "需要你處理"),
    ("Next step: unknown", "下一步：未知"),
    ("just now", "剛剛"),
    ("{} min ago", "{} 分鐘前"),
    ("{} h ago", "{} 小時前"),
    ("{} d ago", "{} 天前"),
    ("Updating — last written {}", "更新中 — 上次撰寫於{}"),
    ("Writing the summary…", "正在撰寫摘要…"),
    ("Could not update ({}) — last written {}", "無法更新（{}）— 上次撰寫於{}"),
    ("Could not write the summary ({})", "無法撰寫摘要（{}）"),
    ("Summary up to date", "摘要已更新"),
    ("Writing", "撰寫中"),
    ("Failed", "失敗"),
    ("Off", "已關閉"),
    ("Nothing is running", "目前沒有執行中的工作"),
    ("+{} more", "另有 {} 個"),
    ("{} — {}", "{}，接下來：{}"),
    (", ", "，"),
    ("{}.", "{}。"),
];

pub(super) fn facts_sentence(f: &BoardFacts, lang: Lang) -> String {
    let mut parts = vec![translate(lang, TEXT, f.status.label()).to_owned()];
    parts.push(if f.total == 0 {
        translate(lang, TEXT, "no plan yet").to_owned()
    } else {
        fill(translate(lang, TEXT, "{} of {} verified"), &[&f.verified.to_string(), &f.total.to_string()])
    });
    if f.running > 0 {
        parts.push(fill(translate(lang, TEXT, "{} running"), &[&f.running.to_string()]));
    }
    if f.failed > 0 {
        parts.push(fill(translate(lang, TEXT, "{} failed"), &[&f.failed.to_string()]));
    }
    if f.needs_you { parts.push(translate(lang, TEXT, "needs you").to_owned()); }
    fill(translate(lang, TEXT, "{}."), &[&parts.join(translate(lang, TEXT, ", "))])
}

pub(super) fn age_text(age_s: i64, lang: Lang) -> String {
    let (key, n) = if age_s < 60 { return translate(lang, TEXT, "just now").to_owned(); }
        else if age_s < 3600 { ("{} min ago", age_s / 60) }
        else if age_s < 86400 { ("{} h ago", age_s / 3600) }
        else { ("{} d ago", age_s / 86400) };
    fill(translate(lang, TEXT, key), &[&n.to_string()])
}

#[derive(Debug, PartialEq)]
pub(super) struct BoardLines {
    pub(super) facts: String,
    pub(super) recent: Option<String>,
    pub(super) next: Option<String>,
    pub(super) note: Option<String>,
}

pub(super) fn board_lines(run: &Run, entry: Option<&BoardEntry>, now_unix: i64, lang: Lang) -> BoardLines {
    let mut lines = BoardLines { facts: facts_sentence(&board_facts(run), lang), recent: None, next: None, note: None };
    let Some(entry) = entry else { return lines; };
    if matches!(entry.status, BoardStatus::Off) { return lines; }
    if let Some(text) = &entry.text {
        lines.recent = Some(text.recent.to_string());
        lines.next = Some(text.next.as_ref().map_or_else(|| translate(lang, TEXT, "Next step: unknown").to_owned(), ToString::to_string));
    }
    lines.note = match (&entry.status, &entry.text) {
        (BoardStatus::Writing, Some(text)) => Some(fill(translate(lang, TEXT, "Updating — last written {}"), &[&age_text(now_unix.saturating_sub(text.written_unix), lang)])),
        (BoardStatus::Writing, None) => Some(translate(lang, TEXT, "Writing the summary…").to_owned()),
        (BoardStatus::Failed(msg), Some(text)) => Some(fill(translate(lang, TEXT, "Could not update ({}) — last written {}"), &[msg.as_ref(), &age_text(now_unix.saturating_sub(text.written_unix), lang)])),
        (BoardStatus::Failed(msg), None) => Some(fill(translate(lang, TEXT, "Could not write the summary ({})"), &[msg.as_ref()])),
        _ => None,
    };
    lines
}

#[derive(Debug, PartialEq)]
pub(super) struct CardParts {
    pub(super) facts: String,
    pub(super) recent: Option<String>,
    pub(super) next: Option<String>,
    pub(super) note: Option<String>,
    pub(super) tip: Option<String>,
}

pub(super) fn board_line(lines: &BoardLines, lang: Lang) -> String {
    match (lines.recent.as_deref(), lines.next.as_deref()) {
        (Some(recent), Some(next)) => fill(translate(lang, TEXT, "{} — {}"), &[recent.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '。' | ',' | '，' | ';' | '；' | '!' | '！' | '?' | '？')), next]),
        (Some(recent), None) => recent.to_owned(),
        (None, Some(next)) => next.to_owned(),
        (None, None) => String::new(),
    }
}

pub(super) fn line_with_note(line: &str, note: Option<&str>) -> String {
    match note {
        Some(note) if line.is_empty() => note.to_owned(),
        Some(note) => format!("{line} · {note}"),
        None => line.to_owned(),
    }
}

pub(super) fn strip_content(project: &str, facts: &str, line: &str) -> String {
    if line.is_empty() { format!("{project} · {facts}") } else { format!("{project} · {facts} · {line}") }
}

pub(super) fn card_parts(lines: &BoardLines, compact: bool, lang: Lang) -> CardParts {
    CardParts {
        facts: lines.facts.clone(),
        recent: if compact { None } else { lines.recent.clone() },
        next: if compact { let line = line_with_note(&board_line(lines, lang), lines.note.as_deref()); (!line.is_empty()).then_some(line) } else { lines.next.clone() },
        note: if compact { None } else { lines.note.clone() },
        tip: if compact { Some(std::iter::once(lines.facts.as_str()).chain(lines.recent.as_deref()).chain(lines.next.as_deref()).chain(lines.note.as_deref()).collect::<Vec<_>>().join("\n")) } else { None },
    }
}

pub(super) const COMPACT_CARD_ESTIMATE: f32 = 64.0;
pub(super) const MORE_ROW_HEIGHT: f32 = 20.0;
pub(super) const STRIP_ROW_HEIGHT: f32 = 24.0;

pub(super) fn cards_that_fit(card_heights: &[f32], budget: f32) -> usize {
    if card_heights.is_empty() { return 0; }
    let budget = if budget.is_nan() { 0.0 } else { budget };
    let mut total = 0.0;
    let mut count = 0;
    for &height in card_heights {
        if !height.is_finite() || height < 0.0 || total + height > budget { break; }
        total += height;
        count += 1;
    }
    count.max(1)
}

pub(super) fn compact_visible_count(count: usize, height: f32, spacing: f32) -> usize {
    let heights: Vec<_> = (0..count).map(|i| COMPACT_CARD_ESTIMATE + if i == 0 { 0.0 } else { spacing }).collect();
    let budget = height - 2.0 * spacing;
    let all = cards_that_fit(&heights, budget);
    if all == count { all } else { cards_that_fit(&heights, budget - spacing - MORE_ROW_HEIGHT) }
}

pub(super) fn more_text(hidden: usize, lang: Lang) -> String {
    fill(translate(lang, TEXT, "+{} more"), &[&hidden.to_string()])
}

pub(super) fn hidden_names(projects: &[&str], visible: usize) -> String {
    projects.get(visible..).unwrap_or_default().join("\n")
}

pub(super) fn card_tooltip(entry: Option<&BoardEntry>, lang: Lang) -> SharedString {
    let key = match entry.map(|entry| &entry.status) {
        Some(BoardStatus::Idle) => "Summary up to date",
        Some(BoardStatus::Writing) => "Writing",
        Some(BoardStatus::Failed(_)) => "Failed",
        Some(BoardStatus::Off) | None => "Off",
    };
    translate(lang, TEXT, key).into()
}

fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let text = text.clone();
        Tooltip::element(move |_, _| div().max_w(theme::READ_WIDTH).whitespace_normal().child(text.clone())).build(window, cx)
    }
}

pub(in crate::desktop) fn board(state: &AppState, compact: bool, cx: &App) -> AnyElement {
    board_with(state, compact, &|_| 0.0, cx)
}

pub(super) fn board_with(state: &AppState, compact: bool, lift: &dyn Fn(&SharedString) -> f32, cx: &App) -> AnyElement {
    board_with_hover(state, compact, false, false, None, lift, std::rc::Rc::new(|_, _, _| {}), cx)
}

pub(super) fn board_with_hover(state: &AppState, compact: bool, strip: bool, inline_more: bool, limit: Option<usize>, lift: &dyn Fn(&SharedString) -> f32, hover: std::rc::Rc<dyn Fn(SharedString, bool, &mut App)>, cx: &App) -> AnyElement {
    let t = cx.tokens();
    let d = theme::density(cx);
    let lang = *cx.global::<Lang>();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| i64::try_from(time.as_secs()).unwrap_or(i64::MAX));
    let runs = board_runs(state);
    let mut list = v_flex().gap(if strip { px(0.0) } else { theme::space(d, Step::Sm) }).w_full()
        .when(!compact, |this| this.max_w(theme::READ_WIDTH));
    if runs.is_empty() {
        return list.child(div().text_size(theme::font(d, TypeSize::Body)).text_color(t.muted).child(tr(cx, TEXT, "Nothing is running"))).into_any_element();
    }
    let visible = limit.unwrap_or(runs.len()).min(runs.len());
    for &index in &runs[..visible] {
        let run = &state.runs[index];
        let entry = state.board.get(&run.session);
        let mut lines = board_lines(run, entry, now, lang);
        for text in [&mut lines.recent, &mut lines.next].into_iter().flatten() {
            *text = plain_text::plain(cx, &SharedString::from(text.as_str())).shown.to_string();
        }
        let CardParts { facts, recent, next, note, tip } = card_parts(&lines, compact, lang);
        let tip = tip.map(|text| plain_text::plain(cx, &SharedString::from(text)).shown);
        let (icon, color) = match entry.map(|entry| &entry.status) {
            Some(BoardStatus::Idle) => (IconName::CircleCheck, t.ok),
            Some(BoardStatus::Writing) => (IconName::LoaderCircle, t.run),
            Some(BoardStatus::Failed(_)) => (IconName::CircleX, t.fail),
            Some(BoardStatus::Off) | None => (IconName::Dash, t.muted),
        };
        let sentence = |text: String, muted: bool| div().min_w_0().text_size(theme::font(d, TypeSize::Body))
            .text_color(if muted { t.muted } else { t.text }).when(compact, |this| this.truncate().line_height(px(16.0))).child(text);
        let key = run.session.clone();
        let on_hover = hover.clone();
        if strip {
            let line = line_with_note(&board_line(&lines, lang), lines.note.as_deref());
            let content = strip_content(&run.project, &facts, &line);
            let row = ornament::raised_card(&t, lift(&run.session), 0.0)
                .id(("board-row", index)).h(px(STRIP_ROW_HEIGHT)).flex().flex_none().w_full().min_w_0().justify_start().overflow_hidden().px(px(4.0))
                .when_some(tip, |this, tip| this.tooltip(tooltip(tip)))
                .on_hover(move |inside: &bool, _, cx| on_hover(key.clone(), *inside, cx))
                .child(h_flex().size_full().flex_1().min_w_0().justify_start().overflow_hidden().items_center().gap(px(4.0))
                    .child(div().flex_1().min_w_0().truncate().text_size(theme::font(d, TypeSize::Body)).text_color(t.text).child(content))
                    .when(inline_more && visible < runs.len(), |this| this.child(div().flex_none().text_size(theme::font(d, TypeSize::Body)).text_color(t.muted).child(more_text(runs.len() - visible, lang)))));
            list = list.child(row);
            continue;
        }
        let card = ornament::raised_card(&t, lift(&run.session), 0.0)
            .id(("board-row", index))
            .when_some(tip, |this, tip| this.tooltip(tooltip(tip)))
            .on_hover(move |inside: &bool, _, cx| on_hover(key.clone(), *inside, cx))
            .w_full().p(if compact { px(4.0) } else { theme::space(d, Step::Sm) })
            .when(compact, |this| this.h(px(COMPACT_CARD_ESTIMATE)).flex_none())
            .child(v_flex().gap(if compact { px(2.0) } else { theme::space(d, Step::Xs) }).min_w_0()
                .child(h_flex().gap(theme::space(d, Step::Sm)).items_center()
                    .child(div().flex_1().min_w_0().text_size(theme::font(d, TypeSize::Label)).text_color(t.text)
                        .when(compact, |this| this.truncate().line_height(px(16.0))).child(run.project.clone()))
                    .child(div().id(("board-status", index)).tooltip(tooltip(card_tooltip(entry, lang)))
                        .child(Icon::new(icon).size(theme::font(d, TypeSize::Title)).text_color(color))))
                .child(sentence(facts, false))
                .when_some(recent, |this, text| this.child(sentence(text, false)))
                .when_some(next, |this, text| this.child(sentence(text, false)))
                .when_some(note, |this, text| this.child(sentence(text, true))));
        list = list.child(card);
    }
    if visible < runs.len() && !inline_more {
        let projects: Vec<&str> = runs.iter().map(|&i| state.runs[i].project.as_ref()).collect();
        let names = SharedString::from(hidden_names(&projects, visible));
        list = list.child(div().id("board-more").h(px(if strip { STRIP_ROW_HEIGHT } else { MORE_ROW_HEIGHT })).flex_none()
            .text_size(theme::font(d, TypeSize::Body)).text_color(t.muted)
            .tooltip(tooltip(names)).child(more_text(runs.len() - visible, lang)));
    }
    list.into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn board_is_visible_to_desktop_mini() {
        let run_view = include_str!("../run_view.rs");
        let board = include_str!("board.rs").split_once("#[cfg(test)]").unwrap().0;
        assert!(run_view.contains("pub(super) mod board;"));
        assert!(board.contains("pub(in crate::desktop) fn board("));
    }
}
