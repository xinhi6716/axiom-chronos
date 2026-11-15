use axiom::usage::review_stats::{collect, command, render_text, to_json};
use axiom::usage::review_stats::{collect_first_pass, astra_summary, to_json_with_first_pass, render_text_with_first_pass};
#[path = "common/mod.rs"]
mod common;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn store(home: &Path, wish: &str) -> PathBuf {
    let path = home.join("orchestrate").join("project").join(wish);
    fs::create_dir_all(&path).unwrap();
    path
}
fn event(seq: u64, kind: &str, data: Value) -> Value {
    json!({"seq":seq,"ts":"2026-01-01T00:00:00Z","ts_ms":1767225600000i64 + seq as i64 * 1000,
        "kind":kind,"wish":"ignored","module":null,"unit":null,"data":data})
}
fn review(
    seq: u64,
    id: &str,
    role: &str,
    reviewer: &str,
    model: &str,
    unit: &str,
    paths: &[&str],
    pass: bool,
    findings: usize,
) -> Value {
    event(
        seq,
        "review_recorded",
        json!({"review_id":id,"unit":unit,"role":role,"reviewer":reviewer,"model":model,"paths":paths,"pass":pass,"findings":vec![json!({"number":1}); findings]}),
    )
}
fn verify(
    seq: u64,
    id: &str,
    finding: u64,
    path: &str,
    severity: &str,
    status: &str,
    failed: bool,
) -> Value {
    event(
        seq,
        "finding_verified",
        json!({"review_id":id,"finding":finding,"location":path,"severity":severity,"status":status,"failed":failed,"reviewer":"orphan"}),
    )
}
fn write_events(store: &Path, events: &[Value]) {
    let text = events.iter().map(|e| format!("{e}\n")).collect::<String>();
    fs::write(store.join("events.jsonl"), text).unwrap();
}

