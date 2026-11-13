use super::{ResearchSeams, Tab, Workbench};
use crate::{
    base::{Failure, Res},
    desktop::{
        i18n::{translate, Lang, Table},
        theme::Tokened as _,
    },
    research_workspace::{
        dataset::{self, DataLayer, DatasetQuery},
        export::{self, Delivery, Exported, HandOff},
        index::Index,
        manifest::Located,
        repro::{self, ReproStatus},
        session::TabView,
    },
    seams::Clock,
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        button::Button,
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    *,
};
use std::{collections::BTreeSet, path::Path, sync::Arc};

const T: Table = &[
    ("No research open", "尚未開啟研究"),
    ("Destination", "目的地"),
    ("Enter a destination", "請輸入目的地"),
    ("Export CSV", "匯出 CSV"),
    ("Export evidence bundle", "匯出證據包"),
    ("Freeze snapshot", "凍結快照"),
    (
        "No evidence in the current dataset filter",
        "目前資料集篩選條件中沒有證據",
    ),
    ("Research home is not configured", "尚未設定研究主目錄"),
    (
        "Paper writing is not connected yet, so the snapshot stays here for you to share",
        "論文寫作尚未連接，快照會留在此處供您分享",
    ),
    (
        "Vault is not connected yet, so the snapshot stays here for you to share",
        "知識庫尚未連接，快照會留在此處供您分享",
    ),
    ("Delivered", "已交付"),
    ("Delivery failed", "交付失敗"),
    ("Repro: Pass", "重現性：通過"),
    ("Repro: Fail", "重現性：失敗"),
    ("Repro: Not applicable", "重現性：不適用"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
pub(crate) fn delivery_text(d: &Delivery, seam: &str, lang: Lang) -> String {
    match d {
        Delivery::Delivered { to } => format!("{}: {to}", tr(lang, "Delivered")),
        Delivery::Unwired => tr(
            lang,
            if seam == "paper" {
                "Paper writing is not connected yet, so the snapshot stays here for you to share"
            } else {
                "Vault is not connected yet, so the snapshot stays here for you to share"
            },
        )
        .into(),
        Delivery::Failed { kind, message } => {
            format!("{}: {kind}: {message}", tr(lang, "Delivery failed"))
        }
    }
}
pub(crate) fn layer_of(view: &TabView) -> DataLayer {
    if view.node.as_deref() == Some("raw") {
        DataLayer::Raw
    } else {
        DataLayer::Extracted
    }
}
pub(crate) fn query(view: &TabView) -> DatasetQuery {
    DatasetQuery {
        layer: layer_of(view),
        filters: view.filters.clone(),
        sort: None,
        offset: 0,
        limit: 500,
    }
}
pub(crate) fn collect_evidence_ids(ix: &Index, q: &DatasetQuery) -> Res<Vec<String>> {
    let mut q = q.clone();
    q.offset = 0;
    q.limit = 500;
    let mut ids = BTreeSet::new();
    loop {
        let page = dataset::page(ix, &q)?;
        let count = page.rows.len();
        for row in page.rows {
            ids.extend(row.evidence.into_iter().filter(|id| !id.is_empty()));
        }
        q.offset += count;
        if q.offset >= page.total {
            break;
        }
        if count == 0 {
            return Err(Failure::new(
                "unavailable",
                "dataset paging made no progress",
            ));
        }
    }
    Ok(ids.into_iter().collect())
}
fn repro_text(status: &ReproStatus, lang: Lang) -> String {
    match status {
        ReproStatus::Pass => tr(lang, "Repro: Pass").into(),
        ReproStatus::Fail { overlap } => {
            format!("{}: {}", tr(lang, "Repro: Fail"), overlap.join(", "))
        }
        ReproStatus::NotApplicable { reason } => {
            let detail = if lang == Lang::ZhTw {
                if reason == "groups not declared" {
                    "未宣告群組".to_owned()
                } else if reason == "inventory missing or corrupt" {
                    "文獻清單遺失或損毀".to_owned()
                } else if let Some(id) = reason
                    .strip_prefix("artifact ")
                    .and_then(|s| s.strip_suffix(" missing or corrupt"))
                {
                    format!("項目 {id} 遺失或損毀")
                } else {
                    reason.clone()
                }
            } else {
                reason.clone()
            };
            format!("{}: {detail}", tr(lang, "Repro: Not applicable"))
        }
    }
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
#[derive(Clone, Copy)]
pub(crate) enum Action {
    Csv,
    Bundle,
    Snapshot,
}
pub(crate) enum Outcome {
    Export(Exported),
    Snapshot(Exported, HandOff),
    Empty,
    NoHome,
}

pub(crate) fn perform_action(
    action: Action,
    index: &Index,
    located: &Located,
    q: &DatasetQuery,
    seams: &ResearchSeams,
    clock: &dyn Clock,
    dest: &Path,
) -> Res<Outcome> {
    let dest = export::check_dest(located, dest)?;
    match action {
        Action::Csv => {
            let name = match q.layer {
                DataLayer::Raw => "dataset-raw.csv",
                DataLayer::Extracted => "dataset-extracted.csv",
            };
            Ok(Outcome::Export(export::export_csv(index, q, &dest, name)?))
        }
        Action::Bundle => {
            let ids = collect_evidence_ids(index, q)?;
            if ids.is_empty() {
                Ok(Outcome::Empty)
            } else {
                Ok(Outcome::Export(export::export_evidence_bundle(
                    index, &ids, &dest,
                )?))
            }
        }
        Action::Snapshot => {
            let Some(home) = &seams.research_home else {
                return Ok(Outcome::NoHome);
            };
            let (snapshot, dir) = export::snapshot(index, home, &dest, clock)?;
            let sha256 = crate::research_workspace::read::hash_file(&dir.join("snapshot.json"))?;
            let handoff = export::hand_off(
                &snapshot,
                &dir,
                seams.snapshot_consumer.as_deref(),
                seams.vault.as_deref(),
            );
            Ok(Outcome::Snapshot(
                Exported {
                    path: dir,
                    rows: snapshot.files.len(),
                    sha256,
                },
                handoff,
            ))
        }
    }
}

pub struct ExportsPanel {
    wb: Entity<Workbench>,
    dest: Entity<InputState>,
    index: Option<Arc<Index>>,
    repro: Option<ReproStatus>,
    results: Vec<Exported>,
    handoff: Option<HandOff>,
    error: Option<String>,
    busy: bool,
}
impl ExportsPanel {
    pub fn new(wb: Entity<Workbench>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dest = cx.new(|cx| InputState::new(window, cx));
        cx.observe(&wb, |this, wb, cx| {
            let index = wb.read(cx).index.clone();
            if match (&this.index, &index) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            } {
                return;
            }
            this.index = index.clone();
            this.repro = None;
            this.results.clear();
            this.handoff = None;
            this.error = None;
            if let Some(index) = index {
                let worker_index = index.clone();
                let task = cx.background_spawn(async move { repro::guard(&worker_index).status });
                cx.spawn(async move |this, cx| {
                    let status = task.await;
                    let _ = this.update(cx, |this, cx| {
                        if this
                            .index
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &index))
                        {
                            this.repro = Some(status);
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            cx.notify();
        })
        .detach();
        Self {
            wb,
            dest,
            index: None,
            repro: None,
            results: vec![],
            handoff: None,
            error: None,
            busy: false,
        }
    }
    fn run(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let dest = self.dest.read(cx).value().to_string();
        if dest.trim().is_empty() {
            self.error = Some(tr(*cx.global::<Lang>(), "Enter a destination").into());
            cx.notify();
            return;
        }
        let wb = self.wb.read(cx);
        let (Some(index), Some(located)) = (wb.index.clone(), wb.located.clone()) else {
            return;
        };
        let q = query(&wb.view(Tab::Dataset));
        let seams = wb.seams.clone();
        let clock = wb.clock.clone();
        let lang = *cx.global::<Lang>();
        self.busy = true;
        self.error = None;
        cx.notify();
        let task = cx.background_spawn(async move {
            perform_action(
                action,
                &index,
                &located,
                &q,
                &seams,
                clock.as_ref(),
                Path::new(dest.trim()),
            )
        });
        let active_index = self.index.clone();
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                let same = match (&this.index, &active_index) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                };
                if same {
                    match outcome {
                        Ok(Outcome::Export(result)) => this.results.push(result),
                        Ok(Outcome::Snapshot(result, handoff)) => {
                            this.results.push(result);
                            this.handoff = Some(handoff);
                        }
                        Ok(Outcome::Empty) => {
                            this.error =
                                Some(tr(lang, "No evidence in the current dataset filter").into())
                        }
                        Ok(Outcome::NoHome) => {
                            this.error = Some(tr(lang, "Research home is not configured").into())
                        }
                        Err(e) => this.error = Some(format!("{}: {}", e.kind, e.message)),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for ExportsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let mut panel = v_flex()
            .id("research-exports-panel")
            .test_support()
            .gap(px(6.))
            .text_color(colors.text);
        if self.wb.read(cx).index.is_none() {
            return panel
                .child(tr(lang, "No research open").test_text(tr(lang, "No research open")));
        }
        panel = panel
            .child(tr(lang, "Destination"))
            .child(Input::new(&self.dest).id("research-exports-dest"))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        Button::new("research-exports-csv")
                            .label(tr(lang, "Export CSV"))
                            .on_click(cx.listener(|this, _, _, cx| this.run(Action::Csv, cx))),
                    )
                    .child(
                        Button::new("research-exports-bundle")
                            .label(tr(lang, "Export evidence bundle"))
                            .on_click(cx.listener(|this, _, _, cx| this.run(Action::Bundle, cx))),
                    )
                    .child(
                        Button::new("research-exports-snapshot")
                            .label(tr(lang, "Freeze snapshot"))
                            .on_click(cx.listener(|this, _, _, cx| this.run(Action::Snapshot, cx))),
                    ),
            );
        if let Some(status) = &self.repro {
            panel = panel.child(marked("research-exports-repro", repro_text(status, lang)));
        }
        if let Some(error) = &self.error {
            panel = panel.child(marked("research-exports-error", error.clone()));
        }
        for (i, result) in self.results.iter().enumerate() {
            panel = panel.child(marked(
                SharedString::from(format!("research-exports-result-{i}")),
                format!(
                    "{} · {} · {}",
                    result.path.display(),
                    result.rows,
                    &result.sha256[..result.sha256.len().min(12)]
                ),
            ));
        }
        if let Some(handoff) = &self.handoff {
            panel = panel.child(marked(
                "research-exports-handoff",
                format!(
                    "{} · {}",
                    delivery_text(&handoff.consumer, "paper", lang),
                    delivery_text(&handoff.vault, "vault", lang)
                ),
            ));
        }
        panel
    }
}
