mod common;
use axiom::{
    base::Res,
    seams::FakeApproval,
    workshop::{
        api::Api,
        host::{Host, HostConfig, ModuleCtx, RequestHandler},
        services::{self, *},
        DataDirs, Grants, Manifest, Panel,
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn ctx(home: &Path, name: &str, permissions: &[&str], max_parallel: u32) -> Arc<ModuleCtx> {
    let manifest = Manifest {
        name: name.into(),
        version: "1".into(),
        entry: vec!["python".into()],
        permissions: permissions.iter().map(|s| s.to_string()).collect(),
        max_parallel,
        panel: Panel {
            title: "test".into(),
            description: "test".into(),
        },
    };
    let grants = Grants::from_manifest(&manifest).unwrap();
    let dirs = DataDirs::new(home, name);
    dirs.ensure().unwrap();
    Arc::new(ModuleCtx {
        name: name.into(),
        manifest,
        grants,
        dirs,
        cancel: Arc::new(AtomicBool::new(false)),
    })
}
fn api(
    home: &Path,
    slots: Arc<dyn ModelSlots>,
    backend: Arc<dyn ModelBackend>,
    router: Arc<dyn DecisionRouter>,
) -> Arc<Api> {
    Arc::new(Api::new(Services {
        engine_home: home.into(),
        slots,
        claude: backend.clone(),
        codex: backend,
        router,
        network: Arc::new(NoNetwork),
    }))
}
fn basic(home: &Path) -> Arc<Api> {
    api(
        home,
        Arc::new(FakeSlots::default()),
        Arc::new(FakeBackend::default()),
        Arc::new(FakeRouter::default()),
    )
}
fn req(method: &str, params: Value) -> Value {
    json!({"v":1,"id":42,"method":method,"params":params})
}
fn call(api: &Api, c: &ModuleCtx, method: &str, params: Value) -> Value {
    api.handle(c, req(method, params))
}
fn wait(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(10));
    }
}
fn model() -> Value {
    json!({"provider":"codex","model":"m","prompt":"hi"})
}
#[derive(Default)]
struct Blocking {
    active: AtomicUsize,
    peak: AtomicUsize,
    started: AtomicUsize,
    release: AtomicBool,
}
impl ModelBackend for Blocking {
    fn call(&self, _: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(n, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(10));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        services::cancelled(cancel)?;
        Ok(ModelReply {
            text: "ok".into(),
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        })
    }
}
fn spawn(api: Arc<Api>, ctx: Arc<ModuleCtx>) -> thread::JoinHandle<Value> {
    thread::spawn(move || call(&api, &ctx, "model.call", model()))
}

