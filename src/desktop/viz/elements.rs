//! Presentation components. Callers provide theme tokens and stable element identities.
use super::super::{motion, theme};
use super::data::*;
use gpui_kit::*;
use std::sync::OnceLock;
use std::time::Instant;

fn tone(t: &theme::Tokens, tone: Tone) -> Hsla {
    match tone {
        Tone::Neutral => t.muted,
        Tone::Good => t.ok,
        Tone::Warn => t.review,
        Tone::Bad => t.fail,
    }
}

// IDs for data-backed items are stable across reordering; indexed items are scoped by
// their parent element in the rendered tree.
pub(super) fn item_id(id: &str) -> ElementId {
    SharedString::from(id.to_owned()).into()
}

pub(super) fn stage_id(index: usize) -> ElementId {
    ("funnel-stage", index).into()
}

pub(super) fn series_id(index: usize) -> ElementId {
    ("chart-series", index).into()
}

pub(super) fn unit_interval(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0., 1.)
    } else {
        0.
    }
}

pub fn timeline(
    id: impl Into<ElementId>,
    items: &[TimelineItem],
    selection: &Selection<String>,
    t: &theme::Tokens,
    d: theme::Density,
) -> impl IntoElement {
    let mut list = div()
        .id(id)
        .flex()
        .flex_col()
        .gap(theme::space(d, theme::Step::Xs));
    for item in items {
        let selected = selection.key() == Some(&item.id);
        list = list.child(
            div()
                .id(item_id(&item.id))
                .flex()
                .items_center()
                .gap(theme::space(d, theme::Step::Sm))
                .p(theme::space(d, theme::Step::Sm))
                .rounded(theme::RADIUS)
                .bg(if selected { t.raised } else { t.surface })
                .shadow(if selected {
                    theme::selection_glow(t, 1.)
                } else {
                    Vec::new()
                })
                .child(div().text_color(t.muted).child(item.at_ms.to_string()))
                .child(
                    div()
                        .text_color(tone(t, item.tone))
                        .child(item.label.clone()),
                ),
        );
    }
    list
}

pub fn funnel(
    id: impl Into<ElementId>,
    stages: &[FunnelStage],
    t: &theme::Tokens,
    d: theme::Density,
) -> impl IntoElement {
    let max = stages.iter().map(|s| s.count).max().unwrap_or(0);
    let mut list = div()
        .id(id)
        .flex()
        .flex_col()
        .gap(theme::space(d, theme::Step::Sm));
    for (i, stage) in stages.iter().enumerate() {
        let width = if max == 0 {
            0.
        } else {
            stage.count as f32 / max as f32
        };
        list = list.child(
            div()
                .id(stage_id(i))
                .flex()
                .flex_col()
                .gap(theme::space(d, theme::Step::Xs))
                .child(
                    div()
                        .text_color(t.text)
                        .child(format!("{}  {}", stage.label, stage.count)),
                )
                .child(
                    div()
                        .w_full()
                        .h(theme::space(d, theme::Step::Sm))
                        .bg(t.raised)
                        .child(div().w(relative(width)).h_full().bg(t.gold)),
                ),
        );
    }
    list
}

pub(super) fn phase(driver: &mut motion::Driver, active: bool) -> f32 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    driver.keep(active);
    if !active || driver.reduce() {
        return 0.;
    }
    motion::spin_phase(
        driver
            .now()
            .saturating_duration_since(*EPOCH.get_or_init(|| driver.now())),
    )
}

pub fn orbit(
    id: impl Into<ElementId>,
    members: &[OrbitMember],
    driver: &mut motion::Driver,
    t: &theme::Tokens,
    d: theme::Density,
) -> impl IntoElement {
    let active = members.iter().any(|m| m.state == OrbitState::Speaking);
    let turn = phase(driver, active) * std::f32::consts::TAU;
    let mut view = div().id(id).relative().w(px(320.)).h(px(240.)).child(
        div()
            .absolute()
            .left(px(55.))
            .top(px(35.))
            .w(px(210.))
            .h(px(170.))
            .rounded_full()
            .border_1()
            .border_color(t.orbit),
    );
    for (i, member) in members.iter().enumerate() {
        let angle = turn + i as f32 * std::f32::consts::TAU / members.len() as f32;
        let color = match member.state {
            OrbitState::Idle => t.idle,
            OrbitState::Speaking => t.run,
            OrbitState::Waiting => t.review,
            OrbitState::Degraded => t.fail,
            OrbitState::Done => t.ok,
        };
        let weight = unit_interval(member.weight);
        view = view.child(
            div()
                .id(item_id(&member.id))
                .absolute()
                .left(px(160. + 105. * angle.cos() - 45.))
                .top(px(120. + 85. * angle.sin() - 16.))
                .w(px(90.))
                .text_center()
                .text_size(theme::font(d, theme::TypeSize::Caption))
                .text_color(t.text)
                .child(
                    div()
                        .mx_auto()
                        .size(px(8. + 12. * weight))
                        .rounded_full()
                        .bg(color),
                )
                .child(member.label.clone()),
        );
    }
    view
}

