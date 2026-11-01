use axiom::codex::*;
use serde_json::json;
use std::sync::{atomic::AtomicBool, Arc};

fn request(path: &std::path::Path, text: &str) -> RunRequest {
    RunRequest {
        brief: text.into(),
        cwd: path.to_string_lossy().into(),
        model: "test".into(),
        sandbox: "read-only".into(),
        effort: None,
        output_schema: None,
        developer_instructions: None,
        timeout_s: 10,
        lean: false,
        keep_plugins: vec![],
        web_search: false,
    }
}
fn options(handler: Option<Arc<dyn ToolHandler>>) -> RunOptions {
    RunOptions {
        attempt_id: "attempt".into(),
        thread: ThreadMode::FreshPersistent,
        tools: if handler.is_some() {
            vec![DynamicTool {
                name: "child".into(),
                description: "child".into(),
                input_schema: json!({"type":"object"}),
            }]
        } else {
            vec![]
        },
        handler,
        observer: None,
        cancel: Arc::new(AtomicBool::new(false)),
        cancel_grace_ms: 500,
        skill_paths: vec![],
    }
}
struct Nested {
    server: Arc<AppServer>,
    cwd: std::path::PathBuf,
}
impl ToolHandler for Nested {
    fn handle(&self, _: ToolCall) -> axiom::base::Res<ToolReply> {
        let r = self
            .server
            .run_with_options(&request(&self.cwd, "child"), options(None))?;
        assert!(matches!(r.terminal, TransportTerminal::Completed));
        Ok(ToolReply {
            success: true,
            content_items: vec![json!({"type":"inputText","text":"ok"})],
        })
    }
}
#[test]
fn parallel_callbacks_can_wait_for_child_runs() {
    let dir = crate::common::temp();
    let server = Arc::new(AppServer::new(crate::common::fake("fake_codex")));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let server = server.clone();
            let path = dir.path().to_path_buf();
            std::thread::spawn(move || {
                let handler = Arc::new(Nested {
                    server: server.clone(),
                    cwd: path.clone(),
                });
                server
                    .run_with_options(&request(&path, "TOOLCALL"), options(Some(handler)))
                    .unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert!(matches!(
            thread.join().unwrap().terminal,
            TransportTerminal::Completed
        ));
    }
}
struct Reject;
impl RunObserver for Reject {
    fn on_event(&self, event: &RunEvent) -> axiom::base::Res<()> {
        if matches!(event.kind, RunEventKind::BeforeTurn) {
            Err(axiom::base::Failure::new("rejected", "no"))
        } else {
            Ok(())
        }
    }
}
struct RejectLost;
impl RunObserver for RejectLost {
    fn on_event(&self, event: &RunEvent) -> axiom::base::Res<()> {
        if matches!(event.kind, RunEventKind::ConnectionLost { .. }) {
            Err(axiom::base::Failure::new(
                "observer_storage",
                "durable observer failed",
            ))
        } else {
            Ok(())
        }
    }
}
#[test]
fn connection_lost_observer_error_is_not_swallowed() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let mut opts = options(None);
    opts.observer = Some(Arc::new(RejectLost));
    assert_eq!(
        server
            .run_with_options(&request(dir.path(), "CRASH"), opts)
            .unwrap_err()
            .kind,
        "observer_storage"
    );
}
#[test]
fn observer_rejection_stops_before_turn() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let mut opts = options(None);
    opts.observer = Some(Arc::new(Reject));
    assert_eq!(
        server
            .run_with_options(&request(dir.path(), "HANG"), opts)
            .unwrap_err()
            .kind,
        "rejected"
    );
}

