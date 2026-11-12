//! Minimal XLSX ZIP and worksheet reader.
use crate::base::{Failure, Res};
use flate2::read::DeflateDecoder;
use std::{collections::HashMap, io::Read};

const LIMIT: usize = 64 * 1024 * 1024;
fn bad(message: impl Into<String>) -> Failure {
    Failure::new("xlsx", message)
}
fn slice(b: &[u8], start: usize, len: usize) -> Res<&[u8]> {
    b.get(
        start
            ..start
                .checked_add(len)
                .ok_or_else(|| bad("invalid ZIP offset"))?,
    )
    .ok_or_else(|| bad("truncated ZIP"))
}
fn u16_at(b: &[u8], p: usize) -> Res<u16> {
    Ok(u16::from_le_bytes(slice(b, p, 2)?.try_into().unwrap()))
}
fn u32_at(b: &[u8], p: usize) -> Res<u32> {
    Ok(u32::from_le_bytes(slice(b, p, 4)?.try_into().unwrap()))
}
fn unzip(bytes: &[u8]) -> Res<HashMap<String, Vec<u8>>> {
    if bytes.len() < 22 {
        return Err(bad("missing ZIP directory"));
    }
    let lower = bytes.len().saturating_sub(22 + 65535);
    let end = (lower..=bytes.len() - 22)
        .rev()
        .find(|&p| {
            bytes.get(p..p + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, p + 20).ok().map(|n| p + 22 + n as usize) == Some(bytes.len())
        })
        .ok_or_else(|| bad("missing ZIP directory"))?;
    let disk = u16_at(bytes, end + 4)?;
    let dir_disk = u16_at(bytes, end + 6)?;
    let disk_count = u16_at(bytes, end + 8)?;
    let count = u16_at(bytes, end + 10)?;
    let size = u32_at(bytes, end + 12)?;
    let offset = u32_at(bytes, end + 16)?;
    if disk != 0
        || dir_disk != 0
        || disk_count != count
        || count == u16::MAX
        || size == u32::MAX
        || offset == u32::MAX
    {
        return Err(bad("unsupported ZIP64 or multi-disk archive"));
    }
    let mut pos = offset as usize;
    let dir_end = pos
        .checked_add(size as usize)
        .ok_or_else(|| bad("invalid ZIP directory"))?;
    if dir_end > end {
        return Err(bad("invalid ZIP directory"));
    }
    let mut files = HashMap::new();
    for _ in 0..count {
        if u32_at(bytes, pos)? != 0x02014b50 {
            return Err(bad("invalid ZIP directory entry"));
        }
        let flags = u16_at(bytes, pos + 8)?;
        let method = u16_at(bytes, pos + 10)?;
        let compressed = u32_at(bytes, pos + 20)?;
        let plain = u32_at(bytes, pos + 24)?;
        let name_len = u16_at(bytes, pos + 28)? as usize;
        let extra_len = u16_at(bytes, pos + 30)? as usize;
        let comment_len = u16_at(bytes, pos + 32)? as usize;
        let start_disk = u16_at(bytes, pos + 34)?;
        let local = u32_at(bytes, pos + 42)?;
        if flags & 1 != 0 {
            return Err(bad("encrypted ZIP entry"));
        }
        if compressed == u32::MAX || plain == u32::MAX || local == u32::MAX || start_disk != 0 {
            return Err(bad("unsupported ZIP64 entry"));
        }
        if plain as usize > LIMIT || compressed as usize > LIMIT {
            return Err(bad("ZIP entry exceeds size limit"));
        }
        let name = std::str::from_utf8(slice(bytes, pos + 46, name_len)?)
            .map_err(|e| bad(e.to_string()))?
            .to_owned();
        let next = pos
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(|| bad("invalid ZIP entry"))?;
        if next > dir_end {
            return Err(bad("invalid ZIP entry"));
        }
        let local = local as usize;
        if u32_at(bytes, local)? != 0x04034b50
            || u16_at(bytes, local + 6)? & 1 != 0
            || u16_at(bytes, local + 8)? != method
        {
            return Err(bad("invalid ZIP local header"));
        }
        let start = local
            .checked_add(
                30 + u16_at(bytes, local + 26)? as usize + u16_at(bytes, local + 28)? as usize,
            )
            .ok_or_else(|| bad("invalid ZIP offset"))?;
        let data = slice(bytes, start, compressed as usize)?;
        if start
            .checked_add(data.len())
            .map_or(true, |n| n > offset as usize)
        {
            return Err(bad("invalid ZIP entry data"));
        }
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::new();
                DeflateDecoder::new(data)
                    .take((LIMIT + 1) as u64)
                    .read_to_end(&mut out)
                    .map_err(|e| bad(e.to_string()))?;
                out
            }
            _ => return Err(bad("unsupported ZIP compression")),
        };
        if content.len() != plain as usize || content.len() > LIMIT {
            return Err(bad("invalid ZIP entry size"));
        }
        files.insert(name, content);
        pos = next;
    }
    if pos != dir_end {
        return Err(bad("invalid ZIP directory size"));
    }
    Ok(files)
}

