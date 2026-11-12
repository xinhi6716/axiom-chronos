use axiom::research_workspace::{
    funnel::{funnel, rows, sample, Decision, Funnel, Node},
    index::Index,
    manifest::{locate, LocateOptions},
    read::Limits,
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path, sync::Arc};

fn index(root: &Path) -> Index {
    Index::build(
        Arc::new(locate(root, &LocateOptions::default()).unwrap()),
        Limits::default(),
        &|_, _| {},
    )
    .unwrap()
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name)
}

fn project(inventory: &str, screening: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("research-manifest.json"), r#"{"manifest_version":1,"research_id":"funnel","research_version":"1","title":"Funnel","settings":{},"versions":{},"capabilities":["literature","screening"],"artifacts":[{"id":"inventory","role":"inventory","path":"inventory.csv","format":"csv","columns":{"path":"rel_path"}},{"id":"screening","role":"screening","path":"screening.csv","format":"csv","columns":{"sha256":"pdf_sha256","reason":"scope_reason"}}]}"#).unwrap();
    fs::write(
        dir.path().join("inventory.csv"),
        format!("rel_path,sha256,doi,title,duplicate_group,canonical,read_error\n{inventory}"),
    )
    .unwrap();
    fs::write(
        dir.path().join("screening.csv"),
        format!("pdf_sha256,verdict,scope_reason,pending_reason\n{screening}"),
    )
    .unwrap();
    dir
}

fn check_invariant(f: &Funnel) -> Result<(), String> {
    if f.deduplicated != f.screened_in + f.screened_out + f.pending + f.unprocessed + f.read_failed
    {
        return Err("deduplicated leaf counts mismatch".into());
    }
    if f.total != f.deduplicated + f.duplicates || f.read != f.screened_in {
        return Err("total or read counts mismatch".into());
    }
    Ok(())
}

fn verify_nodes(ix: &Index, f: &Funnel) {
    check_invariant(f).unwrap();
    let nodes = [
        (Node::Total, f.total),
        (Node::Deduplicated, f.deduplicated),
        (Node::Duplicates, f.duplicates),
        (Node::ScreenedIn, f.screened_in),
        (Node::ScreenedOut { reason: None }, f.screened_out),
        (Node::Pending, f.pending),
        (Node::Unprocessed, f.unprocessed),
        (Node::Read, f.read),
        (Node::ReadFailed { reason: None }, f.read_failed),
    ];
    for (node, count) in &nodes {
        let (full, total) = rows(ix, node, 0, usize::MAX).unwrap();
        assert_eq!((total, full.len()), (*count, *count), "{node:?}");
        let mut paged = Vec::new();
        for offset in (0..*count).step_by(3) {
            let (page, page_total) = rows(ix, node, offset, 3).unwrap();
            assert_eq!(page_total, *count);
            paged.extend(page);
        }
        assert_eq!(paged, full, "{node:?}");
        assert_eq!(rows(ix, node, usize::MAX, 3).unwrap(), (vec![], *count));
        assert_eq!(rows(ix, node, 0, 0).unwrap(), (vec![], *count));
    }
    let all: HashSet<_> = rows(ix, &Node::Total, 0, usize::MAX)
        .unwrap()
        .0
        .into_iter()
        .map(|r| r.record_id)
        .collect();
    let mut leaves = HashSet::new();
    for node in [
        Node::Duplicates,
        Node::ScreenedIn,
        Node::ScreenedOut { reason: None },
        Node::Pending,
        Node::Unprocessed,
        Node::ReadFailed { reason: None },
    ] {
        for row in rows(ix, &node, 0, usize::MAX).unwrap().0 {
            assert!(
                leaves.insert(row.record_id),
                "row occurs in multiple leaves"
            );
        }
    }
    assert_eq!(all.len(), f.total);
    assert_eq!(all, leaves);
    assert_eq!(
        rows(ix, &Node::Read, 0, usize::MAX).unwrap(),
        rows(ix, &Node::ScreenedIn, 0, usize::MAX).unwrap()
    );
}

#[test]
fn real_fixture_counts_pages_leaves_and_source_unchanged() {
    let root = fixture("rram/project");
    let path = root.join("data/literature_inventory.csv");
    let before = Sha256::digest(fs::read(&path).unwrap());
    let ix = index(&root);
    let f = funnel(&ix).unwrap();
    verify_nodes(&ix, &f);
    for (reason, count) in &f.out_reasons {
        assert_eq!(
            rows(
                &ix,
                &Node::ScreenedOut {
                    reason: Some(if reason == "(none)" {
                        "".into()
                    } else {
                        reason.clone()
                    })
                },
                0,
                usize::MAX
            )
            .unwrap()
            .1,
            *count
        );
    }
    for (reason, count) in &f.fail_reasons {
        assert_eq!(
            rows(
                &ix,
                &Node::ReadFailed {
                    reason: Some(reason.clone())
                },
                0,
                usize::MAX
            )
            .unwrap()
            .1,
            *count
        );
    }
    sample(&ix, &Node::Total, 10, 42).unwrap();
    assert_eq!(before, Sha256::digest(fs::read(path).unwrap()));
}

