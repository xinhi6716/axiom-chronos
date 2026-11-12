use super::{context, runs, store::Transcript, Message, Run, Session};
use crate::base::Res;
use std::path::{Path, PathBuf};

pub struct ModesApi {
    home: PathBuf,
}
impl ModesApi {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }
    pub fn open_session(&self, project: Option<&Path>, scope: &[String]) -> Res<Session> {
        runs::create_session(&self.home, project, scope)
    }
    pub fn session(&self, id: &str) -> Res<Session> {
        runs::load_session(&self.home, id)
    }
    pub fn transcript(&self, id: &str, after_seq: u64, limit: usize) -> Res<Vec<Message>> {
        Transcript::open(&self.home, id)?.read(after_seq, limit)
    }
    pub fn context(&self, id: &str, cap: usize) -> Res<String> {
        Ok(context::bounded(
            &Transcript::open(&self.home, id)?.snapshot(None)?.messages,
            cap,
        ))
    }
    pub fn runs(&self, id: &str) -> Res<Vec<Run>> {
        runs::session_runs(&self.home, id)
    }
}
