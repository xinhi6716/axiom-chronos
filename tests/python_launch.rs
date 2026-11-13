//! Python literal inventory (the line numbers refer to the pre-fix search).
//!
//! Launch/probe sites:
//! - src/codex/rpc.rs:463; src/desktop/ux/real/modes_accept.rs:368;
//!   tests/workshop.rs:206: covered by this test.
//! - tests/common/mod.rs:11,13; src/council/member.rs:741,743;
//!   src/engine/calls.rs:117,119; src/session/wait.rs:545,547;
//!   src/session/run.rs:1405; src/desktop/live.rs:2342;
//!   src/desktop/live/accept.rs:171; src/desktop/ux/real/background_accept.rs:20;
//!   src/desktop/ux/real/extreme_accept.rs:26,28;
//!   src/desktop/ux/real/interjections_accept.rs:29: AXIOM_TEST_PYTHON first.
//! - src/host/mod.rs:303: test uses AXIOM_PYTHON, not AXIOM_TEST_PYTHON.
//! - src/workshop/host.rs:262,266: production manifest matching and PATH probe.
//! - src/research_workspace/pdf.rs:283,285,306,311,314: production PATH
//!   probing/default command; its real-process test separately prefers the override.
//! - tests/p5b_cli.rs:41: manifest fixture, indirectly run by workshop_lifecycle.
//! - tests/python_launch.rs: intentionally probes the failing stubs; other
//!   literals here are scan patterns, the negative control, or stub names.
//!
//! Pure fixtures / non-command language labels:
//! - tests/base.rs:45,48; tests/compute.rs:34; tests/compute_guards.rs:24,271,338;
//!   tests/laya_train.rs:119,125; tests/workshop_api.rs:28;
//!   tests/workshop_router.rs:266: fixture argv/manifest.
//! - src/graph/code/model.rs:35; src/graph/code/index/resolve.rs:901;
//!   tests/graph_code_model.rs:30; tests/graph_code.rs:89;
//!   tests/graph_code_script.rs:25,144; tests/graph_frozen.rs:8: language labels.
//!
//! FAILING-FIRST RECORD
//! With failing stubs first on PATH and a valid override:
//! - RPC: cargo exited 101: "codex app-server exited during initialize".
//! - Modes: cargo was blocked by an unrelated E0425 (`fxture`) in
//!   interjections_accept.rs. The existing pre-fix app test executable exited
//!   101: "assertion failed: ... any(|r| r.mode == Mode::Quick)".
//! - Workshop: cargo reported "1 passed" vacuously: the bare probes failed and
//!   the test returned without starting anything. The post-fix behavioural test
//!   requires a marker printed only after the module's done() completes.

use std::{env, fs, path::Path, process::Command};

// Scope the check to an individual test, so a lookup in a different test cannot
// mask a bare launch. This intentionally recognises the patterns in these sites.
fn flags_bare_launch(test: &str) -> bool {
    let uses_bare_command = test.lines().any(|line| {
        let bare = line.contains("\"python\"") || line.contains("\"python3\"");
        bare && (line.contains("if cfg!(windows)")
            || line.contains("\"python\".to_owned()")
            || line.contains("\"python3\".to_owned()")
            || line.contains("\"python\".to_string()")
            || line.contains("\"python3\".to_string()")
            || line.contains("\"python\".into()")
            || line.contains("\"python3\".into()")
            || line.contains("Command::new(")
            || line.contains("proc::cmd(")
            || line.contains("proc::command(")
            || line.contains(".into_iter().any("))
    });
    let has_override_lookup = test.contains("std::env::var(\"AXIOM_TEST_PYTHON\")")
        || test.contains("std::env::var_os(\"AXIOM_TEST_PYTHON\")");
    uses_bare_command && !has_override_lookup
}

fn test_body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("fn {name}(")).expect("test missing");
    let rest = &source[start..];
    let end = rest
        .find("\n    #[test]")
        .or_else(|| rest.find("\n#[test]"));
    &rest[..end.unwrap_or(rest.len())]
}

#[test]
fn no_unprotected_bare_python_launch() {
    let cases = [
        (
            include_str!("../src/codex/rpc.rs"),
            "notifications_and_cancel_reach_the_registered_turn",
        ),
        (
            include_str!("../src/desktop/ux/real/modes_accept.rs"),
            "lazy_quick_uses_engine_home_even_when_environment_home_differs",
        ),
        (include_str!("workshop.rs"), "resolves_python_from_path"),
    ];
    for (source, name) in cases {
        assert!(
            !flags_bare_launch(test_body(source, name)),
            "bare Python launch in {name}"
        );
    }
    assert!(flags_bare_launch(
        r#"let python = if cfg!(windows) { "python" } else { "python3" };"#
    ));
}

fn stubs() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    assert!(!dir.path().canonicalize().unwrap().starts_with(repo));
    for name in ["python", "python3"] {
        #[cfg(windows)]
        {
            fs::write(dir.path().join(format!("{name}.cmd")), "@exit /b 1\r\n").unwrap();
            // Rust's Windows executable search can choose a later .exe before a
            // preceding .cmd. where.exe exits nonzero for the tested arguments.
            let system = Path::new(&env::var_os("SystemRoot").unwrap()).join("System32/where.exe");
            fs::copy(system, dir.path().join(format!("{name}.exe"))).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.path().join(name);
            fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    dir
}

fn run_site(args: &[&str], marker: Option<&str>) {
    let Some(python) = env::var_os("AXIOM_TEST_PYTHON") else {
        eprintln!("skipping: AXIOM_TEST_PYTHON is unset");
        return;
    };
    assert!(
        Command::new(&python)
            .arg("--version")
            .status()
            .expect("override must run")
            .success(),
        "AXIOM_TEST_PYTHON is not usable"
    );
    let dir = stubs();
    let path = env::join_paths(
        std::iter::once(dir.path().to_path_buf())
            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    for name in ["python", "python3"] {
        assert!(
            !Command::new(name)
                .arg("--version")
                .env("PATH", &path)
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .status()
                .expect("stub must run")
                .success(),
            "{name} was not stubbed"
        );
    }
    let output = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PATH", path)
        .env("AXIOM_TEST_PYTHON", python)
        .args(args)
        .output()
        .expect("run cargo test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "cargo failed:\n{stdout}\n{stderr}");
    assert!(
        stdout.contains("1 passed"),
        "expected one passing test:\n{stdout}\n{stderr}"
    );
    if let Some(marker) = marker {
        assert!(
            stdout.contains(marker),
            "test did not start the module:\n{stdout}\n{stderr}"
        );
    }
}

#[test]
fn rpc_uses_override() {
    run_site(
        &[
            "test",
            "--locked",
            "--offline",
            "--lib",
            "--",
            "--exact",
            "codex::rpc::tests::notifications_and_cancel_reach_the_registered_turn",
        ],
        None,
    );
}

#[test]
fn modes_uses_override() {
    run_site(&["test", "--locked", "--offline", "--features", "app", "--lib", "--", "--exact", "desktop::ux::real::modes_accept::lazy_quick_uses_engine_home_even_when_environment_home_differs"], None);
}

#[test]
fn workshop_uses_override_and_runs_module() {
    run_site(
        &[
            "test",
            "--locked",
            "--offline",
            "--test",
            "workshop",
            "--",
            "--exact",
            "resolves_python_from_path",
            "--nocapture",
        ],
        Some("AXIOM_WORKSHOP_MODULE_STARTED"),
    );
}
