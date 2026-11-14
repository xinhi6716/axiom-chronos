use crate::{
    base::Res,
    desktop::i18n::{translate, Lang, Table},
    research_workspace::{
        dataset::{
            self, Cell, Conversion, DataLayer, DatasetPage, DatasetQuery, DatasetRow, MissingRate,
            Validation,
        },
        index::{Filter, Index},
    },
};
const PAGE: usize = 200;
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
