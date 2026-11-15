mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    marker: PathBuf,
    fake: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        let marker = temp.path().join("launched");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/mod.py"), "old\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&project)
                .status()
                .unwrap()
                .success());
        }
        let wrapper = temp.path().join("fake.py");
        fs::write(&wrapper, "import os, sys\nfrom pathlib import Path\nPath(os.environ['LAUNCH_MARKER']).write_text('launched')\np = sys.argv[1]\nsys.argv = [p] + sys.argv[2:]\nsource = open(p, encoding='utf-8').read()\nsource = source.replace(\"paths = ['src/mod.py']\", \"paths = ['src/mod.py']; (cwd / paths[0]).write_text('fake-written', encoding='utf-8')\")\nexec(compile(source, p, 'exec'), {'__name__': '__main__', '__file__': p})\n").unwrap();
        let fake = serde_json::to_string(&vec![
            common::python(),
            "-I".into(),
            wrapper.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ])
        .unwrap();
        Self {
            _temp: temp,
            home,
            project,
            marker,
            fake,
        }
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .current_dir(&self.project)
            .env("AXIOM_HOME", &self.home)
            .env("AXIOM_DATA_HOME", &self.home)
            .env("AXIOM_CODEX_CMD", &self.fake)
            .env("LAUNCH_MARKER", &self.marker)
            .env("FAKE_CODEX_LOG", self._temp.path().join("codex.log"))
            .env_remove("ORCH_STORE")
            .args(args)
            .output()
            .unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn empty(path: &Path) -> bool {
    fs::read_dir(path).unwrap().next().is_none()
}

#[test]
fn do_runs_verification_and_prints_text_receipt() {
    for (verify, code, outcome) in [("exit 0", 0, "passed"), ("exit 1", 1, "failed")] {
        let f = Fixture::new();
        let out = f.command(&[
            "do",
            "--project",
            f.project.to_str().unwrap(),
            "--scope",
            "src/mod.py,src/extra.py",
            "src/another.py",
            "--scope",
            "src/fourth.py",
            "--model",
            "test-model",
            "--verify",
            verify,
            "Implement a change",
        ]);
        assert_eq!(
            out.status.code(),
            Some(code),
            "stdout: {} stderr: {}",
            text(&out.stdout),
            text(&out.stderr)
        );
        let stdout = text(&out.stdout);
        assert!(stdout.contains(&format!("outcome {outcome}")), "{stdout}");
        assert!(
            stdout.contains("mode quick") && stdout.contains("store "),
            "{stdout}"
        );
        assert!(f.marker.exists());
        let store = stdout
            .lines()
            .find_map(|line| line.strip_prefix("store "))
            .unwrap();
        let brief = fs::read_to_string(Path::new(store).join("quick.brief")).unwrap();
        assert!(brief.contains("SCOPE src/mod.py src/extra.py src/another.py src/fourth.py"));
        assert!(fs::read_to_string(f._temp.path().join("codex.log"))
            .unwrap()
            .contains("test-model"));
        assert_eq!(
            fs::read_to_string(f.project.join("src/mod.py")).unwrap(),
            "fake-written"
        );
    }
}

#[test]
fn invalid_do_never_launches_or_creates_home_entries() {
    let f = Fixture::new();
    for args in [
        vec!["do", "--verify", "exit 0", "task"],
        vec!["do", "--scope", "src/mod.py", "task"],
        vec!["do", "--scope", "src/mod.py", "--verify", "exit 0"],
        vec![
            "do",
            "--scope",
            "src/mod.py",
            "--verify",
            "exit 0",
            "--unknown",
            "task",
        ],
    ] {
        let out = f.command(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{:?}: {}",
            args,
            text(&out.stderr)
        );
        assert!(text(&out.stderr).starts_with("axiom: "));
        assert!(empty(&f.home), "{:?} created home entries", args);
        assert!(!f.marker.exists(), "{:?} launched codex", args);
    }
}

#[test]
fn modes_version_and_docs() {
    let f = Fixture::new();
    let out = f.command(&["modes"]);
    assert_eq!(out.status.code(), Some(0));
    let modes = text(&out.stdout);
    let docs = fs::read_to_string(common::repo_root().join("docs/modes.md"))
        .unwrap()
        .to_lowercase();
    for name in ["chat", "quick", "wish", "research", "routine", "plaza"] {
        assert!(modes.contains(name), "missing {name}: {modes}");
        assert!(docs.contains(name), "docs missing {name}");
    }
    assert_eq!(f.command(&["modes", "extra"]).status.code(), Some(2));
    let version = f.command(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(text(&version.stdout).trim(), "axiom-chronos 3.0.0-beta.1");
}
