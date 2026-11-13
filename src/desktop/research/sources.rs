use super::{
    actions::{self, Actions},
    LoadState, Tab, Workbench,
};
use crate::{
    desktop::{
        i18n::{translate, Lang, Table},
        theme::Tokened as _,
    },
    research_workspace::{
        funnel::{self, Decision, Funnel, FunnelRow, Node},
        index::Index,
    },
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{h_flex, v_flex, v_virtual_list, VirtualListScrollHandle},
    *,
};
use std::{rc::Rc, sync::Arc};

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

pub(crate) fn funnel_nodes(f: &Funnel, lang: Lang) -> Vec<(Node, String)> {
    let mut v = Vec::new();
    let mut add =
        |node, label: &'static str, count| v.push((node, format!("{}: {count}", tr(lang, label))));
    add(Node::Total, "Total", f.total);
    add(Node::Deduplicated, "Deduplicated", f.deduplicated);
    add(Node::Duplicates, "Duplicates", f.duplicates);
    add(Node::ScreenedIn, "Screened in", f.screened_in);
    add(
        Node::ScreenedOut { reason: None },
        "Screened out",
        f.screened_out,
    );
    for (reason, count) in &f.out_reasons {
        v.push((
            Node::ScreenedOut {
                reason: Some(if reason == "(none)" {
                    String::new()
                } else {
                    reason.clone()
                }),
            },
            format!("  {reason}: {count}"),
        ));
    }
    v.push((
        Node::Pending,
        format!("{}: {}", tr(lang, "Pending"), f.pending),
    ));
    v.push((
        Node::Unprocessed,
        format!("{}: {}", tr(lang, "Unprocessed"), f.unprocessed),
    ));
    v.push((Node::Read, format!("{}: {}", tr(lang, "Read"), f.read)));
    v.push((
        Node::ReadFailed { reason: None },
        format!("{}: {}", tr(lang, "Read failed"), f.read_failed),
    ));
    for (reason, count) in &f.fail_reasons {
        v.push((
            Node::ReadFailed {
                reason: Some(if reason == "(none)" {
                    String::new()
                } else {
                    reason.clone()
                }),
            },
            format!("  {reason}: {count}"),
        ));
    }
    v
}
fn row_text(row: &FunnelRow, lang: Lang) -> String {
    let decision = match &row.decision {
        Decision::Included => tr(lang, "Included").to_owned(),
        Decision::Excluded { reason } => format!("{}: {reason}", tr(lang, "Excluded")),
        Decision::Pending { reason } => format!("{}: {reason}", tr(lang, "Pending")),
        Decision::Failed { reason } => format!("{}: {reason}", tr(lang, "Failed")),
        Decision::Duplicate { of_group } => format!("{}: {of_group}", tr(lang, "Duplicate")),
        Decision::Unprocessed => tr(lang, "Unprocessed").to_owned(),
    };
    let hash: String = row.sha256.chars().take(12).collect();
    format!(
        "{hash} · {} · {}: {} · {decision}",
        if row.doi.is_empty() {
            &row.title
        } else {
            &row.doi
        },
        tr(lang, "Group"),
        row.duplicate_group
    )
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
    index: Option<Arc<Index>>,
    key: Option<(String, usize, Option<u64>)>,
    generation: u64,
    funnel: Option<Funnel>,
    rows: Vec<FunnelRow>,
    total: usize,
    sample: Vec<String>,
    error: Option<(String, String)>,
    loading: bool,
    scroll: VirtualListScrollHandle,
}
impl SourcesPanel {
    pub fn new(wb: Entity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
        cx.observe(&actions, |_, _, cx| cx.notify()).detach();
        cx.observe(&wb, |this, _, cx| this.sync(cx)).detach();
        let mut panel = Self {
            wb,
            actions,
            index: None,
            key: None,
            generation: 0,
            funnel: None,
            rows: vec![],
            total: 0,
            sample: vec![],
            error: None,
            loading: false,
            scroll: VirtualListScrollHandle::new(),
        };
        panel.sync(cx);
        panel
    }
    fn sync(&mut self, cx: &mut Context<Self>) {
        let wb = self.wb.read(cx);
        let index = wb.index.clone();
        let view = wb.view(Tab::Sources);
        let node = view
            .node
            .as_deref()
            .and_then(Node::from_key)
            .unwrap_or(Node::Total);
        let seed = view
            .selection
            .as_deref()
            .and_then(|s| s.strip_prefix("sample:"))
            .and_then(|s| s.parse::<u64>().ok());
        let key = (node.key(), view.offset, seed);
        let same = match (&self.index, &index) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same && self.key.as_ref() == Some(&key) {
            cx.notify();
            return;
        }
        if self
            .key
            .as_ref()
            .is_none_or(|old| old.0 != key.0 || old.1 != key.1)
        {
            self.scroll = VirtualListScrollHandle::new();
        }
        self.index = index.clone();
        self.key = Some(key.clone());
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.funnel = None;
        self.rows.clear();
        self.sample.clear();
        self.total = 0;
        self.error = None;
        self.loading = index.is_some();
        if let Some(index) = index {
            let task = cx.background_spawn(async move {
                let result = (|| {
                    let f = funnel::funnel(&index)?;
                    let (rows, total) = funnel::rows(&index, &node, key.1, PAGE)?;
                    let sample = match seed {
                        Some(seed) => funnel::sample(&index, &node, SAMPLE, seed)?,
                        None => vec![],
                    };
                    Ok::<_, crate::base::Failure>((f, rows, total, sample))
                })();
                result
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return;
                    }
                    this.loading = false;
                    match result {
                        Ok((f, rows, total, sample)) => {
                            this.funnel = Some(f);
                            this.rows = rows;
                            this.total = total;
                            this.sample = sample;
                        }
                        Err(e) => this.error = Some((e.kind, e.message)),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
    fn set_page(&self, offset: usize, cx: &mut Context<Self>) {
        self.wb.update(cx, |wb, cx| {
            wb.set_view(Tab::Sources, |v| v.offset = offset, cx)
        });
    }
}
impl Render for SourcesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let mut panel = v_flex()
            .id("research-sources-panel")
            .test_support()
            .w_full()
            .gap(px(4.));
        if self.loading {
            return panel.child(marked(
                "research-sources-loading",
                tr(lang, "Loading sources…").into(),
            ));
        }
        if let Some((kind, message)) = &self.error {
            let text = if kind == "not_capable" {
                tr(lang, "This research does not have literature sources.").to_owned()
            } else {
                format!("{}: {message}", tr(lang, "Sources could not be loaded"))
            };
            return panel.child(marked("research-sources-error", text));
        }
        let Some(funnel) = &self.funnel else {
            let text = match &self.wb.read(cx).load {
                LoadState::Failed { message, .. } => {
                    format!("{}: {message}", tr(lang, "Sources could not be loaded"))
                }
                LoadState::Opening | LoadState::Indexing { .. } => {
                    tr(lang, "Loading sources…").into()
                }
                _ => tr(lang, "No research open").into(),
            };
            return panel.child(marked("research-sources-status", text));
        };
        let colors = cx.tokens();
        let mut nodes = v_flex().gap(px(4.));
        let mut stage = h_flex().flex_wrap().gap(px(8.));
        for (node, text) in funnel_nodes(funnel, lang) {
            if matches!(node, Node::ScreenedIn | Node::Read) {
                nodes = nodes.child(stage).child("↓");
                stage = h_flex().flex_wrap().gap(px(8.));
            }
            let key = node.key();
            let selected = self.key.as_ref().is_some_and(|k| k.0 == key);
            let wb = self.wb.clone();
            let id = format!("research-sources-node-{key}");
            stage = stage.child(
                div()
                    .id(SharedString::from(id))
                    .test_support()
                    .cursor_pointer()
                    .text_color(if selected { colors.gold } else { colors.text })
                    .child(text.clone().test_text(text))
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_view(
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
        panel = panel.child(nodes.child(stage));
        let offset = self.key.as_ref().map_or(0, |k| k.1);
        let sizes = Rc::new(self.rows.iter().map(|_| size(px(0.), px(30.))).collect());
        panel = panel.child(
            div().h(px(180.)).w_full().child(
                v_virtual_list(
                    cx.entity(),
                    "research-sources-list",
                    sizes,
                    move |this, range, _, cx| {
                        let lang = *cx.global::<Lang>();
                        range
                            .map(|i| {
                                let text = row_text(&this.rows[i], lang);
                                div()
                                    .id(SharedString::from(format!(
                                        "research-sources-row-{}",
                                        offset + i
                                    )))
                                    .test_support()
                                    .h(px(30.))
                                    .child(text.clone().test_text(text))
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&self.scroll),
            ),
        );
        let mut paging = h_flex().gap(px(12.)).child(marked(
            "research-sources-page",
            format!(
                "{}–{} / {}",
                if self.total == 0 { 0 } else { offset + 1 },
                (offset + self.rows.len()).min(self.total),
                self.total
            ),
        ));
        if offset > 0 {
            paging = paging.child(
                div()
                    .id("research-sources-previous")
                    .test_support()
                    .cursor_pointer()
                    .child(tr(lang, "Previous").test_text(tr(lang, "Previous")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_page(offset.saturating_sub(PAGE), cx)
                    })),
            );
        }
        if offset.saturating_add(PAGE) < self.total {
            paging = paging.child(
                div()
                    .id("research-sources-next")
                    .test_support()
                    .cursor_pointer()
                    .child(tr(lang, "Next").test_text(tr(lang, "Next")))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_page(offset + PAGE, cx))),
            );
        }
        panel = panel.child(paging);
        if self.total > 0 {
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
                            wb.set_view(
                                Tab::Sources,
                                |v| v.selection = Some(format!("sample:{seed}")),
                                cx,
                            );
                        })
                    }),
            );
        }
        if let Some((node, _, Some(seed))) = &self.key {
            panel = panel.child(marked(
                "research-sources-seed",
                format!("{}: {seed}", tr(lang, "Seed")),
            ));
            for (i, target) in self.sample.iter().enumerate() {
                panel = panel.child(
                    h_flex()
                        .gap(px(8.))
                        .child(marked(
                            SharedString::from(format!("research-sources-sampled-{i}")),
                            target.clone(),
                        ))
                        .child(actions::spot_check_buttons(
                            &self.actions,
                            target,
                            Some(node),
                            cx,
                        )),
                );
            }
            panel = panel.child(actions::receipts(&self.actions, cx));
        }
        panel
    }
}
