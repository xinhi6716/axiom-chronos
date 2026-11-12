use super::{
    actions::{self, Actions},
    LoadState, Tab, Workbench,
};
use crate::{
    base::Res,
    desktop::{
        i18n::{translate, Lang, Table},
        theme::Tokened as _,
    },
    research_workspace::{
        dataset::{
            self, Cell, Conversion, DataLayer, DatasetPage, DatasetQuery, DatasetRow, MissingRate,
            Validation,
        },
        index::{Filter, Index, Op},
        session::TabView,
    },
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex, v_virtual_list, VirtualListScrollHandle,
    },
    *,
};
use std::{rc::Rc, sync::Arc};

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
    ("empty", "空值"),
    ("not convertible", "無法轉換"),
    ("no grade", "無評級"),
    ("no evidence recorded", "未記錄證據"),
    ("Correct", "修正"),
    ("Valid", "有效"),
    ("Invalid", "無效"),
    ("Stale", "已過期"),
    ("Not run", "尚未執行"),
    ("errors", "個錯誤"),
    ("Original", "原始值"),
    ("Canonical", "標準值"),
    ("Grade", "評級"),
    ("Flags", "旗標"),
    ("No missing-rate columns", "沒有缺失率欄位"),
    (
        "Indexing in progress; showing the first page only.",
        "正在建立索引；目前僅顯示第一頁。",
    ),
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
pub(crate) fn cell_text(c: &Cell, lang: Lang) -> String {
    match c {
        Cell::Value(v) => v.clone(),
        Cell::Empty { reason } => match reason {
            Some(r) if !r.is_empty() => format!("{}: {r}", tr(lang, "empty")),
            _ => tr(lang, "empty").into(),
        },
    }
}
fn prefix(s: &str) -> String {
    s.chars().take(12).collect()
}
pub(crate) fn validation_text(v: &Validation, lang: Lang) -> String {
    match v {
        Validation::Valid { data_sha256 } => {
            format!("{}: {}", tr(lang, "Valid"), prefix(data_sha256))
        }
        Validation::Invalid {
            errors,
            data_sha256,
        } => format!(
            "{}: {errors} {} · {}",
            tr(lang, "Invalid"),
            tr(lang, "errors"),
            prefix(data_sha256)
        ),
        Validation::Stale { recorded, current } => format!(
            "{}: {} → {}",
            tr(lang, "Stale"),
            prefix(recorded),
            prefix(current)
        ),
        Validation::NotRun => tr(lang, "Not run").into(),
    }
}
pub(crate) fn layer_of(v: &TabView) -> DataLayer {
    if v.node.as_deref() == Some("raw") {
        DataLayer::Raw
    } else {
        DataLayer::Extracted
    }
}
pub(crate) fn load_page(
    ix: &Index,
    layer: DataLayer,
    filters: &[Filter],
    offset: usize,
) -> Res<(DatasetPage, Validation, Vec<MissingRate>)> {
    let filters = filters
        .iter()
        .cloned()
        .map(|mut f| {
            f.column = match f.column.as_str() {
                "entity" => "entity_canonical",
                "paper_or_source" if layer == DataLayer::Raw => "source_file",
                "paper_or_source" => "paper_id",
                _ => &f.column,
            }
            .to_owned();
            f
        })
        .collect();
    let page = dataset::page(
        ix,
        &DatasetQuery {
            layer,
            filters,
            sort: None,
            offset,
            limit: PAGE,
        },
    )?;
    Ok((
        page,
        dataset::validation(ix),
        dataset::missing_rates(ix, layer)?,
    ))
}
pub(crate) fn filter_value(v: &TabView, column: &str) -> String {
    v.filters
        .iter()
        .find_map(|f| {
            if f.column == column {
                if let Op::Contains(s) = &f.op {
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
pub(crate) fn row_text(r: &DatasetRow, lang: Lang) -> String {
    let conversion = match &r.conversion {
        Conversion::NotConvertible { reason } => {
            format!(" · {}: {reason}", tr(lang, "not convertible"))
        }
        Conversion::Converted { rule } => format!(" · {rule}"),
        _ => String::new(),
    };
    format!(
        "{} · {} · {} · {} · {}: {} {} · {}: {} {}{} · {}: {} · {}: {}",
        r.paper_or_source,
        r.condition_id,
        r.variable,
        r.entity,
        tr(lang, "Original"),
        cell_text(&r.original, lang),
        r.unit_original,
        tr(lang, "Canonical"),
        cell_text(&r.canonical, lang),
        r.unit_canonical,
        conversion,
        tr(lang, "Grade"),
        r.quality_grade.as_deref().unwrap_or(tr(lang, "no grade")),
        tr(lang, "Flags"),
        r.flags.join(", ")
    )
}

pub struct DatasetPanel {
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
    inputs: Vec<Entity<InputState>>,
    index: Option<Arc<Index>>,
    view: TabView,
    generation: u64,
    pages: Vec<DatasetPage>,
    total: usize,
    validation: Option<Validation>,
    missing: Vec<MissingRate>,
    error: Option<String>,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: VirtualListScrollHandle,
    loading: Option<usize>,
    restoring: bool,
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
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        if value == filter_value(&this.view, COLUMNS[i]) {
                            return;
                        }
                        this.wb.update(cx, |wb, cx| {
                            wb.set_view(
                                Tab::Dataset,
                                |v| {
                                    v.filters.retain(|f| f.column != COLUMNS[i]);
                                    if !value.is_empty() {
                                        v.filters.push(Filter {
                                            column: COLUMNS[i].into(),
                                            op: Op::Contains(value),
                                        });
                                    }
                                    v.offset = 0;
                                },
                                cx,
                            )
                        });
                    }
                },
            )
            .detach();
        }
        cx.observe_in(&wb, window, |this, _, window, cx| this.sync(window, cx))
            .detach();
        let mut this = Self {
            wb,
            actions,
            inputs,
            index: None,
            view: TabView::default(),
            generation: 0,
            pages: vec![],
            total: 0,
            validation: None,
            missing: vec![],
            error: None,
            sizes: Rc::default(),
            scroll: VirtualListScrollHandle::new(),
            loading: None,
            restoring: false,
        };
        this.sync(window, cx);
        this
    }
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wb = self.wb.read(cx);
        let index = wb.index.clone();
        let view = wb.view(Tab::Dataset);
        let same = match (&self.index, &index) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let query_changed =
            !same || self.view.filters != view.filters || layer_of(&self.view) != layer_of(&view);
        let offset_changed = self.view.offset != view.offset;
        if !query_changed && !offset_changed {
            return;
        }
        self.index = index;
        self.view = view;
        for (i, input) in self.inputs.iter().enumerate() {
            let value = filter_value(&self.view, COLUMNS[i]);
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
        if query_changed {
            self.generation = self.generation.wrapping_add(1);
            self.pages.clear();
            self.total = 0;
            self.sizes = Rc::default();
            self.error = None;
            self.validation = None;
            self.missing.clear();
            self.loading = None;
            self.restoring = true;
            self.request(self.view.offset / PAGE * PAGE, cx);
        }
        if query_changed || offset_changed {
            self.scroll
                .set_offset(point(px(0.), px(-(self.view.offset as f32) * 84.)));
        }
        cx.notify();
    }
    fn request(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.loading == Some(offset) || self.pages.iter().any(|p| p.offset == offset) {
            return;
        }
        let Some(index) = self.index.clone() else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loading = Some(offset);
        let layer = layer_of(&self.view);
        let filters = self.view.filters.clone();
        let task = cx.background_spawn(async move { load_page(&index, layer, &filters, offset) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = None;
                match result {
                    Ok((page, validation, missing)) => {
                        this.total = page.total;
                        this.sizes = Rc::new(vec![size(px(1.), px(84.)); page.total]);
                        this.pages
                            .retain(|p| p.offset.abs_diff(page.offset) <= PAGE);
                        this.pages.push(page);
                        this.validation = Some(validation);
                        this.missing = missing;
                        this.error = None;
                        if this.restoring {
                            let offset = this.view.offset.min(this.total.saturating_sub(1));
                            this.scroll
                                .set_offset(point(px(0.), px(-(offset as f32) * 84.)));
                            this.restoring = false;
                        }
                    }
                    Err(e) => {
                        this.pages.clear();
                        this.total = 0;
                        this.sizes = Rc::default();
                        this.error = Some(format!("{}: {}", e.kind, e.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn render_row(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let lang = *cx.global::<Lang>();
        let Some(row) = self
            .pages
            .iter()
            .find_map(|p| i.checked_sub(p.offset).and_then(|n| p.rows.get(n)))
            .cloned()
        else {
            return div().h(px(84.)).into_any_element();
        };
        let id = SharedString::from(format!("research-dataset-row-{i}"));
        let target = row.record_id.clone();
        let a = self.actions.clone();
        let correct = tr(lang, "Correct").to_owned();
        let mut links = h_flex().gap(px(8.));
        if let Some(evidence) = row.evidence.first().cloned() {
            let wb = self.wb.clone();
            links = links.child(
                div()
                    .id(SharedString::from(format!("research-dataset-evidence-{i}")))
                    .test_support()
                    .cursor_pointer()
                    .child(evidence.clone().test_text(evidence.clone()))
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| wb.open_evidence(&evidence, cx))
                    }),
            );
        } else {
            let text = tr(lang, "no evidence recorded");
            links = links.child(text.test_text(text));
        }
        links = links
            .child(actions::spot_check_buttons(
                &self.actions,
                &row.record_id,
                Some(if row.layer == DataLayer::Raw {
                    "raw"
                } else {
                    "extracted"
                }),
                cx,
            ))
            .child(
                div()
                    .id(SharedString::from(format!("research-dataset-correct-{i}")))
                    .test_support()
                    .cursor_pointer()
                    .child(correct.clone().test_text(correct))
                    .on_click(move |_, _, cx| {
                        a.update(cx, |a, cx| {
                            a.begin_correction(&target, "value_original", cx)
                        })
                    }),
            );
        v_flex()
            .id(id)
            .test_support()
            .h(px(84.))
            .child(row_text(&row, lang).clone().test_text(row_text(&row, lang)))
            .child(links)
            .into_any_element()
    }
}
impl Render for DatasetPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let mut panel = v_flex()
            .id("research-dataset-panel")
            .test_support()
            .gap(px(4.))
            .text_color(colors.text)
            .w_full();
        let wb = self.wb.read(cx);
        let indexing = matches!(wb.load, LoadState::Indexing { .. });
        let view = wb.view(Tab::Dataset);
        let showing_first = matches!(wb.load, LoadState::Ready)
            && self.pages.is_empty()
            && self.error.is_none()
            && view.filters.is_empty()
            && view.offset == 0
            && (layer_of(&view) == DataLayer::Raw) == wb.preview_raw;
        if indexing || showing_first {
            let rows = wb.preview.clone();
            let raw = layer_of(&view) == DataLayer::Raw;
            let sizes = Rc::new(vec![size(px(1.), px(84.)); rows.len()]);
            let list = v_virtual_list(
                cx.entity(),
                "research-dataset-preview-rows",
                sizes,
                move |_, range, _, _| {
                    range
                        .map(|i| {
                            let fields = &rows[i].fields;
                            let get = |key: &str| fields.get(key).map(String::as_str).unwrap_or("");
                            let entity = if get("entity_canonical").is_empty() {
                                get("entity_raw")
                            } else {
                                get("entity_canonical")
                            };
                            let source = if raw {
                                get("source_file")
                            } else {
                                get("paper_id")
                            };
                            let text = format!(
                                "{} · {} · {} · {} · {}: {} {} · {}: {} {}",
                                source,
                                get("condition_id"),
                                get("variable"),
                                entity,
                                tr(lang, "Original"),
                                get("value_original"),
                                get("unit_original"),
                                tr(lang, "Canonical"),
                                get("value_canonical"),
                                get("unit_canonical")
                            );
                            div()
                                .id(SharedString::from(format!("research-dataset-row-{i}")))
                                .test_support()
                                .h(px(84.))
                                .child(text.clone().test_text(text))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            );
            let notice = tr(lang, "Indexing in progress; showing the first page only.");
            return panel
                .children(
                    indexing.then(|| marked("research-dataset-preview-notice", notice.into())),
                )
                .child(div().h(px(200.)).w_full().child(list));
        }
        if self.wb.read(cx).index.is_none() {
            let text = tr(lang, "No research open");
            return panel.child(text.test_text(text));
        }
        let mut layers = h_flex().gap(px(12.));
        for (key, label) in [("extracted", "Extracted"), ("raw", "Raw")] {
            let wb = self.wb.clone();
            let text = tr(lang, label).to_owned();
            layers = layers.child(
                div()
                    .id(SharedString::from(format!("research-dataset-layer-{key}")))
                    .test_support()
                    .cursor_pointer()
                    .child(text.clone().test_text(text))
                    .on_click(move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_view(
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
        let mut filters = h_flex().gap(px(4.));
        for (i, column) in COLUMNS.iter().enumerate() {
            filters = filters.child(v_flex().flex_1().min_w_0().child(tr(lang, column)).child(
                Input::new(&self.inputs[i]).id(SharedString::from(format!(
                    "research-dataset-filter-{column}"
                ))),
            ));
        }
        panel = panel.child(layers).child(filters);
        if let Some(v) = &self.validation {
            panel = panel.child(marked(
                "research-dataset-validation",
                validation_text(v, lang),
            ));
        }
        if self.validation.is_some() {
            panel = panel.child(marked(
                "research-dataset-missing",
                if self.missing.is_empty() {
                    tr(lang, "No missing-rate columns").to_owned()
                } else {
                    self.missing
                        .iter()
                        .map(|m| format!("{} {}/{}", m.column, m.missing, m.total))
                        .collect::<Vec<_>>()
                        .join(" · ")
                },
            ));
        }
        if let Some(error) = &self.error {
            panel = panel.child(marked("research-dataset-error", error.clone()));
        }
        let list = v_virtual_list(
            cx.entity(),
            "research-dataset-rows",
            self.sizes.clone(),
            |this, range, _, cx| {
                // The list also calls us for item 0 during layout measurement.
                if !range.is_empty() && range != (0..1) {
                    let start = range.start / PAGE * PAGE;
                    let end = (range.end - 1) / PAGE * PAGE;
                    if !this.pages.iter().any(|p| p.offset == start) {
                        this.request(start, cx);
                    } else if !this.pages.iter().any(|p| p.offset == end) {
                        this.request(end, cx);
                    }
                    if this.view.offset != range.start && !this.restoring {
                        let offset = range.start;
                        this.view.offset = offset;
                        this.wb.update(cx, |wb, cx| {
                            wb.set_view(Tab::Dataset, |v| v.offset = offset, cx)
                        });
                    }
                }
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll);
        panel
            .child(div().h(px(200.)).w_full().child(list))
            .child(actions::correction_form(&self.actions, window, cx))
            .child(actions::receipts(&self.actions, cx))
    }
}
