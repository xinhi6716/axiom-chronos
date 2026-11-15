use axiom::laya::{
    curate::*,
    router::{DecisionKind, DecisionRecord},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    sync::Mutex,
};

static ENV: Mutex<()> = Mutex::new(());
struct EnvGuard(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in [("USERNAME", &self.0), ("USER", &self.1)] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
fn env_guard() -> EnvGuard {
    let guard = EnvGuard(std::env::var_os("USERNAME"), std::env::var_os("USER"));
    std::env::set_var("USERNAME", "EnvPerson");
    std::env::set_var("USER", "DifferentUser");
    guard
}
fn rec(id: &str, wish: Option<&str>, context: &str, ts: i64) -> DecisionRecord {
    DecisionRecord {
        schema: 1,
        id: id.into(),
        ts_ms: ts,
        kind: DecisionKind::Triage,
        wish_id: wish.map(str::to_owned),
        subject: "subject".into(),
        context: context.into(),
        claude: "minor".into(),
        laya: None,
        laya_error: None,
        policy_version: "v".into(),
        mode: "shadow".into(),
        executed: "claude".into(),
    }
}
fn outcome(id: &str, label: &str, ts: i64) -> Outcome {
    Outcome {
        record_id: id.into(),
        label: label.into(),
        verified_by: "unit_verified".into(),
        ts_ms: ts,
    }
}
struct Source {
    name: &'static str,
    rows: Vec<Outcome>,
}
impl OutcomeSource for Source {
    fn name(&self) -> String {
        self.name.into()
    }
    fn outcomes(&self) -> axiom::base::Res<Vec<Outcome>> {
        Ok(self.rows.clone())
    }
}
fn split(seed: u64, wish: &str) -> Split {
    let h = Sha256::digest(format!("{seed}:{wish}").as_bytes());
    let x = u64::from_be_bytes(h[..8].try_into().unwrap()) as f64 / 18446744073709551616.0;
    if x < 0.6 {
        Split::Train
    } else if x < 0.75 {
        Split::Calibration
    } else if x < 0.9 {
        Split::Dev
    } else {
        Split::Acceptance
    }
}

#[test]
fn scrubs_sensitive_families_and_preserves_text() {
    let _lock = ENV.lock().unwrap();
    let _env = env_guard();
    let rules = ScrubRules {
        user_names: vec!["Alice".into(), "x".into()],
    };
    for (input, expected) in [
        (r"C:\Users\Bob\doc.txt", "<path>"),
        (r"\\server\share\file.txt", "<path>"),
        ("/home/bob/file", "<path>"),
        ("Hello ALICE and EnvPerson", "Hello <user> and <user>"),
        ("a.person@example.org", "<email>"),
        ("sk-abcdefgh1234", "<secret>"),
        ("ghp_abcdefgh1234", "<secret>"),
        ("github_pat_abcdefgh1234", "<secret>"),
        ("AKIA123456789ABCDE", "<secret>"),
        ("xoxb-1234-5678", "<secret>"),
        (
            "-----BEGIN PRIVATE KEY-----\nabcdef\n-----END PRIVATE KEY-----",
            "<secret>",
        ),
        ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc123xyz", "<secret>"),
        ("password=hunter2", "password=<secret>"),
        ("api_key: abcde", "api_key: <secret>"),
        ("abcdef0123456789abcdef0123456789", "<secret>"),
        ("QWxhZGRpbjpvcGVuIHNlc2FtZTIzNDU2==", "<secret>"),
        (
            "https://host/x and/or x short",
            "https://host/x and/or x short",
        ),
    ] {
        assert_eq!(scrub(input, &rules), expected, "{input}");
    }
    assert_eq!(
        scrub("UnlistedPerson", &ScrubRules::default()),
        "UnlistedPerson"
    );
}

#[test]
fn outcome_persistence_filter_and_concurrency() {
    let home = tempfile::tempdir().unwrap();
    let source = VerifiedOutcomeSource {
        home: home.path().into(),
    };
    assert_eq!(source.name(), "verified_outcomes");
    assert!(source.outcomes().unwrap().is_empty());
    let mut bad = outcome("bad", "major", 0);
    bad.verified_by = "untrusted".into();
    assert!(record_outcome(home.path(), &bad).is_err());
    std::thread::scope(|s| {
        for t in 0..2 {
            let home = home.path();
            s.spawn(move || {
                for i in 0..40 {
                    record_outcome(home, &outcome(&format!("{t}-{i}"), "major", i)).unwrap();
                }
            });
        }
    });
    let path = axiom::laya::laya_dir(home.path()).join("outcomes.jsonl");
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&bad).unwrap()).unwrap();
    writeln!(
        file,
        "{}",
        serde_json::to_string(&outcome("unknown", "invalid", 1)).unwrap()
    )
    .unwrap();
    writeln!(file, "not json").unwrap();
    let found = source.outcomes().unwrap();
    assert_eq!(found.len(), 80);
    assert_eq!(
        found
            .iter()
            .map(|o| &o.record_id)
            .collect::<BTreeSet<_>>()
            .len(),
        80
    );
}

