#![allow(dead_code)]
use axiom::{
    bridge::{call, Bridge, Config},
    orch,
    usage::price,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    pub provider: Provider,
    pub role: String,
    pub model: String,
    pub unit: Option<String>,
    pub attempt: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReworkClass {
    SelfCheckFix,
    FirstReviewMajor,
    FixUnit,
    SupersedeNoFailure,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RoleTotals {
    pub calls: u64,
    pub tokens: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Metrics {
    pub verified_units: usize,
    pub claude_calls_per_unit: Option<f64>,
    pub claude_tokens_per_unit: Option<f64>,
    pub by_model_role: BTreeMap<String, RoleTotals>,
    pub rework: BTreeMap<String, u64>,
    pub rework_rate: Option<f64>,
    pub fixture_usd: Option<f64>,
}

pub struct Workload {
    pub home: tempfile::TempDir,
    pub project: String,
    pub store: PathBuf,
    pub repo: PathBuf,
    bridge: Bridge,
    rules: Value,
}
impl Workload {
    fn core_stages(&self) {
        use axiom::{goal, session};
        let home = self.home.path();
        let id = "12345678-1234-1234-1234-123456789abd";
        let g = goal::open(home, id, "fixture core stages", self.repo.to_str()).unwrap();
        let store = orch::store_for_goal(home, &g).unwrap();
        fs::create_dir_all(store.join("astra")).unwrap();
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::write(store.join("reports/_full.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nfull\tfixture\t0\t\t9999\n").unwrap();
        let record = home.join("claude-core.json");
        let wrapper = home.join("core-fake.py");
        fs::write(&wrapper, format!("import os,runpy,sys\nos.environ['FAKE_CLAUDE_SCENARIO']='stages'\nos.environ['FAKE_CLAUDE_SET_MODEL']='supported'\nos.environ['FAKE_CLAUDE_SESSION']={id}\nos.environ['FAKE_CLAUDE_RECORD']={record}\nsys.argv=sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n", id=json!(id), record=json!(record.to_string_lossy()))).unwrap();
        let config = session::Config {home:home.into(), bin:home.join("axiom"), clock:session::Clock::system(), claude:Ok(vec![crate::common::python(),"-I".into(),wrapper.to_string_lossy().into_owned(),crate::common::repo_root().join("tests/fakes/fake_claude.py").to_string_lossy().into_owned()])};
        let spawned = session::spawn(&config,"wish","fixture core stages",&self.repo,None,None).unwrap();
        let mut n = 0;
        let code = session::drive(spawned,&config,&mut Vec::new(),&std::sync::atomic::AtomicBool::new(false),&mut |event| {
            if event["type"] != "result" {return;}
            n += 1;
            if n == 1 {
                fs::write(store.join("astra/plan-final.md"), "fixture plan").unwrap();
                fs::write(store.join("modules.md"), "fixture map").unwrap();
            } else { goal::set_status(home,id,"done",None).unwrap(); }
        }).unwrap();
        assert_eq!(code,0);
        self.record_claude_log(&record, "core", None);
    }
    fn record_claude_log(&self, path: &Path, role: &str, unit: Option<&str>) {
        for c in calls_from_log(path, role, unit).unwrap() {
            orch::events::append_event(&self.store,"phase7_call",None,unit,serde_json::to_value(c).unwrap()).unwrap();
        }
    }
    fn invoke(&self, tool: &str, input: Value) -> Value {
        call(&self.bridge, tool, &input)["structuredContent"].clone()
    }
    fn orch(&self, args: Value) -> Value {
        self.invoke("orch", json!({"project":self.project,"args":args}))
    }
    fn claude(&self, role: &str, unit: Option<&str>, id: &str, logged: bool) {
        let key = format!("{role}_model");
        let model = self.rules[&key].as_str().unwrap();
        let command = crate::common::fake("fake_claude");
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .args(["--model", model])
            .env("FAKE_CLAUDE_SCENARIO", "stages")
            .env(
                "FAKE_CLAUDE_RECORD",
                self.home.path().join(format!("claude-{id}.json")),
            )
            .env("FAKE_CLAUDE_SET_MODEL", "supported")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{}", json!({"type":"user","message":{"role":"user","content":format!("{role} {}", unit.unwrap_or(""))}})).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if logged {
            self.record_claude_log(&self.home.path().join(format!("claude-{id}.json")),role,unit);
            return;
        }
        let result: Value = String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .find(|v| v["type"] == "result")
            .unwrap();
        let usage = &result["modelUsage"][model];
        let c = Call {
            provider: Provider::Claude,
            role: role.into(),
            model: model.into(),
            unit: unit.map(str::to_owned),
            attempt: None,
            input_tokens: usage["inputTokens"].as_u64(),
            output_tokens: usage["outputTokens"].as_u64(),
            id: id.into(),
        };
        orch::events::append_event(
            &self.store,
            "phase7_call",
            None,
            unit,
            serde_json::to_value(c).unwrap(),
        )
        .unwrap();
    }
    pub fn resume(&self) {
        let r = self.orch(json!(["resume", "--repo", self.repo]));
        assert_eq!(r["status"], "completed", "{r}");
    }
    pub fn supersede_without_review(&self) {
        for args in [
            json!([
                "unit",
                "add",
                "old",
                "--track",
                "missing",
                "--brief",
                self.store.join("missing.md")
            ]),
            json!([
                "unit",
                "add",
                "replacement",
                "--track",
                "missing",
                "--brief",
                self.store.join("missing.md")
            ]),
            json!(["unit", "supersede", "old", "--by", "replacement"]),
        ] {
            let r = self.orch(args);
            assert_eq!(r["status"], "completed", "{r}");
        }
    }
    fn drive_scripted(&self, unit: &str, brief: &Path) {
        self.drive_cycle(unit,brief,false);
    }
    fn drive(&self, unit: &str, brief: &Path) {
        self.drive_cycle(unit,brief,true);
    }
    fn drive_cycle(&self, unit: &str, brief: &Path, routed: bool) {
        if !routed {
            self.claude("core", Some(unit), &format!("core-{unit}"), false);
            self.claude("dispatcher", Some(unit), &format!("dispatcher-{unit}"), false);
        }
        let current = brief.to_path_buf();
        for n in 0..=self.rules["max_fix_attempts"].as_u64().unwrap() {
            let input=json!({"project":self.project,"unit":unit,"role":"sol","cwd":self.repo,"cycle":true,"track":unit,"brief_file":current});
            let result = self.invoke("codex_run", input);
            assert!(
                result["status"] == "completed" || result["next"] == "fix",
                "{unit}: {result}"
            );
            if let Some(items) = result["review"].as_array() {
                for (i, item) in items.iter().enumerate() {
                    let reviewed = item["unit"].as_str().unwrap();
                    let reviewer = item["reviewer"].as_str().unwrap();
                    self.claude(
                        "reviewer",
                        Some(reviewed),
                        &format!("review-{unit}-{n}-{i}"),
                        routed,
                    );
                    let tree = orch::evidence(&self.store, reviewed)
                        .unwrap()
                        .into_iter()
                        .rev()
                        .find(|r| r.kind == "verify")
                        .unwrap()
                        .commit;
                    let verdict = self
                        .home
                        .path()
                        .join(format!("verdict-{reviewed}-{n}.json"));
                    fs::write(&verdict, json!({"unit":reviewed,"tree":tree,"verdict":self.rules["review_verdict"],"findings":[]}).to_string()).unwrap();
                    let r = self.orch(json!([
                        "review",
                        reviewed,
                        "--repo",
                        self.repo,
                        "--reviewer",
                        reviewer,
                        "--verdict-file",
                        verdict
                    ]));
                    assert_eq!(r["status"], "completed", "{r}");
                }
            }
            match result["next"].as_str().unwrap() {
                "fix" => {
                    assert!(
                        n < self.rules["max_fix_attempts"].as_u64().unwrap(),
                        "fix limit: {result}"
                    );
                    let mut content = fs::read_to_string(&current).unwrap();
                    content.push_str(&format!(
                        "\nCONTEXT Fix requested: {}\n",
                        result["reason"].as_str().unwrap_or("")
                    ));
                    fs::write(&current, content).unwrap();
                }
                "review" | "done" | "wait" => return,
                other => panic!("unexpected next {other}: {result}"),
            }
        }
    }
}
pub fn run_workload_scripted(seeds: &Path) -> Workload {
    workload(seeds, false)
}
pub fn run_workload(seeds: &Path) -> Workload {
    workload(seeds, true)
}
fn workload(seeds: &Path, routed: bool) -> Workload {
    let rules: Value =
        serde_json::from_slice(&fs::read(seeds.parent().unwrap().join("driver.json")).unwrap())
            .unwrap();
    let home = crate::common::temp();
    let repo = home.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("README"), "fixture\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "README"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let project = "phase7".to_string();
    let store = orch::store_for(home.path(), &project).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    let mut paths: Vec<_> = fs::read_dir(seeds)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    let mut modules = String::new();
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for p in &paths {
        let name = p.file_stem().unwrap().to_str().unwrap();
        let brief = store.join(format!("{name}.md"));
        fs::copy(p, &brief).unwrap();
        let test_path = if routed {format!(" tests/{name}.py")} else {String::new()};
        modules.push_str(&format!("## {name}\nPURPOSE fixture\nPATHS {name}.txt{test_path}\nDEPENDS none\nCONTRACT fixture\nACCEPT test `echo ok`\n"));
        if routed {
            let (tier,effort,risk)=if name=="vacuous" {("astra","high","novel")} else {("sol","xhigh","parser")};
            modules.push_str(&format!("ROUTING\n  default {tier}/{effort} {risk} seed contract\n  {name} {tier}/{effort} {risk} seed contract\n"));
            // Keep the frozen seed verbatim as context. Its legacy matrix has no
            // file::case target; supply an executable matrix in the store only.
            let seed = fs::read_to_string(&brief).unwrap();
            let seed = seed.replace(&format!("SCOPE {name}.txt"), &format!("SCOPE {name}.txt tests/{name}.py"));
            let contract = seed.lines().find(|s| s.starts_with("CONTRACT ")).unwrap();
            let command = format!("\"{}\" tests/{name}.py",crate::common::python());
            let header = format!("TIER {tier}\nEFFORT {effort}\n{contract}\n  FAKE_MATRIX_STUB tests/{name}.py {name}.txt\nTEST_MATRIX {name} | failure | tests/{name}.py::test_seed_result | checks required result | {command}\n");
            let pos = seed.find("CONTEXT").unwrap();
            let finding = if name == "vacuous" {"vacuous_check"} else {"untested"};
            let report = json!({"status":"pass","changed":true,"fixed":1,"findings":[{"kind":finding,"reference":format!("{name}.txt::seed_result"),"detail":"fixed seed check","fixed":true}]});
            fs::write(&brief,format!("{}{header}{}\nFAKE_SELFCHECK {report}\nFAKE_SELFCHECK_WRITE {name}.txt\nFAKE_REVIEW {{\"verdict\":\"pass\",\"findings\":[]}}\n",&seed[..pos],&seed[pos..])).unwrap();
        }
        units.push_str(&format!(
            "{name}\t{name}\tpending\t\t\t\t{}\n",
            brief.display()
        ));
    }
    fs::write(store.join("modules.md"), modules).unwrap();
    if routed {orch::parse::parse_map(&fs::read_to_string(store.join("modules.md")).unwrap()).unwrap();}
    fs::write(store.join("units.tsv"), units).unwrap();
    let mut config = Config::from_env();
    config.data_home = Some(home.path().into());
    let mut codex = crate::common::fake("fake_codex");
    if routed {codex.push(format!("--fake-log={}",home.path().join("codex.log").display()));}
    config.codex = Ok(codex);
    config.wsl = None;
    config.on_windows = false;
    let workload = Workload {
        home,
        project,
        store,
        repo,
        bridge: Bridge::new(config),
        rules,
    };
    // Unit role sol is admitted only after a real failing full-test receipt.
    let full = workload.orch(json!(["fulltest","--repo",workload.repo,"--command",format!("\"{}\" -c \"raise AssertionError('fixture baseline')\"",crate::common::python())]));
    assert_eq!(full["exit_code"],1,"{full}");
    for p in paths {
        let name = p.file_stem().unwrap().to_str().unwrap();
        if routed {workload.drive(name, &workload.store.join(format!("{name}.md")));}
        else {workload.drive_scripted(name, &workload.store.join(format!("{name}.md")));}
    }
    if routed {workload.core_stages(); verify_claude_log(&workload).unwrap();}
    workload
}

fn calls_from_log(path: &Path, role: &str, unit: Option<&str>) -> Result<Vec<Call>, String> {
    let log: Value = serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut calls = Vec::new();
    for r in log["results"].as_array().ok_or("missing result log")? {
        for (model, usage) in r["modelUsage"].as_object().ok_or("missing model usage")? {
            calls.push(Call {provider:Provider::Claude, role:role.into(), model:model.clone(), unit:unit.map(str::to_owned), attempt:None,
                input_tokens:usage["inputTokens"].as_u64(), output_tokens:usage["outputTokens"].as_u64(), id:r["uuid"].as_str().ok_or("missing result id")?.into()});
        }
    }
    Ok(calls)
}
pub fn verify_claude_log(w: &Workload) -> Result<(), String> {
    let mut logged = BTreeMap::new();
    for entry in fs::read_dir(w.home.path()).map_err(|e|e.to_string())? {
        let path = entry.map_err(|e|e.to_string())?.path();
        let name = path.file_name().and_then(|n|n.to_str()).unwrap_or_default();
        if !name.starts_with("claude-") || !name.ends_with(".json") {continue;}
        let role = if name == "claude-core.json" {"core"} else {"reviewer"};
        for c in calls_from_log(&path,role,None)? { logged.insert(c.id.clone(),c); }
    }
    let mut recorded = BTreeMap::new();
    for e in orch::events::read_events(&w.store,0).map_err(|e|e.to_string())? {
        if e.kind != "phase7_call" {continue;}
        let c: Call=serde_json::from_value(e.data).map_err(|e|e.to_string())?;
        if c.provider == Provider::Claude { recorded.entry(c.id.clone()).or_insert(c); }
    }
    if recorded.len() != logged.len() {return Err("Claude result count mismatch".into());}
    for (id,c) in recorded {
        let actual=logged.get(&id).ok_or_else(||format!("unlogged Claude result {id}"))?;
        if (c.model,c.role,c.input_tokens,c.output_tokens) != (actual.model.clone(),actual.role.clone(),actual.input_tokens,actual.output_tokens) {return Err(format!("Claude usage mismatch {id}"));}
    }
    Ok(())
}
pub fn collect(home: &Path, project: &str) -> Metrics {
    let store = orch::store_for(home, project).unwrap();
    let verified_units = orch::receipts::compute(&store, 1).unwrap().verified_units;
    let events = orch::events::read_events(&store, 0).unwrap();
    let mut calls = BTreeMap::<String, Call>::new();
    for event in &events {
        let c = if event.kind == "phase7_call" {
            serde_json::from_value::<Call>(event.data.clone()).ok()
        } else if event.kind == axiom::engine::kinds::ROLE_SPEND {
            let d = &event.data;
            let model = d["model"].as_str().unwrap_or("");
            let provider =
                if model.starts_with("claude-") || matches!(model, "sonnet" | "opus" | "haiku") {
                    Provider::Claude
                } else {
                    Provider::Codex
                };
            Some(Call {
                provider,
                role: d["role"].as_str().unwrap_or("").into(),
                model: model.into(),
                unit: event.unit.clone(),
                attempt: None,
                input_tokens: if d["usd"].is_null()
                    && d["input_tokens"] == 0
                    && d["output_tokens"] == 0
                {
                    None
                } else {
                    d["input_tokens"].as_u64()
                },
                output_tokens: if d["usd"].is_null()
                    && d["input_tokens"] == 0
                    && d["output_tokens"] == 0
                {
                    None
                } else {
                    d["output_tokens"].as_u64()
                },
                id: d["id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("ledger-{}", event.seq)),
            })
        } else {
            None
        };
        if let Some(c) = c {
            calls.entry(c.id.clone()).or_insert(c);
        }
    }
    let mut by_model_role = BTreeMap::<String, RoleTotals>::new();
    let mut claude_calls = 0u64;
    let mut claude_tokens = Some(0u64);
    let mut usd = Some(0.0);
    for c in calls.values() {
        let tokens = c
            .input_tokens
            .zip(c.output_tokens)
            .and_then(|(a, b)| a.checked_add(b));
        let total = by_model_role
            .entry(format!("{}|{}", c.model, c.role))
            .or_insert(RoleTotals {
                calls: 0,
                tokens: Some(0),
            });
        total.calls += 1;
        total.tokens = total.tokens.zip(tokens).and_then(|(a, b)| a.checked_add(b));
        if c.provider == Provider::Claude {
            claude_calls += 1;
            claude_tokens = claude_tokens
                .zip(tokens)
                .and_then(|(a, b)| a.checked_add(b));
        } else if tokens.is_none() {
            claude_tokens = None;
        }
        let cost = c
            .input_tokens
            .zip(c.output_tokens)
            .and_then(|(input, output)| match c.provider {
                Provider::Claude => price::usd(
                    &c.model,
                    &price::CallUsage {
                        input,
                        output,
                        ..Default::default()
                    },
                ),
                // The budget ledger omits cached-input tokens; pricing them as zero
                // would invent a cost rather than report unknown.
                Provider::Codex => None,
            });
        usd = usd.zip(cost).map(|(a, b)| a + b);
    }
    let mut rework = BTreeMap::new();
    let units = orch::read_units(&store).unwrap();
    let mut affected = BTreeSet::new();
    for u in &units {
        let rows = orch::evidence(&store, &u.id).unwrap();
        if u.superseded_by().is_some()
            && !rows.iter().any(|r| r.kind == "review" && r.exit_code != 0)
        {
            *rework.entry("supersede_no_failure".into()).or_insert(0) += 1;
            affected.insert(u.id.clone());
        }
        if rows.iter().any(|r| r.kind == orch::FIXES_KIND) {
            *rework.entry("fix_unit".into()).or_insert(0) += 1;
            affected.insert(u.id.clone());
        }
        if events
            .iter()
            .find(|e| {
                e.kind == axiom::engine::kinds::REVIEW_RECORDED && e.unit.as_deref() == Some(&u.id)
            })
            .is_some_and(|e| {
                e.data["findings"].as_array().is_some_and(|fs| {
                    fs.iter()
                        .any(|f| f["severity"] == "major" || f["severity"] == "blocker")
                })
            })
        {
            *rework.entry("first_review_major".into()).or_insert(0) += 1;
            affected.insert(u.id.clone());
        }
    }
    let denominator = verified_units as f64;
    Metrics {
        verified_units,
        claude_calls_per_unit: (verified_units > 0).then_some(claude_calls as f64 / denominator),
        claude_tokens_per_unit: if verified_units > 0 {
            claude_tokens.map(|v| v as f64 / denominator)
        } else {
            None
        },
        by_model_role,
        rework,
        rework_rate: (verified_units > 0).then_some(affected.len() as f64 / denominator),
        fixture_usd: usd,
    }
}
pub fn passes(before: &Metrics, after: &Metrics) -> Result<(), String> {
    match (
        before.claude_calls_per_unit,
        after.claude_calls_per_unit,
        before.claude_tokens_per_unit,
        after.claude_tokens_per_unit,
        before.rework_rate,
        after.rework_rate,
    ) {
        (Some(bc), Some(ac), Some(bt), Some(at), Some(br), Some(ar))
            if ac < bc && at < bt && ar <= br =>
        {
            Ok(())
        }
        _ => Err("missing metrics or improvement requirements not met".into()),
    }
}
pub fn fixture_cost_label(m: &Metrics) -> String {
    format!("fixture-only USD: {:?}", m.fixture_usd)
}
