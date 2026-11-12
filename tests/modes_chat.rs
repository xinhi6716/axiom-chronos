use axiom::{
    base::Res,
    modes::{
        chat::{Chat, ChatOptions},
        launch::{self, LaunchKind, LaunchRequest, Launched, Launcher},
        providers::{Ask, Member, Provider, Reply, MAX_CALLS_PER_TURN},
        readonly::{ReadOnly, ToolRequest, READ_TOOLS},
        runs,
        store::Transcript,
        Mode, Source,
    },
};
use serde_json::json;
use std::{cell::RefCell, fs, path::PathBuf};
use tempfile::TempDir;

struct FakeProvider {
    asks: RefCell<Vec<Ask>>,
    requests: Vec<ToolRequest>,
    always: bool,
}
impl FakeProvider {
    fn new(requests: Vec<ToolRequest>, always: bool) -> Self {
        Self {
            asks: RefCell::new(vec![]),
            requests,
            always,
        }
    }
}
impl Provider for FakeProvider {
    fn ask(&self, ask: &Ask) -> Res<Reply> {
        let mut asks = self.asks.borrow_mut();
        let count = asks.iter().filter(|a| a.member == ask.member).count();
        asks.push(ask.clone());
        Ok(Reply {
            text: format!("{} reply {}", ask.member.name(), count + 1),
            tool_requests: if self.always || count == 0 {
                self.requests.clone()
            } else {
                vec![]
            },
            cost_usd: None,
            tokens: None,
        })
    }
}
struct FakeLauncher {
    home: PathBuf,
    requests: RefCell<Vec<LaunchRequest>>,
}
impl FakeLauncher {
    fn new(home: &TempDir) -> Self {
        Self {
            home: home.path().into(),
            requests: RefCell::new(vec![]),
        }
    }
}
impl Launcher for FakeLauncher {
    fn launch(&self, req: &LaunchRequest) -> Res<Launched> {
        self.requests.borrow_mut().push(req.clone());
        Ok(Launched {
            run: launch::start_child(&self.home, req)?,
            detail: json!({}),
        })
    }
}
fn open(home: &TempDir, scope: &[String], opts: ChatOptions) -> Chat {
    Chat::open(home.path(), Some(home.path()), scope, opts).unwrap()
}
fn request(tool: &str, path: &str) -> ToolRequest {
    ToolRequest {
        tool: tool.into(),
        args: json!({"path": path, "content": "bad"}),
    }
}

#[test]
fn chat_promotes_full_snapshot_to_wish() {
    let home = TempDir::new().unwrap();
    fs::write(home.path().join("source"), "file contents").unwrap();
    let scope = vec!["src".into(), "tests/test.rs".into()];
    let chat = open(
        &home,
        &scope,
        ChatOptions {
            with_astra: true,
            ..Default::default()
        },
    );
    let provider = FakeProvider::new(vec![request("read_file", "source")], false);
    let tools = ReadOnly::new(Some(home.path()), home.path());
    chat.say(&provider, &tools, "Build it").unwrap();
    let transcript = Transcript::open(home.path(), &chat.session.id).unwrap();
    let snapshot = transcript.snapshot(None).unwrap();
    let sources: Vec<Source> = snapshot
        .messages
        .iter()
        .filter(|m| ["person", "opus", "astra", "fable", "argon"].contains(&m.author.as_str()))
        .map(|m| Source {
            session: chat.session.id.clone(),
            seq: m.seq,
            author: m.author.clone(),
        })
        .collect();
    assert!(snapshot.messages.iter().any(|m| m.kind == "tool"));
    let launcher = FakeLauncher::new(&home);
    let promotion = chat
        .promote(&launcher, LaunchKind::Wish, "Build it", None)
        .unwrap();
    let requests = launcher.requests.borrow();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.kind, LaunchKind::Wish);
    assert_eq!(req.session, chat.session.id);
    assert_eq!(req.parent_run, chat.run.id);
    assert_eq!(req.scope, scope);
    assert_eq!(req.context.as_ref(), Some(&snapshot));
    assert_eq!(req.sources, sources);
    assert_eq!(promotion.sources, sources);
    assert_eq!(promotion.from_run, chat.run.id);
    assert_eq!(promotion.snapshot_seq, snapshot.upto_seq);
    let child = runs::load_run(home.path(), &promotion.child.run.id).unwrap();
    assert_eq!(child.session, chat.session.id);
    assert_eq!(child.parent.as_deref(), Some(chat.run.id.as_str()));
    assert_eq!(child.mode, Mode::Wish);
    let after = transcript.snapshot(None).unwrap();
    let message = after.messages.last().unwrap();
    assert_eq!(message.kind, "promotion");
    assert_eq!(
        message.data,
        json!({"kind":"wish", "child_run":child.id, "snapshot_seq":snapshot.upto_seq})
    );
}

