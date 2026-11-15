use axiom::{
    laya::{
        infer::{Abstain, FakeClassifier, Prediction},
        router::{self, Decision, DecisionKind, DecisionObserver, FakeObserver, ShadowObserver},
    },
    usage::extensions,
    workshop::{
        api::Api,
        host::{ModuleCtx, RequestHandler},
        router::BackendRouter,
        services::{
            DecisionReply, DecisionRequest, DecisionRouter, FakeBackend, FakeSlots, ModelBackend,
            ModelCall, ModelReply, NoNetwork, Services,
        },
        DataDirs, Grants, Manifest, Panel, Provider,
    },
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

static GLOBAL: Mutex<()> = Mutex::new(());
struct Override;
impl Drop for Override {
    fn drop(&mut self) {
        router::install_override(None);
    }
}
fn override_with(observer: Arc<dyn DecisionObserver>) -> Override {
    router::install_override(Some(observer));
    Override
}
fn backend(text: &str) -> Arc<FakeBackend> {
    let backend = Arc::new(FakeBackend::default());
    *backend.reply.lock().unwrap() = ModelReply {
        text: text.into(),
        input_tokens: 12,
        cached_input_tokens: 3,
        output_tokens: 4,
        usd: Some(0.25),
    };
    backend
}
fn make_router(home: &Path, backend: Arc<dyn ModelBackend>) -> BackendRouter {
    BackendRouter {
        backend,
        model: "test-model".into(),
        engine_home: home.join("engine"),
        laya_home: home.join("laya-home"),
        cwd: home.into(),
        timeout_s: 120,
    }
}
fn request() -> DecisionRequest {
    DecisionRequest {
        question: "Which role?".into(),
        options: vec!["terra".into(), "sol".into()],
    }
}
fn assert_backend_choice(reply: &DecisionReply) {
    assert_eq!(
        reply.choice, "sol",
        "backend choice must remain authoritative"
    );
}

struct InspectBackend {
    fake: Arc<FakeBackend>,
    home: std::path::PathBuf,
}
impl ModelBackend for InspectBackend {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> axiom::base::Res<ModelReply> {
        assert_eq!(call.provider, Provider::Claude);
        assert_eq!(call.model, "test-model");
        assert_eq!(call.cwd, self.home);
        assert_eq!(call.timeout_s, 120);
        assert!(call.prompt.contains("Which role?"));
        assert!(call.prompt.contains("terra") && call.prompt.contains("sol"));
        assert!(call.prompt.contains("choice") && call.prompt.contains("reason"));
        self.fake.call(call, cancel)
    }
}

#[test]
fn valid_choice_is_billed_and_observed() {
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let backend =
        backend("Here is the answer:\n```json\n{\"choice\":\"sol\",\"reason\":\"best fit\"}\n```");
    let router = make_router(
        home.path(),
        Arc::new(InspectBackend {
            fake: backend.clone(),
            home: home.path().into(),
        }),
    );
    let observer = Arc::new(FakeObserver::default());
    let _override = override_with(observer.clone());
    let reply = router.route("sample", &request()).unwrap();
    assert_backend_choice(&reply);
    assert_eq!(reply.reason, "best fit");
    assert_eq!(*backend.calls.lock().unwrap(), vec!["test-model"]);
    let (records, skipped) = extensions::read(&router.engine_home).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.module.as_deref(), Some("sample"));
    assert_eq!(record.provider, "claude");
    assert_eq!(record.model.as_deref(), Some("test-model"));
    assert_eq!(
        (
            record.input_tokens,
            record.cached_input_tokens,
            record.output_tokens,
            record.usd
        ),
        (12, 3, 4, Some(0.25))
    );
    let seen = observer.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].kind, DecisionKind::Routing);
    assert_eq!(seen[0].subject, "workshop:sample");
    assert_eq!(seen[0].wish_id, None);
    assert_eq!(seen[0].claude, "sol");
    assert!(seen[0].context.contains("Which role?"));
    assert!(seen[0].context.contains("terra") && seen[0].context.contains("sol"));
}

#[test]
#[should_panic(expected = "backend choice must remain authoritative")]
fn negative_control_overwriting_backend_choice_is_detected() {
    struct OverwritingRouter(BackendRouter);
    impl DecisionRouter for OverwritingRouter {
        fn route(
            &self,
            module: &str,
            request: &DecisionRequest,
        ) -> axiom::base::Res<DecisionReply> {
            let mut reply = self.0.route(module, request)?;
            reply.choice = "terra".into();
            Ok(reply)
        }
    }
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let router = OverwritingRouter(make_router(
        home.path(),
        backend(r#"{"choice":"sol","reason":"yes"}"#),
    ));
    let reply = router.route("sample", &request()).unwrap();
    assert_backend_choice(&reply);
}

#[test]
fn invalid_and_malformed_replies_are_billed() {
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    for text in [r#"{"choice":"astra","reason":"no"}"#, "not JSON"] {
        let home = tempfile::tempdir().unwrap();
        let router = make_router(home.path(), backend(text));
        assert_eq!(
            router.route("sample", &request()).unwrap_err().kind,
            "invalid_response"
        );
        let records = extensions::read(&router.engine_home).unwrap().0;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].module.as_deref(), Some("sample"));
    }
}

