use crate::{
    base::{Failure, Res},
    paper::{
        evidence::DataTable,
        figures::{fonts::TextMeasure, layout::escape_latex},
        model::{RenderedTable, TableSpec},
    },
};

pub fn booktabs(
    spec: &TableSpec,
    data: &DataTable,
    text_width_mm: f64,
    m: &dyn TextMeasure,
    family: &str,
) -> Res<RenderedTable> {
    let indices: Vec<usize> = spec
        .columns
        .iter()
        .map(|c| {
            data.columns
                .iter()
                .position(|x| x == &c.column)
                .ok_or_else(|| Failure::new("missing_column", &c.column))
        })
        .collect::<Res<_>>()?;
    let units: Vec<_> = indices
        .iter()
        .map(|&i| data.units.get(i).cloned().flatten())
        .collect();
    let headers: Vec<_> = spec
        .columns
        .iter()
        .zip(&units)
        .map(|(c, unit)| match unit {
            Some(u) if !c.header.contains('[') => format!("{} [{}]", c.header, u),
            _ => c.header.clone(),
        })
        .collect();
    let cells: Vec<Vec<String>> = data
        .rows
        .iter()
        .map(|row| {
            indices
                .iter()
                .zip(&spec.columns)
                .map(|(&i, c)| {
                    let value = row
                        .get(i)
                        .ok_or_else(|| Failure::new("bad_value", "missing cell"))?;
                    Ok(match value.parse::<f64>() {
                        Ok(n) if n.is_finite() => {
                            format!("{:.prec$}", n, prec = c.precision as usize)
                        }
                        _ => value.clone(),
                    })
                })
                .collect::<Res<_>>()
        })
        .collect::<Res<_>>()?;
    let mut width_mm = 0.0;
    for (i, header) in headers.iter().enumerate() {
        let mut width = m.measure(header, family, 8.0)?.0;
        for row in &cells {
            width = width.max(m.measure(&row[i], family, 8.0)?.0);
        }
        width_mm += width + 2.0 * 2.1167;
    }
    if width_mm > text_width_mm {
        return Err(Failure::new("table_too_wide", "table exceeds text width"));
    }
    let align: String = (0..headers.len())
        .map(|i| if i == 0 { 'l' } else { 'r' })
        .collect();
    let row = |cells: &[String]| {
        format!(
            "{} \\\\",
            cells
                .iter()
                .map(|s| escape_latex(s))
                .collect::<Vec<_>>()
                .join(" & ")
        )
    };
    let mut lines = vec![
        format!("\\begin{{tabular}}{{{align}}}"),
        "\\toprule".into(),
        row(&headers),
        "\\midrule".into(),
    ];
    lines.extend(cells.iter().map(|r| row(r)));
    lines.extend(["\\bottomrule".into(), "\\end{tabular}".into()]);
    Ok(RenderedTable {
        id: spec.id.clone(),
        latex: lines.join("\n"),
        cells,
        headers,
        units,
        width_mm,
    })
}
