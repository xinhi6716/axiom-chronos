mod common;

use axiom::{bridge::{call, Bridge, Config}, goal, hook, orch};
use serde_json::{json, Value};
use std::{collections::{BTreeMap, HashMap}, fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}, time::Duration};

const ORCH: &str = "mcp__plugin_chronos_bridge__orch";
const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
const MEMORY: &str = "mcp__plugin_chronos_bridge__memory_search";
const UNITS: [&str; 6] = ["a1", "a2", "b1", "b2", "c1", "c2"];

struct Fixture { temp: tempfile::TempDir, repo: PathBuf, store: PathBuf, project: String, bridge: Bridge }
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let home = temp.path();
        let repo = home.join("repo");
        fs::create_dir_all(&repo).unwrap();
        for m in ["a", "b", "c"] { fs::create_dir_all(repo.join(m)).unwrap(); fs::write(repo.join(m).join(".gitkeep"), "").unwrap(); }
        fs::write(repo.join("check.py"), "import pathlib, sys\nfiles = sys.argv[1:]\nif files == ['all']:\n    files = [f'{m}/{n}.txt' for m in 'abc' for n in ('one', 'two')]\nsys.exit(0 if files and all(pathlib.Path(f).is_file() and 'fake-written' in pathlib.Path(f).read_text(encoding='utf-8') for f in files) else 1)\n").unwrap();
        for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "fixture"]] {
            assert!(Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
        }
        let g = goal::open(home, "session", "Build six files", Some(repo.to_str().unwrap())).unwrap();
        goal::set_mode(home, "session", "change").unwrap();
        let project = g.project.unwrap();
        let store = orch::store_for(home, &project).unwrap();
        fs::create_dir_all(store.join("astra")).unwrap();
        orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
        for file in ["plan.json", "review.json", "ruling.md"] { fs::write(store.join("astra").join(file), "ok").unwrap(); }
        fs::write(store.join("astra/plan-final.md"), "### A Alpha\nmodule a\n### B Beta\nmodule b\n### C Gamma\nmodule c\n").unwrap();
        fs::write(store.join("decisions.md"), "# Decisions\n- Build the six files.\n").unwrap();
        let mut modules = String::new();
        for (m, dep, serves) in [("a", "none", "A"), ("b", "a", "B"), ("c", "none", "C")] {
            modules.push_str(&format!("## {m}\nPURPOSE build {m}\nPATHS {m}/\nDEPENDS {dep}\nCONTRACT write the files\nACCEPT files `python -I check.py {m}/one.txt {m}/two.txt`\nSERVES {serves}\n"));
        }
        fs::write(store.join("modules.md"), modules).unwrap();
        let mut config = Config::from_env(); config.data_home = Some(home.into()); config.codex = Ok(common::fake("fake_codex")); config.wsl = None; config.on_windows = false;
        let f = Self { temp, repo, store, project, bridge: Bridge::new(config) };
        let init = f.orch(json!(["init"]));
        assert_eq!(init["exit_code"], 0, "init: {init}");
        let result = f.orch(json!(["map", "--repo", f.repo]));
        assert_eq!(result["exit_code"], 0, "map: {result}");
        f
    }
    fn orch(&self, args: Value) -> Value { call(&self.bridge, "orch", &json!({"project":self.project,"args":args}))["structuredContent"].clone() }
    fn brief(&self, unit: &str) -> (PathBuf, String) {
        let module = &unit[..1];
        let file = format!("{module}/{}.txt", if unit.ends_with('1') { "one" } else { "two" });
        let text = format!("GOAL Build {file}\nSCOPE {file}\nVERIFY python -I check.py {file}\nCONTEXT contract: write the file\nFAKE_DELAY_MS 500\nFAKE_WRITE {file}\n");
        let parsed = orch::parse_brief(&text).unwrap();
        assert_eq!(parsed.verify, format!("python -I check.py {file}"));
        assert!(parsed.text.contains(&format!("FAKE_WRITE {file}")));
        (self.store.join("briefs").join(format!("{unit}.md")), text)
    }
}

