mod common;

use axiom::council::Status;
use axiom::research_program::{
    dossier::{self, Confidence},
    investigation::{self, InvestigationRequest, MemberCall, MemberReply, ResearchMember},
};
use serde_json::{json, Value};
use std::{
    fs,
    sync::{atomic::AtomicBool, Arc, Barrier, Mutex},
};

struct Fake {
    name: &'static str,
    reply: MemberReply,
    calls: Mutex<Vec<MemberCall>>,
    barrier: Option<Arc<Barrier>>,
}
impl ResearchMember for Fake {
    fn name(&self) -> &str {
        self.name
    }
    fn call(&self, call: &MemberCall, _: &AtomicBool) -> MemberReply {
        self.calls.lock().unwrap().push(call.clone());
        if let Some(barrier) = &self.barrier {
            barrier.wait();
        }
        self.reply.clone()
    }
}
fn fake(name: &'static str, output: Value, barrier: Option<Arc<Barrier>>) -> Fake {
    Fake {
        name,
        calls: Mutex::new(Vec::new()),
        barrier,
        reply: MemberReply {
            status: Status::Ok,
            output: Some(output),
            raw: Some(format!("raw {name}")),
            model: None,
            provider: "fake".into(),
            usd: None,
            tokens: 0,
            detail: None,
        },
    }
}
fn valid() -> Value {
    json!({"items": [{"claim":"a", "source":"s", "confidence":"high"}]})
}

#[test]
fn investigation_is_concurrent_independent_and_validates() {
    let temp = common::temp();
    let cwd = temp.path().join("project");
    let dir = temp.path().join("results");
    fs::create_dir(&cwd).unwrap();
    let req = InvestigationRequest {
        question: "Why?".into(),
        sources: vec!["docs/source.txt".into()],
        cwd: cwd.clone(),
    };
    let barrier = Arc::new(Barrier::new(3));
    let a = fake("one", valid(), Some(barrier.clone()));
    let b = fake("two", valid(), Some(barrier.clone()));
    let c = fake("three", json!({"items":[{"claim":"bad"}]}), Some(barrier));
    let outcomes =
        investigation::investigate(&dir, &req, &[&a, &b, &c], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|o| o.member.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    let calls: Vec<_> = [&a, &b, &c]
        .iter()
        .map(|f| f.calls.lock().unwrap()[0].clone())
        .collect();
    for call in &calls {
        assert_eq!(call.prompt.as_bytes(), calls[0].prompt.as_bytes());
        assert_eq!(call.readable, calls[0].readable);
        assert_eq!(call.readable, [cwd.clone(), cwd.join("docs/source.txt")]);
        assert_eq!(call.stage, "investigate");
        assert_eq!(call.schema, investigation::findings_schema());
        assert!(call.timeout_s > 0);
        let inputs = format!("{} {:?}", call.prompt, call.readable)
            .replace('\\', "/")
            .to_lowercase();
        assert!(!inputs.contains("plan"));
        assert!(!inputs.contains("astra/"));
        for member in ["one", "two", "three"] {
            assert!(!inputs.contains(
                &dir.join(member)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase()
            ));
        }
        assert_eq!(call.out_dir, dir.join(&call.member));
        assert_eq!(
            fs::read_to_string(call.out_dir.join("prompt.txt")).unwrap(),
            call.prompt
        );
        assert_eq!(
            fs::read_to_string(call.out_dir.join("raw.txt")).unwrap(),
            format!("raw {}", call.member)
        );
        let expected = if call.member == "three" {
            vec!["prompt.txt", "raw.txt"]
        } else {
            vec!["findings.json", "prompt.txt", "raw.txt"]
        };
        let mut entries: Vec<_> = fs::read_dir(&call.out_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, expected);
    }
    assert_eq!(outcomes[2].status, Status::Malformed);
    assert!(outcomes[2].findings.is_none());
    for outcome in &outcomes[..2] {
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(outcome.findings.as_ref().unwrap()).unwrap())
                .unwrap(),
            valid()
        );
    }
}

#[test]
fn investigation_requires_two_valid_and_removes_stale_findings() {
    let temp = common::temp();
    let req = InvestigationRequest {
        question: "q".into(),
        sources: vec![],
        cwd: temp.path().into(),
    };
    let a = fake("one", valid(), None);
    let b = fake("two", json!({"items": [{"claim":"missing source"}]}), None);
    fs::create_dir_all(temp.path().join("out/two")).unwrap();
    fs::write(temp.path().join("out/two/findings.json"), "stale").unwrap();
    let failure = investigation::investigate(
        &temp.path().join("out"),
        &req,
        &[&a, &b],
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(failure.kind, "too_few_members");
    assert!(!temp.path().join("out/two/findings.json").exists());
}

#[test]
fn dossier_merges_rejects_and_links_contradictions() {
    let findings = vec![
        (
            "alice".into(),
            json!({"items":[
                {"claim":"Claim A.","source":"Source A","confidence":"high","evidence":"proof","contradicts":"Claim B!"},
                {"claim":" claim   a ","source":"source a.","confidence":"low","contradicts":"Unknown claim"},
                {"claim":"Claim B","source":"Source B","confidence":"medium"},
                {"claim":"bad","confidence":"low"},
                {"claim":"bad","source":"s"},
                {"claim":"bad","source":"s","confidence":"certain"},
                {"claim":"  ","source":"s","confidence":"low"}
            ]}),
        ),
        (
            "bob".into(),
            json!({"items":[{"claim":"CLAIM A","source":"SOURCE A","confidence":"medium","evidence":"second"}]}),
        ),
    ];
    let d = dossier::build("question", &findings);
    assert_eq!(d.items.len(), 2);
    assert_eq!(d.rejected.len(), 4);
    assert!(d
        .rejected
        .iter()
        .all(|r| r.author == "alice" && !r.reason.is_empty()));
    let a = &d.items[0];
    assert_eq!(a.id, "E1");
    assert_eq!(a.independent_support, 2);
    assert_eq!(a.sources.len(), 2);
    assert_eq!(a.sources[0].author, "alice");
    assert_eq!(a.sources[0].evidence.as_deref(), Some("proof"));
    assert_eq!(a.sources[1].author, "bob");
    assert_eq!(a.confidence, Confidence::Medium);
    assert!(a.contradictions.contains(&"E2".into()));
    assert!(a.contradictions.contains(&"Unknown claim".into()));
    assert_eq!(d.items[1].contradictions, ["E1"]);
    assert_eq!(dossier::normalize("  Hello   WORLD!!! "), "hello world");
    let temp = common::temp();
    let path = temp.path().join("dossier.json");
    dossier::write(&path, &d).unwrap();
    assert_eq!(dossier::read(&path).unwrap(), d);
    fs::write(&path, "{").unwrap();
    assert_eq!(dossier::read(&path).unwrap_err().kind, "parse");
    assert_eq!(
        dossier::read(&temp.path().join("missing"))
            .unwrap_err()
            .kind,
        "io"
    );
}

#[test]
fn schema_keeps_required_shape() {
    let schema = investigation::findings_schema();
    assert_eq!(schema["required"], json!(["items"]));
    let item = &schema["properties"]["items"]["items"];
    assert_eq!(item["required"], json!(["claim", "source", "confidence"]));
    assert_eq!(
        item["properties"]["confidence"]["enum"],
        json!(["low", "medium", "high"])
    );
    assert_eq!(item["additionalProperties"], false);
    for key in ["claim", "source", "confidence", "evidence", "contradicts"] {
        assert_eq!(item["properties"][key]["type"], "string");
    }
}
