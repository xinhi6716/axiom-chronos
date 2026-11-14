use super::ResearchSeams;
use crate::{
    base::{Failure, Res},
    desktop::i18n::{translate, Lang, Table},
    research_workspace::{
        dataset::{self, DataLayer, DatasetQuery},
        export::{self, Delivery, Exported, HandOff},
        index::Index,
        manifest::Located,
        repro::ReproStatus,
        session::TabView,
    },
    seams::Clock,
};
use std::{collections::BTreeSet, path::Path};
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
        "Vault and memory are not in this version",
        "此版本不包含知識庫與記憶功能",
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
                "Vault and memory are not in this version"
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
pub(crate) fn repro_text(status: &ReproStatus, lang: Lang) -> String {
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