fn add(script: &mut Vec<Value>, agent: &str, name: &str, input: Value) { script.push(json!({"agent":agent,"name":name,"input":input})); }
fn review(script: &mut Vec<Value>, f: &Fixture, module: &str, unit: &str) {
    let agent = format!("d-{module}"); let reviewer = format!("rv-{unit}"); let pack = orch::pack_path(&f.store, unit);
    script.push(json!({"agent":agent,"name":"Agent","id":reviewer,"input":{"subagent_type":"chronos:reviewer","prompt":format!("Review unit {unit} in {} using {}", f.repo.display(), pack.display())}}));
    add(script, &reviewer, "Read", json!({"file_path":pack}));
    add(script, &reviewer, "Write", json!({"file_path":orch::verdict_path(&f.store, unit),"content":json!({"unit":unit,"tree":"@TREE@","verdict":"pass","findings":[]}).to_string()}));
}
fn script(f: &Fixture, after: bool) -> Vec<Value> {
    let mut s = Vec::new();
    for m in ["a", "b", "c"] {
        let agent = format!("d-{m}"); let units = [format!("{m}1"), format!("{m}2")];
        if after {
            add(&mut s, &agent, ORCH, json!({"args":["module-context",m,"--repo",f.repo]}));
            let mut files = serde_json::Map::new(); let mut command = String::new();
            for u in &units { let (p, text) = f.brief(u); command.push_str(&format!("cat > '{}' <<'BRIEF'\n{text}BRIEF\n", p.display())); files.insert(p.to_string_lossy().into_owned(), json!(text)); }
            add(&mut s, &agent, "Bash", json!({"command":command,"files":files}));
        } else {
            for p in [f.store.join("astra/plan-final.md"), f.store.join("modules.md"), f.store.join("decisions.md")] { add(&mut s, &agent, "Read", json!({"file_path":p})); }
            for args in [json!(["init"]), json!(["map","--repo",f.repo]), json!(["ready"])] { add(&mut s, &agent, ORCH, json!({"args":args})); }
        }
        for u in &units {
            let (brief, text) = f.brief(u);
            if !after { add(&mut s, &agent, MEMORY, json!({"query":format!("build {u}")})); add(&mut s, &agent, "Write", json!({"file_path":brief,"content":text})); }
        }
        for u in &units {
            let (brief, _) = f.brief(u);
            s.push(json!({"agent":agent,"group":m,"name":CODEX,"input":{"unit":u,"role":"terra","cwd":f.repo,"cycle":after,"track":m,"brief_file":brief}}));
        }
        for u in &units {
            review(&mut s, f, m, u);
            if !after { add(&mut s, &agent, ORCH, json!({"args":["review",u,"--repo",f.repo,"--reviewer","chronos:reviewer"]})); }
        }
        if !after { add(&mut s, &agent, ORCH, json!({"args":["report","--repo",f.repo,"--module",m]})); }
    }
    for args in [json!(["fulltest","--repo",f.repo,"--command","python -I check.py all"]), json!(["status"]), json!(["report","--repo",f.repo])] { add(&mut s, "d-full", ORCH, json!({"args":args})); }
    s
}