// Return matching element contents and their opening tags.
fn elements<'a>(xml: &'a str, name: &str) -> Res<Vec<(&'a str, &'a str)>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let open = format!("<{name}");
    let close = format!("</{name}>");
    while let Some(rel) = xml[pos..].find(&open) {
        let start = pos + rel;
        let boundary = xml.as_bytes().get(start + open.len()).copied();
        if !matches!(boundary, Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n')) {
            pos = start + open.len();
            continue;
        }
        let end = xml[start..]
            .find('>')
            .map(|n| start + n)
            .ok_or_else(|| bad("unterminated XML tag"))?;
        let tag = &xml[start + open.len()..end];
        if tag.trim_end().ends_with('/') {
            out.push((tag, ""));
            pos = end + 1;
            continue;
        }
        let finish = xml[end + 1..]
            .find(&close)
            .map(|n| end + 1 + n)
            .ok_or_else(|| bad("unclosed XML element"))?;
        out.push((tag, &xml[end + 1..finish]));
        pos = finish + close.len();
    }
    Ok(out)
}
fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name = &tag[start..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *b.get(i)?;
        if quote != b'\'' && quote != b'"' {
            return None;
        }
        i += 1;
        let begin = i;
        while i < b.len() && b[i] != quote {
            i += 1;
        }
        if i == b.len() {
            return None;
        }
        if name == key {
            return Some(&tag[begin..i]);
        }
        i += 1;
    }
    None
}
fn decode(s: &str) -> Res<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p + 1..];
        let end = rest.find(';').ok_or_else(|| bad("invalid XML entity"))?;
        let entity = &rest[..end];
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let number = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    entity.strip_prefix('#').and_then(|n| n.parse().ok())
                };
                number
                    .and_then(char::from_u32)
                    .ok_or_else(|| bad("invalid XML entity"))?
            }
        };
        out.push(ch);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}
fn text_runs(xml: &str) -> Res<String> {
    let mut out = String::new();
    for (_, text) in elements(xml, "t")? {
        out.push_str(&decode(text)?);
    }
    Ok(out)
}
fn column(reference: &str) -> Res<usize> {
    let mut n = 0usize;
    let mut letters = 0;
    for b in reference.bytes() {
        if !b.is_ascii_alphabetic() {
            break;
        }
        n = n
            .checked_mul(26)
            .and_then(|n| n.checked_add((b.to_ascii_uppercase() - b'A' + 1) as usize))
            .ok_or_else(|| bad("invalid cell reference"))?;
        letters += 1;
    }
    if letters == 0 || n > 16384 {
        return Err(bad("invalid cell reference"));
    }
    Ok(n - 1)
}
pub fn read_first_sheet(bytes: &[u8]) -> Res<Vec<Vec<String>>> {
    let files = unzip(bytes)?;
    let xml = |name: &str| -> Res<&str> {
        std::str::from_utf8(
            files
                .get(name)
                .ok_or_else(|| bad(format!("missing {name}")))?,
        )
        .map_err(|e| bad(e.to_string()))
    };
    let workbook = xml("xl/workbook.xml")?;
    if elements(workbook, "sheet")?.is_empty() {
        return Err(bad("workbook has no sheet"));
    }
    let shared = match files.get("xl/sharedStrings.xml") {
        Some(data) => {
            let s = std::str::from_utf8(data).map_err(|e| bad(e.to_string()))?;
            elements(s, "si")?
                .into_iter()
                .map(|(_, s)| text_runs(s))
                .collect::<Res<Vec<_>>>()?
        }
        None => Vec::new(),
    };
    let sheet = xml("xl/worksheets/sheet1.xml")?;
    let mut rows = Vec::new();
    for (_, body) in elements(sheet, "row")? {
        let mut row = Vec::new();
        for (tag, cell) in elements(body, "c")? {
            let col = match attr(tag, "r") {
                Some(r) => column(r)?,
                None => row.len(),
            };
            let kind = attr(tag, "t").unwrap_or("");
            let value = if kind == "inlineStr" {
                text_runs(cell)?
            } else {
                let value = elements(cell, "v")?.first().map(|(_, v)| *v).unwrap_or("");
                if kind == "s" {
                    let idx: usize = value
                        .trim()
                        .parse()
                        .map_err(|_| bad("invalid shared string index"))?;
                    shared
                        .get(idx)
                        .cloned()
                        .ok_or_else(|| bad("invalid shared string index"))?
                } else {
                    decode(value)?
                }
            };
            if row.len() <= col {
                row.resize(col + 1, String::new());
            }
            row[col] = value;
        }
        rows.push(row);
    }
    Ok(rows)
}