#[test]
fn protocol_permissions_storage_and_usage() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(FakeBackend::default());
    *backend.reply.lock().unwrap() = ModelReply {
        text: "hi".into(),
        input_tokens: 5,
        output_tokens: 2,
        usd: Some(0.25),
        ..Default::default()
    };
    let slots = Arc::new(FakeSlots::default());
    let router = Arc::new(FakeRouter::default());
    let api = api(home, slots.clone(), backend.clone(), router.clone());
    let a = ctx(
        home,
        "a",
        &[
            "memory",
            "files",
            "model:codex:m",
            "network",
            "decision_router",
        ],
        1,
    );
    let b = ctx(home, "b", &["memory"], 1);
    let missing_version = api.handle(&a, json!({"id":"x","method":"api.hello"}));
    assert_eq!(missing_version["error"]["kind"], "unsupported_version");
    assert_eq!(missing_version["error"]["supported"], json!([1]));
    assert_eq!(
        api.handle(&a, json!({"v":2,"id":"x","method":"api.hello"}))["error"]["supported"],
        json!([1])
    );
    assert_eq!(
        api.handle(&a, json!({"v":1,"module":"b","method":"api.hello"}))["error"]["kind"],
        "identity_spoof"
    );
    assert_eq!(
        call(&api, &a, "api.hello", json!({"module":"b"}))["error"]["kind"],
        "identity_spoof"
    );
    assert_eq!(
        call(
            &api,
            &a,
            "model.call",
            json!({"provider":"codex","model":"other","prompt":"hi"})
        )["error"]["kind"],
        "permission_denied"
    );
    assert!(slots.calls.lock().unwrap().is_empty());
    assert!(backend.calls.lock().unwrap().is_empty());
    assert_eq!(
        call(
            &api,
            &a,
            "memory.put",
            json!({"key":"key","value":"secret"})
        )["ok"],
        true
    );
    assert!(call(&api, &b, "memory.get", json!({"key":"key"}))["result"]["value"].is_null());
    assert_eq!(
        call(&api, &a, "file.read", json!({"path":"../outside"}))["error"]["kind"],
        "path_escape"
    );
    assert_eq!(
        call(
            &api,
            &a,
            "process.spawn",
            json!({"argv":["fake"],"timeout_s":1})
        )["error"]["kind"],
        "permission_denied"
    );
    assert_eq!(
        call(
            &api,
            &a,
            "decision.route",
            json!({"question":"q","options":["yes","no"]})
        )["result"]["choice"],
        "yes"
    );
    assert!(FakeRouter::calls()
        .iter()
        .any(|(module, request)| module == "a" && request.question == "q"));
    assert_eq!(
        call(&api, &a, "net.request", json!({}))["error"]["kind"],
        "unsupported"
    );
    assert_eq!(
        call(&api, &a, "model.call", model())["result"]["text"],
        "hi"
    );
    let usage = call(&api, &a, "usage.get", json!({}));
    assert_eq!(usage["result"]["input_tokens"], 5);
    assert_eq!(usage["result"]["records"], 1);
    assert_eq!(
        call(&api, &b, "usage.get", json!({}))["result"]["records"],
        0
    );
}
#[test]
fn global_slot_serializes_modules() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(Blocking::default());
    let api = api(
        home,
        Arc::new(GlobalSlots {
            home: home.into(),
            codex_limit: 1,
        }),
        backend.clone(),
        Arc::new(NoRouter),
    );
    let a = ctx(home, "a", &["model:codex:m"], 2);
    let b = ctx(home, "b", &["model:codex:m"], 2);
    let one = spawn(api.clone(), a);
    wait(|| backend.started.load(Ordering::SeqCst) == 1);
    let two = spawn(api, b);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(backend.started.load(Ordering::SeqCst), 1);
    backend.release.store(true, Ordering::SeqCst);
    assert_eq!(one.join().unwrap()["ok"], true);
    assert_eq!(two.join().unwrap()["ok"], true);
    assert_eq!(backend.peak.load(Ordering::SeqCst), 1);
}
#[test]
fn kill_releases_waiting_permit_and_slot_and_isolated() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(Blocking::default());
    let api = api(
        home,
        Arc::new(GlobalSlots {
            home: home.into(),
            codex_limit: 2,
        }),
        backend.clone(),
        Arc::new(NoRouter),
    );
    let a = ctx(home, "a", &["model:codex:m"], 1);
    let b = ctx(home, "b", &["model:codex:m"], 1);
    let one = spawn(api.clone(), a.clone());
    wait(|| backend.started.load(Ordering::SeqCst) == 1);
    let waiting = spawn(api.clone(), a.clone());
    let other = spawn(api.clone(), b);
    wait(|| backend.started.load(Ordering::SeqCst) == 2);
    a.cancel.store(true, Ordering::SeqCst);
    assert_eq!(one.join().unwrap()["error"]["kind"], "cancelled");
    assert_eq!(waiting.join().unwrap()["error"]["kind"], "cancelled");
    let replacement = ctx(home, "a", &["model:codex:m"], 1);
    let next = spawn(api, replacement);
    wait(|| backend.started.load(Ordering::SeqCst) == 3);
    backend.release.store(true, Ordering::SeqCst);
    assert_eq!(other.join().unwrap()["ok"], true);
    assert_eq!(next.join().unwrap()["ok"], true);
}
#[test]
fn cancelled_global_slot_wait_releases_module_permit() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(Blocking::default());
    let api = api(
        home,
        Arc::new(GlobalSlots {
            home: home.into(),
            codex_limit: 1,
        }),
        backend.clone(),
        Arc::new(NoRouter),
    );
    let a = ctx(home, "a", &["model:codex:m"], 1);
    let b = ctx(home, "b", &["model:codex:m"], 1);
    let one = spawn(api.clone(), a);
    wait(|| backend.started.load(Ordering::SeqCst) == 1);
    let two = spawn(api.clone(), b.clone());
    thread::sleep(Duration::from_millis(100));
    b.cancel.store(true, Ordering::SeqCst);
    assert_eq!(two.join().unwrap()["error"]["kind"], "cancelled");
    backend.release.store(true, Ordering::SeqCst);
    assert_eq!(one.join().unwrap()["ok"], true);
    let fresh = ctx(home, "b", &["model:codex:m"], 1);
    assert_eq!(spawn(api, fresh).join().unwrap()["ok"], true);
}
#[test]
fn hello_module_host_roundtrip() {
    let tmp = common::temp();
    let home = tmp.path();
    let dest = home.join("workshop/hello");
    fs::create_dir_all(&dest).unwrap();
    let source = common::repo_root().join("examples/workshop/hello");
    for file in ["manifest.json", "main.py"] {
        fs::copy(source.join(file), dest.join(file)).unwrap();
    }
    let files = home.join("workshop-data/hello/files");
    fs::create_dir_all(&files).unwrap();
    fs::write(
        files.join("probes.json"),
        json!([{"method":"api.hello","params":{}}]).to_string(),
    )
    .unwrap();
    let host = Host::new(
        HostConfig {
            home: home.into(),
            python: Some(common::python()),
        },
        basic(home),
        Arc::new(FakeApproval::approving()),
    );
    host.start("hello").unwrap();
    wait(|| files.join("done").exists());
    let output = fs::read_to_string(files.join("results.jsonl")).unwrap();
    let v: Value = serde_json::from_str(output.trim()).unwrap();
    assert_eq!(v["reply"]["result"]["module"], "hello");
    assert_eq!(v["reply"]["result"]["versions"], json!([1]));
}

