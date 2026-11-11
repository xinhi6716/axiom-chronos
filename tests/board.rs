use axiom::{
    base::{Failure, Res},
    board::{self, Cache, Choice, Lang, Model, Provider, Request, Scheduler, Summary, TokenUse},
};
use serde_json::json;
use std::{
    sync::{atomic::AtomicBool, Mutex},
    time::{Duration, Instant},
};

struct Fake {
    calls: Mutex<Vec<&'static str>>,
    luna: Res<(String, TokenUse)>,
    haiku: Res<(String, TokenUse)>,
}
impl Fake {
    fn new(luna: Res<(String, TokenUse)>, haiku: Res<(String, TokenUse)>) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            luna,
            haiku,
        }
    }
    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}
impl Model for Fake {
    fn luna(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, TokenUse)> {
        self.calls.lock().unwrap().push("luna");
        self.luna.clone()
    }
    fn haiku(&self, _: &str, _: Instant, _: &AtomicBool) -> Res<(String, TokenUse)> {
        self.calls.lock().unwrap().push("haiku");
        self.haiku.clone()
    }
}
fn ok(text: &str, input: Option<u64>) -> Res<(String, TokenUse)> {
    Ok((
        text.into(),
        TokenUse {
            input,
            output: None,
        },
    ))
}
fn err(kind: &str) -> Res<(String, TokenUse)> {
    Err(Failure::new(kind, kind))
}
fn req() -> Request {
    Request::new("session", Lang::En, "digest", &["event".into()])
}
fn run(f: &Fake, choice: Choice) -> Res<Summary> {
    board::summarize(
        f,
        choice,
        &req(),
        Instant::now() + Duration::from_secs(10),
        &AtomicBool::new(false),
    )
}

#[test]
fn request_limits_and_keys() {
    let events: Vec<_> = (0..23)
        .map(|n| format!("event{n}:{}", "字".repeat(250)))
        .collect();
    let digest = "文".repeat(6100);
    let r = Request::new("a", Lang::En, &digest, &events);
    let prompt = r.prompt();
    assert!(prompt.contains(&"文".repeat(6000)));
    assert!(!prompt.contains(&"文".repeat(6001)));
    assert!(!prompt.contains("event2:"));
    assert!(prompt.contains("event3:"));
    assert!(prompt.contains(&format!("event3:{}", "字".repeat(233))));
    assert!(!prompt.contains(&format!("event3:{}", "字".repeat(234))));
    assert!(prompt.contains("unknown"));
    let zh = Request::new("a", Lang::ZhTw, &digest, &events);
    assert!(zh.prompt().contains("未知"));
    assert_ne!(r.key(Choice::Luna), zh.key(Choice::Luna));
    assert_ne!(r.key(Choice::Luna), r.key(Choice::Haiku));
    assert_ne!(
        r.key(Choice::Luna),
        Request::new("a", Lang::En, "different", &events).key(Choice::Luna)
    );
    assert_ne!(
        r.key(Choice::Luna),
        Request::new("a", Lang::En, &digest, &events[4..]).key(Choice::Luna)
    );
    assert_eq!(
        r.key(Choice::Luna),
        Request::new("b", Lang::En, &digest, &events).key(Choice::Luna)
    );
}

#[test]
fn summarize_success_fallback_and_errors() {
    let f = Fake::new(
        ok("RECENT: done\nNEXT: next", Some(42)),
        ok("unused", Some(3)),
    );
    let summary = run(&f, Choice::Luna).unwrap();
    assert_eq!(f.calls(), ["luna"]);
    assert_eq!(summary.provider, Provider::Luna);
    assert_eq!(summary.tokens.input, Some(42));
    for kind in ["not_found", "timeout", "codex_exited"] {
        let f = Fake::new(err(kind), ok("RECENT: recovered\nNEXT: unknown", Some(7)));
        let s = run(&f, Choice::Luna).unwrap();
        assert_eq!(f.calls(), ["luna", "haiku"]);
        assert_eq!(s.provider, Provider::Haiku);
        assert_eq!(s.tokens.input, Some(7));
        assert_eq!(s.next, None);
    }
    let f = Fake::new(ok("  ", Some(9)), ok("RECENT: ok\nNEXT: 未知", None));
    let s = run(&f, Choice::Luna).unwrap();
    assert_eq!(f.calls(), ["luna", "haiku"]);
    assert_eq!(s.next, None);
    assert_eq!(s.tokens, TokenUse::default());
    let f = Fake::new(err("cancelled"), ok("unused", None));
    assert_eq!(run(&f, Choice::Luna).unwrap_err().kind, "cancelled");
    assert_eq!(f.calls(), ["luna"]);
    let f = Fake::new(err("not_found"), err("haiku_error"));
    assert_eq!(run(&f, Choice::Luna).unwrap_err().kind, "haiku_error");
    assert_eq!(f.calls(), ["luna", "haiku"]);
    let f = Fake::new(err("wrong"), ok("RECENT: hello\nNEXT: UnKnOwN", None));
    assert_eq!(run(&f, Choice::Haiku).unwrap().next, None);
    assert_eq!(f.calls(), ["haiku"]);
}

