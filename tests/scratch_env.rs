mod common;

use axiom::{
    base::proc,
    codex::{AppServer, RunRequest},
    orch,
};
use serde_json::Value;
use std::{fs, path::Path, process::Command, time::Duration};

#[test]
fn run_shell_env_sets_child_only() {
    let dir = common::temp();
    let key = "AXIOM_SCRATCH_CHILD_ONLY_TEST";
    let before = std::env::var_os(key);
    let cmd = if cfg!(windows) {
        "echo %AXIOM_SCRATCH_CHILD_ONLY_TEST%"
    } else {
        "echo $AXIOM_SCRATCH_CHILD_ONLY_TEST"
    };
    let run = proc::run_shell_env(
        cmd,
        dir.path(),
        Duration::from_secs(5),
        &[(key.into(), "child-value".into())],
    )
    .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "child-value");
    assert_eq!(std::env::var_os(key), before);
}

#[test]
fn scratch_env_appends_basetemp() {
    // Use isolated test processes for both inherited values; never mutate this process's environment.
    if std::env::var_os("AXIOM_SCRATCH_ENV_SUBTEST").is_none() {
        for inherited in [None, Some("-q --strict-markers")] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "scratch_env_appends_basetemp", "--nocapture"])
                .env("AXIOM_SCRATCH_ENV_SUBTEST", "1");
            match inherited {
                Some(v) => {
                    child.env("PYTEST_ADDOPTS", v);
                }
                None => {
                    child.env_remove("PYTEST_ADDOPTS");
                }
            }
            assert!(child.status().unwrap().success());
        }
        return;
    }
    let dir = common::temp();
    let env = proc::scratch_env(dir.path()).unwrap();
    let tmp = dir.path().join("tmp").to_string_lossy().into_owned();
    assert!(dir.path().join("tmp").is_dir());
    for (i, name) in ["TMP", "TEMP", "TMPDIR"].iter().enumerate() {
        assert_eq!(env[i], (name.to_string(), tmp.clone()));
    }
    let prefix = std::env::var("PYTEST_ADDOPTS")
        .ok()
        .map(|s| format!("{s} "))
        .unwrap_or_default();
    assert_eq!(
        env[3],
        (
            "PYTEST_ADDOPTS".into(),
            format!("{prefix}--basetemp={}", dir.path().join("pytest").display())
        )
    );
}

#[test]
fn codex_thread_gets_shell_env() {
    let dir = common::temp();
    let scratch = common::temp();
    let env = proc::scratch_env(scratch.path()).unwrap();
    let server =
        AppServer::new(common::fake_codex(&[])).with_slots_home(scratch.path().join("slots"));
    let mut req = RunRequest {
        brief: "FAKE_TEMPDIR safe\nFAKE_SHELL_WRITE shell.txt".into(),
        cwd: dir.path().to_string_lossy().into_owned(),
        model: "gpt-6.1-terra".into(),
        sandbox: "workspace-write".into(),
        effort: None,
        output_schema: None,
        developer_instructions: None,
        timeout_s: 10,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    };
    let result = server.run_with_env(&req, &env).unwrap();
    assert_eq!(result["status"], "completed");
    let echo: Value = serde_json::from_str(result["final_message"].as_str().unwrap()).unwrap();
    assert_eq!(
        echo["config"]["shell_environment_policy"]["set"]["TMP"],
        env[0].1
    );
    assert_eq!(echo["config"]["features"]["apps"], false);
    assert!(scratch
        .path()
        .join("tmp/pytest-of-fake/pytest-0/safe/case.txt")
        .is_file());
    assert!(dir.path().join("shell.txt").is_file());
    assert_eq!(
        result["file_changes"],
        serde_json::json!([{"path":"src/mod.py","kind":"add","status":"completed"}])
    );
    req.lean = false;
    req.brief = "empty environment".into();
    let empty = server.run_with_env(&req, &[]).unwrap();
    let echo: Value = serde_json::from_str(empty["final_message"].as_str().unwrap()).unwrap();
    assert_eq!(
        echo["config"]["shell_environment_policy"]["set"],
        serde_json::json!({})
    );
    req.brief = "FAKE_TEMPDIR old".into();
    for lean in [true, false] {
        req.lean = lean;
        let result = server.run(&req).unwrap();
        let echo: Value = serde_json::from_str(result["final_message"].as_str().unwrap()).unwrap();
        assert!(echo["config"].get("shell_environment_policy").is_none());
    }
    assert!(dir
        .path()
        .join("pytest-of-fake/pytest-0/old/case.txt")
        .is_file());
}

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

#[test]
fn engine_verify_leaves_no_axiom_tmp() {
    let home = common::temp();
    let repo = home.path().join("repo");
    let store = home.path().join("data/axiom/store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(repo.join("file.txt"), "fixture").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "file.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let script = home.path().join("check.py");
    fs::write(
        &script,
        r#"import os, pathlib, tempfile
tmp = pathlib.Path(os.environ['TMP'])
assert tmp == pathlib.Path(os.environ['TEMP']) == pathlib.Path(os.environ['TMPDIR'])
assert tmp.is_dir()
pathlib.Path(tmp, 'marker').write_text('ok')
with tempfile.TemporaryDirectory() as made:
    assert pathlib.Path(made).parent == tmp
base = os.environ['PYTEST_ADDOPTS'].rsplit('--basetemp=', 1)[1]
assert pathlib.Path(base) == tmp.parent / 'pytest'
pathlib.Path(base, 'pytest-of-fake', 'pytest-0').mkdir(parents=True)
print(tmp)
"#,
    )
    .unwrap();
    let command = format!("\"{}\" \"{}\"", common::python(), script.display());
    fs::write(
        store.join("brief.txt"),
        format!("GOAL x\nSCOPE file.txt\nVERIFY {command}\n"),
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            store.join("brief.txt").display()
        ),
    )
    .unwrap();
    fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS file.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `{command}`\nFULL TEST `{command}`\n")).unwrap();
    orch::append_evidence(
        &store,
        "u",
        &orch::Evidence {
            kind: "run".into(),
            command: "worker".into(),
            exit_code: 0,
            commit: "".into(),
            ts: "1".into(),
        },
    )
    .unwrap();
    let runs = [
        orch::record_verify(&store, "u", &repo).unwrap(),
        orch::record_verify(&store, "u", &repo).unwrap(),
        orch::record_accept(&store, "m", &repo).unwrap(),
        orch::record_full(&store, &repo, None).unwrap(),
    ];
    let prefixes = [
        store.join("scratch/u"),
        store.join("scratch/u"),
        store.join("scratch/accept-m"),
        store.join("scratch/fulltest"),
    ];
    assert_ne!(runs[0].output, runs[1].output);
    for (run, prefix) in runs.iter().zip(prefixes) {
        assert_eq!(run.exit_code, 0, "{}", run.output);
        let tmp = Path::new(run.output.trim());
        assert!(tmp.starts_with(prefix), "{}", tmp.display());
        assert_eq!(tmp.file_name().unwrap(), "tmp");
        assert!(!tmp.parent().unwrap().exists());
    }
    assert!(!repo.join(".axiom-tmp").exists());
    assert!(!fs::read_dir(&repo).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("pytest-of-")));
}
