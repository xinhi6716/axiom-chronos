use crate::base::{Failure, Res};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn parse(text: &str, delimiter: u8) -> Res<CsvTable> {
    if !delimiter.is_ascii() || matches!(delimiter, b'"' | b'\r' | b'\n') {
        return Err(Failure::new("csv", "invalid delimiter"));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let bytes = text.as_bytes();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = Vec::new();
    let (mut i, mut line, mut row_line) = (0, 1, 1);
    let (mut quoted, mut closed, mut started) = (false, false, false);
    while i < bytes.len() {
        let b = bytes[i];
        if quoted {
            if b == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    field.push(b'"');
                    i += 1;
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(b);
                if b == b'\n' {
                    line += 1;
                }
            }
        } else if b == delimiter {
            row.push(String::from_utf8(std::mem::take(&mut field)).unwrap());
            closed = false;
            started = false;
        } else if b == b'\n' || b == b'\r' {
            if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            row.push(String::from_utf8(std::mem::take(&mut field)).unwrap());
            rows.push((row_line, std::mem::take(&mut row)));
            line += 1;
            row_line = line;
            closed = false;
            started = false;
        } else if b == b'"' && !started && !closed {
            quoted = true;
            started = true;
        } else {
            if closed || b == b'"' {
                return Err(Failure::new("csv", format!("invalid quote at line {line}")));
            }
            field.push(b);
            started = true;
        }
        i += 1;
    }
    if quoted {
        return Err(Failure::new(
            "csv",
            format!("unclosed quote at line {row_line}"),
        ));
    }
    if started || closed || !field.is_empty() || !row.is_empty() {
        row.push(String::from_utf8(field).unwrap());
        rows.push((row_line, row));
    }
    let mut iter = rows.into_iter();
    let headers = iter.next().map(|(_, row)| row).unwrap_or_default();
    let mut data = Vec::new();
    for (line, row) in iter {
        if row.len() != headers.len() {
            return Err(Failure::new("csv", format!("ragged row at line {line}")));
        }
        data.push(row);
    }
    Ok(CsvTable {
        headers,
        rows: data,
    })
}

pub fn split_unit(header: &str) -> (String, Option<String>) {
    let trimmed = header.trim();
    if let Some(prefix) = trimmed.strip_suffix(']') {
        if let Some((name, unit)) = prefix.rsplit_once('[') {
            if !name.trim().is_empty() && !unit.trim().is_empty() {
                return (name.trim().into(), Some(unit.trim().into()));
            }
        }
    }
    (trimmed.into(), None)
}
