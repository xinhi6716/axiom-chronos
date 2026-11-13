use super::*;
const TABLE: Table = &[
    ("Added", "新增"),
    ("Removed", "移除"),
    ("Edited", "編輯"),
    ("Moved", "移動"),
    ("Style", "樣式"),
];
pub fn render(status: Option<&PaperStatus>, lang: Lang) -> Vec<AnyElement> {
    status
        .into_iter()
        .flat_map(|s| s.passes.iter())
        .enumerate()
        .map(|(i, p)| {
            let mut log = v_flex().id(("paper-changelog-slot", i)).child(format!(
                "{} · {}",
                p.pass.number(),
                p.member
            ));
            for c in &p.change_log {
                log = log.child(marked(
                    "paper-changelog",
                    format!(
                        "{} · {} · {}",
                        c.location,
                        i18n::translate(
                            lang,
                            TABLE,
                            match c.kind {
                                crate::paper::ChangeKind::Added => "Added",
                                crate::paper::ChangeKind::Removed => "Removed",
                                crate::paper::ChangeKind::Edited => "Edited",
                                crate::paper::ChangeKind::Moved => "Moved",
                                crate::paper::ChangeKind::Style => "Style",
                            }
                        ),
                        c.reason
                    ),
                ));
            }
            for v in &p.validation {
                log = log.child(marked("paper-validation", v.clone()));
            }
            log.into_any_element()
        })
        .collect()
}
