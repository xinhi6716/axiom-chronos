use crate::base::{Failure, Res};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRef {
    pub snapshot_id: String,
    pub research_id: String,
    pub research_version: String,
    pub path: PathBuf,
}
pub trait SnapshotConsumer: Send + Sync {
    fn name(&self) -> String;
    fn consume(&self, s: &SnapshotRef) -> Res<()>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultLink {
    pub research_id: String,
    pub research_version: String,
    pub snapshot_id: Option<String>,
}
pub trait VaultActions: Send + Sync {
    fn name(&self) -> String;
    fn link(&self, l: &VaultLink) -> Res<()>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct PageImage {
    pub path: PathBuf,
    pub width_px: u32,
    pub height_px: u32,
    pub page_width_pt: f64,
    pub page_height_pt: f64,
    pub rotation: u16,
}
pub trait PageRenderer: Send + Sync {
    fn render(
        &self,
        pdf: &Path,
        expected_sha256: &str,
        physical_page: u32,
        scale: f32,
        out: &Path,
    ) -> Res<PageImage>;
}
#[derive(Default)]
struct State<T> {
    calls: Vec<T>,
    failure: Option<String>,
}
fn run<T>(state: &Mutex<State<T>>, call: T) -> Res<()> {
    let mut s = state.lock().unwrap();
    s.calls.push(call);
    if let Some(k) = s.failure.take() {
        Err(Failure::new(&k, "scripted failure"))
    } else {
        Ok(())
    }
}
pub struct FakeSnapshotConsumer {
    state: Mutex<State<SnapshotRef>>,
}
impl FakeSnapshotConsumer {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                calls: vec![],
                failure: None,
            }),
        }
    }
    pub fn calls(&self) -> Vec<SnapshotRef> {
        self.state.lock().unwrap().calls.clone()
    }
    pub fn fail_next(&self, kind: &str) {
        self.state.lock().unwrap().failure = Some(kind.into())
    }
}
impl Default for FakeSnapshotConsumer {
    fn default() -> Self {
        Self::new()
    }
}
impl SnapshotConsumer for FakeSnapshotConsumer {
    fn name(&self) -> String {
        "fake_snapshot_consumer".into()
    }
    fn consume(&self, s: &SnapshotRef) -> Res<()> {
        run(&self.state, s.clone())
    }
}
pub struct FakeVault {
    state: Mutex<State<VaultLink>>,
}
impl FakeVault {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                calls: vec![],
                failure: None,
            }),
        }
    }
    pub fn calls(&self) -> Vec<VaultLink> {
        self.state.lock().unwrap().calls.clone()
    }
    pub fn fail_next(&self, kind: &str) {
        self.state.lock().unwrap().failure = Some(kind.into())
    }
}
impl Default for FakeVault {
    fn default() -> Self {
        Self::new()
    }
}
impl VaultActions for FakeVault {
    fn name(&self) -> String {
        "fake_vault".into()
    }
    fn link(&self, l: &VaultLink) -> Res<()> {
        run(&self.state, l.clone())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct RenderCall {
    pub pdf: PathBuf,
    pub expected_sha256: String,
    pub physical_page: u32,
    pub scale: f32,
    pub out: PathBuf,
}
pub struct FakeRenderer {
    state: Mutex<State<RenderCall>>,
}
impl FakeRenderer {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                calls: vec![],
                failure: None,
            }),
        }
    }
    pub fn calls(&self) -> Vec<RenderCall> {
        self.state.lock().unwrap().calls.clone()
    }
    pub fn fail_next(&self, kind: &str) {
        self.state.lock().unwrap().failure = Some(kind.into())
    }
}
impl Default for FakeRenderer {
    fn default() -> Self {
        Self::new()
    }
}
impl PageRenderer for FakeRenderer {
    fn render(
        &self,
        pdf: &Path,
        expected_sha256: &str,
        physical_page: u32,
        scale: f32,
        out: &Path,
    ) -> Res<PageImage> {
        run(
            &self.state,
            RenderCall {
                pdf: pdf.into(),
                expected_sha256: expected_sha256.into(),
                physical_page,
                scale,
                out: out.into(),
            },
        )?;
        const PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 114, 156, 82, 103, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ];
        crate::base::atomic_write(out, PNG).map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(PageImage {
            path: out.into(),
            width_px: 1,
            height_px: 1,
            page_width_pt: 1.0,
            page_height_pt: 1.0,
            rotation: 0,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fake_consumer_records_and_fails_once() {
        let f = FakeSnapshotConsumer::new();
        let s = SnapshotRef {
            snapshot_id: "s".into(),
            research_id: "r".into(),
            research_version: "v".into(),
            path: "x".into(),
        };
        f.fail_next("io");
        assert_eq!(f.consume(&s).unwrap_err().kind, "io");
        f.consume(&s).unwrap();
        assert_eq!(f.calls().len(), 2);
    }
}
#[cfg(test)]
mod extra_tests {
    use super::*;
    #[test]
    fn vault_and_renderer_record_failures() {
        let v = FakeVault::new();
        let l = VaultLink {
            research_id: "r".into(),
            research_version: "v".into(),
            snapshot_id: None,
        };
        v.fail_next("io");
        assert_eq!(v.link(&l).unwrap_err().kind, "io");
        v.link(&l).unwrap();
        assert_eq!(v.calls(), vec![l.clone(), l]);
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("out.png");
        let r = FakeRenderer::new();
        r.fail_next("renderer_missing");
        assert_eq!(
            r.render(Path::new("a"), "abc", 1, 1.0, &out)
                .unwrap_err()
                .kind,
            "renderer_missing"
        );
        let p = r.render(Path::new("a"), "abc", 1, 1.0, &out).unwrap();
        assert_eq!((p.width_px, p.height_px), (1, 1));
        assert_eq!(r.calls().len(), 2);
        assert!(std::fs::read(out)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
