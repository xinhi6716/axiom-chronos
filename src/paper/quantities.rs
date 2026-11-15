use super::{
    evidence,
    model::{tokens, Aggregate, EvidenceBundle, Quantity, Token, Uncertainty},
};
use crate::base::{Failure, Res};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct QuantityCheck {
    pub id: String,
    pub recomputed: Option<f64>,
    pub ok: bool,
    pub detail: String,
}

pub fn verify(bundle: &EvidenceBundle, project_root: &Path, q: &Quantity) -> Res<QuantityCheck> {
    let table = evidence::load_table(bundle, project_root, &q.locator.artifact)?;
    let fail = |detail: String| QuantityCheck {
        id: q.id.clone(),
        recomputed: None,
        ok: false,
        detail,
    };
    let Some(column) = q
        .locator
        .column
        .as_ref()
        .and_then(|name| table.columns.iter().position(|c| c == name))
    else {
        return Ok(fail("missing or unknown column".into()));
    };
    if q.locator.row.is_some() == q.locator.aggregate.is_some() {
        return Ok(fail(
            "locator must specify exactly one row or aggregate".into(),
        ));
    }
    let values: Option<Vec<f64>> = if let Some(row) = q.locator.row {
        usize::try_from(row)
            .ok()
            .and_then(|r| table.rows.get(r))
            .and_then(|r| r.get(column))
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .map(|v| vec![v])
    } else {
        table
            .rows
            .iter()
            .map(|r| {
                r.get(column)
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite())
            })
            .collect()
    };
    let Some(mut values) = values.filter(|v| !v.is_empty()) else {
        return Ok(fail("missing or non-numeric data".into()));
    };
    let value = match q.locator.aggregate.as_ref() {
        None => values[0],
        Some(Aggregate::Count) => values.len() as f64,
        Some(Aggregate::Sum) => values.iter().sum(),
        Some(Aggregate::Mean) => values.iter().sum::<f64>() / values.len() as f64,
        Some(Aggregate::Min) => values.iter().copied().fold(f64::INFINITY, f64::min),
        Some(Aggregate::Max) => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Some(Aggregate::Median) => {
            values.sort_by(|a, b| a.total_cmp(b));
            let mid = values.len() / 2;
            if values.len() % 2 == 0 {
                (values[mid - 1] + values[mid]) / 2.0
            } else {
                values[mid]
            }
        }
        Some(Aggregate::Sd | Aggregate::Se) => {
            if values.len() < 2 {
                return Ok(fail(
                    "sample standard deviation requires at least two values".into(),
                ));
            }
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                / (values.len() - 1) as f64)
                .sqrt();
            if matches!(q.locator.aggregate, Some(Aggregate::Se)) {
                sd / (values.len() as f64).sqrt()
            } else {
                sd
            }
        }
    };
    let unit_ok = table.units.get(column) == Some(&q.unit);
    let tolerance = 0.5 * 10_f64.powi(-(q.precision as i32));
    let value_ok = value.is_finite()
        && q.value.is_finite()
        && (value - q.value).abs()
            <= tolerance + f64::EPSILON * value.abs().max(q.value.abs()).max(1.0) * 4.0;
    let detail = if !unit_ok {
        format!(
            "unit mismatch: expected {:?}, found {:?}",
            q.unit, table.units[column]
        )
    } else if !value_ok {
        format!("value mismatch: stated {}, recomputed {}", q.value, value)
    } else {
        "verified".into()
    };
    Ok(QuantityCheck {
        id: q.id.clone(),
        recomputed: Some(value),
        ok: unit_ok && value_ok,
        detail,
    })
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '_' | '%' => {
                out.push('\\');
                out.push(c);
            }
            '^' => out.push_str("\\textasciicircum{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            _ => out.push(c),
        }
    }
    out
}

pub fn render(q: &Quantity) -> String {
    let precision = q.precision as usize;
    let mut result = format!("{:.*}", precision, q.value);
    if let Some(unit) = &q.unit {
        result.push_str(&format!(" {}", escape(unit)));
    }
    match &q.uncertainty {
        Some(Uncertainty::Sd(v)) => result.push_str(&format!(" ± {:.*} SD", precision, v)),
        Some(Uncertainty::Se(v)) => result.push_str(&format!(" ± {:.*} SE", precision, v)),
        Some(Uncertainty::Ci { level, low, high }) => result.push_str(&format!(
            " {:.0}\\% CI [{:.*}, {:.*}]",
            (level * 100.0).round(),
            precision,
            low,
            precision,
            high
        )),
        None => {}
    }
    if let Some(n) = q.n {
        result.push_str(&format!(" n={n}"));
    }
    if let Some(test) = &q.test {
        result.push_str(&format!(" {}", escape(test)));
    }
    if let Some(p) = q.p_value {
        result.push_str(&format!(" p={p}"));
    }
    result
}

pub fn fill(text: &str, bundle: &EvidenceBundle) -> Res<String> {
    let mut out = String::new();
    let mut end = 0;
    for (range, token) in tokens(text) {
        if let Token::Quantity(id) = token {
            let q = bundle
                .quantities
                .iter()
                .find(|q| q.id == id)
                .ok_or_else(|| Failure::new("dangling", format!("unknown quantity {id}")))?;
            out.push_str(&text[end..range.start]);
            out.push_str(&render(q));
            end = range.end;
        }
    }
    out.push_str(&text[end..]);
    Ok(out)
}
