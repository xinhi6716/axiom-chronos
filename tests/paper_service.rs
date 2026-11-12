use axiom::{
    bridge::{paper_tool::ProductionMembers, tools, Bridge, Config},
    council::{self, Status, ARGON_MODEL, FABLE_MODEL},
    paper::{
        cache::FetchMode,
        evidence::BundleInputs,
        http::CannedTransport,
        members::PaperMembers,
        readiness::{self, Condition},
        service::PaperSession,
        tectonic, *,
    },
    research_program::{
        self, dossier,
        investigation::{MemberCall, MemberReply, ResearchMember},
    },
    seams::FakeClock,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use tempfile::{tempdir, TempDir};

const PASSES: [&str; 4] = [
    include_str!("fixtures/paper/runner/pass1.json"),
    include_str!("fixtures/paper/runner/pass2.json"),
    include_str!("fixtures/paper/runner/pass3.json"),
    include_str!("fixtures/paper/runner/pass4.json"),
];
#[derive(Clone, Debug)]
enum TraceItem {
    Call(MemberCall),
    Event(PaperEvent),
}
type Trace = Arc<Mutex<Vec<TraceItem>>>;
struct Fake {
    name: &'static str,
    replies: Mutex<VecDeque<MemberReply>>,
    trace: Trace,
}
impl ResearchMember for Fake {
    fn name(&self) -> &str {
        self.name
    }
    fn call(&self, call: &MemberCall, _: &AtomicBool) -> MemberReply {
        self.trace
            .lock()
            .unwrap()
            .push(TraceItem::Call(call.clone()));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected model call")
    }
}
fn reply(output: Value, model: &str) -> MemberReply {
    MemberReply {
        status: Status::Ok,
        output: Some(output),
        raw: Some("fake".into()),
        model: Some(model.into()),
        provider: "fake".into(),
        usd: Some(0.1),
        tokens: 10,
        detail: None,
    }
}
struct Members {
    astra: Fake,
    fable: Fake,
    argon: Fake,
    reviewer: Fake,
    trace: Trace,
}
impl Members {
    fn new(verdict: &str) -> Self {
        let trace = Arc::new(Mutex::new(vec![]));
        let fake = |name, replies: Vec<MemberReply>| Fake {
            name,
            replies: Mutex::new(replies.into()),
            trace: trace.clone(),
        };
        let pass = |i: usize, model| reply(serde_json::from_str(PASSES[i]).unwrap(), model);
        Self {
            astra: fake("astra", vec![pass(0, "codex-model")]),
            fable: fake("fable", vec![pass(1, FABLE_MODEL), pass(3, FABLE_MODEL)]),
            argon: fake("argon", vec![pass(2, ARGON_MODEL)]),
            reviewer: fake(
                "reviewer",
                vec![reply(
                    json!({"verdict":verdict,"findings":[],"checked":[]}),
                    "review-model",
                )],
            ),
            trace,
        }
    }
    fn refs(&self) -> PaperMembers<'_> {
        PaperMembers {
            astra: &self.astra,
            fable: &self.fable,
            argon: &self.argon,
            reviewer: &self.reviewer,
        }
    }
    fn events(&self) -> Vec<PaperEvent> {
        self.trace
            .lock()
            .unwrap()
            .iter()
            .filter_map(|t| match t {
                TraceItem::Event(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }
    fn calls(&self) -> Vec<MemberCall> {
        self.trace
            .lock()
            .unwrap()
            .iter()
            .filter_map(|t| match t {
                TraceItem::Call(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }
}

// Build a transport directory from the independently maintained ranking fixtures.
// All requests, including bibliography lookups, are handled by CannedTransport.
fn canned(dir: &Path) -> Arc<CannedTransport> {
    fs::create_dir_all(dir).unwrap();
    let journals: Vec<JournalRecord> =
        serde_json::from_str(include_str!("fixtures/paper/journals/journals.json")).unwrap();
    let related: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/paper/journals/related.json")).unwrap();
    let mut works: Vec<Value> = related.iter().map(|w| json!({"id":w["id"],"display_name":w["title"],"publication_year":w["year"],"primary_location":{"source":{"id":w["source_id"]}},"topics":w["topics"].as_array().unwrap().iter().map(|x| json!({"display_name":x})).collect::<Vec<_>>() })).collect();
    for j in &journals[4..] {
        works.push(json!({"id":format!("work-{}",j.id),"display_name":"unrelated","primary_location":{"source":{"id":j.id}}}));
    }
    fs::write(
        dir.join("works.json"),
        serde_json::to_vec(&json!({"results":works,"meta":{"next_cursor":null}})).unwrap(),
    )
    .unwrap();
    let mut index: Vec<Value> = vec![];
    let from_year: i32 = axiom::base::now_iso()[..4].parse::<i32>().unwrap() - 5; // matching::gather currently uses the system year
    for query in [
        "cancer%20immune%20therapy%20trial",
        "cancer%20biomarker",
        "genomics",
    ] {
        index.push(json!({"url":format!("https://api.openalex.org/works?search={query}&filter=from_publication_date:{from_year}-01-01&per-page=200&cursor=*"),"status":200,"file":"works.json"}));
    }
    let mut sjr = String::from("Issn;SJR;Categories;Publisher;Open Access\n");
    for j in &journals {
        let id = &j.id;
        let source = json!({"id":id,"display_name":j.title,"issn":[id],"host_organization_name":"Elsevier","is_oa": !matches!(j.access.value(), Some(Access::Subscription | Access::Hybrid)),"topics":j.topics.iter().map(|x|json!({"display_name":x})).collect::<Vec<_>>()});
        let file = format!("source-{id}.json");
        fs::write(dir.join(&file), serde_json::to_vec(&source).unwrap()).unwrap();
        index.push(json!({"url":format!("https://api.openalex.org/sources/{id}"),"status":200,"file":file}));
        let q = if id == "e-tier" { 2 } else { 1 };
        sjr.push_str(&format!("{id};1.2;Oncology (Q{q});Elsevier;yes\n"));
        let bib = match j.apc.value() {
            Some(Apc::NoFee) => json!({"apc":{"has_apc":false}}),
            Some(Apc::Fee(m)) => {
                json!({"apc":{"has_apc":true,"max":[{"price":m.amount_minor as f64/100.0,"currency":m.currency}]}})
            }
            None => json!({}),
        };
        let payload = if id == "f-access" || id == "h-hybrid" {
            json!({"results":[]})
        } else {
            json!({"results":[{"bibjson":bib}]})
        };
        let file = format!("doaj-{id}.json");
        fs::write(dir.join(&file), serde_json::to_vec(&payload).unwrap()).unwrap();
        index.push(json!({"url":format!("https://doaj.org/api/search/journals/issn%3A{id}"),"status":200,"file":file}));
    }
    fs::write(dir.join("sjr.csv"), sjr).unwrap();
    for year in [2024, 2025] {
        index.push(json!({"url":format!("https://www.scimagojr.com/journalrank.php?year={year}&type=j&out=xls"),"status":200,"file":"sjr.csv"}));
    }
    let cr = include_str!("fixtures/paper/citations/article-crossref.json")
        .replace("10.1038/NATURE14539", "10.1234/example");
    let oa = include_str!("fixtures/paper/citations/article-openalex.json")
        .replace("10.1038/nature14539", "10.1234/example");
    fs::write(dir.join("crossref.json"), cr).unwrap();
    fs::write(dir.join("openalex.json"), oa).unwrap();
    fs::write(dir.join("not-found.json"), "{}").unwrap();
    for (url, file, status) in [
        (
            "https://api.crossref.org/works/10.1234%2Fexample",
            "crossref.json",
            200,
        ),
        (
            "https://api.openalex.org/works/doi:10.1234%2Fexample",
            "openalex.json",
            200,
        ),
        (
            "https://api.openalex.org/works/W123456789",
            "not-found.json",
            404,
        ),
    ] {
        for _ in 0..4 {
            index.push(json!({"url":url,"status":status,"file":file}));
        }
    }
    fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
    Arc::new(CannedTransport::from_dir(dir).unwrap())
}
struct Case {
    _dir: TempDir,
    session: PaperSession,
    inputs: BundleInputs,
    transport: Arc<CannedTransport>,
    stop: AtomicBool,
}
impl Case {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        let store = root.join("store");
        let home = dir.path().join("home");
        fs::create_dir_all(research_program::program_dir(&store)).unwrap();
        let dossier: dossier::Dossier =
            serde_json::from_str(include_str!("fixtures/paper/evidence/dossier.json")).unwrap();
        dossier::write(
            &research_program::program_dir(&store).join("02-dossier.json"),
            &dossier,
        )
        .unwrap();
        for (file, bytes) in [
            (
                "results.json",
                include_bytes!("fixtures/paper/evidence/results.json").as_slice(),
            ),
            (
                "data.csv",
                include_bytes!("fixtures/paper/evidence/data.csv").as_slice(),
            ),
            (
                "model.txt",
                include_bytes!("fixtures/paper/evidence/model.txt").as_slice(),
            ),
        ] {
            fs::write(root.join(file), bytes).unwrap();
        }
        let transport = canned(&dir.path().join("canned"));
        let session = PaperSession::open(
            &store,
            &home,
            &root,
            transport.clone(),
            Arc::new(FakeClock::new(1_735_689_600_000)),
        )
        .unwrap();
        let inputs = BundleInputs {
            store,
            project_root: root.clone(),
            results: vec![root.join("results.json")],
            datasets: vec![root.join("data.csv")],
            models: vec![root.join("model.txt")],
        };
        Self {
            _dir: dir,
            session,
            inputs,
            transport,
            stop: AtomicBool::new(false),
        }
    }
    fn requirements(&self) -> Requirements {
        serde_json::from_str(include_str!("fixtures/paper/journals/requirements.json")).unwrap()
    }
    fn shortlist(&self) -> Shortlist {
        self.session
            .shortlist(&self.requirements(), None, FetchMode::Online)
            .unwrap()
    }
    fn prepare(&self) -> Shortlist {
        self.session.build_evidence(&self.inputs).unwrap();
        let s = self.shortlist();
        self.session
            .choose_journal(&s.ranked[0].journal.id, &s.hash)
            .unwrap();
        s
    }
    fn write(&self, m: &Members, digest: &str) -> axiom::base::Res<readiness::ReadyStatus> {
        self.session.write(&m.refs(), digest, &self.stop, &|e| {
            m.trace.lock().unwrap().push(TraceItem::Event(e.clone()))
        })
    }
    fn setup_tectonic(&self) -> bool {
        let Some(t) = tectonic::find_for_tests() else {
            eprintln!("Tectonic unavailable; skipping pipeline test");
            return false;
        };
        fs::create_dir_all(self.session.home.join("paper")).unwrap();
        fs::write(
            self.session.home.join("paper/settings.json"),
            serde_json::to_vec(&json!({"tectonic":t.argv})).unwrap(),
        )
        .unwrap();
        true
    }
}
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !dir.exists() {
            return;
        }
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(root, &p, files);
            } else {
                files.insert(
                    p.strip_prefix(root).unwrap().to_owned(),
                    fs::read(p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(dir, dir, &mut out);
    out
}

#[test]
fn status_is_read_only() {
    let c = Case::new();
    let m = Members::new("ready");
    let dir = paper_dir(&c.session.store);
    for part_written in [false, true] {
        if part_written {
            c.session.build_evidence(&c.inputs).unwrap();
        }
        let before = files(&dir);
        let requests = c.transport.requests();
        let status = c.session.status().unwrap();
        assert!(!status.running);
        assert_eq!(files(&dir), before);
        assert_eq!(c.transport.requests(), requests);
        assert!(c.transport.requests().is_empty());
        assert!(m.calls().is_empty());
    }
}
#[test]
fn shortlist_never_chooses() {
    let c = Case::new();
    let s = c.shortlist();
    assert!(!paper_dir(&c.session.store)
        .join("journal-choice.json")
        .exists());
    assert!(c.session.status().unwrap().choice.is_none());
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/paper/journals/expected_order.json")).unwrap();
    let ids: Vec<_> = s.ranked.iter().map(|j| j.journal.id.as_str()).collect();
    let wanted: Vec<_> = expected["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, wanted);
    assert_eq!(
        s.ranked.iter().map(|j| &j.bucket).collect::<Vec<_>>(),
        vec![
            &Bucket::Passes,
            &Bucket::Passes,
            &Bucket::NeedsCheck,
            &Bucket::NeedsCheck
        ]
    );
}
#[test]
fn person_selects_only_current_ranked_journal() {
    let c = Case::new();
    let s = c.shortlist();
    assert_eq!(
        c.session
            .choose_journal("unknown", &s.hash)
            .unwrap_err()
            .kind,
        "not_in_shortlist"
    );
    assert!(!s.excluded.is_empty());
    assert_eq!(
        c.session
            .choose_journal(&s.excluded[0].journal.id, &s.hash)
            .unwrap_err()
            .kind,
        "not_in_shortlist"
    );
    assert_eq!(
        c.session
            .choose_journal(&s.ranked[0].journal.id, "old-hash")
            .unwrap_err()
            .kind,
        "stale_shortlist"
    );
    let chosen = c
        .session
        .choose_journal(&s.ranked[0].journal.id, &s.hash)
        .unwrap();
    assert_eq!(chosen.chosen_by, Chooser::Person);
    assert_eq!(chosen.shortlist_hash, s.hash);
    assert_eq!(chosen.at, "2025-01-01T00:00:00Z");
    assert_eq!(c.session.status().unwrap().choice, Some(chosen));
}
#[test]
fn write_requires_choice_and_current_estimate() {
    let c = Case::new();
    let m = Members::new("ready");
    assert_eq!(c.write(&m, "wrong").unwrap_err().kind, "no_journal");
    assert!(m.calls().is_empty());
    c.prepare();
    let e = c.session.estimate().unwrap();
    assert_eq!(c.write(&m, "wrong").unwrap_err().kind, "estimate_changed");
    assert!(m.calls().is_empty());
    // Force a controlled pass rejection, proving the estimate was accepted without needing Tectonic.
    m.astra.replies.lock().unwrap()[0].output =
        Some(serde_json::from_str(include_str!("fixtures/paper/runner/bad_claim.json")).unwrap());
    assert_eq!(c.write(&m, &e.digest).unwrap_err().kind, "rejected");
    assert_eq!(m.calls().len(), 1);
}
#[test]
fn estimate_precedes_first_call() {
    let c = Case::new();
    c.prepare();
    let m = Members::new("ready");
    let e = c.session.estimate().unwrap();
    m.astra.replies.lock().unwrap()[0].output =
        Some(serde_json::from_str(include_str!("fixtures/paper/runner/bad_claim.json")).unwrap());
    assert_eq!(c.write(&m, &e.digest).unwrap_err().kind, "rejected");
    let trace = m.trace.lock().unwrap();
    let estimate = trace
        .iter()
        .position(|t| matches!(t, TraceItem::Event(PaperEvent::Estimate(_))))
        .unwrap();
    let call = trace
        .iter()
        .position(|t| matches!(t, TraceItem::Call(_)))
        .unwrap();
    assert!(estimate < call);
    assert!(
        matches!(&trace[estimate], TraceItem::Event(PaperEvent::Estimate(actual)) if actual.digest == e.digest)
    );
}
fn assert_ready(status: &readiness::ReadyStatus) {
    assert!(status.ready, "{:?}", status.conditions);
    assert_eq!(
        status
            .conditions
            .iter()
            .map(|(c, _, _)| *c)
            .collect::<Vec<_>>(),
        [
            Condition::FourPasses,
            Condition::Citations,
            Condition::Numbers,
            Condition::Compile,
            Condition::Render,
            Condition::Disclosure,
            Condition::Review
        ]
    );
    assert!(status.conditions.iter().all(|(_, ok, _)| *ok));
}
#[test]
fn pipeline_reaches_ready_and_checks_in_order() {
    let c = Case::new();
    if !c.setup_tectonic() {
        return;
    }
    c.prepare();
    let m = Members::new("ready");
    let e = c.session.estimate().unwrap();
    let ready = c.write(&m, &e.digest).unwrap();
    assert_ready(&ready);
    let checks: Vec<_> = m
        .events()
        .into_iter()
        .filter_map(|e| match e {
            PaperEvent::Check { step, ok, .. } => {
                assert!(ok);
                Some(step)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        checks,
        [
            Step::Figures,
            Step::Citations,
            Step::Numbers,
            Step::Compile,
            Step::Render,
            Step::Disclosure,
            Step::Review
        ]
    );
    let status = c.session.status().unwrap();
    assert_ready(status.ready.as_ref().unwrap());
    assert!(status.pdf.as_ref().unwrap().is_file());
    assert!(!status.running);
    assert!(status.last_error.is_none());
}
#[test]
fn invalid_pass_and_cancel_stop_and_persist_error() {
    let c = Case::new();
    c.prepare();
    let m = Members::new("ready");
    let e = c.session.estimate().unwrap();
    m.astra.replies.lock().unwrap()[0].output =
        Some(serde_json::from_str(include_str!("fixtures/paper/runner/bad_claim.json")).unwrap());
    assert_eq!(c.write(&m, &e.digest).unwrap_err().kind, "rejected");
    assert!(m
        .events()
        .iter()
        .any(|e| matches!(e, PaperEvent::Blocked { kind, .. } if kind == "rejected")));
    let status_file: Value =
        serde_json::from_slice(&fs::read(paper_dir(&c.session.store).join("status.json")).unwrap())
            .unwrap();
    assert_eq!(status_file["last_error"]["kind"], "rejected");
    assert_eq!(status_file["running"], false);
    let status = c.session.status().unwrap();
    assert_eq!(status.last_error.as_ref().unwrap().0, "rejected");
    assert_eq!(
        status.last_error.as_ref().unwrap().1,
        status_file["last_error"]["reason"].as_str().unwrap()
    );
    assert!(!status.running);
    let c = Case::new();
    c.prepare();
    let e = c.session.estimate().unwrap();
    let m = Members::new("ready");
    c.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(c.write(&m, &e.digest).unwrap_err().kind, "cancelled");
    assert!(m.calls().is_empty());
    assert!(!c.session.status().unwrap().running);
}
#[test]
fn revision_is_a_valid_but_not_ready_review() {
    let c = Case::new();
    if !c.setup_tectonic() {
        return;
    }
    c.prepare();
    let m = Members::new("revise");
    let e = c.session.estimate().unwrap();
    let ready = c.write(&m, &e.digest).unwrap();
    assert!(!ready.ready);
    assert!(ready
        .conditions
        .iter()
        .any(|(condition, ok, reason)| *condition == Condition::Review
            && !ok
            && !reason.is_empty()));
    assert!(!c.session.status().unwrap().ready.unwrap().ready);
}
#[test]
fn edited_final_manuscript_invalidates_readiness() {
    let c = Case::new();
    if !c.setup_tectonic() {
        return;
    }
    c.prepare();
    let m = Members::new("ready");
    let e = c.session.estimate().unwrap();
    assert_ready(&c.write(&m, &e.digest).unwrap());
    let path = readiness::final_manuscript_path(&c.session.store);
    let mut manuscript: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manuscript["title"] = json!("changed after review");
    fs::write(path, serde_json::to_vec(&manuscript).unwrap()).unwrap();
    let before = files(&paper_dir(&c.session.store));
    let status = c.session.status().unwrap();
    assert!(!status.ready.unwrap().ready);
    assert_eq!(
        files(&paper_dir(&c.session.store)),
        before,
        "status must not remove stale readiness"
    );
    assert!(
        !readiness::evaluate(&c.session.store, &c.session.project_root)
            .unwrap()
            .ready
    );
}
#[test]
fn bridge_members_and_tool_surface() {
    let dir = tempdir().unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(dir.path().into()),
        codex: Ok(vec![]),
        wsl: None,
        on_windows: false,
    });
    let setup = council::Setup::new(Ok(vec![]), Ok(vec![]));
    let members = ProductionMembers::new(&bridge, &setup, dir.path(), dir.path());
    let m = members.members();
    assert_eq!(
        [
            m.astra.name(),
            m.fable.name(),
            m.argon.name(),
            m.reviewer.name()
        ],
        ["astra", "fable", "argon", "reviewer"]
    );
    let desc = tools();
    let names: BTreeSet<_> = desc
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    let expected: BTreeSet<_> = [
        "codex_run",
        "orch",
        "run_on_linux",
        "memory_record",
        "memory_search",
        "memory_get",
        "memory_digest",
        "graph_explore",
        "goal",
        "bridge_status",
        "approval",
    ]
    .into_iter()
    .collect();
    assert_eq!(names, expected);
    assert!(names.iter().all(|n| !n.contains("paper")));
}

#[test]
fn canned_journal_sources_reproduce_ranked_fixture() {
    use axiom::paper::{
        cache::Cache,
        http::ClockPause,
        journals::{
            doaj,
            matching::{self, JournalSources},
            openalex::OpenAlex,
            sjr,
        },
    };
    let dir = tempdir().unwrap();
    let transport = canned(&dir.path().join("canned"));
    let clock = Arc::new(FakeClock::new(1_735_689_600_000));
    let cache = Cache::open(
        &dir.path().join("cache"),
        clock.clone(),
        Arc::new(ClockPause(clock)),
    )
    .unwrap();
    let api = OpenAlex::new(&cache, transport.as_ref(), FetchMode::Online, dir.path()).unwrap();
    let table = sjr::load_year(&cache, transport.as_ref(), FetchMode::Online, 2025).unwrap();
    let doaj_lookup = |id: &str| doaj::journal(&cache, transport.as_ref(), FetchMode::Online, id);
    let req: Requirements =
        serde_json::from_str(include_str!("fixtures/paper/journals/requirements.json")).unwrap();
    let sources = JournalSources {
        openalex: &api,
        sjr: &table,
        doaj: &doaj_lookup,
        jcr: None,
    };
    let (journals, works) = matching::gather(&req, &sources, 100).unwrap();
    let shortlist = matching::rank(&req, &journals, &works, "2025-01-01T00:00:00Z");
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/paper/journals/expected_order.json")).unwrap();
    assert_eq!(
        shortlist
            .ranked
            .iter()
            .map(|j| j.journal.id.as_str())
            .collect::<Vec<_>>(),
        expected["ranked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j["id"].as_str().unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        shortlist
            .ranked
            .iter()
            .map(|j| &j.bucket)
            .collect::<Vec<_>>(),
        [
            &Bucket::Passes,
            &Bucket::Passes,
            &Bucket::NeedsCheck,
            &Bucket::NeedsCheck
        ]
    );
    assert!(!shortlist.excluded.is_empty());
}

#[test]
fn canned_citations_and_integrity_support_runner_manuscript() {
    use axiom::paper::{
        bibliography, cache::Cache, citations, crossref, evidence, http::ClockPause, integrity,
        journals::openalex::OpenAlex,
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    let store = root.join("store");
    fs::create_dir_all(research_program::program_dir(&store)).unwrap();
    let dossier: dossier::Dossier =
        serde_json::from_str(include_str!("fixtures/paper/evidence/dossier.json")).unwrap();
    dossier::write(
        &research_program::program_dir(&store).join("02-dossier.json"),
        &dossier,
    )
    .unwrap();
    for (file, bytes) in [
        (
            "results.json",
            include_bytes!("fixtures/paper/evidence/results.json").as_slice(),
        ),
        (
            "data.csv",
            include_bytes!("fixtures/paper/evidence/data.csv").as_slice(),
        ),
        (
            "model.txt",
            include_bytes!("fixtures/paper/evidence/model.txt").as_slice(),
        ),
    ] {
        fs::write(root.join(file), bytes).unwrap();
    }
    let inputs = BundleInputs {
        store,
        project_root: root.clone(),
        results: vec![root.join("results.json")],
        datasets: vec![root.join("data.csv")],
        models: vec![root.join("model.txt")],
    };
    let bundle = evidence::build(&inputs).unwrap();
    let transport = canned(&dir.path().join("canned"));
    let clock = Arc::new(FakeClock::new(1_735_689_600_000));
    let cache = Cache::open(
        &dir.path().join("cache"),
        clock.clone(),
        Arc::new(ClockPause(clock)),
    )
    .unwrap();
    let openalex =
        OpenAlex::new(&cache, transport.as_ref(), FetchMode::Online, dir.path()).unwrap();
    let cr = |id: &str| crossref::work(&cache, transport.as_ref(), FetchMode::Online, id);
    let oa = |id: &str| openalex.work(id);
    let bib = bibliography::from_seeds(&bundle.references, &|seed| match &seed.doi {
        Some(doi) => cr(doi),
        None => match &seed.openalex {
            Some(id) => oa(id),
            None => Ok(None),
        },
    });
    let manuscript: PassOutput = serde_json::from_str(PASSES[3]).unwrap();
    let report = citations::verify_manuscript(&manuscript.manuscript, &bib, &cr, &oa);
    assert!(report.blocking.is_empty(), "{:?}", report.blocking);
    let report = integrity::check(&manuscript.manuscript, &bundle, &[], &root).unwrap();
    assert!(report.passed, "{:?}", report.issues);
}