fn stream(f: &Fixture, script: &[Value]) -> Vec<Value> {
    let script_path = f.temp.path().join("script.json"); let record = f.temp.path().join("record.json");
    fs::write(&script_path, serde_json::to_vec(script).unwrap()).unwrap();
    let cmd = common::fake("fake_claude");
    let mut child = Command::new(&cmd[0]).args(&cmd[1..]).env("FAKE_CLAUDE_SCENARIO", "script").env("FAKE_CLAUDE_SCRIPT", script_path).env("FAKE_CLAUDE_RECORD", record).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", json!({"type":"user","message":{"role":"user","content":"go"}})).unwrap();
    let out = child.wait_with_output().unwrap(); assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let events: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(events.last().unwrap()["type"], "result"); events
}
fn tools(events: &[Value]) -> Vec<(String, Value)> {
    events.iter().filter(|e| e["type"] == "assistant").flat_map(|e| {
        let parent = e["parent_tool_use_id"].as_str().unwrap_or("").to_owned();
        e["message"]["content"].as_array().unwrap().iter().filter(|v| v["type"] == "tool_use").map(move |v| (parent.clone(), v.clone()))
    }).collect()
}
fn counts(events: &[Value], units: usize) -> (usize, f64, usize, usize, BTreeMap<String,usize>) {
    let mut n=0; let mut rv=0; let mut full=0; let mut names = BTreeMap::new();
    for (parent, tool) in tools(events) {
        if ["d-a", "d-b", "d-c"].contains(&parent.as_str()) { n+=1; *names.entry(tool["name"].as_str().unwrap().trim_start_matches("mcp__plugin_chronos_bridge__").to_lowercase()).or_insert(0)+=1; }
        else if parent.starts_with("rv-") { rv+=1; } else if parent == "d-full" { full+=1; }
    }
    (n, n as f64 / units as f64, rv, full, names)
}
fn within_limit(per_unit: f64) -> bool { per_unit <= 3.0 }
fn write_file(path: &str, text: &str) { let p=Path::new(path); fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p,text).unwrap(); }