fn units(store: &Path, rows: &[(&str, &str)]) {
    fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n{}",
        rows.iter().map(|(id,state)| format!("{id}\tm\t{state}\t\t\t\tmissing\n")).collect::<String>())).unwrap();
}
fn worker(seq: u64, id: &str, unit: &str, model: &str, effort: &str, risk: &str, pass: bool) -> Value {
    let mut e = review(seq, id, "reviewer", "r", "review-model", unit, &[], pass, 0);
    e["data"]["worker_model"] = json!(model);
    e["data"]["worker_effort"] = json!(effort);
    e["data"]["risk"] = json!(risk);
    e
}
fn sample(home: &Path, project: &str, wish: &str, model: &str) -> PathBuf {
    let s = axiom::orch::wish_store(home, project, Some(wish)).unwrap();
    fs::create_dir_all(&s).unwrap();
    units(&s, &[("u", "verified")]);
    write_events(&s, &[worker(1,"r","u",model,"xhigh","parser",true)]);
    s
}
fn root_sample(home: &Path, project: &str, model: &str) -> PathBuf {
    let s = axiom::orch::wish_store(home, project, None).unwrap();
    fs::create_dir_all(&s).unwrap();
    units(&s, &[("u", "verified")]);
    write_events(&s, &[worker(1,"r","u",model,"xhigh","parser",true)]);
    s
}
#[test]
fn root_layout_store_is_summarised() {
    let h = common::temp();
    root_sample(h.path(), "project", "root-model");
    let text = astra_summary(h.path(), "project");
    assert!(text.contains("model=root-model"), "{text}");
    assert!(text.contains("n=1 rate=n/a (n<3) verified=1"), "{text}");
}
#[test]
fn wish_subdir_store_still_summarised() {
    let h = common::temp();
    sample(h.path(), "project", "w-1234567890123-abcdef", "wish-model");
    let text = astra_summary(h.path(), "project");
    assert!(text.starts_with("FIRST-PASS STATS (project project, cutoff 2026-01-01T00:00:00Z)\n"));
    assert_eq!(text.lines().filter(|l| l.starts_with("model=")).count(), 1);
    assert!(text.contains("model=wish-model effort=xhigh risk=parser n=1 rate=n/a (n<3) verified=1"));
}
#[test]
fn root_and_wish_layouts_combine() {
    let h = common::temp();
    let root = root_sample(h.path(), "project", "shared-model");
    let wish = sample(h.path(), "project", "w-1234567890123-abcdef", "shared-model");
    for (s, model) in [(&root, "root-only"), (&wish, "wish-only")] {
        units(s, &[("u", "verified"), ("v", "verified")]);
        write_events(s, &[worker(1,"r","u","shared-model","xhigh","parser",true),
            worker(2,"s","v",model,"xhigh","parser",true)]);
    }
    let text = astra_summary(h.path(), "project");
    assert!(text.contains("model=root-only"), "{text}");
    assert!(text.contains("model=wish-only"), "{text}");
    let shared = text.lines().find(|l| l.starts_with("model=shared-model ")).unwrap();
    assert!(shared.contains("n=2 rate=n/a (n<3) verified=2"), "{shared}");
    assert_eq!(text.lines().filter(|l| l.starts_with("model=")).count(), 3);
}
#[test]
fn root_layout_foreign_home_excluded() {
    let a = common::temp(); let b = common::temp();
    sample(a.path(), "project", "w-1234567890123-abcdef", "local-wish");
    root_sample(a.path(), "project", "local-root");
    root_sample(a.path(), "foreign-project", "foreign-project-root");
    root_sample(b.path(), "project", "foreign-home-root");
    let text = astra_summary(a.path(), "project");
    assert!(text.contains("model=local-wish"));
    assert!(!text.contains("foreign-project-root"), "{text}");
    assert!(!text.contains("foreign-home-root"), "{text}");
    let foreign = astra_summary(b.path(), "project");
    assert!(!foreign.contains("local-root"));
    assert!(!foreign.contains("local-wish"));
    assert!(!foreign.contains("foreign-project-root"));
    assert_eq!(astra_summary(a.path(), "../foreign-project"), "");
}
#[test]
fn empty_home_gives_empty_summary() {
    let h = common::temp();
    assert_eq!(astra_summary(&h.path().join("missing-home"), "project"), "");
    assert_eq!(astra_summary(h.path(), "project"), "");
    fs::create_dir_all(h.path().join("orchestrate")).unwrap();
    assert_eq!(astra_summary(h.path(), "project"), "");
    fs::create_dir_all(h.path().join("orchestrate/project")).unwrap();
    assert_eq!(astra_summary(h.path(), "project"), "");
}
#[test]
fn root_layout_summary_bounded() {
    let h = common::temp();
    let root = root_sample(h.path(), "project", "unused");
    let events: Vec<_> = (0..60).map(|k| {
        let id = format!("u{k}");
        worker(k + 1, &id, &id, &format!("m{k:02}"), "xhigh", "parser", true)
    }).collect();
    write_events(&root, &events);
    let text = astra_summary(h.path(), "project");
    assert_eq!(text.lines().next(), Some("FIRST-PASS STATS (project project, cutoff 2026-01-01T00:00:00Z)"));
    let shown = text.lines().filter(|l| l.starts_with("model=")).count();
    assert!(shown > 0 && shown <= 40);
    assert!(text.len() <= 4000);
    assert!(text.contains(&format!("TRUNCATED: showing {shown} of 60 rows")), "{text}");
}
#[test]
fn first_failure_then_pass() {
    let h = common::temp(); let s = store(h.path(), "w-1234567890123-abcdef");
    units(&s, &[("u1","verified"),("u2","verified")]);
    let mut first = worker(1,"r1","u1","sol","xhigh","parser",false);
    first["data"]["findings"] = json!([{"severity":"major"}]);
    let mut events = vec![worker(2,"r2","u1","later","high","standard",true), first];
    write_events(&s,&events);
    let r = collect_first_pass(&[s.clone()]).unwrap();
    assert_eq!(r.len(),1);
    assert_eq!((r[0].units,r[0].first_pass.num,r[0].first_pass.den,r[0].verified),(1,0,1,1));
    events.push(worker(3,"r3","u2","sol","xhigh","parser",true)); write_events(&s,&events);
    let r = collect_first_pass(&[s.clone()]).unwrap();
    assert_eq!((r[0].first_pass.num,r[0].first_pass.den),(1,2));
    events[0]["data"]["findings"] = json!([{"severity":"blocker"}]);
    events[1]["data"]["pass"] = json!(true); write_events(&s,&events);
    assert_eq!(collect_first_pass(&[s.clone()]).unwrap()[0].first_pass.num,1);
    units(&s,&[("u1","pending"),("u2","pending")]);
    assert_eq!(collect_first_pass(&[s]).unwrap()[0].verified,0);
}
#[test]
fn rows_keyed_by_worker_model_effort_risk() {
    let h = common::temp(); let s = store(h.path(), "w-1234567890123-abcdef");
    write_events(&s,&[worker(1,"a","a","sol","xhigh","parser",true),worker(2,"b","b","sol","high","standard",true), review(3,"c","r","r","m","c",&[],true,0)]);
    let r=collect_first_pass(&[s]).unwrap();
    assert_eq!(r.iter().map(|r| (r.worker_model.as_str(),r.worker_effort.as_str(),r.risk.as_str())).collect::<Vec<_>>(),vec![("","",""),("sol","high","standard"),("sol","xhigh","parser")]);
    let text=render_text_with_first_pass(&[],&r);
    assert!(text.lines().any(|l| l.starts_with("worker_model")));
    assert!(text.contains("-\t-\t-")); assert!(!text.contains("unknown"));
    assert!(!render_text(&[]).contains("worker_model"));
}
#[test]
fn overruled_selfcheck_unfinished_supersede_counted_apart() {
    let h=common::temp(); let s=sample(h.path(),"project","w-1234567890123-abcdef","sol");
    units(&s,&[("u","verified"),("pending","pending"),("old","superseded:u")]);
    fs::create_dir_all(s.join("reports")).unwrap(); fs::write(s.join("reports/u.selfcheck.json"),r#"{"fixed":true}"#).unwrap();
    write_events(&s,&[worker(1,"r","u","sol","xhigh","parser",true),verify(2,"r",1,"x","major","rejected",false),verify(3,"orphan",1,"x","major","rejected",false),verify(4,"r",2,"x","major","rejected",true),verify(5,"r",3,"x","major","confirmed",false)]);
    let r=collect_first_pass(&[s.clone()]).unwrap();
    assert_eq!(r.len(),2);
    assert_eq!((r[1].units,r[1].first_pass.num,r[1].self_check_fixes,r[1].overruled),(1,1,1,1));
    assert_eq!((r[0].units,r[0].unfinished,r[0].unreviewed_supersedes),(0,1,1));
    fs::write(s.join("reports/u.selfcheck.json"),r#"{"fixed":false}"#).unwrap();
    assert_eq!(collect_first_pass(&[s.clone()]).unwrap()[1].self_check_fixes,0);
    let receipt=axiom::orch::receipts::RoutingReceipt { unit:"pending".into(), attempt:"1".into(), tier:None, effort:Some("high".into()),risk:Some("standard".into()), model_assigned:"old".into(),model_used:None,rule:"test".into(),reason:"test".into(),settings_version:None };
    axiom::orch::receipts::record_routing(&s,&receipt).unwrap();
    let mut latest=receipt; latest.model_used=Some("latest".into()); axiom::orch::receipts::record_routing(&s,&latest).unwrap();
    let r=collect_first_pass(&[s]).unwrap(); assert_eq!(r[0].unfinished,0); assert_eq!(r[1].worker_model,"latest"); assert_eq!(r[1].unfinished,1);
}
#[test]
fn resume_duplicates_count_once() {
    let h=common::temp(); let s=sample(h.path(),"project","w-1234567890123-abcdef","sol");
    let a=worker(1,"r","u","sol","xhigh","parser",true); let b=verify(2,"r",1,"x","major","rejected",false);
    write_events(&s,&[a.clone(),b.clone(),a,b]);
    let r=collect_first_pass(&[s.clone(),s]).unwrap(); assert_eq!(r.len(),1); assert_eq!((r[0].units,r[0].overruled),(1,1));
}
#[test]
fn cross_wish_and_cross_home_isolation() {
    let a=common::temp(); let b=common::temp();
    for w in ["w-1234567890123-abcdef","w-1234567890124-abcdef"] { sample(a.path(),"p",w,"local"); }
    sample(a.path(),"q","w-1234567890123-abcdef","other-project"); sample(b.path(),"p","w-1234567890123-abcdef","other-home");
    let s=astra_summary(a.path(),"p"); assert!(s.contains("model=local")); assert!(s.contains("n=2"));
    assert!(!s.contains("other-project")); assert!(!s.contains("other-home"));
    assert!(astra_summary(a.path(),"q").contains("other-project")); assert!(astra_summary(b.path(),"p").contains("other-home"));
    assert!(astra_summary(a.path(),"../q").is_empty());
}
#[test]
fn zero_denominator_is_null() {
    let h=common::temp(); let s=store(h.path(),"w-1234567890123-abcdef"); units(&s,&[("u","pending")]);
    let r=collect_first_pass(&[s.clone()]).unwrap(); assert_eq!(r.len(),1); assert_eq!(r[0].first_pass.rate,None);
    assert_eq!(to_json_with_first_pass(&[],&r)["first_pass"][0]["first_pass"]["rate"],Value::Null);
    assert_eq!(astra_summary(h.path(),"missing"),""); assert!(!astra_summary(h.path(),"project").is_empty());
    assert!(to_json(&[])["first_pass"].as_array().unwrap().is_empty());
}
#[test]
fn summary_is_bounded() {
    let h=common::temp(); let s=store(h.path(),"w-1234567890123-abcdef"); let mut events=vec![];
    for k in 0..60 { for n in 0..if k==0 {3} else {2} { let id=format!("u{k}-{n}"); events.push(worker(events.len() as u64+1,&id,&id,&format!("m{k:02}"),"xhigh","parser",true)); } }
    write_events(&s,&events); let text=astra_summary(h.path(),"project");
    assert!(text.starts_with("FIRST-PASS STATS (project project, cutoff 2026-01-01T00:00:00Z)"));
    assert!(text.len()<=4000); assert!(text.lines().filter(|l|l.starts_with("model=")).count()<=40); assert!(text.contains("TRUNCATED: showing")); assert!(text.contains("of 60 rows")); assert!(text.contains("first_pass=0/3 (0.0%)")); assert!(text.contains("rate=n/a (n<3)"));
    let cutoff="2026-01-02T00:00:00.000Z";
    events[0]["ts"]=json!(cutoff); events[0]["ts_ms"]=json!(1767312000000i64);
    let mut unrelated=event(1000,"heartbeat",json!({})); unrelated["ts"]=json!("2026-01-03T00:00:00.000Z"); unrelated["ts_ms"]=json!(1767398400000i64);
    let mut duplicate=events[0].clone(); duplicate["seq"]=json!(1001); duplicate["ts"]=unrelated["ts"].clone(); duplicate["ts_ms"]=unrelated["ts_ms"].clone();
    let mut with_noise=events.clone(); with_noise.extend([unrelated,duplicate]); write_events(&s,&with_noise);
    let text=astra_summary(h.path(),"project"); assert!(text.lines().next().unwrap().contains(cutoff));
    for e in &mut events { e["data"]["worker_model"]=json!(format!("{}{}",e["data"]["worker_model"].as_str().unwrap(),"模型".repeat(1000))); }
    write_events(&s,&events); let text=astra_summary(h.path(),"project"); assert!(text.len()<=4000); assert!(text.contains("TRUNCATED"));
    write_events(&s,&events[..1]); let text=astra_summary(h.path(),"project"); assert!(text.contains("TRUNCATED"));
    write_events(&s,&[worker(1,"r","u","模型","high","standard",true)]); let text=astra_summary(h.path(),"project"); assert!(text.contains("model=模型")); assert!(!text.contains("TRUNCATED"));
}

fn bridge_prompt(home: &Path, data: bool, mode: &str) -> String {
    use axiom::bridge::{Bridge,Config,call};
    let repo=home.join("repo"); fs::create_dir_all(repo.join("src")).unwrap();
    let git=|args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
    git(&["init","-q"]); fs::write(repo.join("src/mod.py"),"fixture").unwrap(); git(&["add","."]); git(&["-c","user.name=Test","-c","user.email=t@e","commit","-qm","fixture"]);
    let base=axiom::orch::store_for(home,"project").unwrap(); fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(base.join("inbox")).unwrap();
    if data { sample(home,"project","w-1234567890123-abcdef","sol"); }
    let log=home.join("fake.log"); let mut fake=common::fake("fake_codex"); fake.push(format!("--fake-log={}",log.display()));
    let b=Bridge::new(Config {data_home:Some(home.into()),codex:Ok(fake),wsl:None,on_windows:false});
    let mut args=json!({"project":"project","cwd":repo,"role":"astra","sandbox":"read-only","brief":"ORIGINAL BRIEF","output_schema":{"type":"object"}});
    if mode=="save" {args["save_to"]=json!(base.join("astra/plan.json"));}
    if mode=="terra" {
        let brief=base.join("build.brief"); fs::write(&brief,"GOAL build\nRISK trivial\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        fs::write(base.join("units.tsv"),format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nbuild\tm\tpending\t\t\t\t{}\n",brief.display())).unwrap();
        args=json!({"project":"project","cwd":repo,"role":"terra","unit":"build"});
    }
    let result=call(&b,"codex_run",&args); assert!(result["structuredContent"]["error"].is_null(),"{result}");
    fs::read_to_string(log).unwrap().lines().map(|l|serde_json::from_str::<Value>(l).unwrap()).find(|v|v["method"]=="turn/start").unwrap()["params"]["input"][0]["text"].as_str().unwrap().to_owned()
}
#[test]
fn astra_brief_gets_summary() {
    let h=common::temp(); let p=bridge_prompt(h.path(),true,"save"); assert!(p.contains("FIRST-PASS STATS")); assert!(p.find("FIRST-PASS STATS").unwrap()<p.find("ORIGINAL BRIEF").unwrap());
}
#[test]
fn other_calls_unchanged() {
    for (data,mode) in [(false,"save"),(true,"read"),(true,"terra")] {
        let h=common::temp(); let p=bridge_prompt(h.path(),data,mode); assert!(!p.contains("FIRST-PASS STATS"));
        if mode!="terra" {assert_eq!(p,"ORIGINAL BRIEF");} else {assert_eq!(p,"GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");}
    }
    let h=common::temp(); assert!(bridge_prompt(h.path(),true,"save").contains("FIRST-PASS STATS"));
}
fn fix(store: &Path, scope: &str, timestamp: &str) {
    let brief = store.join("fix.md");
    fs::write(&brief, format!("GOAL fix\nSCOPE {scope}\nVERIFY test\n")).unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nfix\tcore\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(store.join("reports/fix.evidence.tsv"), format!("kind\tcommand\texit_code\tcommit\tts\nfixes\toriginal\t0\t\t{timestamp}\nfixes\toriginal\t0\t\t{timestamp}\n")).unwrap();
}

#[test]
fn aggregation_dedup_isolation_and_rendering() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    let b = store(home.path(), "w-1234567890124-abcdef");
    let events = vec![
        review(
            1,
            "r1",
            "reviewer",
            "alice",
            "m1",
            "original",
            &["src/"],
            true,
            2,
        ),
        review(
            2,
            "r2",
            "pm-qa",
            "bob",
            "",
            "other",
            &["src/lib.rs"],
            true,
            0,
        ),
        verify(3, "r3", 1, "src\\lib.rs:12-15", "MAJOR", "confirmed", true),
        review(
            4,
            "r3",
            "reviewer",
            "alice",
            "m1",
            "third",
            &["none/"],
            false,
            1,
        ),
        verify(
            5,
            "r3",
            2,
            "no/coverage.rs:1",
            "blocker",
            "confirmed",
            false,
        ),
        verify(
            6,
            "r3",
            3,
            "later/file.rs:9",
            "major",
            "inconclusive",
            false,
        ),
        review(
            7,
            "later",
            "astra-max",
            "zoe",
            "z",
            "later",
            &["later/"],
            true,
            0,
        ),
        verify(8, "r3", 4, "outside", "minor", "rejected", false),
        verify(9, "r3", 5, "outside", "minor", "rejected", true),
        verify(10, "r3", 6, "outside", "minor", "confirmed", false),
        verify(11, "r3", 6, "outside", "minor", "rejected", false),
        verify(12, "missing", 1, "outside", "minor", "confirmed", false),
    ];
    write_events(&a, &events);
    fix(&a, "src/lib.rs", "2026-01-01T00:00:20.000Z");
    write_events(
        &b,
        &[verify(
            20,
            "elsewhere",
            1,
            "src/lib.rs:1",
            "major",
            "confirmed",
            false,
        )],
    );
    let rows = collect(&[a.clone(), b.clone()]).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (&*r.role, &*r.reviewer, &*r.model))
            .collect::<Vec<_>>(),
        vec![
            ("-", "orphan", ""),
            ("astra-max", "zoe", "z"),
            ("pm-qa", "bob", ""),
            ("reviewer", "alice", "m1")
        ]
    );
    let alice = &rows[3];
    assert_eq!(
        (
            alice.reviews,
            alice.findings,
            alice.finding_rate.num,
            alice.finding_rate.den
        ),
        (2, 3, 3, 2)
    );
    assert_eq!(
        (
            alice.rejected,
            alice.rejection_rate.num,
            alice.rejection_rate.den
        ),
        (2, 2, 4)
    );
    assert_eq!(
        (alice.escapes, alice.escape_rate.num, alice.escape_rate.den),
        (0, 0, 1)
    );
    let bob = &rows[2];
    assert_eq!(
        (bob.escapes, bob.escape_rate.num, bob.escape_rate.den),
        (2, 2, 1)
    );
    assert_eq!((rows[1].escapes, rows[0].escapes), (0, 0));
    assert_eq!(
        (
            rows[0].rejection_rate.num,
            rows[0].rejection_rate.den,
            rows[0].escape_rate.rate
        ),
        (0, 2, None)
    );
    let text = render_text(&rows);
    assert!(!text.to_lowercase().contains("unknown"));
    let json = to_json(&rows);
    for (row, line) in json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .zip(text.lines().skip(1))
    {
        for field in ["finding_rate", "rejection_rate", "escape_rate"] {
            let rate = &row[field];
            assert!(line.contains(&format!("{}/{}", rate["num"], rate["den"])));
        }
    }
    let orphan_line = text.lines().find(|l| l.contains("orphan")).unwrap();
    assert!(!orphan_line.contains("0/0 ("));
    let mut repeated = events.clone();
    repeated.extend(events);
    write_events(&a, &repeated);
    assert_eq!(rows, collect(&[a.clone(), b, a]).unwrap());
    assert_eq!(command(home.path(), &["--json".into()]), 0);
    assert_eq!(
        command(
            home.path(),
            &["--wish".into(), "w-1234567890123-abcdef".into()]
        ),
        2
    );
}

