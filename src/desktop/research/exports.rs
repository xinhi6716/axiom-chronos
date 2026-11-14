use super::{LoadState, Workbench};
use crate::desktop::{
    components::{panel::PanelState, Button, SplitHandle, TextField},
    i18n::{translate, Lang, Table},
    layout::persist,
    theme::{self, Density, Step, Tokened as _},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::input::InputState,
    *,
};
use std::path::PathBuf;

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
        "Vault and memory are not in this version",
        "此版本不包含知識庫與記憶功能",
    ),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
#[derive(Clone, Copy)]
enum Action {
    Csv,
    Bundle,
    Snapshot,
}
pub struct ExportsPanel {
    wb: Entity<Workbench>,
    dest: Entity<InputState>,
    error: Option<String>,
    sizes: Option<Vec<f32>>,
}
impl ExportsPanel {
    pub fn new(wb: Entity<Workbench>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dest = cx.new(|cx| InputState::new(window, cx));
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        Self {
            wb,
            dest,
            error: None,
            sizes: persist::sizes("research.exports", cx),
        }
    }
    fn run(&mut self, action: Action, cx: &mut Context<Self>) {
        let dest = self.dest.read(cx).value().to_string();
        if dest.trim().is_empty() {
            self.error = Some(tr(*cx.global::<Lang>(), "Enter a destination").into());
            cx.notify();
            return;
        }
        self.error = None;
        let dest = PathBuf::from(dest.trim());
        self.wb.update(cx, |wb, cx| {
            match action {
                Action::Csv => wb.port.export_csv(dest),
                Action::Bundle => wb.port.export_bundle(dest),
                Action::Snapshot => wb.port.export_snapshot(dest),
            }
            wb.refresh_tab_feeds(cx);
        });
    }
}
impl Render for ExportsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let gap = theme::space(Density::default(), Step::Sm);
        let wb = self.wb.read(cx);
        let snapshot = wb.exports_feed.snapshot();
        let mut panel = div()
            .id("research-exports-panel")
            .test_support()
            .flex()
            .flex_col()
            .gap(gap)
            .text_color(colors.text)
            .child(marked(
                "research-exports-vault-status",
                tr(lang, "Vault and memory are not in this version").into(),
            ));
        if !matches!(wb.load, LoadState::Ready) {
            return panel
                .child(tr(lang, "No research open").test_text(tr(lang, "No research open")));
        }
        let dest_input = self.dest.clone();
        let left_content = div()
            .flex()
            .flex_col()
            .gap(gap)
            .child(tr(lang, "Destination"))
            .child(
                div()
                    .id("research-exports-dest")
                    .test_support()
                    .on_click(move |_, window, cx| {
                        dest_input.read(cx).focus_handle(cx).focus(window, cx)
                    })
                    .child(TextField::new(
                        "research-exports-dest-field",
                        &self.dest,
                        &colors,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(gap)
                    .child(
                        Button::new("research-exports-csv", tr(lang, "Export CSV"), &colors)
                            .on_click(cx.listener(|this, _, _, cx| this.run(Action::Csv, cx))),
                    )
                    .child(
                        Button::new(
                            "research-exports-bundle",
                            tr(lang, "Export evidence bundle"),
                            &colors,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.run(Action::Bundle, cx))),
                    )
                    .child(
                        Button::new(
                            "research-exports-snapshot",
                            tr(lang, "Freeze snapshot"),
                            &colors,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.run(Action::Snapshot, cx))),
                    ),
            );
        let mut right = div().flex().flex_col().gap(gap);
        if let PanelState::Failed { reason }
        | PanelState::Corrupt { reason }
        | PanelState::Offline { reason } = &snapshot.state
        {
            right = right.child(marked("research-exports-error", reason.clone()));
        }
        if let Some(display) = snapshot.value {
            let repro = if lang == Lang::ZhTw {
                &display.repro_zh
            } else {
                &display.repro_en
            };
            if !repro.is_empty() {
                right = right.child(marked("research-exports-repro", repro.clone()));
            }
            for notice in &display.notices {
                if notice.kind == "unavailable" {
                    continue;
                }
                let message = match notice.message.as_str() {
                    "No evidence in the current dataset filter" => {
                        tr(lang, "No evidence in the current dataset filter").to_owned()
                    }
                    "Research home is not configured" => {
                        tr(lang, "Research home is not configured").to_owned()
                    }
                    _ => format!("{}: {}", notice.kind, notice.message),
                };
                right = right.child(marked("research-exports-error", message));
            }
            for (i, result) in display
                .results
                .iter()
                .filter(|r| r.path.is_some())
                .enumerate()
            {
                let hash = result.sha256.as_deref().unwrap_or("");
                right = right.child(marked(
                    SharedString::from(format!("research-exports-result-{i}")),
                    format!(
                        "{} · {} · {}",
                        result.path.as_ref().unwrap().display(),
                        result.rows,
                        &hash[..hash.len().min(12)]
                    ),
                ));
            }
            let handoff = display.results.iter().rev().find_map(|result| {
                if lang == Lang::ZhTw {
                    result.handoff_zh.as_ref()
                } else {
                    result.handoff.as_ref()
                }
            });
            if let Some(handoff) = handoff {
                right = right.child(marked("research-exports-handoff", handoff.clone()));
            }
        }
        if let Some(error) = &self.error {
            right = right.child(marked("research-exports-error", error.clone()));
        }
        let available = window.viewport_size().width.as_f32();
        let default = available / 2.;
        let widths = self
            .sizes
            .clone()
            .filter(|s| s.len() == 2)
            .unwrap_or_else(|| vec![default, default]);
        let left = widths[0].clamp(0., available);
        let handle = cx.entity().clone();
        let split =
            SplitHandle::new("research-exports-split", &colors).on_resize(move |delta, _, cx| {
                handle.update(cx, |this, cx| {
                    let left = (widths[0] + delta.as_f32()).clamp(0., available);
                    let next = vec![left, (available - left).max(0.)];
                    this.sizes = Some(next.clone());
                    if cx.try_global::<persist::LayoutState>().is_some()
                        && this.wb.read(cx).layout_persistence_safe()
                    {
                        persist::remember("research.exports", next, cx);
                    }
                    cx.notify();
                });
            });
        panel = panel.child(
            div()
                .flex()
                .w_full()
                .child(div().w(px(left)).child(left_content))
                .child(
                    div()
                        .id("research-exports-split-frame")
                        .test_support()
                        .child(split),
                )
                .child(div().flex_1().child(right)),
        );
        panel
    }
}
