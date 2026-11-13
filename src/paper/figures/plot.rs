use super::{
    fonts::TextMeasure,
    layout::{AxisInfo, FigureLayout, PlotGeometry, TextBox, TextRole},
    presets::Preset,
};
use crate::{
    base::{Failure, Res},
    paper::{
        evidence::DataTable,
        model::{FigureSpec, PlotKind},
    },
};

fn fail(kind: &str, message: &str) -> Failure {
    Failure::new(kind, message)
}
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2e80..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef)
}
fn text(text: String, role: TextRole, p: &Preset, m: &dyn TextMeasure) -> Res<TextBox> {
    let family = if text.chars().any(is_cjk) {
        p.cjk_font_family
            .as_ref()
            .ok_or_else(|| fail("glyph_missing", "CJK font is required"))?
    } else {
        &p.font_family
    };
    if !m.covers(&text, family)?.is_empty() {
        return Err(fail("glyph_missing", "font does not cover text"));
    }
    let (w, h) = m.measure(&text, family, p.font_pt)?;
    Ok(TextBox {
        role,
        text,
        x_mm: 0.0,
        y_mm: 0.0,
        w_mm: w,
        h_mm: h,
        font_pt: p.font_pt,
        family: family.clone(),
    })
}
fn intersects(a: &TextBox, b: &TextBox) -> bool {
    a.w_mm > 0.0
        && a.h_mm > 0.0
        && b.w_mm > 0.0
        && b.h_mm > 0.0
        && a.x_mm < b.x_mm + b.w_mm
        && b.x_mm < a.x_mm + a.w_mm
        && a.y_mm < b.y_mm + b.h_mm
        && b.y_mm < a.y_mm + a.h_mm
}
fn ticks(mut lo: f64, mut hi: f64) -> Res<(f64, f64, Vec<(f64, String)>)> {
    if lo == hi {
        let pad = if lo == 0.0 { 1.0 } else { lo.abs() * 0.5 };
        lo -= pad;
        hi += pad;
    }
    let raw = (hi - lo) / 5.0;
    let power = 10f64.powf(raw.log10().floor());
    let r = raw / power;
    let step = (if r <= 1.0 {
        1.0
    } else if r <= 2.0 {
        2.0
    } else if r <= 5.0 {
        5.0
    } else {
        10.0
    }) * power;
    let min = (lo / step).floor() * step;
    let max = (hi / step).ceil() * step;
    let count = ((max - min) / step).round();
    if !count.is_finite() {
        return Err(fail("bad_value", "numeric range cannot be represented"));
    }
    if count >= 100.0 {
        return Err(fail("layout_overlap", "too many ticks"));
    }
    let n = count as usize;
    let vals: Vec<f64> = (0..=n)
        .map(|i| {
            let v = min + i as f64 * step;
            if v.abs() < step * 1e-9 {
                0.0
            } else {
                v
            }
        })
        .collect();
    let precision = (0..=15)
        .find(|&p| {
            vals.windows(2)
                .all(|w| format!("{:.p$}", w[0]) != format!("{:.p$}", w[1]))
        })
        .unwrap_or(15);
    Ok((
        min,
        max,
        vals.into_iter()
            .map(|v| (v, format!("{:.precision$}", v)))
            .collect(),
    ))
}
fn thin(ticks: &mut Vec<(f64, TextBox)>) {
    loop {
        let overlap = ticks
            .iter()
            .enumerate()
            .any(|(i, (_, a))| ticks[i + 1..].iter().any(|(_, b)| intersects(a, b)));
        if !overlap || ticks.len() <= 1 {
            break;
        }
        *ticks = ticks
            .drain(..)
            .enumerate()
            .filter_map(|(i, t)| (i % 2 == 0).then_some(t))
            .collect();
    }
}
pub fn layout(
    spec: &FigureSpec,
    data: &DataTable,
    preset: &Preset,
    m: &dyn TextMeasure,
) -> Res<FigureLayout> {
    if preset.font_pt < preset.min_font_pt {
        return Err(fail("font_too_small", "font is smaller than minimum"));
    }
    let col = |name: &str| {
        data.columns
            .iter()
            .position(|x| x == name)
            .ok_or_else(|| Failure::new("missing_column", name))
    };
    let xi = col(&spec.x)?;
    let yi: Vec<_> = spec.y.iter().map(|s| col(s)).collect::<Res<_>>()?;
    if yi.is_empty() {
        return Err(fail("missing_column", "no y column"));
    }
    let ei = if spec.error.is_empty() {
        if spec.kind == PlotKind::ErrorBar {
            return Err(fail("missing_column", "error column required"));
        }
        None
    } else {
        Some(col(&spec.error)?)
    };
    let unit = |i: usize| data.units.get(i).cloned().flatten();
    let yu = unit(yi[0]);
    let yu = if yi.iter().all(|&i| unit(i) == yu) {
        yu
    } else {
        None
    };
    let axis = |column: String, unit: Option<String>| AxisInfo {
        label: match &unit {
            Some(u) => format!("{column} [{u}]"),
            None => column.clone(),
        },
        column,
        data_unit: unit,
    };
    let axes = vec![axis(spec.x.clone(), unit(xi)), axis(spec.y.join(", "), yu)];
    let mut series: Vec<_> = spec.y.iter().map(|y| (y.clone(), Vec::new())).collect();
    let mut errors = Vec::new();
    let mut categories = Vec::new();
    let number = |row: &[String], i: usize| -> Res<f64> {
        let v = row
            .get(i)
            .ok_or_else(|| fail("bad_value", "missing cell"))?
            .parse::<f64>()
            .map_err(|_| fail("bad_value", "invalid number"))?;
        if !v.is_finite() {
            return Err(fail("bad_value", "non-finite number"));
        }
        Ok(v)
    };
    for (index, row) in data.rows.iter().enumerate() {
        let x = if spec.kind == PlotKind::Bar {
            categories.push(
                row.get(xi)
                    .ok_or_else(|| fail("bad_value", "missing cell"))?
                    .clone(),
            );
            index as f64
        } else {
            number(row, xi)?
        };
        for (j, &i) in yi.iter().enumerate() {
            series[j].1.push((x, number(row, i)?));
        }
        if let Some(i) = ei {
            let e = number(row, i)?;
            if spec.kind == PlotKind::ErrorBar {
                errors.push((x, series[0].1[index].1, e));
            }
        }
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (_, points) in &series {
        for &(x, y) in points {
            xs.push(x);
            ys.push(y);
        }
    }
    for &(_, y, e) in &errors {
        ys.push(y - e.abs());
        ys.push(y + e.abs());
    }
    if spec.kind == PlotKind::Bar {
        ys.push(0.0);
    }
    let bounds = |v: &[f64]| {
        if v.is_empty() {
            (0.0, 1.0)
        } else {
            (
                v.iter().copied().fold(f64::INFINITY, f64::min),
                v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        }
    };
    let (xmin, xmax, xt) = if spec.kind == PlotKind::Bar {
        (
            -0.5,
            (categories.len() as f64 - 0.5).max(0.5),
            categories
                .into_iter()
                .enumerate()
                .map(|(i, s)| (i as f64, s))
                .collect(),
        )
    } else {
        let (a, b) = bounds(&xs);
        ticks(a, b)?
    };
    let (a, b) = bounds(&ys);
    let (ymin, ymax, yt) = ticks(a, b)?;
    if ![xmin, xmax, ymin, ymax].iter().all(|v| v.is_finite())
        || !xt.iter().chain(yt.iter()).all(|(v, _)| v.is_finite())
    {
        return Err(fail("bad_value", "numeric range cannot be represented"));
    }
    let mut xt: Vec<_> = xt
        .into_iter()
        .map(|(v, s)| Ok((v, text(s, TextRole::Tick, preset, m)?)))
        .collect::<Res<_>>()?;
    let mut yt: Vec<_> = yt
        .into_iter()
        .map(|(v, s)| Ok((v, text(s, TextRole::Tick, preset, m)?)))
        .collect::<Res<_>>()?;
    let mut xl = text(axes[0].label.clone(), TextRole::AxisLabel, preset, m)?;
    let mut yl = text(axes[1].label.clone(), TextRole::AxisLabel, preset, m)?;
    std::mem::swap(&mut yl.w_mm, &mut yl.h_mm);
    let mut legends: Vec<_> = if series.len() > 1 {
        series
            .iter()
            .map(|(name, _)| text(name.clone(), TextRole::Legend, preset, m))
            .collect::<Res<_>>()?
    } else {
        Vec::new()
    };
    let max_xw = xt.iter().map(|(_, t)| t.w_mm).fold(0.0, f64::max);
    let max_xh = xt.iter().map(|(_, t)| t.h_mm).fold(0.0, f64::max);
    let max_yw = yt.iter().map(|(_, t)| t.w_mm).fold(0.0, f64::max);
    let max_yh = yt.iter().map(|(_, t)| t.h_mm).fold(0.0, f64::max);
    let left = (yl.w_mm + 1.0 + max_yw + 1.0).max(max_xw / 2.0);
    let bottom = (xl.h_mm + 1.0 + max_xh + 1.0).max(max_yh / 2.0);
    let right = preset.width_mm - (max_xw / 2.0).max(1.0);
    let top = preset.height_mm - (max_yh / 2.0).max(1.0);
    if !left.is_finite()
        || !bottom.is_finite()
        || right <= left
        || top <= bottom
        || xmax <= xmin
        || ymax <= ymin
    {
        return Err(fail("layout_overlap", "canvas too small"));
    }
    for (v, t) in &mut xt {
        t.x_mm = left + (*v - xmin) / (xmax - xmin) * (right - left) - t.w_mm / 2.0;
        t.y_mm = bottom - 1.0 - t.h_mm;
    }
    for (v, t) in &mut yt {
        t.x_mm = left - 1.0 - t.w_mm;
        t.y_mm = bottom + (*v - ymin) / (ymax - ymin) * (top - bottom) - t.h_mm / 2.0;
    }
    thin(&mut xt);
    thin(&mut yt);
    xl.x_mm = (left + right - xl.w_mm) / 2.0;
    xl.y_mm = 0.0;
    yl.x_mm = 0.0;
    yl.y_mm = (bottom + top - yl.h_mm) / 2.0;
    let mut ly = top;
    for l in &mut legends {
        ly -= l.h_mm;
        l.x_mm = right - l.w_mm;
        l.y_mm = ly;
        ly -= 1.0;
    }
    let geometry = PlotGeometry {
        kind: spec.kind.clone(),
        left,
        bottom,
        right,
        top,
        xmin,
        xmax,
        ymin,
        ymax,
        xticks: xt.iter().map(|(v, _)| *v).collect(),
        yticks: yt.iter().map(|(v, _)| *v).collect(),
    };
    let mut texts = vec![xl, yl];
    texts.extend(xt.into_iter().map(|(_, t)| t));
    texts.extend(yt.into_iter().map(|(_, t)| t));
    texts.extend(legends);
    for (i, t) in texts.iter().enumerate() {
        if ![t.x_mm, t.y_mm, t.w_mm, t.h_mm]
            .iter()
            .all(|v| v.is_finite())
            || t.w_mm < 0.0
            || t.h_mm < 0.0
            || t.x_mm < 0.0
            || t.y_mm < 0.0
            || t.x_mm + t.w_mm > preset.width_mm
            || t.y_mm + t.h_mm > preset.height_mm
            || texts[i + 1..].iter().any(|b| intersects(t, b))
        {
            return Err(fail("layout_overlap", "text boxes overlap or leave canvas"));
        }
    }
    let mut layout = FigureLayout {
        id: spec.id.clone(),
        width_mm: preset.width_mm,
        height_mm: preset.height_mm,
        texts,
        axes,
        series,
        errors,
        paths: 0,
    };
    layout.set_geometry(&geometry, preset.font_pt, &preset.font_family);
    layout.paths = layout.path_count();
    Ok(layout)
}
