//! Serial summary work, independent of GPUI.
use crate::{
    base::{Failure, Res},
    board::{self, Cache, Choice, Lang, Model, Request, Summary},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(super) struct Job {
    pub session: String,
    pub revision: u64,
    pub choice: Choice,
    pub lang: Lang,
    pub store: Option<PathBuf>,
    pub repo: Option<PathBuf>,
    pub events: Vec<String>,
    pub cancel: Arc<AtomicBool>,
}

pub(super) fn execute(
    job: &Job,
    cache: &Mutex<Cache>,
    model: &mut Option<Arc<dyn Model>>,
    build: &dyn Fn() -> Res<Arc<dyn Model>>,
) -> (Res<Summary>, bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    if job.cancel.load(Ordering::Acquire) {
        return (Err(Failure::new("cancelled", "request cancelled")), false);
    }
    let digest = match (&job.store, &job.repo) {
        (Some(store), Some(repo)) => crate::orch::digest(store, repo, None).unwrap_or_default(),
        _ => String::new(),
    };
    if job.cancel.load(Ordering::Acquire) {
        return (Err(Failure::new("cancelled", "request cancelled")), false);
    }
    let request = Request::new(&job.session, job.lang, &digest, &job.events);
    let key = request.key(job.choice);
    if let Some(summary) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
        return (Ok(summary), false);
    }
    if job.cancel.load(Ordering::Acquire) {
        return (Err(Failure::new("cancelled", "request cancelled")), false);
    }
    if model.is_none() {
        match build() {
            Ok(built) => *model = Some(built),
            Err(e) => return (Err(e), false),
        }
    }
    let result = board::summarize(
        model.as_ref().unwrap().as_ref(),
        job.choice,
        &request,
        deadline,
        &job.cancel,
    );
    if let Ok(summary) = &result {
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, summary.clone());
    }
    (result, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Fake(AtomicUsize);
    impl Model for Fake {
        fn luna(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, board::TokenUse)> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok((
                "RECENT: work\nNEXT: unknown".into(),
                board::TokenUse::default(),
            ))
        }
        fn haiku(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, board::TokenUse)> {
            panic!("unexpected haiku")
        }
    }

    #[test]
    fn cancelled_job_never_builds_a_model() {
        let mut model = None;
        let cache = Mutex::new(Cache::new());
        let job = Job {
            session: "a".into(),
            revision: 1,
            choice: Choice::Luna,
            lang: Lang::En,
            store: None,
            repo: None,
            events: vec![],
            cancel: Arc::new(AtomicBool::new(true)),
        };
        let (result, called) = execute(&job, &cache, &mut model, &|| panic!("must not build"));
        assert_eq!(result.unwrap_err().kind, "cancelled");
        assert!(!called);
    }

    #[test]
    fn execute_caches_bounded_request_without_calling_the_model_again() {
        let fake = Arc::new(Fake::default());
        let mut model: Option<Arc<dyn Model>> = Some(fake.clone());
        let cache = Mutex::new(Cache::new());
        let build = || -> Res<Arc<dyn Model>> { panic!("model already provided") };
        let job = Job {
            session: "a".into(),
            revision: 1,
            choice: Choice::Luna,
            lang: Lang::En,
            store: None,
            repo: None,
            events: vec!["event".into()],
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let (first, called) = execute(&job, &cache, &mut model, &build);
        assert!(called);
        let (second, called) = execute(&job, &cache, &mut model, &build);
        assert!(!called);
        assert_eq!(first.unwrap(), second.unwrap());
        assert_eq!(fake.0.load(Ordering::SeqCst), 1);
    }
}
