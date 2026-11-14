use super::*;
const TABLE: Table = &[
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
    ("Unknown", "未知"),
    ("Open access", "開放取用"),
    ("Hybrid", "混合取用"),
    ("Subscription", "訂閱制"),
    ("No fee", "不收費"),
    ("Official page", "官方網站"),
    ("Met", "符合"),
    ("Unmet", "不符合"),
    ("Program", "計畫"),
    ("Dataset", "資料集"),
    ("Person", "個人"),
];
fn label<'a>(lang: Lang, key: &'a str) -> &'a str {
    TABLE
        .iter()
        .find(|(en, _)| *en == key)
        .map(|(en, zh)| if lang == Lang::En { *en } else { *zh })
        .unwrap_or(key)
}
pub fn render(
    status: Option<&PaperView>,
    lang: Lang,
    cx: &mut Context<PaperPage>,
) -> Vec<AnyElement> {
    let Some(status) = status else { return vec![] };
    let t = cx.tokens();
    let caption = crate::desktop::theme::font(
        crate::desktop::theme::density(cx),
        crate::desktop::theme::TypeSize::Caption,
    );
    let first_choice = status.shortlist.iter().position(|row| !row.excluded);
    status
        .shortlist
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let chosen = is_chosen(status.journal.as_ref(), &r.id, &r.hash);
            let title = if r.excluded {
                format!("{} · {}", label(lang, "Excluded"), r.title)
            } else {
                r.title.clone()
            };
            let body = {
                let mut body = v_flex()
                    .text_size(caption)
                    .line_height(caption * 1.2)
                    .id(if r.excluded {
                        "paper-excluded"
                    } else {
                        "paper-row"
                    })
                    .test_support()
                    .child(title.clone().test_text(title))
                    .when(!r.excluded, |e| {
                        let text = label(lang, &r.bucket).to_string();
                        e.child(text.clone().test_text(text))
                    })
                    .when(chosen, |e| {
                        let text = label(lang, "Chosen").to_string();
                        e.child(text.clone().test_text(text))
                    });
                for f in &r.filters {
                    let detail = format!("{}: {} · {}", f.name, label(lang, &f.status), f.detail);
                    body = body.child(detail.clone().test_text(detail));
                }
                let mut metrics = v_flex().min_w_0();
                for m in &r.metrics {
                    let value = if let Some(reason) = m.value.strip_prefix("Unknown (") {
                        format!("{} ({reason}", label(lang, "Unknown"))
                    } else {
                        label(lang, &m.value).to_string()
                    };
                    metrics = metrics.child(marked(
                        "paper-metric",
                        format!(
                            "{}: {} · {} · {}",
                            label(lang, &m.label),
                            value,
                            label(lang, &m.source),
                            label(lang, &m.date)
                        ),
                    ));
                }
                body.child(metrics)
            };
            let mut row = v_flex().id(("paper-row-slot", i));
            if !r.excluded {
                let id = r.id.clone();
                let hash = r.hash.clone();
                let button =
                    Button::new(("paper-choose", i), label(lang, "Choose").to_string(), &t)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose(id.clone(), hash.clone(), cx)
                        }));
                row = if Some(i) == first_choice {
                    row.child(
                        div()
                            .id("action:paper.shortlist.choose")
                            .test_support()
                            .child(button),
                    )
                } else {
                    row.child(button)
                };
            }
            row.child(body).into_any_element()
        })
        .collect()
}
