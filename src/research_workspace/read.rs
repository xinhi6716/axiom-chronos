use super::manifest::{Format, Located};
use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};
fn err(kind: &str, msg: impl ToString) -> Failure {
    Failure::new(kind, msg.to_string())
}
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_record_bytes: usize,
    pub max_page_rows: usize,
    pub max_json_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_record_bytes: 1024 * 1024,
            max_page_rows: 500,
            max_json_bytes: 16 * 1024 * 1024,
        }
    }
}
pub fn guard(root: &Path, rel: &str) -> Res<PathBuf> {
    let bad = || err("escape", format!("unsafe path {rel}"));
    if rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains('\\')
        || rel.as_bytes().get(1) == Some(&b':')
    {
        return Err(bad());
    }
    let base = fs::canonicalize(root).map_err(|e| err("io", e))?;
    let mut path = base.clone();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(x) => path.push(x),
            Component::CurDir => continue,
            _ => return Err(bad()),
        };
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err(bad());
                }
                let canon = fs::canonicalize(&path).map_err(|_| bad())?;
                if !canon.starts_with(&base) {
                    return Err(bad());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err("io", e)),
        }
    }
    Ok(path)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceVersion {
    pub bytes: u64,
    pub modified_ns: i128,
    pub sha256: Option<String>,
}
pub fn version(p: &Path) -> Res<SourceVersion> {
    let m = fs::metadata(p).map_err(|e| {
        err(
            if e.kind() == std::io::ErrorKind::NotFound {
                "missing_artifact"
            } else {
                "io"
            },
            e,
        )
    })?;
    let t = m.modified().map_err(|e| err("io", e))?;
    let modified_ns = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    Ok(SourceVersion {
        bytes: m.len(),
        modified_ns,
        sha256: None,
    })
}
pub fn hash_file(p: &Path) -> Res<String> {
    let mut f = File::open(p).map_err(|e| {
        err(
            if e.kind() == std::io::ErrorKind::NotFound {
                "missing_artifact"
            } else {
                "io"
            },
            e,
        )
    })?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| err("io", e))?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn artifact<'a>(l: &'a Located, id: &str) -> Res<&'a super::manifest::Artifact> {
    l.artifact(id)
        .ok_or_else(|| err("missing_artifact", format!("artifact {id} missing")))
}
pub fn verify(l: &Located, id: &str) -> Res<String> {
    let a = artifact(l, id)?;
    let h = hash_file(&l.artifact_path(a)?)?;
    if a.sha256
        .as_ref()
        .is_some_and(|v| !v.eq_ignore_ascii_case(&h))
    {
        return Err(err("corrupt_artifact", format!("sha256 mismatch: {id}")));
    }
    Ok(h)
}
pub struct CsvReader<R: BufRead> {
    reader: R,
    lim: Limits,
    bytes: u64,
    first: bool,
}
impl<R: BufRead> CsvReader<R> {
    pub fn new(reader: R, lim: Limits) -> Self {
        Self {
            reader,
            lim,
            bytes: 0,
            first: true,
        }
    }
    pub fn bytes_read(&self) -> u64 {
        self.bytes
    }
    fn byte(&mut self) -> Res<Option<u8>> {
        let buf = self.reader.fill_buf().map_err(|e| err("io", e))?;
        if buf.is_empty() {
            return Ok(None);
        }
        let b = buf[0];
        self.reader.consume(1);
        self.bytes += 1;
        Ok(Some(b))
    }
    fn peek(&mut self) -> Res<Option<u8>> {
        Ok(self
            .reader
            .fill_buf()
            .map_err(|e| err("io", e))?
            .first()
            .copied())
    }
    pub fn next_record(&mut self) -> Res<Option<Vec<String>>> {
        let start = self.bytes;
        let mut fields = Vec::new();
        let mut field = Vec::new();
        let mut quoted = false;
        let mut closed = false;
        let mut started = false;
        loop {
            let b = self.byte()?;
            if self.bytes - start > self.lim.max_record_bytes as u64 {
                return Err(err("oversized_record", "CSV record exceeds limit"));
            }
            let Some(b) = b else {
                if quoted {
                    return Err(err("corrupt_artifact", "unterminated CSV quote"));
                }
                if !started && fields.is_empty() && field.is_empty() {
                    return Ok(None);
                }
                fields.push(field);
                break;
            };
            started = true;
            if quoted {
                if b == b'"' {
                    if self.peek()? == Some(b'"') {
                        self.byte()?;
                        field.push(b'"')
                    } else {
                        quoted = false;
                        closed = true
                    }
                } else {
                    field.push(b)
                }
                continue;
            }
            if closed && b != b',' && b != b'\r' && b != b'\n' {
                return Err(err("corrupt_artifact", "invalid CSV after quote"));
            }
            match b {
                b'"' if !closed
                    && (field.is_empty()
                        || (self.first && fields.is_empty() && field == [0xef, 0xbb, 0xbf])) =>
                {
                    field.clear();
                    quoted = true
                }
                b'"' => return Err(err("corrupt_artifact", "invalid CSV quote")),
                b',' => {
                    fields.push(std::mem::take(&mut field));
                    closed = false
                }
                b'\n' => {
                    fields.push(field);
                    break;
                }
                b'\r' => {
                    if self.peek()? == Some(b'\n') {
                        self.byte()?;
                    }
                    fields.push(field);
                    break;
                }
                _ => field.push(b),
            }
        }
        if self.bytes - start > self.lim.max_record_bytes as u64 {
            return Err(err("oversized_record", "CSV record exceeds limit"));
        }
        if self.first {
            self.first = false;
            if fields
                .first()
                .is_some_and(|f| f.starts_with(&[0xef, 0xbb, 0xbf]))
            {
                fields[0].drain(..3);
            }
        }
        fields
            .into_iter()
            .map(|v| String::from_utf8(v).map_err(|e| err("corrupt_artifact", e)))
            .collect::<Res<Vec<_>>>()
            .map(Some)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub artifact: String,
    pub bytes: u64,
    pub modified_ns: i128,
    pub offset: u64,
    pub row: u64,
}
impl Cursor {
    pub fn to_token(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_token(s: &str) -> Res<Self> {
        serde_json::from_str(s).map_err(|e| err("invalid_argument", e))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub number: u64,
    pub fields: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowsPage {
    pub rows: Vec<Row>,
    pub next: Option<Cursor>,
    pub version: SourceVersion,
}
fn remap(
    mut map: BTreeMap<String, String>,
    a: &super::manifest::Artifact,
) -> BTreeMap<String, String> {
    for (canonical, actual) in &a.columns {
        if let Some(v) = map.remove(actual) {
            map.insert(canonical.clone(), v);
        }
    }
    map
}
pub fn read_rows(
    l: &Located,
    id: &str,
    after: Option<&Cursor>,
    n: usize,
    lim: &Limits,
) -> Res<RowsPage> {
    let a = artifact(l, id)?;
    if a.format != Format::Csv && a.format != Format::Jsonl {
        return Err(err("invalid_argument", "artifact is not CSV or JSONL"));
    }
    let p = l.artifact_path(a)?;
    let v = version(&p)?;
    if a.sha256.is_some() {
        verify(l, id)?;
    }
    if let Some(c) = after {
        if c.artifact != id
            || c.bytes != v.bytes
            || c.modified_ns != v.modified_ns
            || c.offset > v.bytes
        {
            return Err(err("stale_cursor", "source changed"));
        }
    }
    let f = File::open(&p).map_err(|e| err("io", e))?;
    let mut r = BufReader::new(f);
    let mut header = Vec::new();
    let mut offset = 0;
    if a.format == Format::Csv {
        let mut csv = CsvReader::new(r, lim.clone());
        header = csv.next_record()?.unwrap_or_default();
        offset = csv.bytes_read();
        r = csv.reader;
    }
    let mut row = 0;
    if let Some(c) = after {
        if c.offset < offset {
            return Err(err("stale_cursor", "invalid offset"));
        }
        offset = c.offset;
        row = c.row;
    }
    r.seek(SeekFrom::Start(offset)).map_err(|e| err("io", e))?;
    let mut rows = Vec::new();
    let cap = n.min(lim.max_page_rows);
    while rows.len() < cap && offset < v.bytes {
        let fields = if a.format == Format::Csv {
            let mut csv = CsvReader::new(r, lim.clone());
            csv.first = false;
            let record = csv.next_record()?;
            offset += csv.bytes_read();
            r = csv.reader;
            let Some(record) = record else { break };
            header
                .iter()
                .cloned()
                .zip(record.into_iter().chain(std::iter::repeat(String::new())))
                .collect()
        } else {
            let mut line = Vec::new();
            let count = (&mut r)
                .take(lim.max_record_bytes as u64 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|e| err("io", e))?;
            offset += count as u64;
            if count > lim.max_record_bytes {
                return Err(err("oversized_record", format!("JSONL line {}", row + 1)));
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                row += 1;
                continue;
            }
            let obj: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&line)
                .map_err(|e| err("corrupt_artifact", format!("JSONL line {}: {e}", row + 1)))?;
            obj.into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        match v {
                            serde_json::Value::String(s) => s,
                            serde_json::Value::Null => String::new(),
                            v => v.to_string(),
                        },
                    )
                })
                .collect()
        };
        row += 1;
        rows.push(Row {
            number: row,
            fields: remap(fields, a),
        });
    }
    let next = if offset < v.bytes {
        Some(Cursor {
            artifact: id.into(),
            bytes: v.bytes,
            modified_ns: v.modified_ns,
            offset,
            row,
        })
    } else {
        None
    };
    Ok(RowsPage {
        rows,
        next,
        version: v,
    })
}
fn bounded(l: &Located, id: &str, lim: &Limits) -> Res<Vec<u8>> {
    let a = artifact(l, id)?;
    let p = l.artifact_path(a)?;
    let v = version(&p)?;
    if v.bytes > lim.max_json_bytes {
        return Err(err("oversized_record", format!("{id} exceeds limit")));
    }
    if a.sha256.is_some() {
        verify(l, id)?;
    }
    let mut b = Vec::new();
    File::open(p)
        .map_err(|e| err("io", e))?
        .take(lim.max_json_bytes.saturating_add(1))
        .read_to_end(&mut b)
        .map_err(|e| err("io", e))?;
    if b.len() as u64 > lim.max_json_bytes {
        return Err(err("oversized_record", id));
    }
    Ok(b)
}
pub fn read_json(l: &Located, id: &str, lim: &Limits) -> Res<serde_json::Value> {
    serde_json::from_slice(&bounded(l, id, lim)?).map_err(|e| err("corrupt_artifact", e))
}
pub fn read_text(l: &Located, id: &str, lim: &Limits) -> Res<String> {
    String::from_utf8(bounded(l, id, lim)?).map_err(|e| err("corrupt_artifact", e))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_bom_and_limit() {
        let mut r = CsvReader::new(
            &b"\xef\xbb\xbfa,b\r\n\"x,\ny\",z\r\n"[..],
            Limits::default(),
        );
        assert_eq!(r.next_record().unwrap().unwrap(), vec!["a", "b"]);
        assert_eq!(r.next_record().unwrap().unwrap(), vec!["x,\ny", "z"]);
        assert!(r.next_record().unwrap().is_none());
        let mut lim = Limits::default();
        lim.max_record_bytes = 2;
        assert_eq!(
            CsvReader::new(&b"abc\n"[..], lim)
                .next_record()
                .unwrap_err()
                .kind,
            "oversized_record"
        );
    }
    #[test]
    fn guard_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(guard(dir.path(), "../outside").unwrap_err().kind, "escape");
        assert_eq!(guard(dir.path(), "C:/outside").unwrap_err().kind, "escape");
    }
}
#[cfg(test)]
mod extra_tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Located) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("research-manifest.json"),r#"{"manifest_version":1,"research_id":"test","research_version":"1","title":"T","settings":{},"versions":{},"artifacts":[{"id":"a","role":"inventory","path":"a.csv","format":"csv","columns":{"path":"name"}},{"id":"j","role":"completed","path":"j.jsonl","format":"jsonl"}]}"#).unwrap();
        std::fs::write(d.path().join("a.csv"), "name,x\none,1\ntwo,2\n").unwrap();
        std::fs::write(d.path().join("j.jsonl"), "{\"x\":1}\n{\"x\":").unwrap();
        let l = super::super::manifest::locate(d.path(), &Default::default()).unwrap();
        (d, l)
    }
    #[test]
    fn pages_mapping_stale_and_jsonl_error() {
        let (d, l) = setup();
        let lim = Limits::default();
        let p = read_rows(&l, "a", None, 1, &lim).unwrap();
        assert_eq!(p.rows[0].fields["path"], "one");
        let cursor = p.next.unwrap();
        assert_eq!(Cursor::from_token(&cursor.to_token()).unwrap(), cursor);
        let p = read_rows(&l, "a", Some(&cursor), 5, &lim).unwrap();
        assert_eq!(p.rows[0].number, 2);
        assert!(p.next.is_none());
        std::fs::write(d.path().join("a.csv"), "different").unwrap();
        assert_eq!(
            read_rows(&l, "a", Some(&cursor), 1, &lim).unwrap_err().kind,
            "stale_cursor"
        );
        let err = read_rows(&l, "j", None, 10, &lim).unwrap_err();
        assert_eq!(err.kind, "corrupt_artifact");
        assert!(err.message.contains('2'));
    }
    #[test]
    fn checksum_and_bound() {
        let (d, mut l) = setup();
        l.manifest.artifacts[0].sha256 = Some("wrong".into());
        assert_eq!(verify(&l, "a").unwrap_err().kind, "corrupt_artifact");
        std::fs::write(d.path().join("j.jsonl"), "123456").unwrap();
        let mut lim = Limits::default();
        lim.max_json_bytes = 3;
        assert_eq!(
            read_text(&l, "j", &lim).unwrap_err().kind,
            "oversized_record"
        );
    }
    #[test]
    fn bom_before_quoted_first_field() {
        let mut r = CsvReader::new(&b"\xef\xbb\xbf\"a,b\",c\n"[..], Limits::default());
        assert_eq!(r.next_record().unwrap().unwrap(), vec!["a,b", "c"]);
    }
    #[test]
    fn jsonl_final_line_blanks_mapping_and_line_number() {
        let (d, mut l) = setup();
        l.manifest.artifacts[1]
            .columns
            .insert("path".into(), "name".into());
        fs::write(
            d.path().join("j.jsonl"),
            "\n{\"name\":\"first\"}\n  \n{\"name\":\"last\"}",
        )
        .unwrap();
        let first = read_rows(&l, "j", None, 1, &Limits::default()).unwrap();
        assert_eq!(first.rows[0].fields["path"], "first");
        let last = read_rows(&l, "j", first.next.as_ref(), 5, &Limits::default()).unwrap();
        assert_eq!(last.rows.len(), 1);
        assert_eq!(last.rows[0].fields["path"], "last");
        assert!(last.next.is_none());
        fs::write(d.path().join("j.jsonl"), "\n{\"x\":1}\n \n{\"x\":").unwrap();
        let e = read_rows(&l, "j", None, 10, &Limits::default()).unwrap_err();
        assert_eq!(e.kind, "corrupt_artifact");
        assert!(e.message.contains("JSONL line 4"), "{}", e.message);
    }
    #[test]
    fn jsonl_valid_unterminated_final_line() {
        let (d, l) = setup();
        fs::write(d.path().join("j.jsonl"), "{\"x\":1}\n{\"x\":2}").unwrap();
        let rows = read_rows(&l, "j", None, 10, &Limits::default())
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].fields["x"], "2");
    }
    #[test]
    fn csv_unequal_fields_are_padded_or_dropped() {
        let (d, l) = setup();
        fs::write(d.path().join("a.csv"), "name,x\none\ntwo,2,extra\n").unwrap();
        let rows = read_rows(&l, "a", None, 10, &Limits::default())
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fields["x"], "");
        assert_eq!(rows[1].fields["x"], "2");
        assert_eq!(rows[1].fields.len(), 2);
    }
    #[test]
    fn read_limits_json_and_missing() {
        let (d, mut l) = setup();
        fs::write(
            d.path().join("a.csv"),
            format!(
                "name,x\n{}",
                (0..510).map(|i| format!("n{i},{i}\n")).collect::<String>()
            ),
        )
        .unwrap();
        let p = read_rows(&l, "a", None, 1000, &Limits::default()).unwrap();
        assert_eq!(p.rows.len(), 500);
        assert_eq!(
            read_rows(&l, "a", p.next.as_ref(), 1000, &Limits::default())
                .unwrap()
                .rows
                .len(),
            10
        );
        let lim = Limits {
            max_record_bytes: 12,
            ..Default::default()
        };
        fs::write(d.path().join("a.csv"), "name,x\nlong-long-value,1\n").unwrap();
        assert_eq!(
            read_rows(&l, "a", None, 1, &lim).unwrap_err().kind,
            "oversized_record"
        );
        fs::write(d.path().join("j.jsonl"), "{\"long\":123456}\n").unwrap();
        assert_eq!(
            read_rows(&l, "j", None, 1, &lim).unwrap_err().kind,
            "oversized_record"
        );
        l.manifest.artifacts[1].format = Format::Json;
        fs::write(d.path().join("j.jsonl"), "{\"ok\":true}").unwrap();
        assert_eq!(read_json(&l, "j", &Limits::default()).unwrap()["ok"], true);
        assert_eq!(verify(&l, "absent").unwrap_err().kind, "missing_artifact");
        fs::remove_file(d.path().join("j.jsonl")).unwrap();
        assert_eq!(verify(&l, "j").unwrap_err().kind, "missing_artifact");
    }
    #[test]
    fn guard_absolute_and_link_component() {
        let d = tempfile::tempdir().unwrap();
        for path in ["/absolute", "C:/absolute", "\\absolute", "../outside"] {
            assert_eq!(guard(d.path(), path).unwrap_err().kind, "escape");
        }
        let outside = tempfile::tempdir().unwrap();
        let link = d.path().join("link");
        #[cfg(windows)]
        let created = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let created = false;
        if !created {
            eprintln!("skipping link guard: link creation failed");
            return;
        }
        assert_eq!(guard(d.path(), "link/file").unwrap_err().kind, "escape");
    }
}
