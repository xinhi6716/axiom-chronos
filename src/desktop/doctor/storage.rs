use super::super::{
    i18n::{fill, translate, Lang, Table},
    model::AppState,
    theme::{self, Density, Step, Tokened as _, Tokens, TypeSize},
    ux::seams::{CleanupPlan, CleanupProgress, DiskSpace, StorageItem, StorageService},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{h_flex, v_flex},
    *,
};
use super::super::components::{Button, ButtonVariant, VirtualList, VirtualListHandle};
use std::{collections::BTreeMap, rc::Rc, sync::Arc};

const T: Table = &[
    ("Storage", "儲存空間"),
    ("unknown", "未知"),
    ("Category", "類別"),
    ("Project", "專案"),
    ("Wish", "願望"),
    ("Unassigned", "未指定"),
    ("Review cleanup", "檢視清理項目"),
    ("Confirm cleanup", "確認清理"),
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
pub(crate) fn item_size_text(item: &StorageItem, lang: Lang) -> String {
    if item.measured {
        bytes_text(item.bytes, lang)
    } else {
        translate(lang, T, "unknown").into()
    }
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
        if item.measured {
            entry.0 = entry.0.saturating_add(item.bytes);
        }
        if item.measured && item.reclaimable {
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
    Busy,
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

pub struct StoragePage {
    state: Entity<AppState>,
    service: Option<Arc<dyn StorageService>>,
    feed: super::super::data::Feed<Vec<StorageItem>>,
    panic_pending: bool,
    rows: Vec<StorageRow>,
    list: VirtualListHandle,
    by: GroupBy,
    flow: Flow,
    progress: Option<CleanupProgress>,
    timer: Option<Task<()>>,
}
#[derive(Clone, PartialEq)]
enum StorageRow { Group(String), Item(usize, StorageItem) }
impl StoragePage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let service = state.read(cx).ux.seams.storage.clone();
        let feed = super::super::data::Feed::spawn("storage.page", super::adapter::StorageSource(service.clone()), super::super::components::POLL_INTERVAL, cx);
        feed.observe(cx);
        Self {
            state,
            service,
            feed,
            panic_pending: false,
            rows: Vec::new(),
            list: VirtualListHandle::uniform(0, f32::from(theme::row_height(theme::density(cx)))),
            by: GroupBy::Category,
            flow: Flow::Idle,
            progress: None,
            timer: None,
        }
    }
    fn run_action(&mut self, confirm: bool, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else { return; };
        let flow = std::mem::replace(&mut self.flow, Flow::Busy);
        cx.spawn(async move |this, cx| {
            let flow = cx.background_spawn(async move {
                if confirm { flow.confirm(service.as_ref()) } else { flow.preview(service.as_ref()) }
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.flow = flow;
                if matches!(this.flow, Flow::Running { .. }) {
                    this.progress = None;
                    this.start_polling(cx);
                }
                cx.notify();
            });
        }).detach();
    }
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else { return; };
        let Flow::Running { job, .. } = &self.flow else { return; };
        let job = job.clone();
        self.timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(super::super::components::Dur::Gentle.get()).await;
            let service = service.clone();
            let job = job.clone();
            let result = cx.background_spawn(async move { super::adapter::progress(service.as_ref(), &job) }).await;
            let keep = this.update(cx, |this, cx| {
                let Flow::Running { plan, .. } = &this.flow else { return false; };
                match result {
                    Ok(p) => {
                        this.progress = Some(p.clone());
                        if p.finished { this.flow = Flow::Finished { plan: plan.clone(), progress: p }; }
                    }
                    Err(e) => this.flow = Flow::Failed(e.to_string()),
                }
                cx.notify();
                matches!(this.flow, Flow::Running { .. })
            }).unwrap_or(false);
            if !keep { break; }
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
        let snapshot = self.feed.snapshot();
        let items = snapshot.value.as_deref().cloned().unwrap_or_default();
        let selected = match &self.flow {
            Flow::Selecting(ids) => ids.clone(),
            _ => vec![],
        };
        let can_select = matches!(self.flow, Flow::Idle | Flow::Selecting(_));
        let confirm = match &self.flow {
            Flow::Previewed(plan) => fill(translate(lang, T, "Delete {} items ({})"),
                &[&plan.items.len().to_string(), &bytes_text(plan.bytes, lang)]),
            _ => translate(lang, T, "Confirm cleanup").to_owned(),
        };
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
                Button::new("storage-back", translate(lang, T, "Back"), &t)
                    .accessibility_label(translate(lang, T, "Back"))
                    .on_click({
                        let state = self.state.clone();
                        move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx))
                    }).test_text(translate(lang, T, "Back")),
            )
            .child(h_flex().gap(theme::space(d, Step::Sm))
                .child(div().id("storage-preview-label").test_support().child(
                    Button::new("action:storage.preview", translate(lang, T, "Review cleanup"), &t)
                        .disabled(selected.is_empty() || !can_select)
                        .on_click(cx.listener(|this, _, _, cx| { this.run_action(false, cx); cx.notify(); }))
                        .test_text(translate(lang, T, "Review cleanup"))))
                .child(div().id("storage-confirm-label").test_support().child(
                    Button::new("action:storage.confirm", confirm.clone(), &t)
                        .disabled(!matches!(self.flow, Flow::Previewed(_)))
                        .on_click(cx.listener(|this, _, _, cx| { this.run_action(true, cx); cx.notify(); }))
                        .test_text(confirm))));
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
                        Button::new(("storage-group-choice", i), translate(lang, T, label), &t)
                            .accessibility_label(translate(lang, T, label))
                            .selected(self.by == by)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.by = by;
                                cx.notify();
                            })).test_text(translate(lang, T, label))
                    })),
            );
            let mut rows = Vec::new();
            for (name, total, reclaimable) in group(&items, self.by) {
                let display = if name.is_empty() { translate(lang, T, "Unassigned") } else { &name };
                rows.push(StorageRow::Group(format!("{} — {}", display, fill(
                    translate(lang, T, "{} total, {} reclaimable"),
                    &[&bytes_text(total, lang), &bytes_text(reclaimable, lang)]))));
                rows.extend(items.iter().enumerate().filter(|(_, item)| {
                    (match self.by {
                        GroupBy::Category => item.category.as_str(),
                        GroupBy::Project => item.project.as_deref().unwrap_or(""),
                        GroupBy::Wish => item.wish.as_deref().unwrap_or(""),
                    }) == name
                }).map(|(index, item)| StorageRow::Item(index, item.clone())));
            }
            if self.rows != rows {
                self.list = VirtualListHandle::uniform(rows.len(), f32::from(theme::row_height(d)));
                self.rows = rows;
            }
            let rows = self.rows.clone();
            let row_selected = selected.clone();
            let listener = Rc::new(cx.listener(move |this, id: &String, _, cx| {
                let mut ids = match &this.flow { Flow::Selecting(ids) => ids.clone(), _ => vec![] };
                if let Some(i) = ids.iter().position(|x| x == id) { ids.remove(i); } else { ids.push(id.clone()); }
                this.flow = if ids.is_empty() { Flow::Idle } else { Flow::Selecting(ids) };
                cx.notify();
            }));
            body = body.child(div().grid().flex_1().min_h_0().min_w_0().w_full().overflow_hidden().child(
                VirtualList::new("storage-list", self.list.clone(), &t, move |row| match &rows[row] {
                    StorageRow::Group(text) => text_element("storage-row", text.clone()),
                    StorageRow::Item(index, item) => {
                        let label = format!("{} — {}", item.path, item_size_text(item, lang));
                        if !item.reclaimable { return text_element("storage-item-fixed", label); }
                        let id = item.id.clone();
                        let listener = listener.clone();
                        Button::new(("storage-item", *index), label.clone(), &t)
                            .selected(row_selected.contains(&id)).variant(ButtonVariant::Secondary)
                            .on_click(move |_, window, cx| listener(&id, window, cx))
                            .test_text(label).into_any_element()
                    }
                })
            ));

        }
        match &self.flow {
            Flow::Previewed(plan) => {
                let list = plan
                    .items
                    .iter()
                    .map(|i| format!("{} — {}", i.path, item_size_text(i, lang)))
                    .collect::<Vec<_>>()
                    .join("\n");
                body = body.child(text_element("storage-plan", list));
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
                Button::new("storage-cancel", translate(lang, T, "Cancel"), &t)
                    .accessibility_label(translate(lang, T, "Cancel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.flow = Flow::Idle;
                        this.feed.retry();
                        cx.notify();
                    })).test_text(translate(lang, T, "Cancel")),
            );
        }
        let content = body.text_color(t.text)
            .text_size(theme::font(d, TypeSize::Body)).into_any_element();
        let active = super::super::data::faults::active("storage.page", cx) == Some(super::super::data::faults::Fault::Panic);
        if active { self.panic_pending = true; }
        let mut panel_state = snapshot.state;
        if self.panic_pending && !active {
            panel_state = super::super::components::panel::PanelState::Failed { reason: "Panel rendering failed".into() };
        }
        super::super::components::panel::Panel::new("storage.page", &panel_state)
            .retry(cx.listener(|this, _, _, cx| { this.panic_pending = false; this.feed.retry(); cx.notify(); }))
            .content(move |_, _| content)

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