#[test]
fn negative_controls_and_empty_output() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    write_events(
        &a,
        &[
            review(
                1,
                "first",
                "reviewer",
                "a",
                "m",
                "one",
                &["other/"],
                true,
                0,
            ),
            verify(
                2,
                "source",
                1,
                "uncovered/file:1",
                "major",
                "confirmed",
                false,
            ),
            verify(
                3,
                "source",
                2,
                "later/file:1",
                "blocker",
                "inconclusive",
                false,
            ),
            review(4, "last", "reviewer", "b", "m", "two", &["later/"], true, 0),
        ],
    );
    let rows = collect(&[a]).unwrap();
    assert!(rows.iter().all(|r| r.escapes == 0));
    assert!(render_text(&[]).to_lowercase().contains("no review events"));
}

#[test]
fn fix_time_and_self_review_and_same_review_finding_do_not_count() {
    let home = tempfile::tempdir().unwrap();
    let a = store(home.path(), "w-1234567890123-abcdef");
    write_events(
        &a,
        &[
            review(
                1,
                "self",
                "reviewer",
                "self",
                "m",
                "fix",
                &["src/"],
                true,
                0,
            ),
            verify(2, "self", 1, "src/file.rs:3", "major", "confirmed", false),
            review(
                3,
                "too-late",
                "reviewer",
                "late",
                "",
                "other",
                &["src/"],
                true,
                0,
            ),
        ],
    );
    fix(&a, "src/file.rs", "2026-01-01T00:00:02.000Z");
    let rows = collect(&[a]).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.escapes == 0));
    assert_eq!(
        command(
            home.path(),
            &[
                "--project".into(),
                "project".into(),
                "--wish".into(),
                "w-1234567890123-abcdef".into(),
                "--json".into()
            ]
        ),
        0
    );
    assert_eq!(command(home.path(), &["--project".into()]), 2);
    assert_eq!(command(home.path(), &["--bad".into()]), 2);
}