struct PanicObserver;
impl DecisionObserver for PanicObserver {
    fn observe(&self, _: &Decision) {
        panic!("observer failure");
    }
}
#[test]
fn shadow_abstention_load_error_and_panic_cannot_replace_choice() {
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let router = make_router(home.path(), backend(r#"{"choice":"sol","reason":"yes"}"#));
    let prediction = Prediction {
        labels: vec!["terra".into(), "sol".into(), "astra".into()],
        probs: vec![0.8, 0.1, 0.1],
        top: 0,
        confidence: 0.8,
        tokens: 3,
        abstain: Abstain::Abstain,
    };
    let classifier = FakeClassifier {
        version: "fake".into(),
        table: BTreeMap::from([("routing".into(), prediction)]),
    };
    {
        let _override = override_with(Arc::new(ShadowObserver::new(
            router.laya_home.clone(),
            Some(Arc::new(classifier)),
            None,
        )));
        assert_backend_choice(&router.route("sample", &request()).unwrap());
    }
    {
        let _override = override_with(Arc::new(ShadowObserver::new(
            router.laya_home.clone(),
            None,
            Some("load_error".into()),
        )));
        assert_backend_choice(&router.route("sample", &request()).unwrap());
    }
    let records = router::read_records(&router.laya_home).unwrap().0;
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].laya.as_ref().unwrap().prediction.abstain,
        Abstain::Abstain
    );
    assert_eq!(records[0].claude, "sol");
    assert_eq!(records[0].kind, DecisionKind::Routing);
    assert_eq!(records[0].subject, "workshop:sample");
    assert_eq!(records[0].wish_id, None);
    assert_eq!(records[0].executed, "claude");
    assert_eq!(records[1].laya_error.as_deref(), Some("load_error"));
    {
        let _override = override_with(Arc::new(PanicObserver));
        assert_backend_choice(&router.route("sample", &request()).unwrap());
    }
}

#[test]
fn laya_kill_file_and_environment_leave_choice_unchanged() {
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let router = make_router(home.path(), backend(r#"{"choice":"sol","reason":"yes"}"#));
    let _override = override_with(Arc::new(ShadowObserver::new(
        router.laya_home.clone(),
        None,
        Some("load_error".into()),
    )));
    router::kill(&router.laya_home).unwrap();
    assert_backend_choice(&router.route("sample", &request()).unwrap());
    router::unkill(&router.laya_home).unwrap();
    let old = std::env::var_os("AXIOM_LAYA_KILL");
    unsafe {
        std::env::set_var("AXIOM_LAYA_KILL", "1");
    }
    let result = router.route("sample", &request());
    unsafe {
        if let Some(value) = old {
            std::env::set_var("AXIOM_LAYA_KILL", value);
        } else {
            std::env::remove_var("AXIOM_LAYA_KILL");
        }
    }
    assert_backend_choice(&result.unwrap());
    assert!(router::read_records(&router.laya_home)
        .unwrap()
        .0
        .is_empty());
}

fn ctx(home: &Path, permissions: &[&str]) -> ModuleCtx {
    let manifest = Manifest {
        name: "sample".into(),
        version: "1".into(),
        entry: vec!["python".into()],
        permissions: permissions.iter().map(|s| s.to_string()).collect(),
        max_parallel: 1,
        panel: Panel {
            title: "test".into(),
            description: "test".into(),
        },
    };
    let grants = Grants::from_manifest(&manifest).unwrap();
    let dirs = DataDirs::new(home, "sample");
    dirs.ensure().unwrap();
    ModuleCtx {
        name: "sample".into(),
        manifest,
        grants,
        dirs,
        cancel: Arc::new(AtomicBool::new(false)),
    }
}
#[test]
fn api_permission_gates_backend() {
    let _lock = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    let backend = backend(r#"{"choice":"sol","reason":"yes"}"#);
    let router = make_router(home.path(), backend.clone());
    let api = Api::new(Services {
        engine_home: router.engine_home.clone(),
        slots: Arc::new(FakeSlots::default()),
        claude: backend.clone(),
        codex: backend.clone(),
        router: Arc::new(router),
        network: Arc::new(NoNetwork),
    });
    let call = || json!({"v":1,"id":1,"method":"decision.route","params":{"question":"Which role?","options":["terra","sol"]}});
    let denied = api.handle(&ctx(home.path(), &[]), call());
    assert_eq!(denied["error"]["kind"], "permission_denied");
    assert!(backend.calls.lock().unwrap().is_empty());
    let allowed = api.handle(&ctx(home.path(), &["decision_router"]), call());
    assert_eq!(allowed["ok"], true);
    assert_eq!(allowed["result"]["choice"], "sol");
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
}
