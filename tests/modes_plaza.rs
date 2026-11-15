use axiom::{
    base::Res,
    modes::{
        plaza::{
            self,
            access::{self, Budget, Budgets},
            protocol,
            router::{self, Pick, Router, Routing},
        },
        providers::{Ask, Member, Provider, Reply, MAX_CALLS_PER_TURN},
        readonly::{ReadOnly, ToolRequest},
        runs,
        store::Transcript,
        Mode,
    },
};
use serde_json::json;
use std::{cell::RefCell, collections::BTreeMap, fs};
use tempfile::TempDir;

struct Fake {
    asks: RefCell<Vec<Ask>>,
    write: bool,
}
impl Fake {
    fn new() -> Self {
        Self {
            asks: RefCell::new(vec![]),
            write: false,
        }
    }
}
impl Provider for Fake {
    fn ask(&self, ask: &Ask) -> Res<Reply> {
        let mut asks = self.asks.borrow_mut();
        let previous = asks.iter().filter(|a| a.member == ask.member).count();
        asks.push(ask.clone());
        let tool_requests = if self.write && previous == 0 {
            vec![ToolRequest {
                tool: "write_file".into(),
                args: json!({"path":"keep", "content":"bad"}),
            }]
        } else {
            vec![]
        };
        Ok(Reply {
            text: match ask.member {
                Member::Opus => "yes",
                Member::Astra => "no",
                Member::Fable => "maybe",
                Member::Argon => "yes",
            }
            .into(),
            tool_requests,
            cost_usd: None,
            tokens: None,
        })
    }
}
struct ToolProvider {
    asks: RefCell<Vec<Ask>>,
    always: bool,
    cost: Option<f64>,
}
impl Provider for ToolProvider {
    fn ask(&self, ask: &Ask) -> Res<Reply> {
        let mut asks = self.asks.borrow_mut();
        asks.push(ask.clone());
        let count = asks.len();
        Ok(Reply {
            text: format!("reply {count}"),
            tool_requests: if self.always || count == 1 {
                vec![ToolRequest {
                    tool: "read_file".into(),
                    args: json!({"path": "source.txt"}),
                }]
            } else {
                vec![]
            },
            cost_usd: self.cost,
            tokens: None,
        })
    }
}
struct Route {
    routing: Routing,
    calls: RefCell<usize>,
}
impl Router for Route {
    fn route(&self, _: &str, _: &str) -> Res<Routing> {
        *self.calls.borrow_mut() += 1;
        Ok(self.routing.clone())
    }
}
fn route(members: &[Member]) -> Route {
    Route {
        routing: Routing {
            picks: members
                .iter()
                .map(|&member| Pick {
                    member,
                    reason: format!("reason {}", member.name()),
                })
                .collect(),
            note: "".into(),
        },
        calls: RefCell::new(0),
    }
}
fn setup(calls: u32) -> (TempDir, plaza::Room) {
    let budgets = Budgets(
        Member::ALL
            .into_iter()
            .map(|m| {
                (
                    m,
                    Budget {
                        max_calls: calls,
                        max_usd: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
    );
    setup_with_budgets(budgets)
}
fn setup_with_budgets(budgets: Budgets) -> (TempDir, plaza::Room) {
    let tmp = tempfile::tempdir().unwrap();
    let room = plaza::open(tmp.path(), Some(tmp.path()), &[], budgets).unwrap();
    (tmp, room)
}
#[test]
fn routing_mentions_resume_and_budgets() {
    let (tmp, room) = setup(5);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake::new();
    let router = route(&[Member::Opus, Member::Astra, Member::Fable]);
    let budgets_path = tmp
        .path()
        .join("modes")
        .join("sessions")
        .join(&room.session.id)
        .join("budgets.json");
    let before = fs::read(&budgets_path).unwrap();
    let result = room
        .post("hello", None, &router, &provider, &tools)
        .unwrap();
    assert_eq!(
        result
            .replies
            .iter()
            .map(|m| m.author.as_str())
            .collect::<Vec<_>>(),
        vec!["opus", "astra", "fable"]
    );
    assert_eq!(*router.calls.borrow(), 1);
    for (member, count) in [
        (Member::Opus, 1),
        (Member::Astra, 1),
        (Member::Fable, 1),
        (Member::Argon, 0),
    ] {
        assert_eq!(
            provider
                .asks
                .borrow()
                .iter()
                .filter(|a| a.member == member)
                .count(),
            count
        );
    }
    let messages = Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .thread(&result.thread)
        .unwrap();
    assert_eq!(messages[0].thread.as_deref(), Some(result.thread.as_str()));
    assert_eq!(messages[1].data["picks"][0]["reason"], "reason opus");
    let empty = route(&[]);
    let empty_result = room.post("hello", None, &empty, &provider, &tools).unwrap();
    assert!(empty_result.replies.is_empty());
    assert!(Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .thread(&empty_result.thread)
        .unwrap()
        .iter()
        .any(|m| m.kind == "router"));
    let mentions = room
        .post(
            "@FABLE, @opus. @fable @opusx",
            None,
            &router,
            &provider,
            &tools,
        )
        .unwrap();
    assert_eq!(
        mentions
            .replies
            .iter()
            .map(|m| m.author.as_str())
            .collect::<Vec<_>>(),
        vec!["fable", "opus"]
    );
    assert_eq!(*router.calls.borrow(), 1);
    assert_eq!(fs::read(&budgets_path).unwrap(), before);
    assert_eq!(
        runs::session_runs(tmp.path(), &room.session.id)
            .unwrap()
            .len(),
        1
    );
    let resumed = plaza::resume(tmp.path(), &room.session.id).unwrap();
    assert_eq!(resumed.run.id, room.run.id);
    assert_eq!(
        access::spend(tmp.path(), &room.session.id).unwrap()[&Member::Opus].unpriced_calls,
        2
    );
    assert_eq!(
        access::spend(tmp.path(), &room.session.id).unwrap()[&Member::Opus].usd,
        0.0
    );
    assert_eq!(
        Transcript::open(tmp.path(), &resumed.session.id)
            .unwrap()
            .thread(&result.thread)
            .unwrap(),
        messages
    );
    assert!(provider
        .asks
        .borrow()
        .iter()
        .all(|a| a.tools == access::grant() && a.mode == Mode::Plaza));
}
#[test]
fn router_validation_and_mentions() {
    for value in [
        json!({"picks":[{"member":"unknown","reason":"a"}]}),
        json!({"picks":[],"permissions":["write"]}),
        json!({"picks":[{"member":"opus","reason":"a","tools":[]}]}),
        json!({"picks":[{"member":"opus","reason":" " }]}),
        json!({"picks":[{"member":"opus","reason":"a"},{"member":"opus","reason":"b"}]}),
        json!({"picks":[{"member":"opus","reason":"a"},{"member":"astra","reason":"b"},{"member":"fable","reason":"c"},{"member":"argon","reason":"d"}]}),
    ] {
        assert_eq!(
            router::parse_routing(&value).unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert_eq!(
        protocol::mentions("@opusx @ASTRA. @opus, @astra @unknown"),
        vec![Member::Astra, Member::Opus]
    );
}
#[test]
fn round_debate_vote_and_read_only() {
    let (tmp, room) = setup(10);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake::new();
    assert_eq!(
        room.round("t1", "round", &[], &provider, &tools)
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        room.round("t1", "x", &[Member::Opus, Member::Opus], &provider, &tools)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let debate = room
        .debate(
            "t1",
            "topic",
            Member::Opus,
            Member::Astra,
            Member::Fable,
            &provider,
            &tools,
        )
        .unwrap();
    assert_eq!(
        debate.iter().map(|m| m.author.as_str()).collect::<Vec<_>>(),
        vec!["opus", "astra", "fable"]
    );
    let asks = provider.asks.borrow();
    let judge = asks.last().unwrap();
    assert!(judge.prompt.contains("opus: yes") && judge.prompt.contains("astra: no"));
    assert!(asks.iter().all(|a| a.tools == access::grant()));
    drop(asks);
    assert_eq!(
        room.debate(
            "t1",
            "x",
            Member::Opus,
            Member::Opus,
            Member::Fable,
            &provider,
            &tools
        )
        .unwrap_err()
        .kind,
        "invalid_argument"
    );
    let path = tmp
        .path()
        .join("modes")
        .join("sessions")
        .join(&room.session.id)
        .join("budgets.json");
    let before = fs::read(&path).unwrap();
    let options = vec!["yes".into(), "no".into()];
    let vote = room
        .vote(
            "t1",
            "question",
            &options,
            &[Member::Opus, Member::Astra, Member::Fable],
            &provider,
        )
        .unwrap();
    assert_eq!(vote.tally["yes"], 1);
    assert_eq!(vote.tally["no"], 1);
    assert_eq!(vote.tally["invalid"], 1);
    let asks = provider.asks.borrow();
    assert_eq!(asks.len(), 10);
    assert!(asks[7..].iter().all(|a| a.tools.is_empty()));
    drop(asks);
    assert!(room.vote("t1", "x", &options[..1], &[], &provider).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        runs::session_runs(tmp.path(), &room.session.id)
            .unwrap()
            .len(),
        1
    );
    assert!(Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .thread("t1")
        .unwrap()
        .iter()
        .any(|m| m.kind == "vote" && m.data["tally"]["invalid"] == 1));
    fs::write(tmp.path().join("keep"), "original").unwrap();
    let writer = Fake {
        asks: RefCell::new(vec![]),
        write: true,
    };
    room.post("@argon", None, &route(&[]), &writer, &tools)
        .unwrap();
    assert_eq!(
        fs::read_to_string(tmp.path().join("keep")).unwrap(),
        "original"
    );
    assert!(Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .read(0, 100)
        .unwrap()
        .iter()
        .any(|m| m.kind == "refused" && m.text.contains("write_file")));
    assert!(writer
        .asks
        .borrow()
        .iter()
        .all(|a| a.tools == access::grant()));
}
#[test]
fn budget_prevents_calls_and_persists_spend() {
    let (tmp, room) = setup(0);
    let tools = ReadOnly::new(None, tmp.path());
    let provider = Fake::new();
    let result = room
        .post("@opus", None, &route(&[]), &provider, &tools)
        .unwrap();
    assert!(provider.asks.borrow().is_empty());
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].0, Member::Opus);
    assert!(plaza::resume(tmp.path(), &room.session.id).is_ok());
}
#[test]
fn each_tool_round_is_budgeted_and_refusal_is_returned() {
    let (tmp, room) = setup(1);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake {
        asks: RefCell::new(vec![]),
        write: true,
    };
    let outcome = room
        .post("@opus", None, &route(&[]), &provider, &tools)
        .unwrap();
    assert_eq!(provider.asks.borrow().len(), 1);
    assert_eq!(outcome.skipped[0].0, Member::Opus);
    assert!(outcome.replies.is_empty());
    let spend = access::spend(tmp.path(), &room.session.id).unwrap();
    assert_eq!(spend[&Member::Opus].calls, 1);
    assert_eq!(spend[&Member::Opus].unpriced_calls, 1);

    let (tmp, room) = setup(3);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake {
        asks: RefCell::new(vec![]),
        write: true,
    };
    room.post("@opus", None, &route(&[]), &provider, &tools)
        .unwrap();
    let asks = provider.asks.borrow();
    assert_eq!(asks.len(), 2);
    assert!(asks[1].context.contains("refused tool write_file"));
    assert_eq!(
        access::spend(tmp.path(), &room.session.id).unwrap()[&Member::Opus].calls,
        2
    );
}

#[test]
fn post_rejects_four_router_picks_before_asking_members() {
    let (tmp, room) = setup(5);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake::new();
    let error = room
        .post("hello", None, &route(&Member::ALL), &provider, &tools)
        .err()
        .unwrap();
    assert_eq!(error.kind, "invalid_argument");
    assert!(provider.asks.borrow().is_empty());
}

#[test]
fn granted_tool_result_is_returned_and_turn_is_capped() {
    let (tmp, room) = setup(10);
    fs::write(tmp.path().join("source.txt"), "unique file contents").unwrap();
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = ToolProvider {
        asks: RefCell::new(vec![]),
        always: false,
        cost: None,
    };
    let outcome = room
        .post("@opus", None, &route(&[]), &provider, &tools)
        .unwrap();
    let asks = provider.asks.borrow();
    assert_eq!(asks.len(), 2);
    assert!(asks[0].tools.contains(&"read_file".to_owned()));
    assert!(asks[1]
        .context
        .contains("Tool read_file: unique file contents"));
    assert_eq!(outcome.replies[0].text, "reply 2");
    drop(asks);

    let provider = ToolProvider {
        asks: RefCell::new(vec![]),
        always: true,
        cost: None,
    };
    let outcome = room
        .post("@astra", None, &route(&[]), &provider, &tools)
        .unwrap();
    assert_eq!(provider.asks.borrow().len(), MAX_CALLS_PER_TURN);
    assert!(provider
        .asks
        .borrow()
        .iter()
        .all(|a| a.member == Member::Astra));
    assert_eq!(outcome.replies.len(), 1);
    assert_eq!(
        outcome.replies[0].text,
        format!("reply {MAX_CALLS_PER_TURN}")
    );
    let messages = Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .thread(&outcome.thread)
        .unwrap();
    let texts: Vec<_> = messages
        .iter()
        .filter(|m| m.author == "astra" && m.kind == "text")
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].text, format!("reply {MAX_CALLS_PER_TURN}"));
}

#[test]
fn priced_and_absent_member_budgets_prevent_further_asks() {
    let budgets = Budgets(
        [(
            Member::Opus,
            Budget {
                max_calls: 10,
                max_usd: Some(0.5),
            },
        )]
        .into_iter()
        .collect(),
    );
    let (tmp, room) = setup_with_budgets(budgets);
    fs::write(tmp.path().join("source.txt"), "contents").unwrap();
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = ToolProvider {
        asks: RefCell::new(vec![]),
        always: true,
        cost: Some(0.25),
    };
    let outcome = room
        .post("@opus @astra", None, &route(&[]), &provider, &tools)
        .unwrap();
    assert_eq!(provider.asks.borrow().len(), 2);
    assert!(provider
        .asks
        .borrow()
        .iter()
        .all(|a| a.member == Member::Opus));
    assert_eq!(
        outcome.skipped.iter().map(|s| s.0).collect::<Vec<_>>(),
        vec![Member::Opus, Member::Astra]
    );
    let spend = access::spend(tmp.path(), &room.session.id).unwrap();
    assert_eq!(spend[&Member::Opus].calls, 2);
    assert_eq!(spend[&Member::Opus].usd, 0.5);
    assert_eq!(spend[&Member::Opus].unpriced_calls, 0);
    assert!(!spend.contains_key(&Member::Astra));
    let later = room
        .post("@opus", None, &route(&[]), &provider, &tools)
        .unwrap();
    assert_eq!(later.skipped[0].0, Member::Opus);
    assert_eq!(provider.asks.borrow().len(), 2);
}

#[test]
fn zero_budget_members_are_skipped_in_round_debate_and_vote() {
    let budgets = Budgets(
        Member::ALL
            .into_iter()
            .map(|m| {
                (
                    m,
                    Budget {
                        max_calls: if m == Member::Opus { 0 } else { 10 },
                        max_usd: None,
                    },
                )
            })
            .collect(),
    );
    let (tmp, room) = setup_with_budgets(budgets);
    let tools = ReadOnly::new(Some(tmp.path()), tmp.path());
    let provider = Fake::new();
    let round = room
        .round(
            "t1",
            "round",
            &[Member::Opus, Member::Astra],
            &provider,
            &tools,
        )
        .unwrap();
    assert_eq!(
        round.iter().map(|m| m.author.as_str()).collect::<Vec<_>>(),
        vec!["astra"]
    );
    let debate = room
        .debate(
            "t1",
            "topic",
            Member::Opus,
            Member::Astra,
            Member::Fable,
            &provider,
            &tools,
        )
        .unwrap();
    assert_eq!(
        debate.iter().map(|m| m.author.as_str()).collect::<Vec<_>>(),
        vec!["astra", "fable"]
    );
    let vote = room
        .vote(
            "t1",
            "question",
            &["yes".into(), "no".into()],
            &[Member::Opus, Member::Astra],
            &provider,
        )
        .unwrap();
    assert_eq!(vote.ballots, vec![(Member::Astra, "no".into())]);
    assert!(provider
        .asks
        .borrow()
        .iter()
        .all(|a| a.member != Member::Opus));
}

#[test]
fn vote_ignores_tool_requests() {
    let (tmp, room) = setup(2);
    fs::write(tmp.path().join("keep"), "original").unwrap();
    let provider = Fake {
        asks: RefCell::new(vec![]),
        write: true,
    };
    let result = room
        .vote(
            "t1",
            "question",
            &["yes".into(), "no".into()],
            &[Member::Opus],
            &provider,
        )
        .unwrap();
    assert_eq!(provider.asks.borrow().len(), 1);
    assert!(provider.asks.borrow()[0].tools.is_empty());
    assert_eq!(result.ballots, vec![(Member::Opus, "yes".into())]);
    assert_eq!(
        fs::read_to_string(tmp.path().join("keep")).unwrap(),
        "original"
    );
    assert!(!Transcript::open(tmp.path(), &room.session.id)
        .unwrap()
        .thread("t1")
        .unwrap()
        .iter()
        .any(|m| m.kind == "refused"));
}
