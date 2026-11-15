use regex::Regex;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn terminate(&mut self) {
        if let Some(mut child) = self.0.take() {
            #[cfg(windows)]
            let _ = Command::new("taskkill")
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn output_tail(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let len = file.metadata().unwrap().len();
    file.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)))
        .unwrap();
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).unwrap();
    String::from_utf8_lossy(&tail).into_owned()
}

#[test]
fn sources_do_not_use_repository_scratch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sources = [
        "tests/scratch_outside_repo.rs",
        "tests/hook_restore.rs",
        "tests/subdir_project.rs",
        "tests/session/render_cases.rs",
        "tests/restore.rs",
        "src/desktop/plain_text.rs",
    ];
    let forbidden = Regex::new(
        r#"(?s)(?:CARGO_MANIFEST_DIR|repo_root\s*\(\s*\))[^;]*?["/\\]\.axiom-tmp(?:[/\\"]|$)|Path\s*::\s*new\s*\(\s*"\.axiom-tmp(?:[/\\"]|$)"#,
    )
    .unwrap();
    let mut violations = Vec::new();
    for source in sources {
        let contents = fs::read_to_string(root.join(source)).unwrap();
        for found in forbidden.find_iter(&contents) {
            let line = contents[..found.start()]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1;
            violations.push(format!("{source}:{line}"));
        }
    }
    assert!(
        violations.is_empty(),
        "repository-rooted scratch paths:\n{}",
        violations.join("\n")
    );
}

fn scratch_status(root: &Path) -> BTreeSet<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "-uall", "--", ".axiom-tmp"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn file_tests_under_long_temp_do_not_write_repository_scratch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    assert!(!temp
        .path()
        .canonicalize()
        .unwrap()
        .starts_with(root.canonicalize().unwrap()));
    let mut long_temp = temp.path().to_path_buf();
    while long_temp.as_os_str().len() < 150 {
        long_temp.push("long-temp-component-0123456789abcdef");
    }
    fs::create_dir_all(&long_temp).unwrap();
    let before = scratch_status(root);
    let output_path = temp.path().join("child-output.log");
    let output = fs::File::create(&output_path).unwrap();
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    // subdir_project hangs when nested under the bridge; the outer VERIFY runs it directly.
    command
        .args([
            "test",
            "--locked",
            "--offline",
            "--test",
            "hook_restore",
            "--test",
            "restore",
        ])
        .current_dir(root)
        .env("TEMP", &long_temp)
        .env("TMP", &long_temp)
        .env("TMPDIR", &long_temp)
        .env_remove("PYTEST_ADDOPTS")
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = ChildGuard(Some(command.spawn().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(600);
    let status = loop {
        if let Some(status) = child.0.as_mut().unwrap().try_wait().unwrap() {
            child.0.take();
            break status;
        }
        if Instant::now() >= deadline {
            child.terminate();
            panic!(
                "child cargo test timed out after 600 seconds; output tail:\n{}",
                output_tail(&output_path)
            );
        }
        thread::sleep(Duration::from_millis(100));
    };
    let after = scratch_status(root);
    let added: Vec<_> = after.difference(&before).collect();
    assert!(
        added.is_empty(),
        "new repository scratch entries: {added:?}"
    );
    assert!(
        status.success(),
        "child cargo test failed: {status}; output tail:\n{}",
        output_tail(&output_path)
    );
}