#[test]
fn summarize_guards_and_parser() {
    let f = Fake::new(ok("unused", None), ok("unused", None));
    assert_eq!(run(&f, Choice::Off).unwrap_err().kind, "off");
    assert_eq!(
        board::summarize(
            &f,
            Choice::Luna,
            &req(),
            Instant::now() + Duration::from_secs(1),
            &AtomicBool::new(true)
        )
        .unwrap_err()
        .kind,
        "cancelled"
    );
    assert_eq!(
        board::summarize(
            &f,
            Choice::Luna,
            &req(),
            Instant::now() - Duration::from_secs(1),
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .kind,
        "timeout"
    );
    assert!(f.calls().is_empty());
    let (recent, next) = board::parse_reply(&format!(
        "RECENT: {}\nNEXT: {}",
        "字".repeat(300),
        "新".repeat(100)
    ))
    .unwrap();
    assert_eq!(recent.chars().count(), 300);
    assert_eq!(next.unwrap().chars().count(), 20);
    assert_eq!(
        board::parse_reply(" hello\nworld "),
        Some(("hello world".into(), None))
    );
    assert_eq!(board::parse_reply("  "), None);
    assert_eq!(board::parse_reply("RECENT: x\nNEXT: ").unwrap().1, None);
}

#[test]
fn scheduler_gap_storm_and_wake() {
    let t = Instant::now();
    let gap = Duration::from_secs(120);
    let mut s = Scheduler::new(gap);
    assert_eq!(s.next_wake(t), None);
    s.observe("b", 1, t);
    s.observe("a", 1, t);
    assert_eq!(s.next_wake(t), Some(t));
    assert_eq!(s.due(t), vec![("a".into(), 1), ("b".into(), 1)]);
    assert!(s.due(t).is_empty());
    assert_eq!(s.next_wake(t), None);
    assert!(s.finished("a", 1, t));
    assert!(s.finished("b", 1, t));
    s.observe("a", 1, t);
    assert_eq!(s.next_wake(t), None);
    for rev in 2..=51 {
        s.observe("a", rev, t);
    }
    assert_eq!(s.next_wake(t), Some(t + gap));
    assert!(s.due(t + Duration::from_secs(119)).is_empty());
    assert_eq!(s.due(t + gap), vec![("a".into(), 51)]);
    assert!(s.due(t + gap).is_empty());
    s.observe("a", 52, t + gap);
    assert!(!s.finished("a", 51, t + gap));
    assert_eq!(s.next_wake(t + gap), Some(t + gap + gap));
    assert!(s.due(t + gap + Duration::from_secs(119)).is_empty());
    assert_eq!(s.due(t + gap + gap), vec![("a".into(), 52)]);
    assert!(s.finished("a", 52, t + gap + gap));
    assert!(s.due(t + Duration::from_secs(999_999)).is_empty());
    s.observe("c", 1, t);
    assert_eq!(s.due(t), vec![("c".into(), 1)]);
    s.forget("c");
    assert!(!s.finished("c", 1, t));
}

#[test]
fn cache_is_lru() {
    let mut c = Cache::new();
    let s = Summary {
        recent: "x".into(),
        next: None,
        provider: Provider::Luna,
        tokens: TokenUse::default(),
    };
    assert_eq!(c.get(1), None);
    for i in 0..256 {
        c.put(i, s.clone());
    }
    assert_eq!(c.get(0), Some(s.clone()));
    c.put(256, s.clone());
    assert_eq!(c.len(), 256);
    assert_eq!(c.get(1), None);
    assert_eq!(c.get(0), Some(s.clone()));
    assert_eq!(c.get(256), Some(s));
}

#[test]
fn real_helpers() {
    assert_eq!(
        board::haiku_args(),
        [
            "-p",
            "--model",
            "claude-haiku-5-5",
            "--tools",
            "",
            "--strict-mcp-config",
            "--setting-sources",
            "local",
            "--output-format",
            "json"
        ]
    );
    let (text, tokens) = board::parse_haiku_output(r#"{"result":"hello","usage":{"input_tokens":2,"cache_creation_input_tokens":3,"cache_read_input_tokens":4,"output_tokens":5}}"#).unwrap();
    assert_eq!(text, "hello");
    assert_eq!(
        tokens,
        TokenUse {
            input: Some(9),
            output: Some(5)
        }
    );
    assert_eq!(
        board::parse_haiku_output(r#"{"result":"x","usage":{"output_tokens":4}}"#)
            .unwrap()
            .1,
        TokenUse {
            input: None,
            output: Some(4)
        }
    );
    assert_eq!(
        board::parse_haiku_output(r#"{"result":"x"}"#).unwrap().1,
        TokenUse::default()
    );
    assert_eq!(
        board::parse_haiku_output(r#"{"is_error":true,"result":"bad"}"#)
            .unwrap_err()
            .kind,
        "haiku_failed"
    );
    assert_eq!(
        board::parse_haiku_output(r#"{}"#).unwrap_err().kind,
        "bad_reply"
    );
    assert_eq!(
        board::parse_haiku_output("oops").unwrap_err().kind,
        "bad_reply"
    );
    assert_eq!(
        board::luna_tokens(&json!({"usage":{"inputTokens":12,"outputTokens":8}})),
        TokenUse {
            input: Some(12),
            output: Some(8)
        }
    );
    assert_eq!(
        board::luna_tokens(&json!({"usage":null})),
        TokenUse::default()
    );
}