struct Delayed {
    panic: bool,
    finished: Arc<AtomicBool>,
}
impl ToolHandler for Delayed {
    fn handle(&self, _: ToolCall) -> axiom::base::Res<ToolReply> {
        std::thread::sleep(std::time::Duration::from_millis(180));
        self.finished
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if self.panic {
            panic!("handler failure");
        }
        Ok(ToolReply {
            success: true,
            content_items: vec![],
        })
    }
}
#[test]
fn terminal_waits_for_preterminal_callback_and_handler_panic_fails() {
    for (brief, panic) in [("PRETERMINAL", false), ("TOOLCALL", true)] {
        let dir = crate::common::temp();
        let server = AppServer::new(crate::common::fake("fake_codex"));
        let finished = Arc::new(AtomicBool::new(false));
        let result = server
            .run_with_options(
                &request(dir.path(), brief),
                options(Some(Arc::new(Delayed {
                    panic,
                    finished: finished.clone(),
                }))),
            )
            .unwrap();
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(matches!(result.terminal, TransportTerminal::Failed), panic);
    }
}
#[test]
fn post_terminal_callback_is_not_accepted_or_awaited() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let finished = Arc::new(AtomicBool::new(false));
    let r = server
        .run_with_options(
            &request(dir.path(), "LATECALL"),
            options(Some(Arc::new(Delayed {
                panic: false,
                finished: finished.clone(),
            }))),
        )
        .unwrap();
    assert!(matches!(r.terminal, TransportTerminal::Completed));
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(!finished.load(std::sync::atomic::Ordering::SeqCst));
}
#[test]
fn options_match_lean_and_structured_output() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let mut req = request(dir.path(), "structured");
    req.lean = true;
    req.web_search = true;
    req.keep_plugins.push("keepme@personal".into());
    req.output_schema = Some(json!({"type":"object","required":["answer"]}));
    let good = server.run_with_options(&req, options(None)).unwrap();
    assert_eq!(good.report["structured_output"], json!({"answer":"ok"}));
    assert_eq!(
        good.report["disabled"]["plugins"],
        json!(["panel@personal"])
    );
    assert_eq!(good.report["started_mcp_servers"], json!(["keepme"]));
    req.brief = "BADJSON".into();
    let bad = server.run_with_options(&req, options(None)).unwrap();
    assert!(matches!(bad.terminal, TransportTerminal::Failed));
    assert_eq!(bad.report["error"]["kind"], "invalid_structured_output");
}
#[test]
fn cancel_during_start_and_ack_without_terminal_are_bounded() {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    for brief in ["HANGREQ", "ACKONLY"] {
        let dir = crate::common::temp();
        let server = AppServer::new(crate::common::fake("fake_codex"));
        let mut opts = options(None);
        opts.cancel_grace_ms = 150;
        let cancel = opts.cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1000));
            cancel.store(true, Ordering::Release);
        });
        let now = Instant::now();
        let result = server
            .run_with_options(&request(dir.path(), brief), opts)
            .unwrap();
        assert!(matches!(result.terminal, TransportTerminal::OutcomeUnknown));
        assert!(now.elapsed() < Duration::from_secs(5));
    }
}
#[test]
fn unsupported_effort_is_rejected_before_turn() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let mut req = request(dir.path(), "test");
    req.effort = Some("ultra".into());
    assert_eq!(
        server
            .run_with_options(&req, options(None))
            .unwrap_err()
            .kind,
        "unsupported_effort"
    );
}
#[test]
fn eof_is_unknown_and_server_can_restart() {
    let dir = crate::common::temp();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let result = server
        .run_with_options(&request(dir.path(), "CRASH"), options(None))
        .unwrap();
    assert!(matches!(result.terminal, TransportTerminal::OutcomeUnknown));
    assert!(matches!(
        server
            .run_with_options(&request(dir.path(), "after"), options(None))
            .unwrap()
            .terminal,
        TransportTerminal::Completed
    ));
}
#[test]
fn exec_check_rejects_outside_scratch_and_uses_protocol() {
    use std::collections::BTreeMap;
    let dir = crate::common::temp();
    let scratch = dir.path().join("scratch");
    let product = dir.path().join("product");
    std::fs::create_dir(&scratch).unwrap();
    std::fs::create_dir(&product).unwrap();
    let server = AppServer::new(crate::common::fake("fake_codex"));
    let mut spec = CommandSpec {
        program: "test".into(),
        args: vec![],
        cwd: scratch.clone(),
        env: BTreeMap::new(),
        read_only_roots: vec![product.clone()],
        writable_roots: vec![scratch.clone()],
        output_dir: scratch.clone(),
        timeout_s: 5,
    };
    let result = server.exec_check(&spec).unwrap();
    assert!(result.policy_applied);
    assert_eq!(
        std::fs::read_to_string(result.stdout_path).unwrap(),
        "sandboxed"
    );
    spec.cwd = product;
    assert_eq!(
        server.exec_check(&spec).unwrap_err().kind,
        "blocked_capability"
    );
}
