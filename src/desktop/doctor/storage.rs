use super::super::{
    i18n::{fill, translate, Lang, Table},
    model::AppState,
    theme::{self, Density, Step, Tokened as _, Tokens, TypeSize},
    ux::seams::{CleanupPlan, CleanupProgress, DiskSpace, StorageItem, StorageService},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{button::Button, h_flex, v_flex, Selectable as _},
    *,
};
use std::{collections::BTreeMap, rc::Rc, sync::Arc, time::Duration};

const T: Table = &[
    ("Storage", "儲存空間"),
    ("Category", "類別"),
    ("Project", "專案"),
    ("Wish", "願望"),
    ("Unassigned", "未指定"),
    ("Review cleanup", "檢視清理項目"),
    ("Cancel", "取消"),
    ("Back", "返回"),
    ("Delete {} items ({})", "刪除 {} 項（{}）"),
    ("Low disk: {} free", "磁碟空間不足：剩 {}"),
    ("{} remaining", "剩餘 {} 項"),
    ("Cleanup complete", "清理完成"),
    ("{} total, {} reclaimable", "共 {}，可清理 {}"),
    ("{} B", "{} 位元組"),
    ("{} KiB", "{} KiB"),
    ("{} MiB", "{} MiB"),
    ("{} GiB", "{} GiB"),
    ("{} TiB", "{} TiB"),
];
pub const LOW_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const LOW_FREE_SHARE: f64 = 0.05;
pub fn is_low(space: &DiskSpace) -> bool {
    space.free_bytes < LOW_FREE_BYTES
        || (space.total_bytes != 0
            && u128::from(space.free_bytes) * 20 < u128::from(space.total_bytes))
}
pub fn bytes_text(n: u64, lang: Lang) -> String {
    let units = ["{} B", "{} KiB", "{} MiB", "{} GiB", "{} TiB"];
    let mut value = n as f64;
    let mut i = 0;
    while value >= 1024. && i < units.len() - 1 {
        value /= 1024.;
        i += 1;
    }
    let number = if i == 0 {
        n.to_string()
    } else {
        format!("{value:.1}")
    };
    fill(translate(lang, T, units[i]), &[&number])
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupBy {
    Category,
    Project,
    Wish,
}
pub fn group(items: &[StorageItem], by: GroupBy) -> Vec<(String, u64, u64)> {
    let mut groups: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for item in items {
        let key = match by {
            GroupBy::Category => Some(item.category.as_str()),
            GroupBy::Project => item.project.as_deref(),
            GroupBy::Wish => item.wish.as_deref(),
        }
        .unwrap_or("");
        let entry = groups.entry(key.to_owned()).or_default();
        entry.0 = entry.0.saturating_add(item.bytes);
        if item.reclaimable {
            entry.1 = entry.1.saturating_add(item.bytes);
        }
    }
    groups
        .into_iter()
        .map(|(k, (total, reclaimable))| (k, total, reclaimable))
        .collect()
}
#[derive(Clone, Debug, PartialEq)]
pub enum Flow {
    Idle,
    Selecting(Vec<String>),
    Previewed(CleanupPlan),
    Running {
        plan: CleanupPlan,
        job: String,
    },
    Finished {
        plan: CleanupPlan,
        progress: CleanupProgress,
    },
    Failed(String),
}
impl Flow {
    pub fn preview(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Selecting(ids) if !ids.is_empty() => match service.preview(&ids) {
                Ok(p) => Self::Previewed(p),
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
    pub fn confirm(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Previewed(plan) => match service.clean(&plan.id, plan.version, true) {
                Ok(job) => Self::Running { plan, job },
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
    pub fn poll(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Running { plan, job } => match service.progress(&job) {
                Ok(progress) if progress.finished => Self::Finished { plan, progress },
                Ok(_) => Self::Running { plan, job },
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
}
pub struct StoragePage {
    state: Entity<AppState>,
    service: Option<Arc<dyn StorageService>>,
    items: Vec<StorageItem>,
    by: GroupBy,
    flow: Flow,
    progress: Option<CleanupProgress>,
    timer: Option<Task<()>>,
}
impl StoragePage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let service = state.read(cx).ux.seams.storage.clone();
        let (items, flow) = match service.as_ref().map(|s| s.report()) {
            Some(Ok(r)) => (r.items, Flow::Idle),
            Some(Err(e)) => (vec![], Flow::Failed(e.to_string())),
            None => (vec![], Flow::Idle),
        };
        Self {
            state,
            service,
            items,
            by: GroupBy::Category,
            flow,
            progress: None,
            timer: None,
        }
    }
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let keep = this
                .update(cx, |this, cx| {
                    let Some(service) = this.service.clone() else {
                        return false;
                    };
                    let Flow::Running { plan, job } = &this.flow else {
                        return false;
                    };
                    match service.progress(job) {
                        Ok(p) => {
                            this.progress = Some(p.clone());
                            if p.finished {
                                this.flow = Flow::Finished {
                                    plan: plan.clone(),
                                    progress: p,
                                };
                            }
                        }
                        Err(e) => this.flow = Flow::Failed(e.to_string()),
                    }
                    cx.notify();
                    matches!(this.flow, Flow::Running { .. })
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }
}
fn text_element(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
        .into_any_element()
}
impl Render for StoragePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let d = theme::density(cx);
        let lang = *cx.global::<Lang>();
        let selected = match &self.flow {
            Flow::Selecting(ids) => ids.clone(),
            _ => vec![],
        };
        let can_select = matches!(self.flow, Flow::Idle | Flow::Selecting(_));
        let mut body = v_flex()
            .id("storage-page")
            .test_support()
            .size_full()
            .p(theme::space(d, Step::Xl))
            .gap(theme::space(d, Step::Md))
            .child(text_element(
                "storage-title",
                translate(lang, T, "Storage").into(),
            ))
            .child(
                Button::new("storage-back")
                    .accessibility_label(translate(lang, T, "Back"))
                    .child(translate(lang, T, "Back").test_text(translate(lang, T, "Back")))
                    .on_click({
                        let state = self.state.clone();
                        move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx))
                    }),
            );
        if can_select {
            let choices = [
                (GroupBy::Category, "Category"),
                (GroupBy::Project, "Project"),
                (GroupBy::Wish, "Wish"),
            ];
            body = body.child(
                h_flex()
                    .id("storage-group")
                    .test_support()
                    .gap(theme::space(d, Step::Sm))
                    .children(choices.into_iter().enumerate().map(|(i, (by, label))| {
                        Button::new(("storage-group-choice", i))
                            .accessibility_label(translate(lang, T, label))
                            .child(translate(lang, T, label).test_text(translate(lang, T, label)))
                            .selected(self.by == by)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.by = by;
                                cx.notify();
                            }))
                    })),
            );
            for (name, total, reclaimable) in group(&self.items, self.by) {
                let display = if name.is_empty() {
                    translate(lang, T, "Unassigned")
                } else {
                    &name
                };
                let summary = format!(
                    "{} — {}",
                    display,
                    fill(
                        translate(lang, T, "{} total, {} reclaimable"),
                        &[&bytes_text(total, lang), &bytes_text(reclaimable, lang)]
                    )
                );
                body = body.child(text_element("storage-row", summary));
                for (index, item) in self.items.iter().enumerate().filter(|(_, item)| {
                    (match self.by {
                        GroupBy::Category => item.category.as_str(),
                        GroupBy::Project => item.project.as_deref().unwrap_or(""),
                        GroupBy::Wish => item.wish.as_deref().unwrap_or(""),
                    }) == name
                }) {
                    let label = format!("{} — {}", item.path, bytes_text(item.bytes, lang));
                    if item.reclaimable {
                        let id = item.id.clone();
                        let checked = selected.contains(&id);
                        body = body.child(
                            Button::new(("storage-item", index))
                                .accessibility_label(label.clone())
                                .child(label.clone().test_text(label))
                                .selected(checked)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut ids = match &this.flow {
                                        Flow::Selecting(ids) => ids.clone(),
                                        _ => vec![],
                                    };
                                    if let Some(i) = ids.iter().position(|x| x == &id) {
                                        ids.remove(i);
                                    } else {
                                        ids.push(id.clone());
                                    }
                                    this.flow = if ids.is_empty() {
                                        Flow::Idle
                                    } else {
                                        Flow::Selecting(ids)
                                    };
                                    cx.notify();
                                })),
                        );
                    } else {
                        body = body.child(text_element("storage-item-fixed", label));
                    }
                }
            }
            if !selected.is_empty() {
                body = body.child(
                    Button::new("storage-preview")
                        .accessibility_label(translate(lang, T, "Review cleanup"))
                        .child(translate(lang, T, "Review cleanup").test_text(translate(
                            lang,
                            T,
                            "Review cleanup",
                        )))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(service) = &this.service {
                                this.flow = std::mem::replace(&mut this.flow, Flow::Idle)
                                    .preview(service.as_ref());
                            }
                            cx.notify();
                        })),
                );
            }
        }
        match &self.flow {
            Flow::Previewed(plan) => {
                let list = plan
                    .items
                    .iter()
                    .map(|i| format!("{} — {}", i.path, bytes_text(i.bytes, lang)))
                    .collect::<Vec<_>>()
                    .join("\n");
                let confirm = fill(
                    translate(lang, T, "Delete {} items ({})"),
                    &[&plan.items.len().to_string(), &bytes_text(plan.bytes, lang)],
                );
                body = body.child(text_element("storage-plan", list)).child(
                    Button::new("storage-confirm")
                        .accessibility_label(confirm.clone())
                        .child(confirm.clone().test_text(confirm))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(service) = &this.service {
                                this.flow = std::mem::replace(&mut this.flow, Flow::Idle)
                                    .confirm(service.as_ref());
                                if matches!(this.flow, Flow::Running { .. }) {
                                    this.progress = None;
                                    this.start_polling(cx);
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            Flow::Running { .. } => {
                let count = self.progress.as_ref().map_or_else(
                    || match &self.flow {
                        Flow::Running { plan, .. } => plan.items.len(),
                        _ => 0,
                    },
                    |p| p.remaining,
                );
                body = body.child(text_element(
                    "storage-progress",
                    fill(translate(lang, T, "{} remaining"), &[&count.to_string()]),
                ));
            }
            Flow::Finished { plan, progress } => {
                body = body.child(text_element(
                    "storage-progress",
                    translate(lang, T, "Cleanup complete").into(),
                ));
                if !progress.failed.is_empty() {
                    let failures = progress
                        .failed
                        .iter()
                        .map(|(id, reason)| {
                            let path = plan
                                .items
                                .iter()
                                .find(|i| &i.id == id)
                                .map_or(id.as_str(), |i| i.path.as_str());
                            format!("{path}: {reason}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    body = body.child(text_element("storage-failures", failures));
                }
            }
            Flow::Failed(error) => {
                body = body.child(text_element("storage-failures", error.clone()));
            }
            _ => {}
        }
        if matches!(
            self.flow,
            Flow::Previewed(_) | Flow::Failed(_) | Flow::Finished { .. }
        ) {
            body = body.child(
                Button::new("storage-cancel")
                    .accessibility_label(translate(lang, T, "Cancel"))
                    .child(translate(lang, T, "Cancel").test_text(translate(lang, T, "Cancel")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(service) = &this.service {
                            match service.report() {
                                Ok(r) => {
                                    this.items = r.items;
                                    this.flow = Flow::Idle;
                                }
                                Err(e) => this.flow = Flow::Failed(e.to_string()),
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        body.text_color(t.text)
            .text_size(theme::font(d, TypeSize::Body))
            .overflow_y_scroll()
    }
}
pub fn low_disk_notice(
    space: Option<DiskSpace>,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
) -> Option<AnyElement> {
    let space = space.filter(is_low)?;
    let message = fill(
        translate(lang, T, "Low disk: {} free"),
        &[&bytes_text(space.free_bytes, lang)],
    );
    Some(
        div()
            .id("low-disk")
            .test_support()
            .cursor_pointer()
            .text_color(t.fail)
            .text_size(theme::font(d, TypeSize::Body))
            .on_click(move |_, window, cx| on_open(window, cx))
            .child(message.clone().test_text(message))
            .into_any_element(),
    )
}