#[test]
fn synthetic_classification_and_reasons() {
    let dir = project(
        "a,h1,same,Title A,g,1,\nb,h2,same,Title B,g,1,\nc,h1,same,Duplicate,g,0,broken\nd,h3,d3,Pending,,1,\ne,h4,d4,Failed,,1,bad pdf\nf,h5,d5,Unprocessed,,1,\ng,h6,d6,Excluded,,1,\nh,h7,d7,Excluded blank,,1,\ni,h8,d8,Excluded too,,1,\nj,h9,d9,Another fail,,1,z error\n",
        "h1,include,,\nh2,exclude,scope,\nh3,pending,,needs review\nh4,include,,\nh6,exclude,other,\nh7,exclude,,\nh8,exclude,scope,\n"
    );
    let ix = index(dir.path());
    let f = funnel(&ix).unwrap();
    assert_eq!(
        (
            f.total,
            f.deduplicated,
            f.duplicates,
            f.screened_in,
            f.screened_out,
            f.pending,
            f.unprocessed,
            f.read,
            f.read_failed
        ),
        (10, 9, 1, 1, 4, 1, 1, 1, 2)
    );
    assert_eq!(
        f.out_reasons,
        vec![
            ("scope".into(), 2),
            ("(none)".into(), 1),
            ("other".into(), 1)
        ]
    );
    assert_eq!(
        f.fail_reasons,
        vec![("bad pdf".into(), 1), ("z error".into(), 1)]
    );
    verify_nodes(&ix, &f);
    let all = rows(&ix, &Node::Total, 0, usize::MAX).unwrap().0;
    let expected = [
        Decision::Included,
        Decision::Excluded {
            reason: "scope".into(),
        },
        Decision::Duplicate {
            of_group: "g".into(),
        },
        Decision::Pending {
            reason: "needs review".into(),
        },
        Decision::Failed {
            reason: "bad pdf".into(),
        },
        Decision::Unprocessed,
        Decision::Excluded {
            reason: "other".into(),
        },
        Decision::Excluded { reason: "".into() },
        Decision::Excluded {
            reason: "scope".into(),
        },
        Decision::Failed {
            reason: "z error".into(),
        },
    ];
    assert_eq!(
        all.iter().map(|r| r.decision.clone()).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        (
            &all[0].path,
            &all[0].sha256,
            &all[0].doi,
            &all[0].title,
            &all[0].duplicate_group,
            &all[0].record_id
        ),
        (
            &"a".into(),
            &"h1".into(),
            &"same".into(),
            &"Title A".into(),
            &"g".into(),
            &"inventory:a".into()
        )
    );
    for (node, expected_paths) in [
        (
            Node::ScreenedOut {
                reason: Some("scope".into()),
            },
            vec!["b", "i"],
        ),
        (
            Node::ScreenedOut {
                reason: Some("".into()),
            },
            vec!["h"],
        ),
        (
            Node::ReadFailed {
                reason: Some("bad pdf".into()),
            },
            vec!["e"],
        ),
        (
            Node::ReadFailed {
                reason: Some("missing".into()),
            },
            vec![],
        ),
    ] {
        let (page, count) = rows(&ix, &node, 0, usize::MAX).unwrap();
        assert_eq!(count, expected_paths.len());
        assert_eq!(
            page.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            expected_paths
        );
    }
    let mut broken = f.clone();
    broken.screened_in += broken.read_failed;
    assert!(check_invariant(&broken).is_err());
}

#[test]
fn no_literature_is_not_capable() {
    let ix = index(&fixture("rawonly"));
    assert_eq!(funnel(&ix).unwrap_err().kind, "not_capable");
    assert_eq!(
        rows(&ix, &Node::Total, 0, 1).unwrap_err().kind,
        "not_capable"
    );
    assert_eq!(
        sample(&ix, &Node::Total, 1, 0).unwrap_err().kind,
        "not_capable"
    );
}

#[test]
fn keys_round_trip() {
    for node in [
        Node::Total,
        Node::Deduplicated,
        Node::Duplicates,
        Node::ScreenedIn,
        Node::ScreenedOut { reason: None },
        Node::ScreenedOut {
            reason: Some("".into()),
        },
        Node::ScreenedOut {
            reason: Some("a:b / 中文".into()),
        },
        Node::Pending,
        Node::Unprocessed,
        Node::Read,
        Node::ReadFailed { reason: None },
        Node::ReadFailed {
            reason: Some("".into()),
        },
        Node::ReadFailed {
            reason: Some("error:detail".into()),
        },
    ] {
        assert_eq!(Node::from_key(&node.key()), Some(node));
    }
    assert_eq!(Node::from_key("garbage"), None);
}

#[test]
fn sampling_is_deterministic_distinct_and_clamped() {
    let inventory: String = (0..64)
        .map(|i| format!("p{i},h{i},d{i},title,,1,\n"))
        .collect();
    let dir = project(&inventory, "");
    let ix = index(dir.path());
    let node = Node::Unprocessed;
    let first = sample(&ix, &node, 100, 123).unwrap();
    assert_eq!(first, sample(&ix, &node, 100, 123).unwrap());
    assert_ne!(first, sample(&ix, &node, 100, 124).unwrap());
    assert_eq!(first.len(), 64);
    let members: HashSet<_> = rows(&ix, &node, 0, usize::MAX)
        .unwrap()
        .0
        .into_iter()
        .map(|r| r.record_id)
        .collect();
    assert_eq!(first.iter().cloned().collect::<HashSet<_>>(), members);
    let partial = sample(&ix, &node, 7, 123).unwrap();
    assert_eq!(partial.len(), 7);
    assert_eq!(partial.iter().collect::<HashSet<_>>().len(), 7);
    assert!(partial.iter().all(|id| members.contains(id)));
    assert!(sample(&ix, &node, 0, 123).unwrap().is_empty());
}
