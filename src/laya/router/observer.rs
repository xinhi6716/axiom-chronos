use super::record::*;
use crate::laya::infer::{Classifier, Laya, LoadSpec};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub trait DecisionObserver: Send + Sync {
    fn observe(&self, d: &Decision);
}
pub struct NoopObserver;
impl DecisionObserver for NoopObserver {
    fn observe(&self, _: &Decision) {}
}
#[derive(Default)]
pub struct FakeObserver {
    pub seen: Mutex<Vec<Decision>>,
}
impl DecisionObserver for FakeObserver {
    fn observe(&self, d: &Decision) {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(d.clone());
    }
}
pub struct ShadowObserver {
    home: PathBuf,
    classifier: Option<Arc<dyn Classifier>>,
    load_error: Option<String>,
}
impl ShadowObserver {
    pub fn new(
        home: PathBuf,
        classifier: Option<Arc<dyn Classifier>>,
        load_error: Option<String>,
    ) -> Self {
        Self {
            home,
            classifier,
            load_error,
        }
    }
}
static COUNTER: AtomicU64 = AtomicU64::new(0);
impl DecisionObserver for ShadowObserver {
    fn observe(&self, d: &Decision) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if killed(&self.home) {
                return;
            }
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(i64::MAX as u128) as i64;
            let id = format!(
                "{}-{}-{}",
                ts,
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let mut end = d.context.len().min(4096);
            while !d.context.is_char_boundary(end) {
                end -= 1;
            }
            let context = d.context[..end].to_owned();
            let mut record = DecisionRecord {
                schema: 1,
                id: id.clone(),
                ts_ms: ts,
                kind: d.kind,
                wish_id: d.wish_id.clone(),
                subject: d.subject.clone(),
                context: context.clone(),
                claude: d.claude.clone(),
                laya: None,
                laya_error: self.load_error.clone(),
                policy_version: crate::engine::build_id(),
                mode: "shadow".into(),
                executed: "claude".into(),
            };
            if let Some(classifier) = &self.classifier {
                let version = classifier.model_version();
                let labels: Vec<String> = d.kind.labels().iter().map(|s| (*s).to_owned()).collect();
                match classifier.predict(
                    d.kind.as_str(),
                    d.kind.instructions(),
                    &labels,
                    &format!("{}\n{}", d.subject, context),
                ) {
                    Ok(prediction) => {
                        let usage = crate::usage::laya::UsageEvent {
                            id,
                            ts_ms: ts,
                            model_version: version.clone(),
                            kind: "inference".into(),
                            backend: "cpu".into(),
                            tokens: Some(prediction.tokens),
                            gpu_hours: None,
                            compute_units: None,
                        };
                        let _ = crate::usage::laya::record(&self.home, &usage);
                        record.laya = Some(LayaSide {
                            model_version: version,
                            prediction,
                        });
                        record.laya_error = None;
                    }
                    Err(e) => record.laya_error = Some(e.to_string()),
                }
            } else if record.laya_error.is_none() {
                record.laya_error = Some("classifier unavailable".into());
            }
            let _ = append_record(&self.home, &record);
        }));
    }
}
type Cached = (
    PathBuf,
    RouterConfig,
    Option<Arc<dyn Classifier>>,
    Option<String>,
);
static CACHE: OnceLock<Mutex<Vec<Cached>>> = OnceLock::new();
static OVERRIDE: OnceLock<Mutex<Option<Arc<dyn DecisionObserver>>>> = OnceLock::new();
pub fn install_override(o: Option<Arc<dyn DecisionObserver>>) {
    *OVERRIDE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = o;
}
pub fn observer(home: &Path) -> Arc<dyn DecisionObserver> {
    if killed(home) {
        return Arc::new(NoopObserver);
    }
    let config = match load_config(home) {
        Ok(c) => c,
        Err(_) => return Arc::new(NoopObserver),
    };
    if config.mode == Mode::Off {
        return Arc::new(NoopObserver);
    }
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((_, _, classifier, error)) =
        cache.iter().find(|(h, c, _, _)| h == home && c == &config)
    {
        return Arc::new(ShadowObserver::new(
            home.to_owned(),
            classifier.clone(),
            error.clone(),
        ));
    }
    let loaded = (|| {
        let model_dir = config
            .model_dir
            .clone()
            .unwrap_or_else(|| crate::laya::laya_dir(home).join("model"));
        let runtime = match &config.runtime_lib {
            Some(p) => p.clone(),
            None => crate::laya::ort::find_runtime(Some(home), None)?,
        };
        Laya::load(&LoadSpec {
            model_dir,
            calibration: config.calibration.clone(),
            runtime,
            intra_threads: 1,
        })
    })();
    let (classifier, error): (Option<Arc<dyn Classifier>>, Option<String>) = match loaded {
        Ok(c) => (Some(Arc::new(c)), None),
        Err(e) => (None, Some(e.to_string())),
    };
    cache.push((home.to_owned(), config, classifier.clone(), error.clone()));
    Arc::new(ShadowObserver::new(home.to_owned(), classifier, error))
}
fn target(home: &Path) -> Arc<dyn DecisionObserver> {
    let current = OVERRIDE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    current.unwrap_or_else(|| observer(home))
}
fn wish(store: Option<&Path>) -> Option<String> {
    store
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .filter(|s| s.starts_with("w-"))
        .map(str::to_owned)
}
pub fn observe_interjection(home: &Path, session: &str, claude_kind: &str) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Ok(Some(goal)) = crate::goal::load(home, session) {
            if let Some(i) = goal.interjection {
                target(home).observe(&Decision {
                    kind: DecisionKind::Interjection,
                    wish_id: goal.wish_id,
                    subject: session.into(),
                    context: i.prompt,
                    claude: claude_kind.into(),
                });
            }
        }
    }));
}
pub fn observe_verdict(store: &Path, v: &crate::orch::Verdict) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(home) = crate::laya::home_from_env() else {
            return;
        };
        let observer = target(&home);
        for f in &v.findings {
            let context = format!(
                "check: {}\nlocation: {}\nevidence: {}\nfix: {}",
                f.check, f.location, f.evidence, f.fix
            );
            let subject = format!("{} finding {}", v.unit, f.number);
            let severity = match f.severity {
                crate::orch::Severity::Blocker => "blocker",
                crate::orch::Severity::Major => "major",
                crate::orch::Severity::Minor => "minor",
            };
            let needs = match f.needs {
                Some(crate::orch::Needs::Smoke) => "smoke",
                Some(crate::orch::Needs::Manual) => "manual",
                None => "none",
            };
            for (kind, claude) in [
                (DecisionKind::Triage, severity),
                (DecisionKind::Needs, needs),
            ] {
                observer.observe(&Decision {
                    kind,
                    wish_id: wish(Some(store)),
                    subject: subject.clone(),
                    context: context.clone(),
                    claude: claude.into(),
                });
            }
        }
    }));
}
pub fn observe_routing(
    home: Option<&Path>,
    store: Option<&Path>,
    unit: Option<&str>,
    role: &str,
    brief: &str,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(home) = home {
            target(home).observe(&Decision {
                kind: DecisionKind::Routing,
                wish_id: wish(store),
                subject: unit.unwrap_or("").into(),
                context: brief.into(),
                claude: role.into(),
            });
        }
    }));
}
