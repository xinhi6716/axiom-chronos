use axiom::orch::*;
use std::{fs, path::Path, process::Command};

fn temp() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "orch-evidence-{}-{}",
        std::process::id(),
        axiom::base::unique()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn row(kind: &str, command: &str, code: i32, commit: &str, ts: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: code,
        commit: commit.into(),
        ts: ts.into(),
    }
}
fn setup(p: &Path) {
    fs::create_dir_all(p.join("reports")).unwrap();
    fs::write(
        p.join("brief.txt"),
        "GOAL x\nSCOPE file.txt\nVERIFY echo ok\n",
    )
    .unwrap();
    fs::write(
        p.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            p.join("brief.txt").display()
        ),
    )
    .unwrap();
    fs::write(p.join("modules.md"), "## m\nPURPOSE x\nPATHS file.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\nFULL TEST `echo ok`\n").unwrap();
}
#[test]
fn state_table() {
    let run = row("run", "gpt-worker", 0, "", "1");
    let verify = row("verify", "test", 0, "", "2");
    let review = row("review", "claude-review", 0, "a", "3");
    assert_eq!(derive_state(&[], false), "pending");
    assert_eq!(derive_state(&[row("run", "x", 1, "", "")], false), "failed");
    assert_eq!(
        derive_state(&[run.clone(), row("scope", "", 1, "", "")], false),
        "failed"
    );
    assert_eq!(
        derive_state(&[run.clone(), row("verify", "", 1, "", "")], false),
        "failed"
    );
    assert_eq!(derive_state(&[run.clone(), verify.clone()], true), "built");
    assert_eq!(
        derive_state(
            &[run.clone(), verify.clone(), row("accept", "", 1, "", "")],
            true
        ),
        "accept-failed"
    );
    assert_eq!(
        derive_state(&[run.clone(), verify.clone(), review.clone()], false),
        "verified"
    );
    assert_eq!(
        derive_state(
            &[
                run.clone(),
                verify.clone(),
                row("review", "gpt-worker", 0, "a", "")
            ],
            false
        ),
        "built"
    );
    assert_eq!(
        derive_state(
            &[
                run.clone(),
                verify.clone(),
                row("review", "claude-review", 1, "a", ""),
                review.clone()
            ],
            false
        ),
        "review-failed"
    );
    assert_eq!(
        derive_state(
            &[
                run,
                verify,
                row("review", "claude-review", 1, "old", ""),
                review
            ],
            false
        ),
        "verified"
    );
}
#[test]
fn snapshot_tracks_worktree() {
    let p = temp();
    assert!(snapshot(&p, &["file.txt".into()]).is_empty());
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&p)
        .status()
        .unwrap()
        .success());
    fs::write(p.join("file.txt"), "one").unwrap();
    let a = snapshot(&p, &["file.txt".into()]);
    assert!(!a.is_empty());
    assert_eq!(a, snapshot(&p, &["file.txt".into()]));
    fs::write(p.join("file.txt"), "two").unwrap();
    assert_ne!(a, snapshot(&p, &["file.txt".into()]));
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn snapshot_tracks_ignored_inputs_empty_directories_and_scoped_changes() {
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("ignored")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("tracked-ignored.txt"), "tracked-original").unwrap();
    assert!(Command::new("git")
        .args(["add", "--", "tracked-ignored.txt"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join(".gitignore"), "ignored/\ntracked-ignored.txt\n").unwrap();
    fs::write(repo.join("normal.txt"), "normal").unwrap();
    fs::write(repo.join("ignored/input.txt"), "ignored-original").unwrap();
    let all = || snapshot(&repo, &[".".into()]);
    let first = all();
    assert!(first.starts_with("tree-v2:"), "{first}");
    assert_eq!(first, all());
    assert!(snapshot(&repo, &[".git/config".into()]).is_empty());
    assert!(snapshot(&repo, &[]).starts_with("tree-v2:"));
    let scoped = snapshot(&repo, &["normal.txt".into()]);
    fs::write(repo.join("ignored/input.txt"), "unique-ignored-change").unwrap();
    let changed = all();
    assert_ne!(first, changed);
    let ignored_hash = Command::new("git")
        .args(["hash-object", "--", "ignored/input.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(ignored_hash.status.success());
    let ignored_oid = String::from_utf8(ignored_hash.stdout).unwrap();
    assert!(
        !Command::new("git")
            .args(["cat-file", "-e", ignored_oid.trim()])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success(),
        "ignored bytes must not be written to Git objects"
    );
    assert_eq!(scoped, snapshot(&repo, &["normal.txt".into()]));
    let direct = snapshot(&repo, &["ignored/input.txt".into()]);
    assert!(direct.starts_with("tree-v2:"), "{direct}");
    fs::write(repo.join("ignored/input.txt"), "second-unique-change").unwrap();
    assert_ne!(direct, snapshot(&repo, &["ignored/input.txt".into()]));
    let before_empty = all();
    fs::create_dir(repo.join("ignored/empty")).unwrap();
    assert_ne!(before_empty, all());
    fs::remove_dir(repo.join("ignored/empty")).unwrap();
    assert_eq!(before_empty, all());
    fs::rename(
        repo.join("ignored/input.txt"),
        repo.join("ignored/renamed.txt"),
    )
    .unwrap();
    assert_ne!(before_empty, all());
    let before_delete = all();
    fs::remove_file(repo.join("ignored/renamed.txt")).unwrap();
    assert_ne!(before_delete, all());

    fs::write(
        repo.join("tracked-ignored.txt"),
        "new-tracked-ignored-secret",
    )
    .unwrap();
    let _ = all();
    let hash = Command::new("git")
        .args(["hash-object", "--", "tracked-ignored.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(hash.status.success());
    let oid = String::from_utf8(hash.stdout).unwrap();
    assert!(
        !Command::new("git")
            .args(["cat-file", "-e", oid.trim()])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success(),
        "ignored changed bytes must not be written to Git objects"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn scoped_ignored_colon_paths_and_directory_permissions_are_included() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    fs::write(root.join(".gitignore"), "a:b/\n").unwrap();
    fs::create_dir(root.join("a:b")).unwrap();
    fs::set_permissions(root.join("a:b"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join("a:b/input"), "before").unwrap();
    let scope = ["a:b/input".into()];
    let first = snapshot(&root, &scope);
    assert!(first.starts_with("tree-v2:"));
    fs::write(root.join("a:b/input"), "after").unwrap();
    let second = snapshot(&root, &scope);
    assert_ne!(first, second);
    let old_mode = fs::metadata(root.join("a:b")).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(root.join("a:b"), fs::Permissions::from_mode(0o700)).unwrap();
    let new_mode = fs::metadata(root.join("a:b")).unwrap().permissions().mode() & 0o777;
    if old_mode == new_mode {
        eprintln!("SKIPPED directory-mode assertion: filesystem did not retain chmod");
    } else {
        assert_ne!(second, snapshot(&root, &scope));
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn scoped_ignored_single_letter_colon_directory_is_included() {
    let root = temp();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    fs::write(root.join(".gitignore"), "a:/\n").unwrap();
    if let Err(error) = fs::create_dir(root.join("a:")) {
        if matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
        ) {
            eprintln!("SKIPPED single-letter colon directory: filesystem does not support name");
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("cannot create synthetic colon directory: {error}");
    }
    fs::write(root.join("a:/input"), "before").unwrap();
    let first = snapshot(&root, &["a:/input".into()]);
    assert!(first.starts_with("tree-v2:"), "{first}");
    fs::write(root.join("a:/input"), "after").unwrap();
    assert_ne!(first, snapshot(&root, &["a:/input".into()]));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_rejects_ambiguous_backslash_names() {
    let root = temp();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    fs::write(root.join(".gitignore"), "a*\n").unwrap();
    fs::create_dir(root.join("a")).unwrap();
    fs::write(root.join("a/b"), "one").unwrap();
    fs::write(root.join("a\\b"), "two").unwrap();
    assert!(snapshot(&root, &[".".into()]).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_rejects_escaping_links_without_reading_the_target() {
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(root.join("outside.txt"), "external").unwrap();
    std::os::unix::fs::symlink(root.join("outside.txt"), repo.join("link")).unwrap();
    assert!(snapshot(&repo, &[".".into()]).is_empty());
    fs::create_dir(root.join("outside-dir")).unwrap();
    fs::write(root.join("outside-dir/input"), "outside-unhashed").unwrap();
    std::os::unix::fs::symlink(root.join("outside-dir"), repo.join("linkdir")).unwrap();
    assert!(snapshot(&repo, &["linkdir/input".into()]).is_empty());
    let oid = Command::new("git")
        .args([
            "hash-object",
            "--",
            root.join("outside-dir/input").to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(oid.status.success());
    assert!(!Command::new("git")
        .args([
            "cat-file",
            "-e",
            String::from_utf8(oid.stdout).unwrap().trim()
        ])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_rejects_non_utf8_input_names() {
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
    };
    let root = temp();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    fs::write(root.join(OsString::from_vec(vec![0xff, b'x'])), "input").unwrap();
    let preserved = fs::read_dir(&root)
        .unwrap()
        .any(|entry| entry.unwrap().file_name().as_os_str().as_bytes() == [0xff, b'x']);
    if !preserved {
        eprintln!("SKIPPED non-UTF-8 assertion: filesystem did not preserve raw filename bytes");
        fs::remove_dir_all(root).unwrap();
        return;
    }
    assert!(snapshot(&root, &[".".into()]).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn scoped_file_link_tracks_internal_target_content() {
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("target.txt"), "before").unwrap();
    std::os::unix::fs::symlink("target.txt", repo.join("link.txt")).unwrap();
    let before = snapshot(&repo, &["link.txt".into()]);
    assert!(before.starts_with("tree-v2:"));
    fs::write(repo.join("target.txt"), "after").unwrap();
    assert_ne!(before, snapshot(&repo, &["link.txt".into()]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_supports_sha256_git_object_format_when_available() {
    let root = temp();
    let result = Command::new("git")
        .args(["init", "-q", "--object-format=sha256"])
        .current_dir(&root)
        .output()
        .unwrap();
    if result.status.success() {
        fs::write(root.join("file.txt"), "value").unwrap();
        let identity = snapshot(&root, &[".".into()]);
        let parts: Vec<_> = identity.split(':').collect();
        assert_eq!(parts.len(), 3, "{identity}");
        assert_eq!(parts[0], "tree-v2");
        assert_eq!(parts[1].len(), 64);
        assert_eq!(parts[2].len(), 64);
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn scoped_ignored_inputs_use_windows_case_insensitive_path_matching() {
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("Src")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join(".gitignore"), "*\n").unwrap();
    fs::write(repo.join("Src/File.txt"), "before").unwrap();
    let before = snapshot(&repo, &["src/file.txt".into()]);
    assert!(before.starts_with("tree-v2:"), "{before}");
    fs::write(repo.join("Src/File.txt"), "after").unwrap();
    assert_ne!(before, snapshot(&repo, &["src/file.txt".into()]));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn snapshot_tracks_ignored_windows_named_streams() {
    let root = temp();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("ignored")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join(".gitignore"), "ignored/\n").unwrap();
    fs::write(repo.join("ignored/file.txt"), "base").unwrap();
    fs::write(repo.join("ignored/file.txt:資料"), "before").unwrap();
    let before = snapshot(&repo, &[".".into()]);
    assert!(before.starts_with("tree-v2:"), "{before}");
    fs::write(repo.join("ignored/file.txt:資料"), "after").unwrap();
    let after = snapshot(&repo, &[".".into()]);
    assert_ne!(before, after);
    fs::write(repo.join("ignored/file.txt:settings"), "settings-before").unwrap();
    let scoped = snapshot(&repo, &["ignored/file.txt:settings".into()]);
    assert!(scoped.starts_with("tree-v2:"), "{scoped}");
    fs::write(repo.join("ignored/file.txt:settings"), "settings-after").unwrap();
    assert_ne!(
        scoped,
        snapshot(&repo, &["ignored/file.txt:settings".into()])
    );
    let after = snapshot(&repo, &[".".into()]);
    fs::write(repo.join("ignored:目錄資料"), "directory-before").unwrap();
    let directory_before = snapshot(&repo, &[".".into()]);
    assert_ne!(after, directory_before);
    fs::write(repo.join("ignored:目錄資料"), "directory-after").unwrap();
    let directory_after = snapshot(&repo, &[".".into()]);
    assert_ne!(directory_before, directory_after);
    fs::remove_file(repo.join("ignored/file.txt:資料")).unwrap();
    assert_ne!(directory_after, snapshot(&repo, &[".".into()]));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn snapshot_rejects_escaping_junctions() {
    let root = temp();
    let repo = root.join("repo");
    let outside = root.join("outside");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&outside).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(repo.join("escape"))
        .arg(&outside)
        .status()
        .unwrap()
        .success());
    assert!(snapshot(&repo, &[".".into()]).is_empty());
    fs::write(outside.join("input.txt"), "outside-unhashed").unwrap();
    assert!(snapshot(&repo, &["escape/input.txt".into()]).is_empty());
    let oid = Command::new("git")
        .args([
            "hash-object",
            "--",
            outside.join("input.txt").to_str().unwrap(),
        ])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(oid.status.success());
    assert!(!Command::new("git")
        .args([
            "cat-file",
            "-e",
            String::from_utf8(oid.stdout).unwrap().trim()
        ])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn records_and_gates() {
    let p = temp();
    setup(&p);
    append_evidence(&p, "u", &row("run", "gpt-worker", 0, "", "2000-01-01")).unwrap();
    append_evidence(&p, "u", &row("verify", "test", 0, "", "2000-01-02")).unwrap();
    assert_eq!(
        record_review(&p, &["u".into()], "gpt-worker", true, &p)
            .unwrap_err()
            .kind,
        "same_executor"
    );
    assert_eq!(record_accept(&p, "m", &p).unwrap().exit_code, 0);
    assert_eq!(evidence(&p, "u").unwrap().last().unwrap().kind, "accept");
    assert_eq!(record_full(&p, &p, Some("exit 7")).unwrap().exit_code, 7);
    assert_eq!(record_full(&p, &p, None).unwrap().exit_code, 0);
    assert_eq!(done_gate(&p, false).unwrap_err().kind, "not_done");
    assert_eq!(astra_gate(&p).unwrap_err().kind, "astra_gate");
    record_stage(&p, "astra", "gpt-6-astra", true).unwrap();
    record_stage(&p, "second_review", "claude-review", false).unwrap();
    fs::create_dir_all(p.join("astra")).unwrap();
    for f in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(p.join("astra").join(f), "").unwrap();
    }
    assert_eq!(astra_gate(&p).unwrap_err().kind, "astra_gate");
    fs::write(p.join("astra/review.json"), "{} ").unwrap();
    assert_eq!(astra_gate(&p).unwrap_err().kind, "astra_gate");
    fs::write(p.join("astra/ruling.md"), "ruling").unwrap();
    assert_eq!(astra_gate(&p).unwrap_err().kind, "astra_gate");
    set_unit_state(&p, "u", "verified").unwrap();
    append_evidence(&p, "_full", &row("full", "test", 0, "", "1999")).unwrap();
    assert_eq!(done_gate(&p, false).unwrap_err().kind, "not_done");
    append_evidence(&p, "_full", &row("full", "test", 0, "", "9999")).unwrap();
    append_evidence(&p, "_stages", &row("rewrite", "x", 1, "", "9999")).unwrap();
    assert_eq!(done_gate(&p, false).unwrap_err().kind, "not_done");
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn review_identity_empty_when_brief_unreadable() {
    let p = temp();
    setup(&p);
    append_evidence(&p, "u", &row("run", "gpt-worker", 0, "", "1")).unwrap();
    append_evidence(&p, "u", &row("verify", "test", 0, "verified-tree", "2")).unwrap();
    fs::remove_file(p.join("brief.txt")).unwrap();
    record_review(&p, &["u".into()], "claude-review", true, &p).unwrap();
    assert_eq!(evidence(&p, "u").unwrap().last().unwrap().commit, "");
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn accept_checks_all_briefs_before_command() {
    let p = temp();
    setup(&p);
    let marker = p.join("marker");
    fs::write(p.join("modules.md"), "## m\nPURPOSE x\nPATHS file.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo executed > marker`\n").unwrap();
    let mut units = fs::read_to_string(p.join("units.tsv")).unwrap();
    units.push_str("v\tm\tpending\t\t\t\t\n");
    fs::write(p.join("units.tsv"), units).unwrap();
    assert_eq!(record_accept(&p, "m", &p).err().unwrap().kind, "no_brief");
    assert!(!marker.exists());
    assert!(evidence(&p, "u").unwrap().is_empty());
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn identities_are_current_git_trees() {
    let root = temp();
    let repo = root.join("repo");
    let store = root.join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    setup(&store);
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("file.txt"), "one").unwrap();
    append_evidence(&store, "u", &row("run", "gpt-worker", 0, "", "1")).unwrap();
    let verify = snapshot(&repo, &["file.txt".into()]);
    append_evidence(&store, "u", &row("verify", "test", 0, &verify, "2")).unwrap();
    record_accept(&store, "m", &repo).unwrap();
    record_full(&store, &repo, Some("echo ok")).unwrap();
    let full_path = store.join("reports/_full.evidence.tsv");
    let first_full = fs::read_to_string(&full_path)
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .split('\t')
        .nth(3)
        .unwrap()
        .to_string();
    fs::write(repo.join("file.txt"), "two").unwrap();
    record_review(&store, &["u".into()], "claude-review", true, &repo).unwrap();
    record_full(&store, &repo, Some("echo ok")).unwrap();
    let rows = evidence(&store, "u").unwrap();
    let review = &rows.last().unwrap().commit;
    assert_ne!(&verify, review);
    let full = fs::read_to_string(full_path).unwrap();
    let last_full = full.lines().last().unwrap().split('\t').nth(3).unwrap();
    assert_ne!(first_full, last_full);
    for identity in [
        &verify,
        &rows.iter().find(|r| r.kind == "accept").unwrap().commit,
        review,
        &first_full,
        &last_full.to_string(),
    ] {
        let hash = identity
            .strip_prefix("tree-v2:")
            .unwrap()
            .split(':')
            .next()
            .unwrap();
        assert_eq!(hash.len(), 40);
        let output = Command::new("git")
            .args(["cat-file", "-t", hash])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "tree");
    }
    fs::remove_dir_all(root).unwrap();
}
