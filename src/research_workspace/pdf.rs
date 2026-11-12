//! PDF page rendering and bounded on-disk cache.
use super::{
    evidence::{Locator, Resolved, Target},
    index::Index,
    seams::{PageImage, PageRenderer},
};
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn sidecar(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".json");
    s.into()
}
fn size(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}
fn remove(p: &Path) {
    let _ = fs::remove_file(p);
    let _ = fs::remove_file(sidecar(p));
}

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    entries: Vec<String>,
} // oldest first
pub struct PageCache {
    dir: PathBuf,
    cap: u64,
    ledger: Mutex<Ledger>,
}
impl PageCache {
    pub fn open(dir: &Path, cap_bytes: u64) -> Res<Self> {
        fs::create_dir_all(dir).map_err(io)?;
        let mut ledger: Ledger = fs::read(dir.join("ledger.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Reconcile the ledger with actual files, including after a damaged ledger.
        let mut files: Vec<String> = fs::read_dir(dir)
            .map_err(io)?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                (e.file_type().ok()?.is_file() && !name.starts_with('.') && name.ends_with(".png"))
                    .then_some(name)
            })
            .collect();
        files.sort();
        ledger.entries.retain(|n| files.contains(n));
        let mut unique = Vec::new();
        for n in ledger.entries.drain(..).chain(files) {
            if !unique.contains(&n) {
                unique.push(n);
            }
        }
        ledger.entries = unique;
        let cache = Self {
            dir: dir.into(),
            cap: cap_bytes,
            ledger: Mutex::new(ledger),
        };
        {
            let mut l = cache.ledger.lock().unwrap_or_else(|e| e.into_inner());
            cache.evict(&mut l)?;
        }
        Ok(cache)
    }
    pub fn path_for(&self, sha: &str, page: u32, scale: f32) -> PathBuf {
        let safe = if !sha.is_empty() && sha.bytes().all(|b| b.is_ascii_alphanumeric()) {
            sha.to_owned()
        } else {
            crate::seams::sha256_hex(sha.as_bytes())
        };
        self.dir.join(format!(
            "{}-{}-{}.png",
            safe,
            page,
            (scale * 1000.0).round() as i64
        ))
    }
    fn total(&self, l: &Ledger) -> u64 {
        l.entries
            .iter()
            .map(|n| {
                let p = self.dir.join(n);
                size(&p).saturating_add(size(&sidecar(&p)))
            })
            .sum()
    }
    fn save(&self, l: &Ledger) -> Res<()> {
        base::atomic_write(
            &self.dir.join("ledger.json"),
            &serde_json::to_vec(l).map_err(io)?,
        )
        .map_err(io)
    }
    fn evict(&self, l: &mut Ledger) -> Res<()> {
        while self.total(l) > self.cap && !l.entries.is_empty() {
            let p = self.dir.join(&l.entries[0]);
            for path in [p.clone(), sidecar(&p)] {
                match fs::remove_file(path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(io(e)),
                }
            }
            l.entries.remove(0);
        }
        Ok(())
    }
    pub fn get(&self, sha: &str, page: u32, scale: f32) -> Option<PathBuf> {
        let p = self.path_for(sha, page, scale);
        let name = p.file_name()?.to_str()?.to_owned();
        let mut l = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        if !p.is_file() {
            l.entries.retain(|n| n != &name);
            return None;
        }
        l.entries.retain(|n| n != &name);
        l.entries.push(name);
        let _ = self.save(&l);
        Some(p)
    }
    pub fn put(&self, sha: &str, page: u32, scale: f32, src: &Path) -> Res<()> {
        self.put_inner(sha, page, scale, src, None)
    }
    fn put_inner(
        &self,
        sha: &str,
        page: u32,
        scale: f32,
        src: &Path,
        metadata: Option<&[u8]>,
    ) -> Res<()> {
        let p = self.path_for(sha, page, scale);
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let mut l = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = fs::read(src).map_err(io)?;
        if bytes.len() as u64 + metadata.map_or(0, |m| m.len() as u64) > self.cap {
            remove(&p);
            l.entries.retain(|n| n != &name);
            self.save(&l)?;
            return Err(Failure::new(
                "cache_too_small",
                "page exceeds cache capacity",
            ));
        }
        l.entries.retain(|n| n != &name);
        remove(&p);
        let result = (|| {
            base::atomic_write(&p, &bytes).map_err(io)?;
            if let Some(m) = metadata {
                base::atomic_write(&sidecar(&p), m).map_err(io)?;
            }
            Ok(())
        })();
        if result.is_err() {
            remove(&p);
            return result;
        }
        l.entries.push(name);
        self.evict(&mut l)?;
        self.save(&l)?;
        if src != p {
            let _ = fs::remove_file(src);
        }
        Ok(())
    }
    pub fn total_bytes(&self) -> u64 {
        self.total(&self.ledger.lock().unwrap_or_else(|e| e.into_inner()))
    }
    fn discard(&self, p: &Path) -> Res<()> {
        let mut l = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        for path in [p.to_path_buf(), sidecar(p)] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(io(e)),
            }
        }
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            l.entries.retain(|n| n != name);
        }
        self.save(&l)
    }
}

