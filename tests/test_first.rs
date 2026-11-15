mod common;
struct RunFixture {
    _temp: tempfile::TempDir,
    repo: std::path::PathBuf,
    store: std::path::PathBuf,
    log: std::path::PathBuf,
    bridge: axiom::bridge::Bridge,
}
impl RunFixture {
    fn new(route: &str, settings: &str) -> Self {
        use std::{fs, process::Command};
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
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
        let store = axiom::orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n{route}\n")).unwrap();
        let path = store.join("u1.brief");
        let risk = if route.is_empty() { "RISK trivial\n" } else { "" };
        fs::write(
            &path,
            format!("GOAL build\n{risk}SCOPE src/mod.py\nVERIFY echo ok\n{settings}\n"),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
                path.display()
            ),
        )
        .unwrap();
        let log = temp.path().join("codex.log");
        let wrapper = temp.path().join("fake.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nos.environ['FAKE_CODEX_MODELS']='gpt-6.1-sol,gpt-6-astra,gpt-6-terra,gpt-6-luna'\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        let mut config = axiom::bridge::Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]);
        config.wsl = None;
        config.on_windows = false;
        let bridge = axiom::bridge::Bridge::new(config);
        Self {
            _temp: temp,
            repo,
            store,
            log,
            bridge,
        }
    }
    fn run(&self, role: &str) -> serde_json::Value {
        axiom::bridge::call(
            &self.bridge,
            "codex_run",
            &serde_json::json!({"project":"demo","unit":"u1","role":role,"cwd":self.repo}),
        )["structuredContent"]
            .clone()
    }
}
#[test]
fn routed_store_requires_matrix() {
    let f = RunFixture::new("ROUTING\n  default sol/xhigh parser reason", "");
    let result = f.run("terra");
    assert_eq!(result["status"], "bridge_error", "{result}");
    assert_eq!(result["error"]["kind"], "contract_required", "{result}");
    let log = std::fs::read_to_string(&f.log).unwrap_or_default();
    assert!(log.is_empty(), "{log}");
}

#[test]
fn unrouted_store_needs_no_matrix() {
    let f = RunFixture::new("", "");
    let result = f.run("terra");
    assert_eq!(result["status"], "completed", "{result}");
    RunFixture::matrix("passing").failure("red_invalid");
}


