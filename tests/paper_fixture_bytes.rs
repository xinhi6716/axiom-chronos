use std::process::Command;

fn lf_to_crlf_conversions(eol_output: &str) -> Vec<&str> {
    eol_output
        .lines()
        .filter(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some("i/lf") && fields.next() == Some("w/crlf")
        })
        .collect()
}

// Failing-first before -text: paper_fixtures_preserve_index_lf_bytes FAILED with i/lf w/crlf fixtures.
#[test]
fn paper_fixtures_preserve_index_lf_bytes() {
    let output = Command::new("git")
        .args(["ls-files", "--eol", "--", "tests/fixtures/paper"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run git ls-files --eol for paper fixtures");
    assert!(
        output.status.success(),
        "git ls-files --eol failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let eol_output = String::from_utf8(output.stdout).expect("UTF-8 git ls-files --eol output");
    assert!(!eol_output.trim().is_empty(), "no tracked paper fixtures");
    let conversions = lf_to_crlf_conversions(&eol_output);
    assert!(
        conversions.is_empty(),
        "paper fixtures with committed LF bytes were checked out as CRLF:\n{}",
        conversions.join("\n")
    );
}

#[test]
fn checker_flags_lf_to_crlf_conversion() {
    let line = "i/lf    w/crlf  attr/-text  x";
    assert_eq!(lf_to_crlf_conversions(line), vec![line]);
}