#[test]
fn end_to_end_dedup_split_balance_and_output() {
    let _lock = ENV.lock().unwrap();
    let _env = env_guard();
    let seed = 73;
    let mut records = Vec::new();
    let mut outcomes = Vec::new();
    let mut baseline = BTreeMap::<Split, BTreeMap<String, usize>>::new();
    for i in 0..160 {
        let wish = format!("private-wish-{i}");
        let id = format!("rec-{i:03}");
        let label = if i % 4 == 0 { "major" } else { "minor" };
        let mut r = rec(
            &id,
            Some(&wish),
            &format!("unique {i} Alice C:\\private\\data sk-abcdefghi123"),
            i as i64,
        );
        r.claude = "Alice password=hiddenvalue".into();
        records.push(r);
        outcomes.push(outcome(&id, label, 10));
        *baseline
            .entry(split(seed, &wish))
            .or_default()
            .entry(label.into())
            .or_default() += 1;
    }
    records.push(rec("unverified", None, "no outcome", 200));
    records.push(rec("no-wish", None, "missing wish", 201));
    outcomes.push(outcome("no-wish", "major", 0));
    records.push(rec("bad-label", Some("bad-wish"), "bad label", 202));
    outcomes.push(outcome("bad-label", "terra", 0));
    for (id, ts, label) in [
        ("dup-first", 210, "major"),
        ("dup-later", 211, "major"),
        ("conflict-a", 212, "major"),
        ("conflict-b", 213, "minor"),
        ("conflict-c", 214, "minor"),
    ] {
        let context = if id.starts_with("dup") {
            "duplicate context"
        } else {
            "conflict context"
        };
        records.push(rec(id, Some(id), context, ts));
        outcomes.push(outcome(id, label, 1));
    }
    let source = Source {
        name: "custom",
        rows: outcomes,
    };
    let mut spec = CurateSpec::new(seed);
    spec.scrub.user_names.push("Alice".into());
    let result = curate(&records, &[&source], &spec).unwrap();
    assert_eq!(
        result.datasheet.excluded,
        BTreeMap::from([
            ("unverified".into(), 1),
            ("no_wish".into(), 1),
            ("unknown_label".into(), 1),
            ("duplicate".into(), 1),
            ("conflict".into(), 3)
        ])
    );
    let all: Vec<_> = result.splits.values().flatten().collect();
    assert!(!all
        .iter()
        .any(|r| r.meta.record == "dup-later" || r.meta.record.starts_with("conflict")));
    assert!(
        all.iter().any(|r| r.meta.record == "dup-first")
            || split(seed, "dup-first") == Split::Train
    );
    let mut wishes = BTreeMap::new();
    for (s, rows) in &result.splits {
        assert!(!rows.is_empty());
        for row in rows {
            assert!(wishes
                .insert(row.meta.wish.clone(), *s)
                .map_or(true, |old| old == *s));
            assert_eq!(
                row.questions["triage"]["criteria"],
                serde_json::json!(["blocker", "major", "minor"])
            );
            assert_eq!(
                row.questions["triage"]["instructions"],
                DecisionKind::Triage.instructions()
            );
            assert_eq!(row.questions["triage"]["type"], "choice");
        }
    }
    let train = &result.splits[&Split::Train];
    let counts = |rows: &Vec<Row>, label: &str| {
        rows.iter()
            .filter(|r| r.expected["triage"] == label)
            .count()
    };
    assert_eq!(counts(train, "major"), counts(train, "minor"));
    for s in [Split::Calibration, Split::Dev, Split::Acceptance] {
        for label in ["major", "minor"] {
            let extra = usize::from(split(seed, "dup-first") == s && label == "major");
            assert_eq!(
                counts(&result.splits[&s], label),
                baseline[&s].get(label).copied().unwrap_or(0) + extra
            );
        }
    }
    assert_eq!(result.datasheet.per_source["custom"] as usize, all.len());
    for (s, rows) in &result.splits {
        let name = match s {
            Split::Train => "train",
            Split::Calibration => "calibration",
            Split::Dev => "dev",
            Split::Acceptance => "acceptance",
        };
        assert_eq!(result.datasheet.per_split[name] as usize, rows.len());
        for row in rows {
            let label = row.expected["triage"].as_str().unwrap();
            assert!(result.datasheet.per_class[&format!("triage:{label}")][name] > 0);
        }
    }
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let files = write(&result, dir1.path()).unwrap();
    let files2 = write(&curate(&records, &[&source], &spec).unwrap(), dir2.path()).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "train.jsonl",
            "calibration.jsonl",
            "dev.jsonl",
            "acceptance.jsonl",
            "datasheet.json",
            "datasheet.md"
        ]
    );
    for (a, b) in files.iter().zip(&files2) {
        let bytes = fs::read(a).unwrap();
        assert_eq!(bytes, fs::read(b).unwrap());
        let contents = String::from_utf8(bytes).unwrap();
        for secret in [
            "private-wish-",
            "Alice",
            "C:\\private",
            "sk-abcdefghi123",
            "hiddenvalue",
        ] {
            assert!(!contents.contains(secret), "{} leaked in {:?}", secret, a);
        }
    }
    let mut other = CurateSpec::new(seed + 1);
    other.scrub = spec.scrub.clone();
    let changed = curate(&records, &[&source], &other).unwrap();
    let assignments = |c: &Curated| {
        c.splits
            .iter()
            .flat_map(|(s, rs)| rs.iter().map(move |r| (r.meta.record.clone(), *s)))
            .collect::<BTreeMap<_, _>>()
    };
    let original = assignments(&result);
    let newer = assignments(&changed);
    assert!(original
        .iter()
        .any(|(id, s)| newer.get(id).is_some_and(|s2| s2 != s)));
}