#[test]
fn storage_receipts_and_bounded_process_output() {
    let tmp = common::temp();
    let api = basic(tmp.path());
    let c = ctx(tmp.path(), "a", &["files", "memory", "subprocess"], 1);
    assert_eq!(
        call(
            &api,
            &c,
            "file.write",
            json!({"path":"a.txt","content":"hello"})
        )["ok"],
        true
    );
    assert_eq!(
        call(&api, &c, "file.read", json!({"path":"a.txt"}))["result"]["content"],
        "hello"
    );
    assert_eq!(
        call(&api, &c, "file.list", json!({}))["result"]["entries"],
        json!(["a.txt"])
    );
    assert_eq!(
        call(&api, &c, "memory.put", json!({"key":"one","value":1}))["ok"],
        true
    );
    assert_eq!(
        call(&api, &c, "memory.put", json!({"key":"two","value":2}))["ok"],
        true
    );
    assert_eq!(
        call(&api, &c, "memory.list", json!({}))["result"]["keys"],
        json!(["one", "two"])
    );
    assert_eq!(
        call(&api, &c, "memory.delete", json!({"key":"one"}))["ok"],
        true
    );
    assert_eq!(
        call(&api, &c, "memory.list", json!({}))["result"]["keys"],
        json!(["two"])
    );
    assert!(call(&api, &c, "memory.get", json!({"key":"one"}))["result"]["value"].is_null());
    assert_eq!(
        call(
            &api,
            &c,
            "memory.put",
            json!({"key":"invalid/key","value":1})
        )["error"]["kind"],
        "invalid_argument"
    );
    let receipt = call(
        &api,
        &c,
        "receipt.write",
        json!({"kind":"test","body":{"x":1}}),
    );
    let stored: Value = serde_json::from_str(
        fs::read_to_string(c.dirs.receipts.join("receipts.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    let mut unsigned = stored.clone();
    unsigned.as_object_mut().unwrap().remove("sha256");
    assert_eq!(
        receipt["result"]["sha256"],
        axiom::seams::sha256_hex(&serde_json::to_vec(&unsigned).unwrap())
    );
    assert_eq!(
        call(
            &api,
            &c,
            "log.write",
            json!({"level":"info","message":"hi"})
        )["ok"],
        true
    );
    let log: Value = serde_json::from_str(
        fs::read_to_string(c.dirs.logs.join("module.log"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(log["level"], "info");
    assert_eq!(log["message"], "hi");
    let result = call(
        &api,
        &c,
        "process.spawn",
        json!({"argv":[common::python(),"-c","import sys; sys.stdout.buffer.write(b'x'*100000); sys.stderr.buffer.write(b'y'*100000)"],"timeout_s":5}),
    );
    assert_eq!(result["result"]["exit_code"], 0);
    assert_eq!(result["result"]["stdout"].as_str().unwrap().len(), 65536);
    assert_eq!(result["result"]["stderr"].as_str().unwrap().len(), 65536);
}

struct SelfSlotBackend {
    home: std::path::PathBuf,
}
impl ModelBackend for SelfSlotBackend {
    fn call(&self, _: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        let _slot = axiom::codex::slots::acquire(&self.home, 1, &mut Default::default(), || {
            services::cancelled(cancel)
        })?;
        Ok(ModelReply {
            text: "slot acquired".into(),
            ..Default::default()
        })
    }
}

#[test]
fn backend_held_slots_does_not_double_acquire() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(SelfSlotBackend { home: home.into() });
    let api = api(
        home,
        Arc::new(BackendHeldSlots),
        backend,
        Arc::new(NoRouter),
    );
    let c = ctx(home, "self-slot", &["model:codex:m"], 1);
    let (tx, rx) = mpsc::channel();
    let api_thread = api.clone();
    let c_thread = c.clone();
    let worker = thread::spawn(move || {
        tx.send(call(&api_thread, &c_thread, "model.call", model()))
            .unwrap();
    });
    let result = rx.recv_timeout(Duration::from_secs(5));
    if result.is_err() {
        c.cancel.store(true, Ordering::SeqCst);
    }
    worker.join().unwrap();
    assert_eq!(
        result.expect("model call blocked on its own slot")["result"]["text"],
        "slot acquired"
    );
}

#[test]
fn permit_limits_are_scoped_to_api_instance() {
    let tmp = common::temp();
    let home = tmp.path();
    let backend = Arc::new(Blocking::default());
    let first = api(
        home,
        Arc::new(FakeSlots::default()),
        backend.clone(),
        Arc::new(NoRouter),
    );
    let second = api(
        home,
        Arc::new(FakeSlots::default()),
        backend.clone(),
        Arc::new(NoRouter),
    );
    let c = ctx(home, "same", &["model:codex:m"], 1);
    let one = spawn(first, c.clone());
    wait(|| backend.started.load(Ordering::SeqCst) == 1);
    let two = spawn(second, c);
    let deadline = Instant::now() + Duration::from_secs(3);
    while backend.started.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let started = backend.started.load(Ordering::SeqCst);
    backend.release.store(true, Ordering::SeqCst);
    assert_eq!(one.join().unwrap()["ok"], true);
    assert_eq!(two.join().unwrap()["ok"], true);
    assert_eq!(started, 2, "independent API instances shared a permit");
}

#[test]
fn process_environment_is_rebuilt() {
    let tmp = common::temp();
    let api = basic(tmp.path());
    // Cargo supplies this variable to test binaries; it is not a baseline variable.
    let env_key = "CARGO_MANIFEST_DIR";
    assert!(std::env::var_os(env_key).is_some());
    let c = ctx(tmp.path(), "environment", &["subprocess"], 1);
    let script = "import os, json; print(json.dumps({'hidden': 'CARGO_MANIFEST_DIR' not in os.environ, 'module': os.environ.get('AXIOM_WORKSHOP_MODULE'), 'home': os.environ.get('HOME')}))";
    let result = call(
        &api,
        &c,
        "process.spawn",
        json!({"argv":[common::python(),"-c",script],"timeout_s":5}),
    );
    assert_eq!(result["result"]["exit_code"], 0, "{result}");
    let env: Value = serde_json::from_str(result["result"]["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(env["hidden"], true);
    assert_eq!(env["module"], "environment");
    assert_eq!(env["home"], c.dirs.files.to_string_lossy().as_ref());
    let granted = ctx(
        tmp.path(),
        "granted",
        &["subprocess", "env:CARGO_MANIFEST_DIR"],
        1,
    );
    let result = call(
        &api,
        &granted,
        "process.spawn",
        json!({"argv":[common::python(),"-c","import os; print('CARGO_MANIFEST_DIR' in os.environ)"],"timeout_s":5}),
    );
    assert_eq!(result["result"]["stdout"].as_str().unwrap().trim(), "True");
}

#[test]
fn process_timeout_and_cancel_kill_child() {
    let tmp = common::temp();
    let api = basic(tmp.path());
    let script = "import pathlib, time; pathlib.Path('started').write_text('yes'); time.sleep(3); pathlib.Path('completed').write_text('yes')";
    for cancelled in [false, true] {
        let c = ctx(
            tmp.path(),
            if cancelled { "cancel" } else { "timeout" },
            &["subprocess"],
            1,
        );
        let api_thread = api.clone();
        let c_thread = c.clone();
        let python = common::python();
        let worker = thread::spawn(move || {
            call(
                &api_thread,
                &c_thread,
                "process.spawn",
                json!({"argv":[python,"-c",script],"timeout_s":if cancelled { 10 } else { 1 }}),
            )
        });
        wait(|| c.dirs.files.join("started").exists());
        if cancelled {
            c.cancel.store(true, Ordering::SeqCst);
        }
        let result = worker.join().unwrap();
        assert_eq!(
            result["error"]["kind"],
            if cancelled { "cancelled" } else { "failed" }
        );
        if !cancelled {
            assert_eq!(result["error"]["message"], "process timed out");
        }
        // Wait past the child's planned write, proving it did not continue after return.
        thread::sleep(Duration::from_millis(3200));
        assert!(!c.dirs.files.join("completed").exists());
    }
}
