use super::*;
const TABLE: Table = &[
    ("Added", "新增"),
    ("Removed", "移除"),
    ("Edited", "編輯"),
    ("Moved", "移動"),
    ("Style", "樣式"),
];
pub fn render(status: Option<&PaperView>, lang: Lang) -> Vec<AnyElement> {
    status
        .into_iter()
        .flat_map(|s| &s.changelog)
        .enumerate()
        .map(|(i, c)| {
            let mut row = v_flex()
                .id(("paper-changelog-slot", i))
                .child(format!("{} · {}", c.pass, c.member));
            if c.validation {
                row = row.child(marked("paper-validation", c.reason.clone()));
            } else {
                let kind = TABLE
                    .iter()
                    .find(|(key, _)| *key == c.kind)
                    .map(|(en, zh)| if lang == Lang::En { *en } else { *zh })
                    .unwrap_or(&c.kind);
                row = row.child(marked(
                    "paper-changelog",
                    format!("{} · {} · {}", c.location, kind, c.reason),
                ));
            }
            row.into_any_element()
        })
        .collect()
}
