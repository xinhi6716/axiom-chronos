use super::*;
use crate::paper::{Bucket, Known, SourceTag, Sourced};
const TABLE: Table = &[
    ("Unknown", "未知"),
    ("Passes", "符合"),
    ("Needs check", "需確認"),
    ("Excluded", "已排除"),
    ("Choose", "選擇"),
    ("Chosen", "已選擇"),
    ("Publisher", "出版社"),
    ("Homepage", "首頁"),
    ("Access", "取用方式"),
    ("APC", "文章處理費"),
    ("Review weeks", "審查週數"),
    ("Works", "作品數"),
    ("Quartile", "分區"),
    ("Open access", "開放取用"),
    ("Hybrid", "混合取用"),
    ("Subscription", "訂閱制"),
    ("No fee", "不收費"),
    ("Met", "符合"),
    ("Unmet", "不符合"),
    ("Official page", "官方網站"),
    ("Program", "計畫"),
    ("Dataset", "資料集"),
    ("Person", "個人"),
];
fn tr(l: Lang, k: &'static str) -> &'static str {
    i18n::translate(l, TABLE, k)
}
fn req_status(lang: Lang, status: &crate::paper::ReqStatus) -> &'static str {
    tr(
        lang,
        match status {
            crate::paper::ReqStatus::Met => "Met",
            crate::paper::ReqStatus::Unmet => "Unmet",
            crate::paper::ReqStatus::Unknown => "Unknown",
        },
    )
}
fn source(s: &SourceTag, lang: Lang) -> String {
    use crate::paper::SourceKind::*;
    tr(
        lang,
        match s.kind {
            Openalex => "OpenAlex",
            Sjr => "SJR",
            Doaj => "DOAJ",
            Crossref => "Crossref",
            JcrImport => "JCR",
            OfficialPage => "Official page",
            Program => "Program",
            Dataset => "Dataset",
            Person => "Person",
        },
    )
    .to_owned()
}
fn line(label: &str, value: String, source: &str, date: &str) -> AnyElement {
    marked(
        "paper-metric",
        format!("{label}: {value} · {source} · {date}"),
    )
    .into_any_element()
}
fn metric<T>(
    label: &str,
    value: &Known<T>,
    lang: Lang,
    fmt: impl FnOnce(&T) -> String,
) -> AnyElement {
    match value {
        Known::Known(Sourced { value, source: s }) => {
            line(label, fmt(value), &source(s, lang), &s.fetched_at)
        }
        Known::Unknown { reason } => line(
            label,
            format!("{} ({reason})", tr(lang, "Unknown")),
            tr(lang, "Unknown"),
            tr(lang, "Unknown"),
        ),
    }
}
fn journal(j: &crate::paper::JournalRecord, lang: Lang) -> Vec<AnyElement> {
    let mut m = vec![
        metric(tr(lang, "Publisher"), &j.publisher, lang, Clone::clone),
        metric(tr(lang, "Homepage"), &j.homepage, lang, Clone::clone),
        metric("SJR", &j.sjr, lang, |v| v.to_string()),
        metric(tr(lang, "Access"), &j.access, lang, |v| {
            tr(
                lang,
                match v {
                    crate::paper::Access::OpenAccess => "Open access",
                    crate::paper::Access::Hybrid => "Hybrid",
                    crate::paper::Access::Subscription => "Subscription",
                },
            )
            .to_owned()
        }),
        metric(tr(lang, "APC"), &j.apc, lang, |v| match v {
            crate::paper::Apc::NoFee => tr(lang, "No fee").to_owned(),
            crate::paper::Apc::Fee(m) => format!("{} {}", m.amount_minor, m.currency),
        }),
        metric(tr(lang, "Review weeks"), &j.review_weeks, lang, |v| {
            v.to_string()
        }),
        metric(tr(lang, "Works"), &j.works_count, lang, |v| v.to_string()),
    ];
    if j.quartiles.is_empty() {
        m.push(line(
            tr(lang, "Quartile"),
            tr(lang, "Unknown").into(),
            tr(lang, "Unknown"),
            tr(lang, "Unknown"),
        ));
    }
    for q in &j.quartiles {
        m.push(line(
            tr(lang, "Quartile"),
            q.value.label(),
            &source(&q.source, lang),
            &q.source.fetched_at,
        ));
    }
    m
}
pub fn render(
    status: Option<&PaperStatus>,
    lang: Lang,
    cx: &mut Context<PaperPage>,
) -> Vec<AnyElement> {
    let Some(list) = status.and_then(|s| s.shortlist.as_ref()) else {
        return vec![];
    };
    let choice = status.and_then(|s| s.choice.as_ref());
    let mut rows = Vec::new();
    for (i, r) in list.ranked.iter().enumerate() {
        let id = r.journal.id.clone();
        let hash = list.hash.clone();
        let chosen = is_chosen(choice, &id, &hash);
        let mut row = v_flex().id(("paper-row-slot", i)).child(
            v_flex()
                .id("paper-row")
                .test_support()
                .child(r.journal.title.clone().test_text(r.journal.title.clone()))
                .child({
                    let bucket = tr(
                        lang,
                        match r.bucket {
                            Bucket::Passes => "Passes",
                            Bucket::NeedsCheck => "Needs check",
                        },
                    );
                    bucket.test_text(bucket)
                })
                .when(chosen, |e| {
                    e.child(tr(lang, "Chosen").test_text(tr(lang, "Chosen")))
                })
                .children(journal(&r.journal, lang)),
        );
        for f in &r.filters {
            row = row.child(format!(
                "{}: {} · {}",
                f.name,
                req_status(lang, &f.status),
                f.detail
            ));
        }
        for f in &r.free_text {
            row = row.child(format!(
                "{}: {} · {}",
                f.text,
                req_status(lang, &f.status),
                f.detail
            ));
        }
        row = row.child(
            Button::new(("paper-choose", i))
                .label(tr(lang, "Choose"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choose(id.clone(), hash.clone(), cx);
                })),
        );
        rows.push(row.into_any_element());
    }
    for (i, r) in list.excluded.iter().enumerate() {
        let title = format!("{} · {}", tr(lang, "Excluded"), r.journal.title);
        let mut row = v_flex()
            .id(("paper-excluded", i))
            .test_support()
            .child(title.clone().test_text(title));
        for f in &r.filters {
            let detail = format!("{}: {} · {}", f.name, req_status(lang, &f.status), f.detail);
            row = row.child(detail.clone().test_text(detail));
        }
        rows.push(row.children(journal(&r.journal, lang)).into_any_element());
    }
    rows
}
