use super::{
    actions::{self, Actions},
    LoadState, Tab, Workbench,
};
use crate::desktop::{
    components::{panel::PanelState, SplitHandle, TextField},
    i18n::{translate, Lang, Table},
    layout::persist,
    theme::{self, Density, Step, Tokened as _},
    ux::seams::research::{DatasetRow, Filter, FilterOp, TabView},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::input::{InputEvent, InputState},
    *,
};
use std::{cell::Cell, rc::Rc};

#[cfg(test)]
pub(crate) use super::adapter::dataset::load_page;
const PAGE: usize = 200;
const COLUMNS: [&str; 4] = ["variable", "entity", "condition_id", "paper_or_source"];
const T: Table = &[
    ("No research open", "尚未開啟研究"),
    ("Extracted", "擷取資料"),
    ("Raw", "原始資料"),
    ("variable", "變數"),
    ("entity", "實體"),
    ("condition_id", "條件"),
    ("paper_or_source", "論文或來源"),
    ("Correct", "修正"),
    ("no evidence recorded", "未記錄證據"),
    ("No missing-rate columns", "沒有缺失率欄位"),
    (
        "Indexing in progress; showing the first page only.",
        "正在建立索引；目前僅顯示第一頁。",
    ),
    ("Previous", "上一頁"),
    ("Next", "下一頁"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
pub(crate) fn filter_value(v: &TabView, column: &str) -> String {
    v.filters
        .iter()
        .find_map(|f| {
            if f.column == column {
                if let FilterOp::Contains(s) = &f.op {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or_default()
}
fn row_text(row: &DatasetRow, lang: Lang) -> String {
    if lang == Lang::ZhTw {
        row.text_zh.clone()
    } else {
        row.text_en.clone()
    }
}
pub struct DatasetPanel {
    panic_pending: bool,
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
    inputs: Vec<Entity<InputState>>,
    sizes: Option<Vec<f32>>,
    scroll: UniformListScrollHandle,
    query: Option<(Option<String>, Vec<Filter>)>,
    restoring: Rc<Cell<Option<(usize, bool)>>>,
}
impl DatasetPanel {
    pub fn new(wb: Entity<Workbench>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
        cx.observe(&actions, |_, _, cx| cx.notify()).detach();
        let inputs: Vec<_> = COLUMNS
            .iter()
            .map(|_| cx.new(|cx| InputState::new(window, cx)))
            .collect();
        for (i, input) in inputs.iter().enumerate() {
            cx.subscribe_in(
                input,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let value = input.read(cx).value().to_string();
                    if value == filter_value(&this.wb.read(cx).tab_view(Tab::Dataset), COLUMNS[i]) {
                        return;
                    }
                    this.wb.update(cx, |wb, cx| {
                        wb.set_tab_view(
                            Tab::Dataset,
                            |v| {
                                v.filters.retain(|f| f.column != COLUMNS[i]);
                                if !value.is_empty() {
                                    v.filters.push(Filter {
                                        column: COLUMNS[i].into(),
                                        op: FilterOp::Contains(value),
                                    });
                                }
                                v.offset = 0;
                            },
                            cx,
                        )
                    });
                },
            )
            .detach();
        }
        cx.observe_in(&wb, window, |this, wb, window, cx| {
            let view = wb.read(cx).tab_view(Tab::Dataset);
            for (i, input) in this.inputs.iter().enumerate() {
                let value = filter_value(&view, COLUMNS[i]);
                if input.read(cx).value().as_ref() != value {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
            }
            cx.notify();
        })
        .detach();
        Self {
            panic_pending: false,
            wb,
            actions,
            inputs,
            sizes: persist::sizes("research.dataset", cx),
            scroll: UniformListScrollHandle::new(),
            query: None,
            restoring: Rc::new(Cell::new(None)),
        }
    }
}
impl DatasetPanel {
    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let gap = theme::space(Density::default(), Step::Sm);
        let wb = self.wb.read(cx);
        let view = wb.tab_view(Tab::Dataset);
        let snapshot = wb.dataset_feed.snapshot();
        let mut panel = div()
            .id("research-dataset-panel")
            .test_support()
            .w_full()
            .flex()
            .flex_col()
            .w_full()
            .gap(gap)
            .text_color(colors.text);
        if matches!(wb.load, LoadState::NoResearch) {
            return panel
                .child(tr(lang, "No research open").test_text(tr(lang, "No research open")));
        }
        let mut layers = div().flex().gap(gap);
        for (key, label) in [("extracted", "Extracted"), ("raw", "Raw")] {
            let wb = self.wb.clone();
            layers = layers.child(
                div()
                    .id(SharedString::from(format!("research-dataset-layer-{key}")))
                    .test_support()
                    .cursor_pointer()
                    .child(tr(lang, label).test_text(tr(lang, label)))
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_tab_view(
                                Tab::Dataset,
                                |v| {
                                    v.node = (key == "raw").then(|| "raw".into());
                                    v.offset = 0;
                                },
                                cx,
                            )
                        })
                    }),
            );
        }
        let mut filters = div().flex().flex_col().gap(gap);
        for (i, column) in COLUMNS.iter().enumerate() {
            let input = self.inputs[i].clone();
            filters = filters.child(
                div().flex().flex_col().child(tr(lang, column)).child(
                    div()
                        .id(SharedString::from(format!(
                            "research-dataset-filter-{column}"
                        )))
                        .test_support()
                        .on_click(move |_, window, cx| {
                            input.read(cx).focus_handle(cx).focus(window, cx)
                        })
                        .child(TextField::new(
                            SharedString::from(format!("research-dataset-filter-{column}-field")),
                            &self.inputs[i],
                            &colors,
                        )),
                ),
            );
        }
        let mut results = div().flex().flex_col().gap(gap);
        if let PanelState::Failed { reason }
        | PanelState::Corrupt { reason }
        | PanelState::Offline { reason } = &snapshot.state
        {
            results = results.child(marked("research-dataset-error", reason.clone()));
        }
        if let Some(display) = snapshot.value {
            for notice in &display.notices {
                results = results.child(marked(
                    "research-dataset-error",
                    format!("{}: {}", notice.kind, notice.message),
                ));
            }
            if display.preview {
                results = results.child(marked(
                    "research-dataset-preview-notice",
                    tr(lang, "Indexing in progress; showing the first page only.").into(),
                ));
            } else if !display.validation_en.is_empty() {
                results = results.child(marked(
                    "research-dataset-validation",
                    if lang == Lang::ZhTw {
                        display.validation_zh.clone()
                    } else {
                        display.validation_en.clone()
                    },
                ));
                results = results.child(marked(
                    "research-dataset-missing",
                    if display.missing.is_empty() {
                        tr(lang, "No missing-rate columns").into()
                    } else {
                        display.missing.join(" · ")
                    },
                ));
            }
            let query = (view.node.clone(), view.filters.clone());
            if !display.preview
                && display.total > 0
                && display.filters == view.filters
                && display.offset == view.offset
                && self.query.as_ref() != Some(&query)
            {
                self.scroll
                    .scroll_to_item_strict(view.offset, ScrollStrategy::Top);
                self.restoring.set(Some((view.offset, false)));
                self.query = Some(query.clone());
            }
            let query_ready =
                self.query.as_ref() == Some(&query) && display.filters == view.filters;
            let list_display = display.clone();
            let list_wb = self.wb.clone();
            let list_actions = self.actions.clone();
            let restoring = self.restoring.clone();
            let list = uniform_list(
                "research-dataset-rows",
                display.total,
                move |range, _, cx| {
                    let display = &list_display;
                    if query_ready && !display.preview && !range.is_empty() && range != (0..1) {
                        let restored = restoring.get();
                        let current = list_wb.read(cx).tab_view(Tab::Dataset).offset;
                        // Do not overwrite a saved position while its deferred scroll is being applied.
                        let update = match restored {
                            Some((target, _)) if range.contains(&target) => {
                                restoring.set(Some((target, true)));
                                false
                            }
                            Some((_, false)) => false,
                            Some((_, true)) => {
                                restoring.set(None);
                                true
                            }
                            None => true,
                        };
                        if update && current != range.start {
                            let offset = range.start;
                            list_wb.update(cx, |wb, cx| {
                                wb.set_tab_view(Tab::Dataset, |v| v.offset = offset, cx)
                            });
                        }
                    }
                    range
                        .map(|i| {
                            let n = i;
                            let Some(row) = i
                                .checked_sub(display.offset)
                                .and_then(|n| display.rows.get(n))
                            else {
                                return div()
                                    .w_full()
                                    .h(theme::row_height(Density::default()) * 2.)
                                    .into_any_element();
                            };
                            let text = if display.preview {
                                let get = |key: &str| {
                                    display
                                        .columns
                                        .iter()
                                        .position(|c| c.key == key)
                                        .and_then(|n| row.cells.get(n))
                                        .map(String::as_str)
                                        .unwrap_or("")
                                };
                                format!(
                                    "{} · {} · {} · {} · {} {} · {} {}",
                                    get("paper_id"),
                                    get("condition_id"),
                                    get("variable"),
                                    get("entity_canonical"),
                                    get("value_original"),
                                    get("unit_original"),
                                    get("value_canonical"),
                                    get("unit_canonical")
                                )
                            } else {
                                row_text(row, lang)
                            };
                            let mut links = div().flex().gap(gap);
                            if let Some(evidence) = row.evidence.first().cloned() {
                                let wb = list_wb.clone();
                                links = links.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "research-dataset-evidence-{n}"
                                        )))
                                        .test_support()
                                        .cursor_pointer()
                                        .child(evidence.clone().test_text(evidence.clone()))
                                        .on_click(move |_, _, cx| {
                                            wb.update(cx, |wb, cx| wb.open_evidence(&evidence, cx))
                                        }),
                                );
                            } else {
                                links = links.child(tr(lang, "no evidence recorded"));
                            }
                            if !display.preview {
                                links = links.child(actions::spot_check_buttons(
                                    &list_actions,
                                    &row.id,
                                    Some(&row.layer),
                                    cx,
                                ));
                                let target = row.id.clone();
                                let actions = list_actions.clone();
                                links = links.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "research-dataset-correct-{n}"
                                        )))
                                        .test_support()
                                        .cursor_pointer()
                                        .child(tr(lang, "Correct").test_text(tr(lang, "Correct")))
                                        .on_click(move |_, _, cx| {
                                            actions.update(cx, |a, cx| {
                                                a.begin_correction(&target, "value_original", cx)
                                            })
                                        }),
                                );
                            }
                            div()
                                .id(SharedString::from(format!("research-dataset-row-{n}")))
                                .test_support()
                                .flex()
                                .flex_col()
                                .h(theme::row_height(Density::default()) * 2.)
                                .child(text.clone().test_text(text))
                                .child(links)
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&self.scroll)
            .h(theme::row_height(Density::default()) * 5.);
            results = results.child(list);
            if !display.preview {
                for (id, label, target, enabled) in [
                    (
                        "previous",
                        "Previous",
                        display.offset.saturating_sub(PAGE),
                        display.offset > 0,
                    ),
                    (
                        "next",
                        "Next",
                        display.offset.saturating_add(PAGE),
                        display.offset.saturating_add(PAGE) < display.total,
                    ),
                ] {
                    if enabled {
                        let wb = self.wb.clone();
                        let scroll = self.scroll.clone();
                        let restoring = self.restoring.clone();
                        results = results.child(
                            div()
                                .id(SharedString::from(format!("research-dataset-{id}")))
                                .test_support()
                                .cursor_pointer()
                                .child(tr(lang, label).test_text(tr(lang, label)))
                                .on_click(move |_, _, cx| {
                                    restoring.set(Some((target, false)));
                                    scroll.scroll_to_item_strict(target, ScrollStrategy::Top);
                                    wb.update(cx, |wb, cx| {
                                        wb.set_tab_view(Tab::Dataset, |v| v.offset = target, cx)
                                    })
                                }),
                        );
                    }
                }
            }
        }
        let available = window.viewport_size().width.as_f32();
        let default = available / 2.;
        let widths = self
            .sizes
            .clone()
            .filter(|s| s.len() == 2)
            .unwrap_or_else(|| vec![default, default]);
        let left = widths[0].clamp(0., available);
        let handle = cx.entity().clone();
        let split =
            SplitHandle::new("research-dataset-split", &colors).on_resize(move |delta, _, cx| {
                handle.update(cx, |this, cx| {
                    let left = (widths[0] + delta.as_f32()).clamp(0., available);
                    let next = vec![left, (available - left).max(0.)];
                    this.sizes = Some(next.clone());
                    if cx.try_global::<persist::LayoutState>().is_some()
                        && this.wb.read(cx).layout_persistence_safe()
                    {
                        persist::remember("research.dataset", next, cx);
                    }
                    cx.notify();
                })
            });
        panel = panel
            .child(layers)
            .child(
                div()
                    .flex()
                    .w_full()
                    .child(div().w(px(left)).flex_shrink_0().child(filters))
                    .child(
                        div()
                            .id("research-dataset-split-frame")
                            .test_support()
                            .child(split),
                    )
                    .child(div().flex_1().child(results)),
            )
            .child(actions::correction_form(&self.actions, window, cx))
            .child(actions::receipts(&self.actions, cx));
        panel
    }
}

impl Render for DatasetPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feed = self.wb.read(cx).dataset_feed.clone();
        let mut state = feed.snapshot().state;
        let active = crate::desktop::data::faults::active("research.dataset.panel", cx)
            == Some(crate::desktop::data::faults::Fault::Panic);
        if active && state == PanelState::Ready {
            self.panic_pending = true;
        }
        if self.panic_pending && !active {
            state = PanelState::Failed {
                reason: "panel rendering failed".into(),
            };
        }
        let handle = cx.entity().clone();
        crate::desktop::components::panel::Panel::new("research.dataset.panel", &state)
            .retry(cx.listener(|this, _, _, cx| {
                this.panic_pending = false;
                this.wb.read(cx).dataset_feed.retry();
                cx.notify();
            }))
            .content(move |window, cx| {
                handle.update(cx, |this, cx| {
                    this.render_content(window, cx).into_any_element()
                })
            })
    }
}
