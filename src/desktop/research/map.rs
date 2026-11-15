use super::{Tab, Workbench};
use crate::desktop::{
    components::{panel::PanelState, SplitHandle},
    i18n::Lang,
    layout::persist,
    theme::{self, Density, Step, Tokened as _},
    viz::{self, FunnelStage},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    *,
};

pub(super) fn funnel_stages(
    map: &crate::desktop::ux::seams::research::MapDisplay,
) -> Vec<FunnelStage> {
    map.funnel
        .iter()
        .map(|node| FunnelStage {
            label: node.label.clone(),
            count: node.count,
        })
        .collect()
}

pub(super) fn map_funnel(
    map: &crate::desktop::ux::seams::research::MapDisplay,
    colors: &theme::Tokens,
) -> impl IntoElement {
    viz::funnel(
        "research-map-funnel",
        &funnel_stages(map),
        colors,
        Density::default(),
    )
}

pub struct MapPanel {
    panic_pending: bool,
    wb: Entity<Workbench>,
    sizes: Option<Vec<f32>>,
}
impl MapPanel {
    pub fn new(wb: Entity<Workbench>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        let feed = wb.read(cx).map_feed.clone();
        feed.observe(cx);
        Self {
            panic_pending: false,
            wb,
            sizes: persist::sizes("research.map", cx),
        }
    }
}
impl MapPanel {
    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let snapshot = self.wb.read(cx).map_feed.snapshot();
        let mut panel = div()
            .id("research-map-panel")
            .test_support()
            .flex()
            .flex_col()
            .w_full()
            .gap(theme::space(Density::default(), Step::Xs));
        let load = &self.wb.read(cx).load;
        if matches!(load, super::LoadState::Indexing { .. }) {
            return panel.child(super::marked(
                "research-map-indexing",
                super::load_text(load, lang),
            ));
        }
        if self.wb.read(cx).map_pending {
            return panel.child(super::marked(
                "research-map-loading",
                super::load_text(load, lang),
            ));
        }
        match &snapshot.state {
            PanelState::Stale { .. } => {
                let notice = match lang {
                    Lang::En => "Map data is stale",
                    Lang::ZhTw => "地圖資料已過期",
                };
                panel = panel.child(super::marked("research-map-feed-stale", notice.into()));
            }
            PanelState::Failed { reason }
            | PanelState::Corrupt { reason }
            | PanelState::Offline { reason } => {
                return panel.child(super::marked("research-map-error", reason.clone()));
            }
            _ => {}
        }
        let Some(map) = snapshot.value else {
            return panel.child(super::marked(
                "research-map-loading",
                super::load_text(&self.wb.read(cx).load, lang),
            ));
        };
        if map.title.is_empty() && map.status.is_empty() {
            return panel.child(super::marked(
                "research-map-loading",
                super::load_text(load, lang),
            ));
        }
        let (status, engineering, repro, stale) = match lang {
            Lang::En => (
                &map.status,
                &map.engineering_en,
                &map.repro_en,
                &map.stale_en,
            ),
            Lang::ZhTw => (
                &map.status_zh,
                &map.engineering_zh,
                &map.repro_zh,
                &map.stale_zh,
            ),
        };
        let mut stages = div().flex().flex_col();
        stages = stages.child(super::marked("research-map-status", status.clone()));
        if let Some(text) = engineering {
            stages = stages.child(super::marked("research-map-engineering", text.clone()));
        }
        if let Some(text) = repro {
            stages = stages.child(super::marked("research-map-repro", text.clone()));
        }
        for (i, line) in map.stages.iter().enumerate() {
            let color = match line.tone.as_str() {
                "Done" => colors.ok,
                "Running" => colors.run,
                "Blocked" => colors.fail,
                "Stale" => colors.review,
                _ => colors.text,
            };
            let text = match lang {
                Lang::En => &line.text_en,
                Lang::ZhTw => &line.text_zh,
            };
            let mut row = div()
                .id(SharedString::from(format!("research-map-stage-{i}")))
                .test_support()
                .w_full()
                .text_color(color)
                .child(text.clone().test_text(text.clone()));
            if line.current {
                row = row.child(super::marked("research-map-current", " ◉".into()));
            }
            if let Some(tab) = line.output {
                let handle = self.wb.clone();
                let label = match lang {
                    Lang::En => "Open outputs",
                    Lang::ZhTw => "開啟輸出",
                };
                row = row.child(
                    div()
                        .id(SharedString::from(format!("research-map-open-{i}")))
                        .test_support()
                        .cursor_pointer()
                        .text_color(colors.gold)
                        .child(label.test_text(label))
                        .on_click(move |_, _, cx| {
                            handle.update(cx, |wb, cx| {
                                if tab == Tab::Dataset {
                                    wb.show_dataset(cx);
                                } else {
                                    wb.set_tab(tab, cx);
                                }
                            })
                        }),
                );
            }
            stages = stages.child(row.border_b_1().border_color(colors.orbit));
        }
        for (i, text) in stale.iter().enumerate() {
            stages = stages.child(super::marked(
                SharedString::from(format!("research-map-stale-{i}")),
                text.clone(),
            ));
        }
        for (i, notice) in map.notices.iter().enumerate() {
            stages = stages.child(super::marked(
                SharedString::from(format!("research-map-notice-{i}")),
                notice.message.clone(),
            ));
        }
        let available = window.viewport_size().width.as_f32();
        let default = available / 2.;
        let widths = self
            .sizes
            .clone()
            .filter(|sizes| sizes.len() == 2)
            .unwrap_or_else(|| vec![default, default]);
        let handle = cx.entity().clone();
        let split =
            SplitHandle::new("research-map-split", &colors).on_resize(move |delta, _, cx| {
                handle.update(cx, |this, cx| {
                    let left = (widths[0] + delta.as_f32()).clamp(0., available);
                    let next = vec![left, (available - left).max(0.)];
                    this.sizes = Some(next.clone());
                    if cx.try_global::<persist::LayoutState>().is_some()
                        && this.wb.read(cx).layout_persistence_safe()
                    {
                        persist::remember("research.map", next, cx);
                    }
                    cx.notify();
                });
            });
        let left = self
            .sizes
            .as_ref()
            .and_then(|s| s.first())
            .copied()
            .unwrap_or(default)
            .clamp(0., available);
        panel = panel.child(
            div()
                .flex()
                .w_full()
                .child(div().w(px(left)).child(stages))
                .child(
                    div()
                        .id("research-map-split-frame")
                        .test_support()
                        .child(split),
                )
                .child(
                    div()
                        .id("research-map-funnel-region")
                        .test_support()
                        .flex_1()
                        .child(map_funnel(&map, &colors)),
                ),
        );
        panel
    }
}

impl Render for MapPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let feed = self.wb.read(cx).map_feed.clone();
        let mut state = feed.snapshot().state;
        let active = crate::desktop::data::faults::active("research.map.panel", cx)
            == Some(crate::desktop::data::faults::Fault::Panic);
        if active && state == PanelState::Ready {
            self.panic_pending = true;
        }
        if self.panic_pending && !active {
            state = PanelState::Failed {
                reason: "panel rendering failed".into(),
            };
        }
        let handle = cx.entity().clone();
        crate::desktop::components::panel::Panel::new("research.map.panel", &state)
            .retry(cx.listener(|this, _, _, cx| {
                this.panic_pending = false;
                this.wb.read(cx).map_feed.retry();
                cx.notify();
            }))
            .content(move |window, cx| {
                handle.update(cx, |this, cx| {
                    this.render_content(window, cx).into_any_element()
                })
            })
    }
}
