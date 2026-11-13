use super::{
    actions::{self, Actions},
    Tab, Workbench,
};
use crate::{
    base::{Failure, Res},
    desktop::{
        i18n::{translate, Lang, Table},
        theme::Tokened as _,
    },
    research_workspace::{
        dataset,
        evidence::{self, EntityGroup, Layer, Resolved, Target},
        index::Index,
        manifest::Capability,
        pdf::{self, PageCache},
        read::Limits,
        session::TabView,
    },
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{h_flex, v_flex},
    *,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
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
pub(crate) fn locator_text(r: &Resolved, lang: Lang) -> String {
    match &r.target {
        Target::PdfPage {
            physical_page,
            printed_page,
            highlight,
            ..
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
                    if highlight.is_some() {
                        "Region recorded; precise highlighting comes in a later version"
                    } else {
                        "No region recorded; showing the whole page"
                    }
                )
            )
        }
        Target::RawField {
            file, row, column, ..
        } => tr(lang, "{file}, row {row}, column {column}")
            .replace("{file}", file)
            .replace("{row}", &row.to_string())
            .replace("{column}", column),
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
fn layer(view: &TabView) -> Layer {
    if view.node.as_deref() == Some("raw") {
        Layer::Raw
    } else {
        Layer::Literature
    }
}
struct Loaded {
    names: Vec<String>,
    total: usize,
    group: Option<EntityGroup>,
    resolved: Option<Resolved>,
    image: Option<PathBuf>,
    error: Option<String>,
}
fn safe_home(home: &Path, ix: &Index) -> bool {
    // Resolve the nearest existing ancestor before creating a cache, including symlinks.
    if !home.is_absolute()
        || home
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let mut ancestor = home;
    while !ancestor.exists() {
        match ancestor.parent() {
            Some(p) => ancestor = p,
            None => return false,
        }
    }
    let Ok(base) = ancestor.canonicalize() else {
        return false;
    };
    let Ok(rest) = home.strip_prefix(ancestor) else {
        return false;
    };
    let target = base.join(rest);
    !ix.located().roots().iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|r| target.starts_with(&r) || r.starts_with(&target))
    })
}
fn load(
    ix: &Index,
    view: &TabView,
    entity: Option<String>,
    seams: &super::ResearchSeams,
) -> Res<Loaded> {
    let (names, total) = evidence::entities(ix, layer(view), view.offset, PAGE_SIZE)?;
    let chosen = entity
        .filter(|e| names.contains(e))
        .or_else(|| names.first().cloned());
    let group = chosen
        .map(|name| evidence::group(ix, &name, layer(view)))
        .transpose()?;
    let selected = view.selection.clone().or_else(|| {
        group.as_ref().and_then(|g| {
            g.conditions
                .iter()
                .flat_map(|c| &c.rows)
                .flat_map(|r| &r.evidence)
                .next()
                .cloned()
        })
    });
    let mut result = Loaded {
        names,
        total,
        group,
        resolved: None,
        image: None,
        error: None,
    };
    if let Some(id) = selected {
        match evidence::resolve(ix, &id, &Limits::default()) {
            Ok(resolved) => {
                if matches!(resolved.target, Target::PdfPage { .. }) {
                    let rendered = (|| {
                        let renderer = seams.renderer.as_ref().ok_or_else(|| {
                            Failure::new("renderer_missing", "renderer unavailable")
                        })?;
                        let home = seams
                            .research_home
                            .as_ref()
                            .ok_or_else(|| Failure::new("no_home", "research home unavailable"))?;
                        let cache_dir = home.join("cache/pages");
                        if !safe_home(&cache_dir, ix) {
                            return Err(Failure::new(
                                "invalid_argument",
                                "cache is inside a research root",
                            ));
                        }
                        let cache = PageCache::open(&cache_dir, seams.page_cache_bytes)?;
                        pdf::page_image(ix, renderer.as_ref(), &cache, &resolved, 1.0)
                    })();
                    match rendered {
                        Ok(image) => result.image = Some(image.path),
                        Err(e) => result.error = Some(e.kind),
                    }
                }
                result.resolved = Some(resolved);
            }
            Err(e) => result.error = Some(e.kind),
        }
    }
    Ok(result)
}
pub struct EvidencePanel {
    wb: Entity<Workbench>,
    actions: Entity<Actions>,
    index: Option<Arc<Index>>,
    view: TabView,
    entity: Option<String>,
    generation: u64,
    loaded: Option<Loaded>,
    error: Option<String>,
    busy: bool,
}
impl EvidencePanel {
    pub fn new(wb: Entity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let actions = cx.new(|cx| Actions::new(wb.clone(), cx));
        cx.observe(&actions, |_, _, cx| cx.notify()).detach();
        cx.observe(&wb, |this, _, cx| this.refresh(cx)).detach();
        let mut this = Self {
            wb,
            actions,
            index: None,
            view: TabView::default(),
            entity: None,
            generation: 0,
            loaded: None,
            error: None,
            busy: false,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let wb = self.wb.read(cx);
        let index = wb.index.clone();
        let view = wb.view(Tab::Evidence);
        let seams = wb.seams.clone();
        let same = match (&self.index, &index) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same && self.view == view {
            return;
        }
        if !same || self.view.node != view.node || self.view.offset != view.offset {
            self.entity = None;
        }
        self.index = index;
        self.view = view;
        self.start(seams, cx);
    }
    fn start(&mut self, seams: super::ResearchSeams, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.loaded = None;
        self.error = None;
        self.busy = false;
        let Some(ix) = self.index.clone() else {
            cx.notify();
            return;
        };
        let view = self.view.clone();
        let entity = self.entity.clone();
        self.busy = true;
        cx.notify();
        let task = cx.background_spawn(async move { load(&ix, &view, entity, &seams) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(loaded) => this.loaded = Some(loaded),
                    Err(e) => this.error = Some(e.kind),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn choose_entity(&mut self, entity: String, cx: &mut Context<Self>) {
        self.entity = Some(entity);
        let seams = self.wb.read(cx).seams.clone();
        self.wb.update(cx, |wb, cx| {
            wb.set_view(Tab::Evidence, |v| v.selection = None, cx)
        });
        self.start(seams, cx);
    }
}
impl Render for EvidencePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let mut panel = v_flex()
            .id("research-evidence-panel")
            .test_support()
            .w_full()
            .gap(px(4.))
            .text_color(colors.text);
        if self.index.is_none() {
            return panel.child(
                tr(lang, "No research open or indexing")
                    .test_text(tr(lang, "No research open or indexing")),
            );
        }
        let mut switches = h_flex().gap(px(8.));
        for (name, label, raw) in [("literature", "Literature", false), ("raw", "Raw", true)] {
            let wb = self.wb.clone();
            switches = switches.child(clickable(
                SharedString::from(format!("research-evidence-layer-{name}")),
                tr(lang, label).into(),
                move |_, _, cx| {
                    wb.update(cx, |wb, cx| {
                        wb.set_view(
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
        if self
            .index
            .as_ref()
            .is_some_and(|ix| !ix.located().has(Capability::PdfPages))
        {
            panel = panel.child(marked(
                "research-evidence-capability",
                tr(lang, "PDF pages are not available for this research").into(),
            ));
        }
        if self.busy {
            return panel.child(marked(
                "research-evidence-page",
                tr(lang, "Loading evidence").into(),
            ));
        }
        if let Some(error) = &self.error {
            return panel.child(marked(
                "research-evidence-degraded",
                degraded_text(error, lang),
            ));
        }
        let Some(loaded) = &self.loaded else {
            return panel.child(tr(lang, "No evidence"));
        };
        let mut entities = h_flex()
            .id("research-evidence-entities")
            .w_full()
            .overflow_x_scroll()
            .gap(px(8.));
        for (i, name) in loaded.names.iter().enumerate() {
            let name = name.clone();
            let label = name.clone();
            entities = entities.child(clickable(
                SharedString::from(format!("research-evidence-entity-{i}")),
                label,
                cx.listener(move |this, _, _, cx| this.choose_entity(name.clone(), cx)),
            ));
        }
        panel = panel.child(entities);
        let mut paging = h_flex().gap(px(8.));
        for (id, label, offset, enabled) in [
            (
                "previous",
                "Previous",
                self.view.offset.saturating_sub(PAGE_SIZE),
                self.view.offset > 0,
            ),
            (
                "next",
                "Next",
                self.view.offset.saturating_add(PAGE_SIZE),
                self.view.offset.saturating_add(PAGE_SIZE) < loaded.total,
            ),
        ] {
            if enabled {
                let wb = self.wb.clone();
                paging = paging.child(clickable(
                    SharedString::from(format!("research-evidence-{id}")),
                    tr(lang, label).into(),
                    move |_, _, cx| {
                        wb.update(cx, |wb, cx| {
                            wb.set_view(
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
        let mut condition_panel = v_flex()
            .id("research-evidence-conditions")
            .flex_1()
            .max_h(px(220.))
            .overflow_y_scroll();
        let mut contradiction_panel = v_flex()
            .id("research-evidence-contradictions")
            .flex_1()
            .max_h(px(220.))
            .overflow_y_scroll();
        if let Some(group) = &loaded.group {
            for (i, c) in group.contradictions.iter().enumerate() {
                let a = format!("{} {}", c.a.value_original, c.a.unit_original);
                let b = format!("{} {}", c.b.value_original, c.b.unit_original);
                contradiction_panel = contradiction_panel.child(
                    h_flex()
                        .id(SharedString::from(format!(
                            "research-evidence-contradiction-{i}"
                        )))
                        .test_support()
                        .gap(px(8.))
                        .child(
                            format!("{} · {}", c.condition_id, c.variable)
                                .test_text(format!("{} · {}", c.condition_id, c.variable)),
                        )
                        .child(a.clone().test_text(a))
                        .child(b.clone().test_text(b)),
                );
            }
            for (i, c) in group.conditions.iter().enumerate() {
                let mut condition = v_flex()
                    .id(SharedString::from(format!(
                        "research-evidence-condition-{i}"
                    )))
                    .test_support()
                    .child(c.condition_id.clone().test_text(c.condition_id.clone()));
                for row in &c.rows {
                    let mut line = h_flex().gap(px(8.)).child(format!(
                        "{}: {} {}",
                        row.variable, row.value_original, row.unit_original
                    ));
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
                    condition = condition.child(line);
                }
                condition_panel = condition_panel.child(condition);
            }
        }
        if let Some(r) = &loaded.resolved {
            panel = panel.child(marked("research-evidence-locator", locator_text(r, lang)));
            let id = r.item.evidence_id.clone();
            let wb = self.wb.clone();
            let selected = id.clone();
            panel = panel.child(clickable(
                "research-evidence-open-dataset",
                tr(lang, "Open in dataset").into(),
                move |_, _, cx| {
                    wb.update(cx, |wb, cx| {
                        wb.show_rows(vec![dataset::evidence_filter(&selected)], cx)
                    })
                },
            ));
            let actions = self.actions.clone();
            let target = r.item.record_id.clone();
            panel = panel.child(clickable(
                "research-evidence-correct",
                tr(lang, "Correct").into(),
                move |_, _, cx| {
                    actions.update(cx, |a, cx| a.begin_correction(&target, "quote", cx))
                },
            ));
        }
        if let Some(error) = &loaded.error {
            panel = panel.child(marked(
                "research-evidence-degraded",
                degraded_text(error, lang),
            ));
        }
        let page = if let Some(path) = &loaded.image {
            div()
                .id("research-evidence-page")
                .test_support()
                .child(img(path.clone()).w(px(150.)).h(px(200.)))
                .into_any_element()
        } else {
            let label = if loaded.error.is_some() {
                "Page unavailable"
            } else if loaded.resolved.is_some() {
                "No PDF page for this evidence"
            } else {
                "No page selected"
            };
            marked("research-evidence-page", tr(lang, label).into()).into_any_element()
        };
        panel
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(condition_panel)
                    .child(contradiction_panel)
                    .child(page),
            )
            .child(actions::correction_form(&self.actions, window, cx))
    }
}
