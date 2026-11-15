use axiom::orch::{claims_columns, is_wish_id, parse_claim_row, run, wish_stores, DiskClaim};

#[test]
fn claim_headers() {
    let four = "unit\tattempt\tscope\tstarted";
    let eight = "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers";
    for (header, expected) in [(four, Some(4)), (eight, Some(8)), ("other", None)] {
        assert_eq!(claims_columns(header), expected);
        assert_eq!(claims_columns(&format!("{header}\r")), expected);
    }
}

#[test]
fn claim_rows() {
    let full = DiskClaim {
        unit: "u1".into(),
        attempt: "a1".into(),
        scope: vec!["src/a".into(), "src/b".into()],
        started: "now".into(),
        module: "core".into(),
        wish: "wish".into(),
        pid: "123".into(),
        peers: vec!["u2".into(), "u3".into()],
    };
    let line = "u1\ta1\t src/a   src/b \tnow\tcore\twish\t123\t u2  u3 ";
    assert_eq!(parse_claim_row(line, 8), Some(full.clone()));
    assert_eq!(parse_claim_row(&format!("{line}\r"), 8), Some(full));
    let old = DiskClaim {
        unit: "u".into(),
        attempt: "a".into(),
        scope: vec!["one".into(), "two".into()],
        started: "then".into(),
        module: String::new(),
        wish: String::new(),
        pid: String::new(),
        peers: vec![],
    };
    assert_eq!(parse_claim_row("u\ta\tone  two\tthen", 4), Some(old));
    assert_eq!(
        parse_claim_row("u\ta\t\t\t\t\t\t\r", 8),
        Some(DiskClaim {
            unit: "u".into(),
            attempt: "a".into(),
            scope: vec![],
            started: String::new(),
            module: String::new(),
            wish: String::new(),
            pid: String::new(),
            peers: vec![],
        })
    );
    for (line, columns) in [
        ("u\ta\ts", 4),
        ("u\ta\ts\tt\textra", 4),
        ("\ta\ts\tt", 4),
        ("u\t\ts\tt", 4),
        ("u\ta\ts", 3),
    ] {
        assert_eq!(parse_claim_row(line, columns), None);
    }
}

#[test]
fn wish_ids_and_store_directories() {
    let first = "w-1234567890123-abcdef";
    let second = "w-1234567890124-012345";
    let file = "w-1234567890125-012345";
    assert!(is_wish_id(first));
    for invalid in [
        "w-1234567890123-abcde",
        "w-1234567890123-abcdeF",
        "w-123456789012x-abcdef",
        "x-1234567890123-abcdef",
        "w-1234567890123_abcdef",
    ] {
        assert!(!is_wish_id(invalid), "{invalid}");
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join(second)).unwrap();
    std::fs::create_dir(root.join(first)).unwrap();
    std::fs::create_dir(root.join("scratch")).unwrap();
    std::fs::write(root.join(file), "").unwrap();
    assert_eq!(wish_stores(root), vec![root.join(first), root.join(second)]);
    let missing = root.join("missing");
    assert!(wish_stores(&missing).is_empty());
    assert!(!missing.exists());
}

#[test]
fn unknown_command_lists_new_commands() {
    let temp = tempfile::tempdir().unwrap();
    let out = run(temp.path(), &["help".into()], false, false);
    assert_eq!(out.exit_code, 1);
    for fragment in [
        "schedule next [--cursor N] [--timeout S] | claim <module> | release <module> --reason <text>",
        "activity start|finish --module <m> --actor core|dispatcher --phase brief|review|decide [--id <id>]",
        "setup start --repo <dir> | status | keep <id>... | cancel [<id>...]",
    ] {
        assert!(out.stderr.contains(fragment), "{}", out.stderr);
    }
}