fn run(after: bool) -> f64 {
    let f=Fixture::new(); let script=script(&f,after); let events=stream(&f,&script);
    let groups: Vec<_> = events.iter().filter(|e| e["type"] == "assistant" && e["message"]["content"].as_array().unwrap().len() > 1).collect();
    assert_eq!(groups.len(), 3);
    for (event, module) in groups.iter().zip(["a", "b", "c"]) {
        assert_eq!(event["parent_tool_use_id"], format!("d-{module}"));
        let blocks = event["message"]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        for (block, suffix) in blocks.iter().zip(["1", "2"]) {
            assert_eq!(block["name"], CODEX);
            assert_eq!(block["input"]["unit"], format!("{module}{suffix}"));
            assert_eq!(events.iter().filter(|e| e["type"] == "user" && e["parent_tool_use_id"] == event["parent_tool_use_id"] && e["message"]["content"][0]["tool_use_id"] == block["id"]).count(), 1);
        }
        assert_ne!(blocks[0]["id"], blocks[1]["id"]);
    }
    let reviews = script.iter().filter(|e| e["name"] == ORCH && e["input"]["args"][0] == "review").count(); assert_eq!(reviews, if after {0} else {6});
    let mut agents: HashMap<String,Value> = HashMap::new(); let mut active: Option<String> = None;
    for event in events.iter().filter(|e| e["type"] == "assistant") {
        let calls = tools(std::slice::from_ref(event));
        if calls.len() > 1 {
            assert!(active.is_none());
            let barrier = std::sync::Barrier::new(calls.len());
            std::thread::scope(|scope| {
                let handles: Vec<_> = calls.iter().map(|(_, tool)| {
                    let mut input = tool["input"].clone();
                    input["project"] = json!(f.project);
                    input["cwd"] = json!(f.repo);
                    let barrier = &barrier;
                    let bridge = &f.bridge;
                    scope.spawn(move || {
                        barrier.wait();
                        let result = call(bridge, "codex_run", &input)["structuredContent"].clone();
                        assert_eq!(result["status"], "completed", "{CODEX} {input}: {result}");
                        if after { assert_ne!(result["next"], "fix", "cycle {input}: {result}"); }
                    })
                }).collect();
                for handle in handles { handle.join().unwrap(); }
            });
            continue;
        }
        for (parent, tool) in calls {
        if active.as_deref().is_some_and(|p| p != parent) {
            let id=active.take().unwrap();
            if after { let event=json!({"hook_event_name":"PostToolUse","session_id":"session","cwd":f.repo,"tool_name":"Agent","tool_input":agents[&id],"tool_response":{"agentId":id,"status":"completed"}}); let result=hook::handle(&event,Some(f.temp.path()),false).unwrap(); let text=result["hookSpecificOutput"]["additionalContext"].as_str().unwrap(); assert!(text.starts_with("Chronos recorded the review."), "{text}"); }
        }
        if parent.is_empty() { continue; }
        if parent.starts_with("rv-") {
            active=Some(parent.clone());
            let event=json!({"hook_event_name":"PreToolUse","session_id":"session","cwd":f.repo,"agent_id":parent,"agent_type":"chronos:reviewer","tool_name":tool["name"],"tool_input":tool["input"]});
            let result=hook::handle(&event,Some(f.temp.path()),false); assert!(result.as_ref().is_none_or(|r| r["hookSpecificOutput"]["permissionDecision"] != "deny"), "{result:?}");
        }
        let name=tool["name"].as_str().unwrap(); let mut input=tool["input"].clone();
        match name {
            "Agent" => { agents.insert(tool["id"].as_str().unwrap().to_owned(), input); }
            ORCH | CODEX | MEMORY => {
                input["project"]=json!(f.project);
                if name == CODEX { input["cwd"]=json!(f.repo); }
                if name == ORCH && input["args"][0] == "fulltest" {
                    let error=goal::set_status(f.temp.path(),"session","done",None).unwrap_err(); assert_eq!(error.kind,"not_done", "{error:?}");
                    std::thread::sleep(Duration::from_millis(1100));
                }
                let short = if name == ORCH {"orch"} else if name == CODEX {"codex_run"} else {"memory_search"};
                let result=call(&f.bridge, short, &input)["structuredContent"].clone();
                if name != MEMORY { assert_eq!(result["status"],"completed", "{name} {input}: {result}"); }
                if name == CODEX && after { assert_ne!(result["next"], "fix", "cycle {input}: {result}"); }
                if name == ORCH { assert_eq!(result["exit_code"],0, "{input}: {result}"); }
            }
            "Read" => { fs::read(input["file_path"].as_str().unwrap()).unwrap(); }
            "Write" => {
                let mut text=input["content"].as_str().unwrap().to_owned();
                if parent.starts_with("rv-") { let unit=parent.strip_prefix("rv-").unwrap(); let tree=orch::evidence(&f.store,unit).unwrap().into_iter().rfind(|r|r.kind=="verify").unwrap().commit; text=text.replace("@TREE@", &tree); }
                write_file(input["file_path"].as_str().unwrap(), &text);
            }
            "Bash" => { assert!(input["command"].as_str().unwrap().contains("<<'BRIEF'")); for (path, content) in input["files"].as_object().unwrap() { write_file(path, content.as_str().unwrap()); } }
            _ => panic!("unexpected tool {name}"),
        }
    }
    }
    assert!(active.is_none());
    for unit in UNITS { let rows=orch::evidence(&f.store,unit).unwrap(); assert_eq!(orch::derive_state(&rows,true),"verified", "{unit}: {rows:?}"); assert_eq!(rows.iter().filter(|r|r.kind=="review").count(),1,"{unit}: {rows:?}"); }
    goal::set_status(f.temp.path(),"session","done",None).unwrap();
    let (n, per, rv, full, names)=counts(&events,6); println!("synthetic {}: module-dispatcher calls={n} ({}) per unit={per:.2} / reviewer-internal={rv} / full-test dispatcher={full}", if after {"after"} else {"before"}, names.iter().map(|(k,v)|format!("{k}={v}")).collect::<Vec<_>>().join(" ")); per
}
#[test]
fn synthetic_dispatch_turns() { let before=run(false); let after=run(true); assert!(within_limit(after), "after={after}"); assert!(after < before, "after={after}, before={before}"); }
#[test]
fn counting_negative_control() { let events:Vec<Value>=(0..7).map(|_|json!({"type":"assistant","parent_tool_use_id":"d-a","message":{"content":[{"type":"tool_use","name":"Read"}]}})).collect(); let (_,per,_,_,_)=counts(&events,2); assert_eq!(per,3.5); assert!(!within_limit(per)); }
