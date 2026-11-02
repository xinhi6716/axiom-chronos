#[path = "common/mod.rs"]
mod common;

use axiom::{
    base::progress::{self, Progress},
    bridge::{serve, Config},
};
use serde_json::{json, Value};
use std::{
    io::{self, Cursor, Write},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn config(path: &std::path::Path) -> Config {
    Config {
        data_home: Some(path.into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    }
}
fn lines(sink: &Sink) -> Vec<Value> {
    String::from_utf8(sink.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn request(id: u64, name: &str, args: Value, token: Option<Value>) -> Value {
    let mut v = json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
    if let Some(token) = token {
        v["params"]["_meta"] = json!({"progressToken":token});
    }
    v
}
fn run(requests: Vec<Value>, path: &std::path::Path) -> Vec<Value> {
    let input = requests
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>();
    let sink = Sink(Arc::new(Mutex::new(Vec::new())));
    assert_eq!(serve(config(path), Cursor::new(input), sink.clone()), 0);
    lines(&sink)
}
fn check(messages: &[Value], id: u64, token: Value, expected: &[&str]) {
    let result = messages.iter().position(|v| v["id"] == id).unwrap();
    assert_eq!(
        messages[result]["result"]["structuredContent"]["status"], "completed",
        "{:?}",
        messages[result]
    );
    let events: Vec<_> = messages
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            v["method"] == "notifications/progress" && v["params"]["progressToken"] == token
        })
        .collect();
    assert_eq!(events.len(), expected.len(), "{messages:?}");
    for (n, ((index, event), method)) in events.iter().zip(expected).enumerate() {
        assert!(*index < result);
        assert_eq!(event["params"]["progress"], n + 1);
        assert_eq!(event["params"]["message"], *method);
    }
}
#[test]
fn codex_and_agy_events_and_invalid_tokens() {
    let temp = common::temp();
    let cwd = temp.path().to_str().unwrap();
    let codex = json!({"cwd":cwd,"role":"terra","sandbox":"read-only","brief":"hello"});
    let msgs = run(
        vec![request(1, "codex_run", codex.clone(), Some(json!("codex")))],
        temp.path(),
    );
    check(
        &msgs,
        1,
        json!("codex"),
        &[
            "item/completed",
            "item/completed",
            "thread/tokenUsage/updated",
            "item/completed",
            "turn/completed",
        ],
    );
    for token in [None, Some(json!(true)), Some(json!(1.5)), Some(json!({}))] {
        let msgs = run(
            vec![request(3, "codex_run", codex.clone(), token)],
            temp.path(),
        );
        assert_eq!(msgs.len(), 1, "{msgs:?}");
    }
}
#[test]
fn concurrent_calls_do_not_cross() {
    let temp = common::temp();
    let cwd = temp.path().to_str().unwrap();
    let args = |brief| json!({"cwd":cwd,"role":"terra","sandbox":"read-only","brief":brief});
    let msgs = run(
        vec![
            request(1, "codex_run", args("BARRIER EXTRA"), Some(json!("a"))),
            request(2, "codex_run", args("BARRIER"), Some(json!("b"))),
        ],
        temp.path(),
    );
    check(
        &msgs,
        1,
        json!("a"),
        &["test/extra", "item/completed", "turn/completed"],
    );
    check(&msgs, 2, json!("b"), &["item/completed", "turn/completed"]);
    assert_eq!(msgs.len(), 7);
}

#[test]
fn scope_restores_and_closes_on_unwind() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let out = values.clone();
    let p = Progress::new(json!("t"), move |v| out.lock().unwrap().push(v));
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        progress::scope(Some(p.clone()), || {
            assert!(progress::current().is_some());
            p.emit("one");
            panic!("test");
        })
    }));
    assert!(progress::current().is_none());
    p.emit("late");
    assert_eq!(p.count(), 1);
    assert_eq!(values.lock().unwrap().len(), 1);
}
