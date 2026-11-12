use regex::Regex;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

fn check(source: &str, ledger: &str) -> Result<(), String> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let hash = format!("{:x}", Sha256::digest(normalized.as_bytes()));
    let last = ledger.lines().last().ok_or("empty ledger")?;
    let (expected, reason) = last.split_once(' ').ok_or("invalid ledger entry")?;
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || reason.trim().is_empty()
    {
        return Err("invalid ledger entry".into());
    }
    if hash != expected {
        return Err(format!("acceptance hash mismatch: {hash} != {expected}"));
    }
    let re = Regex::new(
        r"(?m)#\s*\[\s*test\s*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )
    .unwrap();
    let names: Vec<_> = re
        .captures_iter(&normalized)
        .map(|m| m[1].to_owned())
        .collect();
    let expected: BTreeSet<_> = [
        "needs_manual_wish_finishes_through_adjudicator",
        "approval_runs_exactly_once",
        "interjection_goes_sent_received_applied",
        "rejected_major_finding_spawns_no_fix_unit",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let attributes = Regex::new(r"#\s*\[\s*test\s*\]").unwrap();
    if attributes.find_iter(&normalized).count() != 4
        || names.len() != 4
        || names.into_iter().collect::<BTreeSet<_>>() != expected
    {
        return Err("acceptance test names differ from contract".into());
    }
    Ok(())
}

#[test]
fn acceptance_is_pinned_and_has_exact_test_names() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = fs::read_to_string(root.join("tests/phase2b_acceptance.rs")).unwrap();
    let ledger = fs::read_to_string(root.join("tests/fixtures/phase2b/acceptance.sha256")).unwrap();
    assert!(
        check(&format!("{source}\n// drift"), &ledger).is_err(),
        "negative control did not detect drift"
    );
    check(&source, &ledger).unwrap();
}
