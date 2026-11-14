use super::{
    actions::{self, Actions},
    LoadState, Tab, Workbench,
};
use crate::desktop::{
    components::{panel::PanelState, SplitHandle},
    i18n::{translate, Lang, Table},
    layout::persist,
    theme::{self, Density, Step, Tokened as _},
    ux::seams::research::{FunnelNode, SourceRow},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    *,
};

const PAGE: usize = 200;
const SAMPLE: usize = 5;
const T: Table = &[
    ("Total", "總數"),
    ("Deduplicated", "去重後"),
    ("Duplicates", "重複"),
    ("Screened in", "納入"),
    ("Screened out", "排除"),
    ("Pending", "待處理"),
    ("Unprocessed", "未處理"),
    ("Read", "已讀取"),
    ("Read failed", "讀取失敗"),
    ("Loading sources…", "正在載入來源…"),
    ("Sources could not be loaded", "無法載入來源"),
    (
        "This research does not have literature sources.",
        "此研究沒有文獻來源。",
    ),
    ("Previous", "上一頁"),
    ("Next", "下一頁"),
    ("Seed", "種子"),
    ("Sample 5 for spot-check", "抽樣 5 筆進行抽查"),
    ("Included", "納入"),
    ("Excluded", "排除"),
    ("Failed", "失敗"),
    ("Duplicate", "重複"),
    ("Group", "群組"),
    ("No research open", "尚未開啟研究"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}

pub(crate) fn funnel_nodes(nodes: &[FunnelNode], lang: Lang) -> Vec<(String, String)> {
    nodes
        .iter()
        .map(|node| {
            let label = match node.label.as_str() {
                "Total" => tr(lang, "Total"),
                "Deduplicated" => tr(lang, "Deduplicated"),
                "Duplicates" => tr(lang, "Duplicates"),
                "Screened in" => tr(lang, "Screened in"),
                "Screened out" => tr(lang, "Screened out"),
                "Pending" => tr(lang, "Pending"),
                "Unprocessed" => tr(lang, "Unprocessed"),
                "Read" => tr(lang, "Read"),
                "Read failed" => tr(lang, "Read failed"),
                _ => &node.label,
            };
            (node.key.clone(), format!("{label}: {}", node.count))
        })
        .collect()
}
pub(super) fn row_text(row: &SourceRow, lang: Lang) -> String {
    let (kind, reason) = row.decision.split_once(": ").unwrap_or((&row.decision, ""));
    let label = match kind {
        "Included" => tr(lang, "Included"),
        "Excluded" => tr(lang, "Excluded"),
        "Pending" => tr(lang, "Pending"),
        "Failed" => tr(lang, "Failed"),
        "Duplicate" => tr(lang, "Duplicate"),
        "Unprocessed" => tr(lang, "Unprocessed"),
        _ => kind,
    };
    let decision = if reason.is_empty() {
        label.to_string()
    } else {
        format!("{label}: {reason}")
    };
    let hash: String = row.sha256.chars().take(12).collect();
    let mut text = format!(
        "{hash} · {} · {}: {} · {decision}",
        if row.doi.is_empty() {
            &row.title
        } else {
            &row.doi
        },
        tr(lang, "Group"),
        row.group
    );
    if let Some(page) = row.page {
        let (page_label, state) = match (lang, row.locate_state.as_str()) {
            (Lang::ZhTw, "rendered") => ("頁", "已呈現"),
            (Lang::ZhTw, _) => ("頁", "已定位"),
            (Lang::En, "rendered") => ("Page", "rendered"),
            (Lang::En, _) => ("Page", "located"),
        };
        text.push_str(&format!(" · {page_label} {page} · {state}"));
    }
    text
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
pub struct SourcesPanel {
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
    sizes: Option<Vec<f32>>,
}
impl SourcesPanel {
    pub fn new(wb: Entity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
        cx.observe(&actions, |_, _, cx| cx.notify()).detach();
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        Self {
            wb,
            actions,
            sizes: persist::sizes("research.sources", cx),
        }
    }
}
impl Render for SourcesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let gap = theme::space(Density::default(), Step::Sm);
        let wb = self.wb.read(cx);
        let view = wb.tab_view(Tab::Sources);
        let snapshot = wb.sources_feed.snapshot();
        let mut panel = div()
            .id("research-sources-panel")
            .test_support()
            .flex()
            .flex_col()
            .w_full()
            .gap(gap);
        if wb.map_pending {
            return panel.child(marked(
                "research-sources-loading",
                tr(lang, "Loading sources…").into(),
            ));
        }
        match &snapshot.state {
            PanelState::Failed { reason }
            | PanelState::Corrupt { reason }
            | PanelState::Offline { reason } => {
                let msg = if reason.starts_with("capability ") {
                    tr(lang, "This research does not have literature sources.").to_string()
                } else {
                    format!("{}: {reason}", tr(lang, "Sources could not be loaded"))
                };
                return panel.child(marked("research-sources-error", msg));
            }
            PanelState::Stale { .. } => {
                panel = panel.child(marked(
                    "research-sources-stale",
                    "Sources data is stale".into(),
                ))
            }
            _ => {}
        }
        let Some(display) = snapshot.value else {
            return panel.child(marked(
                "research-sources-loading",
                tr(lang, "Loading sources…").into(),
            ));
        };
        if display.nodes.is_empty() {
            let text = match &wb.load {
                LoadState::Failed { message, .. } => {
                    format!("{}: {message}", tr(lang, "Sources could not be loaded"))
                }
                LoadState::Opening | LoadState::Indexing { .. } | LoadState::Ready => {
                    tr(lang, "Loading sources…").into()
                }
                _ => tr(lang, "No research open").into(),
            };
            return panel.child(marked("research-sources-status", text));
        }
        let mut nodes = div().flex().flex_wrap().gap(gap);
        for (key, text) in funnel_nodes(&display.nodes, lang) {
            let selected = view.node.as_deref().unwrap_or("total") == key;
            let wb = self.wb.clone();
            nodes = nodes.child(
                div()
                    .id(SharedString::from(format!("research-sources-node-{key}")))
                    .test_support()
                    .cursor_pointer()
                    .text_color(if selected { colors.gold } else { colors.text })
                    .child(text.clone().test_text(text))
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_tab_view(
                                Tab::Sources,
                                |v| {
                                    v.node = Some(key.clone());
                                    v.offset = 0;
                                    v.selection = None;
                                },
                                cx,
                            )
                        })
                    }),
            );
        }
        let list_display = display.clone();
        let rows = uniform_list(
            "research-sources-list",
            display.rows.len(),
            move |range, _, _| {
                range
                    .map(|i| {
                        let text = row_text(&list_display.rows[i], lang);
                        div()
                            .id(SharedString::from(format!(
                                "research-sources-row-{}",
                                list_display.offset + i
                            )))
                            .test_support()
                            .h(theme::row_height(Density::default()))
                            .child(text.clone().test_text(text))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .h(theme::row_height(Density::default()) * 5.);
        let available = window.viewport_size().width.as_f32();
        let default = available / 2.;
        let widths = self
            .sizes
            .clone()
            .filter(|sizes| sizes.len() == 2)
            .unwrap_or_else(|| vec![default, default]);
        let handle = cx.entity().clone();
        let split =
            SplitHandle::new("research-sources-split", &colors).on_resize(move |delta, _, cx| {
                handle.update(cx, |this, cx| {
                    let left = (widths[0] + delta.as_f32()).clamp(0., available);
                    let next = vec![left, (available - left).max(0.)];
                    this.sizes = Some(next.clone());
                    if cx.try_global::<persist::LayoutState>().is_some()
                        && this.wb.read(cx).layout_persistence_safe()
                    {
                        persist::remember("research.sources", next, cx);
                    }
                    cx.notify();
                });
            });
        let left = self
            .sizes
            .as_ref()
            .and_then(|s| s.first())
            .copied()
            .unwrap_or(default)
            .clamp(0., available);
        panel = panel.child(
            div()
                .flex()
                .w_full()
                .child(div().w(px(left)).child(nodes))
                .child(
                    div()
                        .id("research-sources-split-frame")
                        .test_support()
                        .child(split),
                )
                .child(div().flex_1().child(rows)),
        );
        let offset = display.offset;
        let mut paging = div().flex().gap(gap).child(marked(
            "research-sources-page",
            format!(
                "{}–{} / {}",
                if display.total == 0 { 0 } else { offset + 1 },
                (offset + display.rows.len()).min(display.total),
                display.total
            ),
        ));
        for (id, label, target, enabled) in [
            (
                "previous",
                "Previous",
                offset.saturating_sub(PAGE),
                offset > 0,
            ),
            (
                "next",
                "Next",
                offset.saturating_add(PAGE),
                offset.saturating_add(PAGE) < display.total,
            ),
        ] {
            if enabled {
                let wb = self.wb.clone();
                paging = paging.child(
                    div()
                        .id(SharedString::from(format!("research-sources-{id}")))
                        .test_support()
                        .cursor_pointer()
                        .child(tr(lang, label).test_text(tr(lang, label)))
                        .on_click(move |_, _, cx| {
                            wb.update(cx, |wb, cx| {
                                wb.set_tab_view(Tab::Sources, |v| v.offset = target, cx)
                            })
                        }),
                );
            }
        }
        panel = panel.child(paging);
        if display.total > 0 {
            let wb = self.wb.clone();
            panel = panel.child(
                div()
                    .id("research-sources-sample")
                    .test_support()
                    .cursor_pointer()
                    .child(
                        tr(lang, "Sample 5 for spot-check")
                            .test_text(tr(lang, "Sample 5 for spot-check")),
                    )
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            let seed = wb.clock.now_ms() as u64;
                            let node = wb
                                .tab_view(Tab::Sources)
                                .node
                                .unwrap_or_else(|| "total".into());
                            wb.set_tab_view(
                                Tab::Sources,
                                |v| v.selection = Some(format!("sample:{seed}")),
                                cx,
                            );
                            wb.port.sample(node, SAMPLE, seed);
                            wb.refresh_tab_feeds(cx);
                        })
                    }),
            );
        }
        if let Some(seed) = view
            .selection
            .as_deref()
            .and_then(|s| s.strip_prefix("sample:"))
        {
            panel = panel.child(marked(
                "research-sources-seed",
                format!("{}: {seed}", tr(lang, "Seed")),
            ));
            for (i, target) in display.sample.iter().enumerate() {
                panel = panel.child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(marked(
                            SharedString::from(format!("research-sources-sampled-{i}")),
                            target.clone(),
                        ))
                        .child(actions::spot_check_buttons(
                            &self.actions,
                            target,
                            view.node.as_deref(),
                            cx,
                        )),
                );
            }
            panel = panel.child(actions::receipts(&self.actions, cx));
        }
        panel
    }
}
