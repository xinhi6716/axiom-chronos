use crate::{
    base::{Failure, Res},
    desktop::ux::seams::{PaperService, Preview},
    paper::{service::PaperStatus, Requirements, SpendEstimate},
};
use std::path::PathBuf;

pub struct SessionPaper {
    home: PathBuf,
}
impl SessionPaper {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    fn unimplemented<T>(&self) -> Res<T> {
        let _ = &self.home;
        Err(Failure::new(
            "unimplemented",
            "paper adapter is not implemented",
        ))
    }
}
impl PaperService for SessionPaper {
    fn status(&self, _: &str) -> Res<PaperStatus> {
        self.unimplemented()
    }
    fn request_shortlist(&self, _: &str, _: Requirements, _: Option<PathBuf>) -> Res<()> {
        self.unimplemented()
    }
    fn choose_journal(&self, _: &str, _: &str, _: &str) -> Res<()> {
        self.unimplemented()
    }
    fn estimate(&self, _: &str) -> Res<SpendEstimate> {
        self.unimplemented()
    }
    fn start(&self, _: &str, _: &str) -> Res<()> {
        self.unimplemented()
    }
    fn cancel(&self, _: &str) -> Res<()> {
        self.unimplemented()
    }
    fn preview(&self, _: &str, _: u32, _: u32) -> Res<Preview> {
        self.unimplemented()
    }
    fn open_pdf(&self, _: &str) -> Res<()> {
        self.unimplemented()
    }
}
