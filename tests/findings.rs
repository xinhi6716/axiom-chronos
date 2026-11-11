use axiom::orch::*;
use std::{collections::BTreeSet, fs};

fn finding(severity: Severity, check: &str, needs: Option<Needs>) -> Finding {
    Finding {
        number: 1,
        check: check.into(),
        severity,
        location: String::new(),
        evidence: String::new(),
        fix: String::new(),
        needs,
    }
}

fn unit(id: &str, state: &str) -> Unit {
    Unit {
        id: id.into(),
        state: state.into(),
        track: String::new(),
        branch: String::new(),
        pr: String::new(),
        sha: String::new(),
        brief: String::new(),
    }
}

fn row(kind: &str, command: &str, commit: &str, exit_code: i32) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code,
        commit: commit.into(),
        ts: String::new(),
    }
}

#[test]
fn parsing_errors_are_ordered_and_descriptive() {
    let invalid = "{";
    let error = parse_verdict(invalid).unwrap_err();
    assert_eq!(error.kind, "invalid_argument");
    assert_eq!(
        error.message,
        serde_json::from_str::<serde_json::Value>(invalid)
            .unwrap_err()
            .to_string()
    );
    for (text, message) in [
        (r#"{}"#, "missing verdict"),
        (r#"{"verdict":"pass"}"#, "missing findings"),
        (r#"{"verdict":"pass","findings":{}}"#, "missing findings"),
        (r#"{"verdict":"pass","findings":[]}"#, "missing unit"),
        (
            r#"{"verdict":"pass","findings":[],"unit":"u"}"#,
            "missing tree",
        ),
        (
            r#"{"verdict":null,"findings":[],"unit":"u","tree":"t"}"#,
            "missing verdict",
        ),
    ] {
        let error = parse_verdict(text).unwrap_err();
        assert_eq!(
            (error.kind.as_str(), error.message.as_str()),
            ("invalid_argument", message)
        );
    }
    // Required fields take precedence over an invalid finding.
    assert_eq!(
        parse_verdict(r#"{"verdict":"fail","findings":[{"needs":"bad"}]}"#)
            .unwrap_err()
            .message,
        "missing unit"
    );
    for bad in [r#""Smoke""#, "null", "42", r#""other""#] {
        let text = format!(
            r#"{{"verdict":"fail","unit":"u","tree":"t","findings":[{{}},{{"needs":{bad}}}]}}"#
        );
        let error = parse_verdict(&text).unwrap_err();
        assert_eq!(error.kind, "invalid_argument");
        assert!(
            error.message.contains("2") && error.message.contains(bad),
            "{}",
            error.message
        );
    }
}

#[test]
fn parse_findings_defaults_severity_and_needs() {
    let v = parse_verdict(r#"{"unit":"u","tree":"t","verdict":"fail","findings":[{"severity":"bLoCkEr","check":"untested","needs":"smoke","location":"here","evidence":"proof","fix":"change"},{"severity":"MAJOR","needs":"manual"},{"severity":"critical"},{}]}"#).unwrap();
    assert_eq!(
        (v.unit.as_str(), v.tree.as_str(), v.verdict.as_str()),
        ("u", "t", "fail")
    );
    assert_eq!(
        v.findings.iter().map(|f| &f.severity).collect::<Vec<_>>(),
        vec![
            &Severity::Blocker,
            &Severity::Major,
            &Severity::Minor,
            &Severity::Minor
        ]
    );
    assert_eq!(
        v.findings.iter().map(|f| &f.needs).collect::<Vec<_>>(),
        vec![&Some(Needs::Smoke), &Some(Needs::Manual), &None, &None]
    );
    assert_eq!(
        v.findings.iter().map(|f| f.number).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        (
            &*v.findings[0].location,
            &*v.findings[0].evidence,
            &*v.findings[0].fix
        ),
        ("here", "proof", "change")
    );
    assert_eq!(
        (
            &*v.findings[3].check,
            &*v.findings[3].location,
            &*v.findings[3].evidence,
            &*v.findings[3].fix
        ),
        ("", "", "", "")
    );
}

#[test]
fn finding_rules_and_passing() {
    let serious = finding(Severity::Major, "tested", Some(Needs::Smoke));
    assert!(serious.is_serious());
    assert!(!serious.needs_check());
    assert!(serious.fails());
    let untested = finding(Severity::Blocker, "untested", Some(Needs::Manual));
    assert!(untested.is_serious() && untested.needs_check() && !untested.fails());
    assert!(finding(Severity::Blocker, "untested", None).fails());
    let minor = finding(Severity::Minor, "tested", None);
    assert!(!minor.is_serious() && !minor.needs_check() && !minor.fails());
    let mut v = Verdict {
        unit: "u".into(),
        tree: "t".into(),
        verdict: "PASS".into(),
        findings: vec![serious],
    };
    assert!(!passes(&v, &BTreeSet::new()));
    assert!(!passes(&v, &BTreeSet::from([2])));
    assert!(passes(&v, &BTreeSet::from([1])));
    v.verdict = "fail".into();
    assert!(passes(&v, &BTreeSet::from([1])));
    v.findings = vec![untested];
    assert!(passes(&v, &BTreeSet::new()));
    v.findings = vec![minor];
    assert!(!passes(&v, &BTreeSet::from([1])));
    v.verdict = "pass".into();
    assert!(passes(&v, &BTreeSet::new()));
}

#[test]
fn waiver_rows_filter_and_round_trip() {
    assert!(waivers(&[row("waive", "1 reason", "t", 0)]).is_empty());
    let rows = vec![
        row("review", "", "t", 1),
        row("waive", "1 old", "t", 0),
        row("review", "", "t", 1),
        row("waive", "2 wrong tree", "other", 0),
        row("waive", "nope reason", "t", 0),
        row("waive", "2", "t", 0),
        row("waive", "2   ", "t", 0),
        row("waive", "0 reason", "t", 0),
        row("other", "3 reason", "t", 0),
        row("waive", "3 valid reason", "t", 1),
        row("waive", "4 closed", "t", 0),
    ];
    assert_eq!(
        waivers(&rows),
        vec![
            Waiver {
                number: 3,
                reason: "valid reason".into(),
                tree: "t".into(),
                closes: false
            },
            Waiver {
                number: 4,
                reason: "closed".into(),
                tree: "t".into(),
                closes: true
            }
        ]
    );
    for closes in [true, false] {
        let waiver = waive_row(5, "accepted risk", "tree:abc", closes);
        assert_eq!(
            (
                &*waiver.kind,
                &*waiver.command,
                &*waiver.commit,
                waiver.exit_code
            ),
            (
                "waive",
                "5 accepted risk",
                "tree:abc",
                if closes { 0 } else { 1 }
            )
        );
        assert!(!waiver.ts.is_empty());
        assert_eq!(
            waivers(&[row("review", "", "tree:abc", 1), waiver]),
            vec![Waiver {
                number: 5,
                reason: "accepted risk".into(),
                tree: "tree:abc".into(),
                closes
            }]
        );
    }
}

#[test]
fn classification_follows_standing_chain_and_precedence() {
    let f = finding(Severity::Major, "tested", None);
    let a = unit("a", "superseded:b");
    let b = unit("b", "verified");
    assert_eq!(
        classify(&[a.clone(), b.clone()], &a, &f, &[]),
        Class::Answered
    );
    let b_next = unit("b", "superseded:c");
    let c = unit("c", "verified");
    assert_eq!(
        classify(&[a.clone(), b_next.clone(), c], &a, &f, &[]),
        Class::Answered
    );
    assert_eq!(
        classify(
            &[a.clone(), b_next.clone(), unit("c", "built")],
            &a,
            &f,
            &[]
        ),
        Class::Open
    );
    assert_eq!(classify(&[a.clone(), b_next], &a, &f, &[]), Class::Open);
    assert_eq!(
        classify(&[a.clone(), unit("b", "superseded:a")], &a, &f, &[]),
        Class::Open
    );
    assert_eq!(classify(&[b.clone()], &b, &f, &[]), Class::Resolved);
    let pending = unit("b", "built");
    assert_eq!(classify(&[pending.clone()], &pending, &f, &[]), Class::Open);
    let untested = finding(Severity::Major, "untested", Some(Needs::Smoke));
    let waiver = Waiver {
        number: 1,
        reason: "accepted".into(),
        tree: "t".into(),
        closes: false,
    };
    assert_eq!(
        classify(&[], &a, &untested, &[waiver]),
        Class::Ruled("accepted".into())
    );
    assert_eq!(classify(&[], &a, &untested, &[]), Class::NeedsCheck);
    assert_eq!(
        classify(&[], &a, &finding(Severity::Minor, "tested", None), &[]),
        Class::Minor
    );
}

#[test]
fn verdict_file_reading() {
    let dir = tempfile::tempdir().unwrap();
    let path = verdict_path(dir.path(), "u");
    assert_eq!(path, dir.path().join("reports/u.verdict.json"));
    assert_eq!(read_verdict(dir.path(), "u").unwrap(), None);
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        r#"{"unit":"u","tree":"t","verdict":"pass","findings":[]}"#,
    )
    .unwrap();
    assert_eq!(
        read_verdict(dir.path(), "u").unwrap().unwrap().verdict,
        "pass"
    );
    fs::write(&path, "invalid").unwrap();
    assert_eq!(
        read_verdict(dir.path(), "u").unwrap_err().kind,
        "invalid_argument"
    );
}
