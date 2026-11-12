use crate::{
    base::Res,
    paper::figures::audit::{AuditIssue, IssueKind},
};
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    ops::RangeInclusive,
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageReport {
    pub pages: u32,
    pub issues: Vec<AuditIssue>,
    pub vector_figures: bool,
}

#[derive(Clone, Debug)]
enum Value {
    Num(f64),
    Name(String),
    Array(Vec<Value>),
    Dict(HashMap<String, Value>),
    Ref(u32),
    Word(String),
    Other,
}
impl Value {
    fn get(&self, key: &str) -> Option<&Value> {
        if let Value::Dict(d) = self {
            d.get(key)
        } else {
            None
        }
    }
    fn number(&self) -> Option<f64> {
        if let Value::Num(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    fn name(&self) -> Option<&str> {
        if let Value::Name(n) = self {
            Some(n)
        } else {
            None
        }
    }
    fn array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
    fn uint(&self) -> Option<usize> {
        let n = self.number()?;
        if n >= 0.0 && n.fract() == 0.0 && n <= usize::MAX as f64 {
            Some(n as usize)
        } else {
            None
        }
    }
}
struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Parser<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }
    fn skip(&mut self) {
        loop {
            while self.pos < self.data.len()
                && (self.data[self.pos].is_ascii_whitespace() || self.data[self.pos] == 0)
            {
                self.pos += 1;
            }
            if self.data.get(self.pos) != Some(&b'%') {
                break;
            }
            while self.pos < self.data.len() && !matches!(self.data[self.pos], b'\r' | b'\n') {
                self.pos += 1;
            }
        }
    }
    fn token(&mut self) -> Option<String> {
        self.skip();
        let start = self.pos;
        while self.pos < self.data.len()
            && !self.data[self.pos].is_ascii_whitespace()
            && !b"()<>[]{}/%\0".contains(&self.data[self.pos])
        {
            self.pos += 1;
        }
        if start == self.pos {
            return None;
        }
        Some(String::from_utf8_lossy(&self.data[start..self.pos]).into_owned())
    }
    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > 100 {
            return None;
        }
        self.skip();
        let c = *self.data.get(self.pos)?;
        if c == b'/' {
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.data.len()
                && !self.data[self.pos].is_ascii_whitespace()
                && !b"()<>[]{}/%\0".contains(&self.data[self.pos])
            {
                self.pos += 1;
            }
            let mut name = Vec::new();
            let mut i = start;
            while i < self.pos {
                if self.data[i] == b'#' && i + 2 < self.pos {
                    if let Ok(n) =
                        u8::from_str_radix(std::str::from_utf8(&self.data[i + 1..i + 3]).ok()?, 16)
                    {
                        name.push(n);
                        i += 3;
                        continue;
                    }
                }
                name.push(self.data[i]);
                i += 1;
            }
            return Some(Value::Name(String::from_utf8_lossy(&name).into_owned()));
        }
        if c == b'[' {
            self.pos += 1;
            let mut out = Vec::new();
            loop {
                self.skip();
                if *self.data.get(self.pos)? == b']' {
                    self.pos += 1;
                    return Some(Value::Array(out));
                }
                out.push(self.value(depth + 1)?);
            }
        }
        if c == b'<' && self.data.get(self.pos + 1) == Some(&b'<') {
            self.pos += 2;
            let mut out = HashMap::new();
            loop {
                self.skip();
                if self.data.get(self.pos..self.pos + 2) == Some(b">>") {
                    self.pos += 2;
                    return Some(Value::Dict(out));
                }
                let Value::Name(key) = self.value(depth + 1)? else {
                    return None;
                };
                out.insert(key, self.value(depth + 1)?);
            }
        }
        if c == b'(' {
            self.pos += 1;
            let mut nesting = 1;
            while nesting > 0 {
                let c = *self.data.get(self.pos)?;
                self.pos += 1;
                match c {
                    b'\\' => {
                        self.pos += 1;
                    }
                    b'(' => nesting += 1,
                    b')' => nesting -= 1,
                    _ => {}
                }
            }
            return Some(Value::Other);
        }
        if c == b'<' {
            self.pos += 1;
            while *self.data.get(self.pos)? != b'>' {
                self.pos += 1;
            }
            self.pos += 1;
            return Some(Value::Other);
        }
        let word = self.token()?;
        if let Ok(n) = word.parse::<f64>() {
            let saved = self.pos;
            if n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
                if let Some(generation) = self.token() {
                    if generation.parse::<u32>().is_ok() && self.token().as_deref() == Some("R") {
                        return Some(Value::Ref(n as u32));
                    }
                }
            }
            self.pos = saved;
            Some(Value::Num(n))
        } else {
            Some(Value::Word(word))
        }
    }
}
#[derive(Clone)]
struct Object {
    value: Value,
    stream: Option<Vec<u8>>,
}
fn indirect(data: &[u8], offset: usize) -> Option<(u32, Object)> {
    let mut p = Parser::new(data, offset);
    let id = p.token()?.parse().ok()?;
    p.token()?.parse::<u32>().ok()?;
    if p.token()? != "obj" {
        return None;
    }
    let value = p.value(0)?;
    p.skip();
    let mut stream = None;
    if data.get(p.pos..p.pos + 6) == Some(b"stream") {
        p.pos += 6;
        if data.get(p.pos) == Some(&b'\r') {
            p.pos += 1;
        }
        if data.get(p.pos) != Some(&b'\n') {
            return None;
        }
        p.pos += 1;
        let end = if let Some(len) = value.get("Length").and_then(Value::uint) {
            p.pos.checked_add(len)?
        } else {
            p.pos
                + data
                    .get(p.pos..)?
                    .windows(9)
                    .position(|w| w == b"endstream")?
        };
        let bytes = data.get(p.pos..end)?.to_vec();
        p.pos = end;
        p.skip();
        if data.get(p.pos..p.pos + 9) != Some(b"endstream") {
            return None;
        }
        p.pos += 9;
        stream = Some(bytes);
        p.skip();
    }
    if data.get(p.pos..p.pos + 6) != Some(b"endobj") {
        return None;
    }
    Some((id, Object { value, stream }))
}
fn decoded(object: &Object) -> Option<Vec<u8>> {
    let mut bytes = object.stream.clone()?;
    let filters: Vec<&str> = match object.value.get("Filter") {
        None => vec![],
        Some(Value::Name(s)) => vec![s],
        Some(Value::Array(a)) => a.iter().map(Value::name).collect::<Option<_>>()?,
        _ => return None,
    };
    for filter in filters {
        if filter != "FlateDecode" && filter != "Fl" {
            return None;
        }
        let mut out = Vec::new();
        ZlibDecoder::new(&bytes[..]).read_to_end(&mut out).ok()?;
        bytes = out;
    }
    Some(bytes)
}
struct Pdf {
    objects: HashMap<u32, Object>,
    root: u32,
}
impl Pdf {
    fn read(data: &[u8]) -> Option<Self> {
        if !data.starts_with(b"%PDF-") {
            return None;
        }
        let end = data.windows(5).rposition(|w| w == b"%%EOF")?;
        if data[end + 5..]
            .iter()
            .any(|b| !b.is_ascii_whitespace() && *b != 0)
        {
            return None;
        }
        let start = data[..end].windows(9).rposition(|w| w == b"startxref")?;
        let mut p = Parser::new(data, start + 9);
        let mut next = Some(p.token()?.parse::<usize>().ok()?);
        let mut seen = HashSet::new();
        let mut entries: HashMap<u32, (u8, usize, usize)> = HashMap::new();
        let mut root = None;
        while let Some(offset) = next {
            if offset >= start || !seen.insert(offset) {
                return None;
            }
            let mut x = Parser::new(data, offset);
            x.skip();
            let trailer;
            if data.get(x.pos..x.pos + 4) == Some(b"xref") {
                x.pos += 4;
                loop {
                    x.skip();
                    if data.get(x.pos..x.pos + 7) == Some(b"trailer") {
                        x.pos += 7;
                        trailer = x.value(0)?;
                        break;
                    }
                    let first: u32 = x.token()?.parse().ok()?;
                    let count: u32 = x.token()?.parse().ok()?;
                    for i in 0..count {
                        let a: usize = x.token()?.parse().ok()?;
                        let b: usize = x.token()?.parse().ok()?;
                        let status = x.token()?;
                        let kind = match status.as_str() {
                            "n" => 1,
                            "f" => 0,
                            _ => return None,
                        };
                        entries.entry(first.checked_add(i)?).or_insert((kind, a, b));
                    }
                }
            } else {
                let (_, obj) = indirect(data, offset)?;
                if obj.value.get("Type")?.name()? != "XRef" {
                    return None;
                }
                let widths = obj
                    .value
                    .get("W")?
                    .array()?
                    .iter()
                    .map(Value::uint)
                    .collect::<Option<Vec<_>>>()?;
                if widths.len() != 3 || widths.iter().any(|w| *w > 8) {
                    return None;
                }
                let size = obj.value.get("Size")?.uint()?;
                let index = match obj.value.get("Index") {
                    Some(v) => v
                        .array()?
                        .iter()
                        .map(Value::uint)
                        .collect::<Option<Vec<_>>>()?,
                    None => vec![0, size],
                };
                if index.len() % 2 != 0 {
                    return None;
                }
                let bytes = decoded(&obj)?;
                let mut pos = 0;
                for pair in index.chunks(2) {
                    for i in 0..pair[1] {
                        let mut vals = [0usize; 3];
                        vals[0] = 1;
                        for j in 0..3 {
                            if widths[j] > 0 {
                                vals[j] = 0;
                                for _ in 0..widths[j] {
                                    vals[j] = vals[j]
                                        .checked_mul(256)?
                                        .checked_add(*bytes.get(pos)? as usize)?;
                                    pos += 1;
                                }
                            }
                        }
                        entries
                            .entry(u32::try_from(pair[0].checked_add(i)?).ok()?)
                            .or_insert((u8::try_from(vals[0]).ok()?, vals[1], vals[2]));
                    }
                }
                trailer = obj.value;
            }
            if root.is_none() {
                if let Some(Value::Ref(id)) = trailer.get("Root") {
                    root = Some(*id);
                }
            }
            next = trailer.get("Prev").and_then(Value::uint);
        }
        let mut objects = HashMap::new();
        for (&id, &(kind, offset, _)) in &entries {
            if kind == 1 {
                let (actual, obj) = indirect(data, offset)?;
                if actual != id {
                    return None;
                }
                objects.insert(id, obj);
            }
        }
        let compressed: Vec<_> = entries
            .iter()
            .filter_map(|(&id, &(kind, container, index))| {
                (kind == 2).then_some((id, container, index))
            })
            .collect();
        for (id, container, index) in compressed {
            let obj = objects.get(&u32::try_from(container).ok()?)?;
            if obj.value.get("Type")?.name()? != "ObjStm" {
                return None;
            }
            let n = obj.value.get("N")?.uint()?;
            let first = obj.value.get("First")?.uint()?;
            if index >= n {
                return None;
            }
            let bytes = decoded(obj)?;
            if first > bytes.len() {
                return None;
            }
            let mut p = Parser::new(&bytes[..first], 0);
            let mut positions = Vec::new();
            for _ in 0..n {
                let obj_id: u32 = p.token()?.parse().ok()?;
                let offset: usize = p.token()?.parse().ok()?;
                positions.push((obj_id, offset));
            }
            if positions[index].0 != id {
                return None;
            }
            let start = first.checked_add(positions[index].1)?;
            let end = if index + 1 < n {
                first.checked_add(positions[index + 1].1)?
            } else {
                bytes.len()
            };
            let value = Parser::new(bytes.get(start..end)?, 0).value(0)?;
            objects.insert(
                id,
                Object {
                    value,
                    stream: None,
                },
            );
        }
        Some(Self {
            objects,
            root: root?,
        })
    }
    fn resolve<'a>(&'a self, value: &'a Value) -> Option<&'a Value> {
        let mut v = value;
        let mut seen = HashSet::new();
        while let Value::Ref(id) = v {
            if !seen.insert(*id) {
                return None;
            }
            v = &self.objects.get(id)?.value;
        }
        Some(v)
    }
    fn object<'a>(&'a self, v: &'a Value) -> Option<&'a Object> {
        if let Value::Ref(id) = v {
            self.objects.get(id)
        } else {
            None
        }
    }
    fn pages(&self) -> Option<Vec<(Value, Value, Value)>> {
        let root = self.objects.get(&self.root)?.value.get("Pages")?;
        let mut pages = Vec::new();
        let mut visited = HashSet::new();
        self.walk(root, None, None, &mut visited, &mut pages)?;
        Some(pages)
    }
    fn walk(
        &self,
        node: &Value,
        media: Option<Value>,
        resources: Option<Value>,
        seen: &mut HashSet<u32>,
        out: &mut Vec<(Value, Value, Value)>,
    ) -> Option<()> {
        if let Value::Ref(id) = node {
            if !seen.insert(*id) {
                return None;
            }
        }
        let dict = self.resolve(node)?;
        let media = dict.get("MediaBox").cloned().or(media);
        let resources = dict.get("Resources").cloned().or(resources);
        match dict.get("Type")?.name()? {
            "Pages" => {
                let before = out.len();
                for child in self.resolve(dict.get("Kids")?)?.array()? {
                    self.walk(child, media.clone(), resources.clone(), seen, out)?;
                }
                if dict.get("Count")?.uint()? != out.len() - before {
                    return None;
                }
            }
            "Page" => {
                out.push((
                    dict.clone(),
                    media?,
                    resources.unwrap_or(Value::Dict(HashMap::new())),
                ));
            }
            _ => return None,
        }
        Some(())
    }
}
// PDF affine matrices transform (x,y) to (ax+cy+e,bx+dy+f).
type Matrix = [f64; 6];
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn image_area(m: Matrix, bounds: [f64; 4]) -> f64 {
    let point = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
    let mut polygon = vec![point(0., 0.), point(1., 0.), point(1., 1.), point(0., 1.)];
    for (axis, edge, greater) in [
        (0, bounds[0], true),
        (0, bounds[2], false),
        (1, bounds[1], true),
        (1, bounds[3], false),
    ] {
        let input = std::mem::take(&mut polygon);
        if input.is_empty() {
            break;
        }
        let coord = |p: (f64, f64)| if axis == 0 { p.0 } else { p.1 };
        let inside = |p| {
            if greater {
                coord(p) >= edge
            } else {
                coord(p) <= edge
            }
        };
        let mut previous = *input.last().unwrap();
        for current in input {
            if inside(current) != inside(previous) {
                let t = (edge - coord(previous)) / (coord(current) - coord(previous));
                polygon.push((
                    previous.0 + t * (current.0 - previous.0),
                    previous.1 + t * (current.1 - previous.1),
                ));
            }
            if inside(current) {
                polygon.push(current);
            }
            previous = current;
        }
    }
    if polygon.len() < 3 {
        return 0.;
    }
    (0..polygon.len())
        .map(|i| {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            a.0 * b.1 - a.1 * b.0
        })
        .sum::<f64>()
        .abs()
        / 2.
}
impl Pdf {
    fn numbers(&self, v: &Value, n: usize) -> Option<Vec<f64>> {
        let a = self.resolve(v)?.array()?;
        if a.len() != n {
            return None;
        }
        a.iter().map(|v| self.resolve(v)?.number()).collect()
    }
    fn content(&self, v: &Value) -> Option<Vec<u8>> {
        if let Some(object) = self.object(v) {
            if object.stream.is_some() {
                return decoded(object);
            }
        }
        let mut out = Vec::new();
        for part in self.resolve(v)?.array()? {
            out.extend(self.content(part)?);
            out.push(b'\n');
        }
        Some(out)
    }
    fn has_raster(
        &self,
        content: &[u8],
        resources: &Value,
        bounds: [f64; 4],
        initial: Matrix,
        depth: usize,
    ) -> Option<bool> {
        if depth > 32 {
            return None;
        }
        let resources = self.resolve(resources)?;
        let xobjects = match resources.get("XObject") {
            Some(v) => Some(self.resolve(v)?),
            None => None,
        };
        let page_area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
        if page_area <= 0. {
            return None;
        }
        let mut parser = Parser::new(content, 0);
        let mut operands = Vec::new();
        let mut matrix = initial;
        let mut stack = Vec::new();
        loop {
            parser.skip();
            if parser.pos == content.len() {
                break;
            }
            let value = parser.value(0)?;
            if let Value::Word(op) = &value {
                match op.as_str() {
                    "q" => stack.push(matrix),
                    "Q" => matrix = stack.pop()?,
                    "cm" => {
                        if operands.len() < 6 {
                            return None;
                        }
                        let a = &operands[operands.len() - 6..];
                        let nums: Vec<f64> = a.iter().map(Value::number).collect::<Option<_>>()?;
                        matrix = multiply(
                            matrix,
                            [nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]],
                        );
                    }
                    "Do" => {
                        let name = operands.last()?.name()?;
                        let xobj = xobjects?.get(name)?;
                        let obj = self.object(xobj)?;
                        match obj.value.get("Subtype").and_then(Value::name) {
                            Some("Image") => {
                                if image_area(matrix, bounds) >= 0.9 * page_area {
                                    return Some(true);
                                }
                            }
                            Some("Form") => {
                                let m = if let Some(v) = obj.value.get("Matrix") {
                                    let n = self.numbers(v, 6)?;
                                    [n[0], n[1], n[2], n[3], n[4], n[5]]
                                } else {
                                    [1., 0., 0., 1., 0., 0.]
                                };
                                let form_resources =
                                    obj.value.get("Resources").unwrap_or(resources);
                                if self.has_raster(
                                    &decoded(obj)?,
                                    form_resources,
                                    bounds,
                                    multiply(matrix, m),
                                    depth + 1,
                                )? {
                                    return Some(true);
                                }
                            }
                            _ => {}
                        }
                    }
                    // Inline images have arbitrary binary content, and cannot safely be skipped as tokens.
                    "BI" => return None,
                    _ => {}
                }
                operands.clear();
            } else {
                operands.push(value);
            }
        }
        Some(false)
    }
}
fn issue(target: String, kind: IssueKind, detail: String) -> AuditIssue {
    AuditIssue {
        target,
        kind,
        detail,
    }
}
fn inspect(data: &[u8]) -> Option<PageReport> {
    let pdf = Pdf::read(data)?;
    let pages = pdf.pages()?;
    let mut issues = Vec::new();
    for (i, (page, media, resources)) in pages.iter().enumerate() {
        let n = pdf.numbers(media, 4)?;
        let bounds = [
            n[0].min(n[2]),
            n[1].min(n[3]),
            n[0].max(n[2]),
            n[1].max(n[3]),
        ];
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            return None;
        }
        if let Some(contents) = page.get("Contents") {
            if pdf.has_raster(
                &pdf.content(contents)?,
                resources,
                bounds,
                [1., 0., 0., 1., 0., 0.],
                0,
            )? {
                issues.push(issue(
                    format!("page:{}", i + 1),
                    IssueKind::PageRaster,
                    "Image covers at least 90% of the page MediaBox".into(),
                ));
            }
        }
    }
    let vector_figures = issues.is_empty();
    Some(PageReport {
        pages: u32::try_from(pages.len()).ok()?,
        issues,
        vector_figures,
    })
}
pub fn check_pdf(
    pdf: &Path,
    log: &Path,
    expect_pages: Option<RangeInclusive<u32>>,
) -> Res<PageReport> {
    let mut report = match fs::read(pdf).ok().and_then(|data| inspect(&data)) {
        Some(report) => report,
        None => {
            return Ok(PageReport {
                pages: 0,
                issues: vec![issue(
                    "page:0".into(),
                    IssueKind::Unreadable,
                    format!("Cannot read or parse PDF: {}", pdf.display()),
                )],
                vector_figures: false,
            })
        }
    };
    if let Ok(text) = fs::read_to_string(log) {
        for line in text
            .lines()
            .filter(|line| line.contains("Missing character"))
        {
            report.issues.push(issue(
                "page:0".into(),
                IssueKind::MissingCharacter,
                line.to_owned(),
            ));
        }
    }
    if let Some(range) = expect_pages {
        if !range.contains(&report.pages) {
            report.issues.push(issue(
                format!("page:{}", report.pages),
                IssueKind::PageCount,
                format!("Page count {} is outside {:?}", report.pages, range),
            ));
        }
    }
    Ok(report)
}