#[derive(Serialize, Deserialize)]
struct ImageInfo {
    path: PathBuf,
    width_px: u32,
    height_px: u32,
    page_width_pt: f64,
    page_height_pt: f64,
    rotation: u16,
}
impl From<PageImage> for ImageInfo {
    fn from(p: PageImage) -> Self {
        Self {
            path: p.path,
            width_px: p.width_px,
            height_px: p.height_px,
            page_width_pt: p.page_width_pt,
            page_height_pt: p.page_height_pt,
            rotation: p.rotation,
        }
    }
}
impl From<ImageInfo> for PageImage {
    fn from(p: ImageInfo) -> Self {
        Self {
            path: p.path,
            width_px: p.width_px,
            height_px: p.height_px,
            page_width_pt: p.page_width_pt,
            page_height_pt: p.page_height_pt,
            rotation: p.rotation,
        }
    }
}

pub struct PdfiumProcess {
    pub command: Vec<String>,
    pub script_dir: PathBuf,
    pub timeout: Duration,
}
impl PdfiumProcess {
    pub fn from_env(research_home: &Path) -> Self {
        Self {
            command: base::resolve_command("AXIOM_PYTHON_CMD", "python")
                .unwrap_or_else(|_| vec!["python".into()]),
            script_dir: research_home.join("cache"),
            timeout: Duration::from_secs(60),
        }
    }
}
impl PageRenderer for PdfiumProcess {
    fn render(
        &self,
        pdf: &Path,
        expected_sha256: &str,
        physical_page: u32,
        scale: f32,
        out: &Path,
    ) -> Res<PageImage> {
        const SCRIPT: &str = include_str!("render_page.py");
        fs::create_dir_all(&self.script_dir).map_err(io)?;
        let hash = crate::seams::sha256_hex(SCRIPT.as_bytes());
        let script = self
            .script_dir
            .join(format!("render_page-{}.py", &hash[..12]));
        base::atomic_write(&script, SCRIPT.as_bytes()).map_err(io)?;
        let mut cmd = base::proc::command(&self.command);
        cmd.arg("-I")
            .arg(script)
            .arg(pdf)
            .arg(expected_sha256)
            .arg(physical_page.to_string())
            .arg(scale.to_string())
            .arg(out)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let result = (|| {
            let child = cmd
                .spawn()
                .map_err(|e| Failure::new("renderer_missing", e.to_string()))?;
            let run = base::proc::capture(child, self.timeout)?;
            if run.timed_out {
                return Err(Failure::new("render_timeout", "PDF rendering timed out"));
            }
            if run.exit_code != 0 {
                let kind = match run.exit_code {
                    3 => "renderer_missing",
                    4 => "corrupt_pdf",
                    5 => "hash_mismatch",
                    6 => "page_out_of_range",
                    _ => "render_failed",
                };
                return Err(Failure::new(
                    kind,
                    String::from_utf8_lossy(&run.stderr).trim().to_owned(),
                ));
            }
            let mut info: ImageInfo = serde_json::from_slice(&run.stdout)
                .map_err(|e| Failure::new("render_failed", e.to_string()))?;
            if !out.is_file() {
                return Err(Failure::new(
                    "render_failed",
                    "renderer did not create image",
                ));
            }
            info.path = out.into();
            Ok(info.into())
        })();
        if result.is_err() {
            let _ = fs::remove_file(out);
        }
        result
    }
}
static SCRATCH_ID: AtomicU64 = AtomicU64::new(0);
pub fn page_image(
    ix: &Index,
    r: &dyn PageRenderer,
    cache: &PageCache,
    resolved: &Resolved,
    scale: f32,
) -> Res<PageImage> {
    let (document_path, page) = match &resolved.target {
        Target::PdfPage {
            document_path,
            physical_page,
            ..
        } => (document_path, *physical_page),
        _ => return Err(Failure::new("not_a_pdf_target", "target is not a PDF page")),
    };
    let sha = match &resolved.item.locator {
        Locator::PdfPage { sha256, .. } => sha256,
        _ => return Err(Failure::new("not_a_pdf_target", "item is not a PDF page")),
    };
    if !scale.is_finite() || scale <= 0.0 || scale * 1000.0 < 0.5 {
        return Err(Failure::new(
            "invalid_argument",
            "scale must be positive and finite",
        ));
    }
    let cache_dir = fs::canonicalize(&cache.dir).map_err(io)?;
    let located = ix.located();
    let mut roots = vec![located.root_path(None)?];
    for name in located.manifest.roots.keys() {
        roots.push(located.root_path(Some(name))?);
    }
    if roots.iter().any(|root| cache_dir.starts_with(root)) {
        return Err(Failure::new(
            "invalid_argument",
            "page cache is inside a manifest root",
        ));
    }
    let pdf = ix.located().document_path(document_path)?;
    if let Some(p) = cache.get(sha, page, scale) {
        if let Ok(bytes) = fs::read(sidecar(&p)) {
            if let Ok(mut info) = serde_json::from_slice::<ImageInfo>(&bytes) {
                info.path = p;
                return Ok(info.into());
            }
        }
        cache.discard(&p)?;
    }
    let scratch = cache.dir.join(format!(
        ".render-{}-{}.png",
        std::process::id(),
        SCRATCH_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let image = r.render(&pdf, sha, page, scale, &scratch)?;
        let mut info = ImageInfo::from(image);
        info.path = cache.path_for(sha, page, scale);
        let metadata = serde_json::to_vec(&info).map_err(io)?;
        cache.put_inner(sha, page, scale, &scratch, Some(&metadata))?;
        Ok(info.into())
    })();
    let _ = fs::remove_file(&scratch);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_workspace::{
        evidence,
        manifest::{self, LocateOptions},
        read::{self, Limits},
        seams::FakeRenderer,
    };
    use std::{collections::BTreeMap, sync::Arc};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram/project")
    }
    fn index() -> Index {
        Index::build(
            Arc::new(manifest::locate(&fixture(), &LocateOptions::default()).unwrap()),
            Limits::default(),
            &|_, _| {},
        )
        .unwrap()
    }
    fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
            for e in fs::read_dir(dir).unwrap().map(Result::unwrap) {
                let p = e.path();
                if e.file_type().unwrap().is_dir() {
                    walk(root, &p, out);
                } else {
                    out.insert(
                        p.strip_prefix(root).unwrap().into(),
                        read::hash_file(&p).unwrap(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        walk(root, root, &mut result);
        result
    }
    #[test]
    fn cache_lru_and_capacity() {
        let d = tempfile::tempdir().unwrap();
        let cache = PageCache::open(&d.path().join("pages"), 10).unwrap();
        let source = d.path().join("source");
        for key in ["a", "b"] {
            fs::write(&source, [1u8; 4]).unwrap();
            cache.put(key, 1, 1.0, &source).unwrap();
            assert!(cache.total_bytes() <= 10);
        }
        assert!(cache.get("a", 1, 1.0).is_some());
        fs::write(&source, [2u8; 4]).unwrap();
        cache.put("c", 1, 1.0, &source).unwrap();
        assert!(cache.total_bytes() <= 10);
        assert!(cache.get("b", 1, 1.0).is_none());
        assert!(cache.get("a", 1, 1.0).is_some());
        assert!(cache.get("c", 1, 1.0).is_some());
        fs::write(&source, [3u8; 11]).unwrap();
        assert_eq!(
            cache.put("big", 1, 1.0, &source).unwrap_err().kind,
            "cache_too_small"
        );
        assert!(cache.get("big", 1, 1.0).is_none());
        let reopened = PageCache::open(&d.path().join("pages"), 10).unwrap();
        assert_eq!(reopened.total_bytes(), 8);
    }
    #[test]
    fn damaged_ledger_is_rebuilt() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("ledger.json"), "broken").unwrap();
        fs::write(d.path().join("abc-1-1000.png"), [0u8; 4]).unwrap();
        let cache = PageCache::open(d.path(), 10).unwrap();
        assert_eq!(cache.total_bytes(), 4);
        assert!(cache.get("abc", 1, 1.0).is_some());
    }
    #[test]
    fn page_image_cache_and_failures() {
        let before = hashes(&fixture());
        let ix = index();
        let resolved = evidence::resolve(&ix, "E1", &Limits::default()).unwrap();
        let d = tempfile::tempdir().unwrap();
        let cache = PageCache::open(&d.path().join("pages"), 4096).unwrap();
        let renderer = FakeRenderer::new();
        let first = page_image(&ix, &renderer, &cache, &resolved, 1.0).unwrap();
        let second = page_image(&ix, &renderer, &cache, &resolved, 1.0).unwrap();
        assert_eq!(first, second);
        assert_eq!(renderer.calls().len(), 1);
        assert!(cache.total_bytes() > fs::metadata(&first.path).unwrap().len());
        let reopened = PageCache::open(&d.path().join("pages"), 4096).unwrap();
        assert_eq!(
            page_image(&ix, &renderer, &reopened, &resolved, 1.0).unwrap(),
            first
        );
        assert_eq!(renderer.calls().len(), 1);
        for kind in ["renderer_missing", "corrupt_pdf"] {
            let dir = tempfile::tempdir().unwrap();
            let empty = PageCache::open(dir.path(), 4096).unwrap();
            renderer.fail_next(kind);
            assert_eq!(
                page_image(&ix, &renderer, &empty, &resolved, 1.0)
                    .unwrap_err()
                    .kind,
                kind
            );
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
        let mut raw = resolved.clone();
        raw.target = Target::RawField {
            file: "x".into(),
            row: 1,
            column: "v".into(),
            value: "x".into(),
        };
        assert_eq!(
            page_image(&ix, &renderer, &cache, &raw, 1.0)
                .unwrap_err()
                .kind,
            "not_a_pdf_target"
        );
        let unsafe_cache = PageCache {
            dir: fixture(),
            cap: 4096,
            ledger: Mutex::new(Ledger::default()),
        };
        assert_eq!(
            page_image(&ix, &renderer, &unsafe_cache, &resolved, 1.0)
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
        assert_eq!(hashes(&fixture()), before);
    }
    #[test]
    fn real_process() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("page.png");
        let pdf = fixture().join("pdfs/a.pdf");
        let sha = "c0b3ff2c5cffd5bdcdf034af5e32399039fdc1a98dbbf4ff2a0214d4164b025b";
        let mut process = PdfiumProcess::from_env(d.path());
        process.command = vec!["axiom-nonexistent-python-executable".into()];
        assert_eq!(
            process.render(&pdf, sha, 2, 1.0, &out).unwrap_err().kind,
            "renderer_missing"
        );
        let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| "python".into());
        process.command = vec![python.clone(), "-c".into(), "import pathlib,sys,time; pathlib.Path(sys.argv[-1]).write_bytes(b'partial'); time.sleep(10)".into()];
        process.timeout = Duration::from_millis(500);
        assert_eq!(
            process.render(&pdf, sha, 2, 1.0, &out).unwrap_err().kind,
            "render_timeout"
        );
        assert!(!out.exists());
        process.timeout = Duration::from_secs(60);
        let import = base::proc::command(&[
            python.clone(),
            "-I".into(),
            "-c".into(),
            "import pypdfium2".into(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
        if !import.is_ok_and(|s| s.success()) {
            eprintln!("skipping real PDFium test: pypdfium2 cannot be imported by {python}");
            return;
        }
        process.command = vec![python];
        let image = process.render(&pdf, sha, 2, 1.0, &out).unwrap();
        assert!(
            image.width_px > 0
                && image.height_px > 0
                && image.page_width_pt > 0.0
                && image.page_height_pt > 0.0
        );
        assert!(fs::read(&out).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            process
                .render(&pdf, "wrong", 2, 1.0, &out)
                .unwrap_err()
                .kind,
            "hash_mismatch"
        );
        assert!(!out.exists());
        assert_eq!(
            process.render(&pdf, sha, 9, 1.0, &out).unwrap_err().kind,
            "page_out_of_range"
        );
        assert_eq!(
            process
                .render(
                    &fixture().join("pdfs/broken.pdf"),
                    "f125c3f4685ec03aa79d05f0d51ad3c024a51b2b115cb02b97604248fd5dc4ff",
                    1,
                    1.0,
                    &out
                )
                .unwrap_err()
                .kind,
            "corrupt_pdf"
        );
        let hash = crate::seams::sha256_hex(include_str!("render_page.py").as_bytes());
        assert!(d
            .path()
            .join("cache")
            .join(format!("render_page-{}.py", &hash[..12]))
            .is_file());
    }
}