pub fn binary_star(
    id: impl Into<ElementId>,
    star: &BinaryStar,
    driver: &mut motion::Driver,
    t: &theme::Tokens,
    d: theme::Density,
) -> impl IntoElement {
    let turn = phase(driver, star.phase == StarPhase::Approaching);
    let agreement = unit_interval(star.agreement);
    let gap = match star.phase {
        StarPhase::Apart => 1.,
        StarPhase::Approaching => 1. - 0.5 * turn,
        StarPhase::Agreed => 0.2,
        StarPhase::Disagreed => 0.8,
    };
    let color = match star.phase {
        StarPhase::Agreed => t.ok,
        StarPhase::Disagreed => t.fail,
        StarPhase::Approaching => t.run,
        StarPhase::Apart => t.idle,
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(theme::space(d, theme::Step::Sm))
        .text_color(t.text)
        .text_size(theme::font(d, theme::TypeSize::Body))
        .child(div().id("star-a").child(star.a_label.clone()))
        .child(
            div()
                .size(theme::space(d, theme::Step::Md))
                .rounded_full()
                .bg(color),
        )
        .child(
            div()
                .w(px(20. + 70. * gap))
                .h(theme::space(d, theme::Step::Xs))
                .bg(t.orbit)
                .child(div().w(relative(agreement)).h_full().bg(t.gold)),
        )
        .child(
            div()
                .size(theme::space(d, theme::Step::Md))
                .rounded_full()
                .bg(color),
        )
        .child(div().id("star-b").child(star.b_label.clone()))
}

pub fn chart(
    id: impl Into<ElementId>,
    spec: &ChartSpec,
    t: &theme::Tokens,
    d: theme::Density,
) -> impl IntoElement {
    let series: Vec<_> = spec
        .series
        .iter()
        .map(|(name, values)| {
            (
                name.clone(),
                values
                    .iter()
                    .filter(|(_, v)| v.is_finite())
                    .map(|(k, v)| (*k, *v))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let kind = spec.kind;
    let colors = [t.gold, t.run, t.ok, t.review, t.fail];
    let mut legend = div().flex().gap(theme::space(d, theme::Step::Sm));
    for (i, (name, _)) in series.iter().enumerate() {
        legend = legend.child(
            div()
                .id(series_id(i))
                .text_color(colors[i % colors.len()])
                .child(name.clone()),
        );
    }
    let drawing = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let values: Vec<_> = series
                .iter()
                .flat_map(|(_, values)| values.iter())
                .collect();
            let Some(first) = values.first() else { return };
            let (mut min_x, mut max_x, mut min_y, mut max_y) =
                (first.0, first.0, first.1.min(0.), first.1.max(0.));
            for value in values {
                min_x = min_x.min(value.0);
                max_x = max_x.max(value.0);
                min_y = min_y.min(value.1);
                max_y = max_y.max(value.1);
            }
            let x = f32::from(bounds.origin.x);
            let y = f32::from(bounds.origin.y);
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let project = |ts: i64, value: f64| {
                point(
                    px(x + width
                        * ((ts as f64 - min_x as f64) / (max_x as f64 - min_x as f64).max(1.))
                            as f32),
                    px(y + height
                        * (1. - ((value - min_y) / (max_y - min_y).max(f64::EPSILON)) as f32)),
                )
            };
            let baseline = project(min_x, 0.).y;
            for (i, (_, samples)) in series.iter().enumerate() {
                if samples.is_empty() {
                    continue;
                }
                let points: Vec<_> = samples.iter().map(|(ts, v)| project(*ts, *v)).collect();
                let color = colors[i % colors.len()];
                if kind == ChartKind::Bar {
                    let bar_width = (width / samples.len() as f32 * 0.6).max(1.);
                    for p in &points {
                        let mut path = PathBuilder::fill();
                        path.add_polygon(
                            &[
                                point(p.x - px(bar_width / 2.), baseline),
                                point(p.x - px(bar_width / 2.), p.y),
                                point(p.x + px(bar_width / 2.), p.y),
                                point(p.x + px(bar_width / 2.), baseline),
                            ],
                            true,
                        );
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                } else {
                    if kind == ChartKind::Area && points.len() > 1 {
                        let mut area = vec![point(points[0].x, baseline)];
                        area.extend(points.iter().copied());
                        area.push(point(points.last().unwrap().x, baseline));
                        let mut path = PathBuilder::fill();
                        path.add_polygon(&area, true);
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color.opacity(0.25));
                        }
                    }
                    if points.len() > 1 {
                        let mut path = PathBuilder::stroke(px(2.));
                        path.add_polygon(&points, false);
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                }
            }
        },
    )
    .w_full()
    .h(px(160.));
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(theme::space(d, theme::Step::Sm))
        .child(legend)
        .child(drawing)
}