#[test]
fn quick_action_and_validation() {
    let home = TempDir::new().unwrap();
    let scope = vec!["src/file.rs".into()];
    let chat = open(&home, &scope, ChatOptions::default());
    let launcher = FakeLauncher::new(&home);
    let child = chat.small_action(&launcher, "fix", "cargo test").unwrap();
    let requests = launcher.requests.borrow();
    let req = &requests[0];
    assert_eq!(req.kind, LaunchKind::Quick);
    assert_eq!(req.scope, scope);
    assert_eq!(req.session, chat.session.id);
    assert_eq!(req.parent_run, chat.run.id);
    assert_eq!(req.verify.as_deref(), Some("cargo test"));
    assert!(!req.review);
    assert_eq!(req.context.as_ref().unwrap().session, chat.session.id);
    assert_eq!(child.run.parent.as_deref(), Some(chat.run.id.as_str()));
    assert_eq!(child.run.mode, Mode::Quick);
    drop(requests);
    assert_eq!(
        chat.small_action(&launcher, "fix", " ").unwrap_err().kind,
        "invalid_argument"
    );
    for verify in [None, Some(" ")] {
        assert_eq!(
            chat.promote(&launcher, LaunchKind::Quick, "fix", verify)
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
    }
    let empty = open(&home, &[], ChatOptions::default());
    assert_eq!(
        empty
            .small_action(&launcher, "fix", "test")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        empty
            .promote(&launcher, LaunchKind::Quick, "fix", Some("test"))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(launcher.requests.borrow().len(), 1);
    empty
        .promote(&launcher, LaunchKind::Wish, "wish", Some("optional"))
        .unwrap();
    assert_eq!(
        launcher.requests.borrow()[1].verify.as_deref(),
        Some("optional")
    );
    assert!(launcher.requests.borrow()[1].scope.is_empty());
}

#[test]
fn membership_and_context_limits() {
    let home = TempDir::new().unwrap();
    let tools = ReadOnly::new(None, home.path());
    let provider = FakeProvider::new(vec![], false);
    let chat = open(
        &home,
        &[],
        ChatOptions {
            context_bytes: 8,
            ..Default::default()
        },
    );
    chat.say(&provider, &tools, "long message 中文").unwrap();
    assert_eq!(
        provider
            .asks
            .borrow()
            .iter()
            .map(|a| a.member)
            .collect::<Vec<_>>(),
        vec![Member::Opus]
    );
    let provider = FakeProvider::new(vec![], false);
    let chat = open(
        &home,
        &[],
        ChatOptions {
            with_astra: true,
            context_bytes: 8,
        },
    );
    let messages = chat.say(&provider, &tools, "long message 中文").unwrap();
    let asks = provider.asks.borrow();
    assert_eq!(
        asks.iter().map(|a| a.member).collect::<Vec<_>>(),
        vec![Member::Opus, Member::Astra]
    );
    assert!(asks.iter().all(|a| a.context.len() <= 8
        && a.mode == Mode::Chat
        && a.session == chat.session.id
        && a.prompt == "long message 中文"
        && a.tools == READ_TOOLS.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
    assert_eq!(
        messages
            .iter()
            .map(|m| m.author.as_str())
            .collect::<Vec<_>>(),
        vec!["person", "opus", "astra"]
    );
}

#[test]
fn tools_are_read_only_and_calls_are_bounded() {
    let home = TempDir::new().unwrap();
    fs::write(home.path().join("source"), "known contents").unwrap();
    let tools = ReadOnly::new(Some(home.path()), home.path());
    let chat = open(
        &home,
        &[],
        ChatOptions {
            with_astra: true,
            context_bytes: 200,
        },
    );
    let provider = FakeProvider::new(
        vec![
            request("write_file", "new_file"),
            request("read_file", "source"),
        ],
        true,
    );
    let messages = chat.say(&provider, &tools, "inspect").unwrap();
    assert!(!home.path().join("new_file").exists());
    for member in [Member::Opus, Member::Astra] {
        let asks = provider.asks.borrow();
        let member_asks: Vec<_> = asks.iter().filter(|a| a.member == member).collect();
        assert_eq!(member_asks.len(), MAX_CALLS_PER_TURN);
        assert!(member_asks[1].context.contains("known contents"));
        drop(asks);
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.kind == "refused" && m.data["member"] == member.name())
                .count(),
            2
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.kind == "tool"
                    && m.data["member"] == member.name()
                    && m.text == "known contents")
                .count(),
            2
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.kind == "reply" && m.author == member.name())
                .count(),
            1
        );
    }
    assert!(messages
        .iter()
        .filter(|m| m.kind == "tool" || m.kind == "refused")
        .all(|m| m.author == "tool"));
    assert!(provider
        .asks
        .borrow()
        .iter()
        .all(|a| a.context.len() <= 200));
}

#[test]
fn resume_and_close() {
    let home = TempDir::new().unwrap();
    let chat = open(&home, &[], ChatOptions::default());
    let resumed = Chat::resume(home.path(), &chat.session.id, ChatOptions::default()).unwrap();
    assert_eq!(resumed.run.id, chat.run.id);
    let closed = resumed.close("done").unwrap();
    assert!(closed.finished.is_some());
    assert_eq!(closed.outcome.as_deref(), Some("done"));
    assert_eq!(resumed.close("again").unwrap_err().kind, "invalid_argument");
    let new_chat = Chat::resume(home.path(), &chat.session.id, ChatOptions::default()).unwrap();
    assert_ne!(new_chat.run.id, chat.run.id);
    assert!(new_chat.run.finished.is_none());
}