#[test]
fn joins_latest_outcome_and_source_tie_and_earliest_duplicate() {
    let records = vec![
        rec("b", Some("w-b"), "same", 2),
        rec("a", Some("w-a"), "same", 1),
        rec("c", Some("w-c"), "other", 3),
    ];
    let a = Source {
        name: "first",
        rows: vec![
            outcome("a", "minor", 10),
            outcome("b", "major", 1),
            outcome("c", "major", 20),
        ],
    };
    let b = Source {
        name: "second",
        rows: vec![outcome("a", "major", 10), outcome("c", "minor", 19)],
    };
    let mut spec = CurateSpec::new(1);
    spec.fractions = [0.0, 0.0, 0.0, 1.0];
    let result = curate(&records, &[&a, &b], &spec).unwrap();
    let rows = &result.splits[&Split::Acceptance];
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].meta.record, "a");
    assert_eq!(rows[0].meta.source, "second");
    assert_eq!(rows[0].expected["triage"], "major");
    assert_eq!(rows[1].meta.source, "first");
    assert_eq!(rows[1].expected["triage"], "major");
    assert_eq!(result.datasheet.excluded["duplicate"], 1);
}

#[test]
fn wish_grouping_and_deterministic_training_selection() {
    let seed = 13;
    let wishes: Vec<String> = (0..100).map(|i| format!("group-{i}")).collect();
    let mut records = Vec::new();
    let mut outcomes = Vec::new();
    for (i, wish) in wishes.iter().enumerate() {
        for j in 0..2 {
            let id = format!("{i:03}-{j}");
            let label = if i % 5 == 0 { "major" } else { "minor" };
            records.push(rec(
                &id,
                Some(wish),
                &format!("state {i} {j}"),
                (i * 2 + j) as i64,
            ));
            outcomes.push(outcome(&id, label, 1));
        }
    }
    let source = Source {
        name: "source",
        rows: outcomes,
    };
    let result = curate(&records, &[&source], &CurateSpec::new(seed)).unwrap();
    let mut seen = BTreeMap::new();
    for (s, rows) in &result.splits {
        for row in rows {
            if let Some(previous) = seen.insert(row.meta.wish.clone(), *s) {
                assert_eq!(previous, *s);
            }
        }
    }
    assert!(seen.len() >= 40);
    assert_eq!(result.splits.values().filter(|v| !v.is_empty()).count(), 4);
    let train_candidates = records
        .iter()
        .filter(|r| split(seed, r.wish_id.as_deref().unwrap()) == Split::Train);
    let mut by_class: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for r in train_candidates {
        let i: usize = r.id[..3].parse().unwrap();
        let label = if i % 5 == 0 { "major" } else { "minor" };
        by_class.entry(label).or_default().push(r);
    }
    let n = by_class.values().map(Vec::len).min().unwrap();
    let mut expected = BTreeSet::new();
    for rs in by_class.values_mut() {
        rs.sort_by_key(|r| Sha256::digest(format!("{seed}:{}", r.id).as_bytes()).to_vec());
        expected.extend(rs.iter().take(n).map(|r| r.id.clone()));
    }
    let actual = result.splits[&Split::Train]
        .iter()
        .map(|r| r.meta.record.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    for rows in result.splits.values() {
        let indices: Vec<_> = rows
            .iter()
            .map(|r| {
                records
                    .iter()
                    .find(|x| x.id == r.meta.record)
                    .unwrap()
                    .ts_ms
            })
            .collect();
        assert!(indices.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
