use super::{
    actions::{self, Actions},
    Tab, Workbench,
};
use crate::desktop::{
    components::{panel::PanelState, SplitHandle},
    i18n::{translate, Lang, Table},
    layout::persist,
    theme::{self, Density, Step, Tokened as _},
    ux::seams::research::EvidenceLocator,
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    *,
};

const PAGE_SIZE: usize = 20;
const T: Table = &[
    ("No research open or indexing", "尚未開啟研究或正在建立索引"),
    (
        "Page {physical} (printed {printed})",
        "第 {physical} 頁（印刷頁碼 {printed}）",
    ),
    ("Page {physical}", "第 {physical} 頁"),
    (
        "{file}, row {row}, column {column}",
        "{file}，第 {row} 列，欄位 {column}",
    ),
    ("Literature", "文獻"),
    ("Raw", "原始資料"),
    ("Previous", "上一頁"),
    ("Next", "下一頁"),
    ("Loading evidence", "正在載入證據"),
    ("No evidence", "沒有證據"),
    ("Loading page", "正在載入頁面"),
    ("No page selected", "未選取頁面"),
    ("Page unavailable", "頁面無法使用"),
    ("No PDF page for this evidence", "此證據沒有 PDF 頁面"),
    ("Open in dataset", "在資料集中開啟"),
    ("Correct", "修正"),
    (
        "PDF pages are not available for this research",
        "此研究不提供 PDF 頁面",
    ),
    (
        "No region recorded; showing the whole page",
        "未記錄區域；顯示整頁",
    ),
    (
        "Region recorded; precise highlighting comes in a later version",
        "已記錄區域；精確標示將於未來版本提供",
    ),
    ("PDF renderer is unavailable", "PDF 頁面轉換工具無法使用"),
    ("The PDF is corrupt", "PDF 已損毀"),
    ("The PDF hash does not match", "PDF 雜湊值不符"),
    ("Page rendering timed out", "頁面轉換逾時"),
    ("The page is out of range", "頁碼超出範圍"),
    (
        "An evidence artifact is missing or corrupt",
        "證據檔案遺失或損毀",
    ),
    (
        "The page or evidence could not be loaded",
        "無法載入頁面或證據",
    ),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
pub(crate) fn locator_text(target: &EvidenceLocator, lang: Lang) -> String {
    match target {
        EvidenceLocator::PdfPage {
            physical_page,
            printed_page,
            highlight,
        } => {
            let key = if printed_page.is_some() {
                "Page {physical} (printed {printed})"
            } else {
                "Page {physical}"
            };
            let page = tr(lang, key)
                .replace("{physical}", &physical_page.to_string())
                .replace("{printed}", printed_page.as_deref().unwrap_or(""));
            format!(
                "{page} · {}",
                tr(
                    lang,
                    if *highlight {
                        "Region recorded; precise highlighting comes in a later version"
                    } else {
                        "No region recorded; showing the whole page"
                    }
                )
            )
        }
        EvidenceLocator::RawField { file, row, column } => {
            tr(lang, "{file}, row {row}, column {column}")
                .replace("{file}", file)
                .replace("{row}", &row.to_string())
                .replace("{column}", column)
        }
    }
}
pub(crate) fn degraded_text(kind: &str, lang: Lang) -> String {
    let key = match kind {
        "renderer_missing" => "PDF renderer is unavailable",
        "corrupt_pdf" => "The PDF is corrupt",
        "hash_mismatch" => "The PDF hash does not match",
        "render_timeout" => "Page rendering timed out",
        "page_out_of_range" => "The page is out of range",
        "missing_artifact" | "corrupt_artifact" => "An evidence artifact is missing or corrupt",
        _ => "The page or evidence could not be loaded",
    };
    format!("{} ({kind})", tr(lang, key))
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
fn clickable(
    id: impl Into<ElementId>,
    text: String,
    f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .cursor_pointer()
        .child(text.clone().test_text(text))
        .on_click(f)
}

pub struct EvidencePanel {
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
    sizes: Option<Vec<f32>>,
    requested: Option<(u64, Option<String>, usize, String)>,
}
impl EvidencePanel {
    pub fn new(wb: Entity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
        cx.observe(&actions, |_, _, cx| cx.notify()).detach();
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        Self {
            wb,
            actions,
            sizes: persist::sizes("research.evidence", cx),
            requested: None,
        }
    }
}
impl Render for EvidencePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let gap = theme::space(Density::default(), Step::Sm);
        let wb = self.wb.read(cx);
        let view = wb.tab_view(Tab::Evidence);
        let generation = wb.generation;
        let snapshot = wb.evidence_feed.snapshot();
        let mut panel = div()
            .id("research-evidence-panel")
            .test_support()
            .flex()
            .flex_col()
            .w_full()
            .gap(gap)
            .text_color(colors.text);
        if matches!(wb.load, super::LoadState::NoResearch) {
            return panel.child(
                tr(lang, "No research open or indexing")
                    .test_text(tr(lang, "No research open or indexing")),
            );
        }
        let mut switches = div().flex().gap(gap);
        for (name, label, raw) in [("literature", "Literature", false), ("raw", "Raw", true)] {
            let wb = self.wb.clone();
            switches = switches.child(clickable(
                SharedString::from(format!("research-evidence-layer-{name}")),
                tr(lang, label).into(),
                move |_, _, cx| {
                    wb.update(cx, |wb, cx| {
                        wb.set_tab_view(
                            Tab::Evidence,
                            |v| {
                                v.node = raw.then(|| "raw".into());
                                v.offset = 0;
                                v.selection = None;
                            },
                            cx,
                        )
                    })
                },
            ));
        }
        panel = panel.child(switches);
        if wb.map_pending {
            return panel.child(marked(
                "research-evidence-page",
                tr(lang, "Loading evidence").into(),
            ));
        }
        match &snapshot.state {
            PanelState::Failed { reason }
            | PanelState::Corrupt { reason }
            | PanelState::Offline { reason } => {
                return panel.child(marked(
                    "research-evidence-degraded",
                    format!(
                        "{}: {reason}",
                        tr(lang, "An evidence artifact is missing or corrupt")
                    ),
                ))
            }
            PanelState::Stale { .. } => {
                panel = panel.child(marked(
                    "research-evidence-stale",
                    "Evidence data is stale".into(),
                ))
            }
            _ => {}
        }
        let Some(display) = snapshot.value else {
            return panel.child(marked(
                "research-evidence-page",
                tr(lang, "Loading evidence").into(),
            ));
        };
        if display.layers.is_empty() {
            return panel.child(marked(
                "research-evidence-page",
                tr(lang, "Loading evidence").into(),
            ));
        }
        if !display.pdf_available {
            panel = panel.child(marked(
                "research-evidence-capability",
                tr(lang, "PDF pages are not available for this research").into(),
            ));
        }
        let mut entities = div()
            .id("research-evidence-entities")
            .flex()
            .w_full()
            .overflow_x_scroll()
            .gap(gap);
        for (i, name) in display.entities.iter().enumerate() {
            let name = name.clone();
            let label = name.clone();
            let wb = self.wb.clone();
            entities = entities.child(clickable(
                SharedString::from(format!("research-evidence-entity-{i}")),
                label,
                move |_, _, cx| {
                    wb.update(cx, |wb, cx| {
                        wb.set_tab_view(Tab::Evidence, |v| v.selection = None, cx);
                        wb.port.select_entity(name.clone());
                        wb.refresh_tab_feeds(cx);
                    })
                },
            ));
        }
        panel = panel.child(entities);
        let mut paging = div().flex().gap(gap);
        for (id, label, offset, enabled) in [
            (
                "previous",
                "Previous",
                view.offset.saturating_sub(PAGE_SIZE),
                view.offset > 0,
            ),
            (
                "next",
                "Next",
                view.offset.saturating_add(PAGE_SIZE),
                view.offset.saturating_add(PAGE_SIZE) < display.total,
            ),
        ] {
            if enabled {
                let wb = self.wb.clone();
                paging = paging.child(clickable(
                    SharedString::from(format!("research-evidence-{id}")),
                    tr(lang, label).into(),
                    move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_tab_view(
                                Tab::Evidence,
                                |v| {
                                    v.offset = offset;
                                    v.selection = None;
                                },
                                cx,
                            )
                        })
                    },
                ));
            }
        }
        panel = panel.child(paging);
        let mut conditions = div()
            .id("research-evidence-conditions")
            .flex()
            .flex_col()
            .max_h(theme::row_height(Density::default()) * 6.)
            .overflow_y_scroll();
        for (i, condition) in display.conditions.iter().enumerate() {
            let mut element = div()
                .id(SharedString::from(format!(
                    "research-evidence-condition-{i}"
                )))
                .test_support()
                .flex()
                .flex_col()
                .child(condition.id.clone().test_text(condition.id.clone()));
            for row in &condition.rows {
                let mut line = div().flex().gap(gap).child(row.text.clone());
                for id in &row.evidence {
                    let id = id.clone();
                    let label = id.clone();
                    let wb = self.wb.clone();
                    line = line.child(clickable(
                        SharedString::from(format!("research-evidence-item-{id}")),
                        label,
                        move |_, _, cx| wb.update(cx, |wb, cx| wb.open_evidence(&id, cx)),
                    ));
                }
                element = element.child(line);
            }
            conditions = conditions.child(element);
        }
        let mut contradictions = div()
            .id("research-evidence-contradictions")
            .flex()
            .flex_col();
        for (i, item) in display.contradictions.iter().enumerate() {
            contradictions = contradictions.child(
                div()
                    .id(SharedString::from(format!(
                        "research-evidence-contradiction-{i}"
                    )))
                    .test_support()
                    .flex()
                    .gap(gap)
                    .child(item.text.clone().test_text(item.text.clone()))
                    .child(item.a.clone().test_text(item.a.clone()))
                    .child(item.b.clone().test_text(item.b.clone())),
            );
        }
        let selected = view.selection.clone().or_else(|| {
            display
                .conditions
                .iter()
                .flat_map(|c| &c.rows)
                .flat_map(|r| &r.evidence)
                .next()
                .cloned()
        });
        if let Some(id) = selected.as_ref() {
            let request = (generation, view.node.clone(), view.offset, id.clone());
            if self.requested.as_ref() != Some(&request) {
                self.requested = Some(request);
                self.wb.update(cx, |wb, cx| {
                    wb.port.locate_page(id.clone());
                    wb.port.render_page(id.clone());
                    wb.refresh_tab_feeds(cx);
                });
            }
        } else {
            self.requested = None;
        }
        let item = selected
            .as_ref()
            .and_then(|id| display.items.iter().find(|item| &item.id == id));
        if let Some(item) = item {
            if let Some(target) = &item.target {
                panel = panel.child(marked(
                    "research-evidence-locator",
                    locator_text(target, lang),
                ));
            }
            let id = item.id.clone();
            let wb = self.wb.clone();
            panel = panel.child(clickable(
                "research-evidence-open-dataset",
                tr(lang, "Open in dataset").into(),
                move |_, _, cx| wb.update(cx, |wb, cx| wb.show_evidence_rows(&id, cx)),
            ));
            let target = item.record_id.clone();
            let actions = self.actions.clone();
            panel = panel.child(clickable(
                "research-evidence-correct",
                tr(lang, "Correct").into(),
                move |_, _, cx| {
                    actions.update(cx, |a, cx| a.begin_correction(&target, "quote", cx))
                },
            ));
            if let Some(notice) = &item.notice {
                panel = panel.child(marked(
                    "research-evidence-degraded",
                    degraded_text(&notice.kind, lang),
                ));
            }
        }
        for (i, notice) in display.notices.iter().enumerate() {
            panel = panel.child(marked(
                SharedString::from(format!("research-evidence-notice-{i}")),
                degraded_text(&notice.kind, lang),
            ));
        }
        let page = if let Some(path) = item.and_then(|item| item.image.clone()) {
            div()
                .id("research-evidence-page")
                .test_support()
                .child(
                    img(path)
                        .w(theme::space(Density::default(), Step::Xl) * 6.)
                        .h(theme::row_height(Density::default()) * 5.),
                )
                .into_any_element()
        } else {
            marked(
                "research-evidence-page",
                tr(
                    lang,
                    if item.and_then(|i| i.notice.as_ref()).is_some() {
                        "Page unavailable"
                    } else if item.and_then(|i| i.target.as_ref()).is_some() {
                        "No PDF page for this evidence"
                    } else {
                        "No page selected"
                    },
                )
                .into(),
            )
            .into_any_element()
        };
        let available = window.viewport_size().width.as_f32();
        let default = available / 2.;
        let widths = self
            .sizes
            .clone()
            .filter(|s| s.len() == 2)
            .unwrap_or_else(|| vec![default, default]);
        let handle = cx.entity().clone();
        let split =
            SplitHandle::new("research-evidence-split", &colors).on_resize(move |delta, _, cx| {
                handle.update(cx, |this, cx| {
                    let left = (widths[0] + delta.as_f32()).clamp(0., available);
                    let next = vec![left, (available - left).max(0.)];
                    this.sizes = Some(next.clone());
                    if cx.try_global::<persist::LayoutState>().is_some()
                        && this.wb.read(cx).layout_persistence_safe()
                    {
                        persist::remember("research.evidence", next, cx);
                    }
                    cx.notify();
                })
            });
        let left = self
            .sizes
            .as_ref()
            .and_then(|s| s.first())
            .copied()
            .unwrap_or(default)
            .clamp(0., available);
        panel
            .child(
                div()
                    .flex()
                    .gap(gap)
                    .child(div().w(px(left)).child(conditions))
                    .child(
                        div()
                            .id("research-evidence-split-frame")
                            .test_support()
                            .child(split),
                    )
                    .child(div().flex_1().child(contradictions).child(page)),
            )
            .child(actions::correction_form(&self.actions, window, cx))
    }
}