impl RunFixture {
    fn matrix(mode: &str) -> Self {
        let f = Self::new("", "");
        std::fs::create_dir_all(f.repo.join("tests")).unwrap();
        let test = r#"import pathlib,sys,unittest
mode = sys.argv.pop() if len(sys.argv)>1 else 'normal'
green = pathlib.Path('src/mod.py').read_text() == 'implemented'
if mode == 'compile':
    print('error[E0425]: could not compile'); sys.exit(1)
if mode == 'missing': sys.exit(127)
if mode == 'bare': sys.exit(1)
if mode == 'wrong':
    print('FAIL: unrelated_case'); sys.exit(1)
if mode == 'passing': green=True
if mode == 'zero' and green:
    print('Ran 0 tests\nOK'); sys.exit(0)
class Matrix(unittest.TestCase):
    def test_case(self):
        self.assertTrue(green and mode != 'finalfail', 'key not found\nerror: assertion detail' if mode == 'notfound' else '')
        self.assertTrue(mode == 'passing' or pathlib.Path('verified.txt').exists())
unittest.main(verbosity=2)
"#;
        std::fs::write(f._temp.path().join("test-template"), test).unwrap();
        let command = format!("\"{}\" tests/matrix.py {mode}", common::python());
        std::fs::write(f.store.join("u1.brief"), format!("GOAL build\nRISK trivial\nSCOPE src/ tests/\nVERIFY echo verified > verified.txt\nCONTRACT matrix behavior is protected\nTEST_MATRIX\na | behaviour | tests/matrix.py::test_case | checks implementation | {command}\nb | edge | tests/matrix.py::test_case | checks again | {command}\nCONTEXT\nFAKE_SELFCHECK {{\"status\":\"pass\",\"changed\":false,\"fixed\":0,\"findings\":[]}}\n")).unwrap();
        let wrapper = f._temp.path().join("fake.py");
        let original = std::fs::read_to_string(&wrapper).unwrap();
        let hook = format!(r#"
import json,pathlib
repo = pathlib.Path({repo})
template = pathlib.Path({template})
mode = {mode}
original_stdin = sys.stdin
class Input:
    def __getattr__(self,k): return getattr(original_stdin,k)
    def __iter__(self): return self
    def __next__(self):
        line = next(original_stdin)
        msg = json.loads(line)
        if msg.get('method') == 'turn/start':
            params=msg['params']; text=params['input'][0]['text']
            if 'TESTS-FIRST' in text:
                (repo/'tests/matrix.py').write_text(template.read_text())
            elif not params.get('outputSchema'):
                (repo/'src/mod.py').write_text('implemented')
                if mode == 'tamper':
                    with (repo/'tests/matrix.py').open('a') as out: out.write('\n# tampered\n')
        return line
sys.stdin = Input()
"#, repo=serde_json::to_string(&f.repo.to_string_lossy()).unwrap(), template=serde_json::to_string(&f._temp.path().join("test-template").to_string_lossy()).unwrap(), mode=serde_json::to_string(mode).unwrap());
        std::fs::write(wrapper, original.replace("runpy.run_path", &(hook + "\nrunpy.run_path"))).unwrap();
        f
    }
    fn rows(&self) -> Vec<axiom::orch::Evidence> { axiom::orch::evidence(&self.store,"u1").unwrap() }
    fn failure(&self, kind: &str) {
        let r=self.run("terra");
        assert_eq!(r["error"]["kind"],kind,"{r}");
        assert!(r["error"]["message"].as_str().unwrap().contains("row a"),"{r}");
    }
}
#[test]
fn verified_unit_never_refused() {
    let f=RunFixture::new("ROUTING\n  default sol/xhigh parser reason", "");
    axiom::orch::append_evidence(&f.store,"u1",&axiom::orch::Evidence{kind:"run".into(),command:"gpt-6-terra".into(),exit_code:0,commit:String::new(),ts:"now".into()}).unwrap();
    axiom::orch::record_verify(&f.store,"u1",&f.repo).unwrap();
    axiom::orch::append_evidence(&f.store,"u1",&axiom::orch::Evidence{kind:"accept".into(),command:"echo ok".into(),exit_code:0,commit:axiom::orch::snapshot(&f.repo,&["src/mod.py".into()]),ts:"now".into()}).unwrap();
    assert!(matches!(axiom::orch::begin_unit(&f.store,"u1","sol",&f.repo,true).unwrap(),axiom::orch::Begin::Reused));
    axiom::orch::append_evidence(&f.store,"u1",&axiom::orch::Evidence{kind:"review".into(),command:"engine-astra:gpt-6-astra".into(),exit_code:0,commit:axiom::orch::snapshot(&f.repo,&["src/mod.py".into()]),ts:"now".into()}).unwrap();
    assert_eq!(axiom::orch::derive_state(&f.rows(),true),"verified","{:?}",f.rows());
    let r=f.run("terra");
    assert_ne!(r["error"]["kind"],"contract_required","{r}");
    assert_eq!(r["state"],"verified","{r}");
    assert!(std::fs::read_to_string(&f.log).unwrap_or_default().is_empty());
}
#[test]
fn red_then_implement_order() {
    let f=RunFixture::matrix("normal"); let r=f.run("terra");
    assert_eq!(r["status"],"completed","{r}");
    let log=std::fs::read_to_string(&f.log).unwrap();
    let turns:Vec<serde_json::Value>=log.lines().map(|s|serde_json::from_str(s).unwrap()).filter(|v:&serde_json::Value|v["method"]=="turn/start").collect();
    assert!(turns.len()>=2,"{log}");
    let red_prompt=turns[0]["params"]["input"][0]["text"].as_str().unwrap();
    for expected in ["TESTS-FIRST", "Write only these matrix test files: tests/matrix.py", "CONTRACT\nmatrix behavior is protected", "tests/matrix.py::test_case"] {
        assert!(red_prompt.contains(expected),"missing {expected}: {red_prompt}");
    }
    assert!(!turns[1]["params"]["input"][0]["text"].as_str().unwrap().contains("TESTS-FIRST"));
    let rows=f.rows(); let red:Vec<_>=rows.iter().filter(|r|r.kind=="red").collect();
    assert_eq!(red.len(),2); for (row,id) in red.iter().zip(["a","b"]) {
        assert_eq!(row.exit_code,1); assert!(row.commit.starts_with("tree:"));
        let brief=axiom::orch::unit_brief(&f.store,"u1").unwrap();
        let command=&brief.test_matrix.iter().find(|r|r.id==id).unwrap().command;
        assert_eq!(row.command,format!("{}|{id}|{command}",r["attempt_id"].as_str().unwrap()));
        assert_eq!(row.commit,axiom::orch::snapshot(&f.repo,&["tests/matrix.py".into()]));
    }
}
#[test]
fn red_compile_error_invalid() { for m in ["compile","missing"] { RunFixture::matrix(m).failure("red_invalid"); } }
#[test]
fn red_assertion_not_found_valid() {
    let f=RunFixture::matrix("notfound"); let r=f.run("terra");
    assert_eq!(r["status"],"completed","{r}");
    let attempt=r["attempt_id"].as_str().unwrap();
    for id in ["a","b"] {
        let output=std::fs::read_to_string(f.store.join(format!("reports/u1.{attempt}.red.{id}.txt"))).unwrap();
        assert!(output.contains("FAIL: test_case"),"{output}");
        assert!(output.replace("\r\n","\n").contains("key not found\nerror: assertion detail"),"{output}");
    }
    assert!(axiom::orch::lifecycle::validate_red(&f.store,"u1",attempt,&f.repo).unwrap());
}
#[test]
fn red_bare_nonzero_invalid() { for m in ["bare","wrong","passing"] { RunFixture::matrix(m).failure("red_invalid"); } }
#[test]
fn red_tampered() { RunFixture::matrix("tamper").failure("red_tampered"); }
#[test]
fn matrix_runs_after_verify() {
    let f=RunFixture::matrix("normal"); let r=f.run("terra");
    assert_eq!(r["status"],"completed","{r}");
    let rows=f.rows(); let v=rows.iter().position(|r|r.kind=="verify").unwrap();
    let matrix:Vec<_>=rows.iter().enumerate().filter(|(_,r)|r.kind=="matrix").collect();
    assert_eq!(matrix.len(),2); for (i,row) in matrix { assert!(i>v); assert_eq!(row.exit_code,0); }
    let path=f.store.join("reports/u1.evidence.tsv");
    let text=std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path,text.lines().filter(|l|!l.starts_with("selfcheck\t")).collect::<Vec<_>>().join("\n")+"\n").unwrap();
    std::fs::write(&f.log,"").unwrap();
    let resumed=f.run("terra");
    assert_eq!(resumed["status"],"completed","{resumed}");
    let log=std::fs::read_to_string(&f.log).unwrap();
    let turns:Vec<serde_json::Value>=log.lines().map(|s|serde_json::from_str(s).unwrap()).filter(|v:&serde_json::Value|v["method"]=="turn/start").collect();
    assert_eq!(turns.len(),1,"{log}");
    assert_eq!(turns[0]["params"]["outputSchema"]["title"],"axiom.self_check.v1");
}
#[test]
fn matrix_failed_and_not_run() {
    for (mode,kind) in [("finalfail","matrix_failed"),("zero","matrix_not_run")] {
        let f=RunFixture::matrix(mode);
        f.failure(kind);
        assert_eq!(f.rows().iter().filter(|r|r.kind=="matrix").count(),2);
        // A passing VERIFY alone cannot let a later call bypass this matrix.
        f.failure(kind);
    }
}
#[test]
fn red_from_other_attempt_rejected() {
    for corrupt in ["attempt", "tree"] {
        let f=RunFixture::matrix("normal"); let r=f.run("terra");
        assert_eq!(r["status"],"completed","{r}");
        let attempt=r["attempt_id"].as_str().unwrap();
        // Successful completion removes the checkpoint. Restore its RED binding
        // to exercise the same validation used before a resumed implement turn.
        let tree=f.rows().into_iter().find(|r|r.kind=="red").unwrap().commit;
        std::fs::create_dir_all(f.store.join("checkpoints")).unwrap();
        std::fs::write(f.store.join("checkpoints/u1.json"),serde_json::to_vec(&serde_json::json!({"unit":"u1","attempt":1,"attempt_id":attempt,"base_tree":"","brief_sha256":"","routing":null,"stage":"claimed","pid":0,"updated":"now","evidence_start":0,"red_tree":tree})).unwrap()).unwrap();
        assert!(axiom::orch::lifecycle::validate_red(&f.store,"u1",attempt,&f.repo).unwrap());
        let path=f.store.join("reports/u1.evidence.tsv");
        let original=std::fs::read_to_string(&path).unwrap();
        let text=original.lines().map(|l| {
            if !l.starts_with("red\t") { return l.to_owned(); }
            let mut fields:Vec<String>=l.split('\t').map(str::to_owned).collect();
            if corrupt=="attempt" { fields[1]=fields[1].replacen(attempt,"another-attempt",1); } else { fields[3]="tree:another".into(); }
            fields.join("\t")
        }).collect::<Vec<_>>().join("\n")+"\n";
        std::fs::write(&path,text).unwrap();
        let err=axiom::orch::lifecycle::validate_red(&f.store,"u1",attempt,&f.repo).unwrap_err();
        assert_eq!(err.kind,"red_invalid","{corrupt}");
        std::fs::write(&path,original).unwrap();
        assert!(axiom::orch::lifecycle::validate_red(&f.store,"u1",attempt,&f.repo).unwrap());
    }
}
#[test]
fn review_pack_shows_matrix() {
    let f=RunFixture::matrix("normal"); let r=f.run("terra");
    assert_eq!(r["status"],"completed","{r}");
    let text=std::fs::read_to_string(r["pack"].as_str().unwrap()).unwrap();
    for expected in ["## CONTRACT", "matrix behavior is protected", "## TEST_MATRIX", "tests/matrix.py::test_case", "red |", "matrix |"] { assert!(text.contains(expected),"missing {expected}: {text}"); }
}
