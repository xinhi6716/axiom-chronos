use crate::common;
use axiom::codex::{
    assess_version, parse_version, probe_version_within, TestedVersions, Verdict, Version,
    TESTED_CODEX,
};
use std::time::{Duration, Instant};

const RANGE: TestedVersions = TestedVersions {
    minimum: Version::new(1, 2, 3),
    last_tested: Version::new(1, 4, 0),
};

#[test]
fn table_is_a_real_range() {
    assert!(TESTED_CODEX.minimum <= TESTED_CODEX.last_tested);
}

#[test]
fn reads_the_real_output_format() {
    let (text, version) = parse_version("codex-cli 0.159.3\n").unwrap();
    assert_eq!(text, "0.159.3");
    assert_eq!(version, Version::new(0, 159, 3));
    for output in [
        "codex-cli 0.159.3\r\n",
        "WARNING: could not update PATH\ncodex-cli 0.159.3\n",
        "codex-cli 0.159.3\nupdate available\n",
    ] {
        assert_eq!(parse_version(output).unwrap().0, "0.159.3", "{output:?}");
    }
    let (text, version) = parse_version("codex-cli 0.160.0-alpha.1").unwrap();
    assert_eq!(text, "0.160.0-alpha.1");
    assert!(version < Version::new(0, 160, 0));
    assert!(version > Version::new(0, 159, 3));
    assert_eq!(
        parse_version("codex-cli 0.160.0+build.7").unwrap().1,
        Version::new(0, 160, 0)
    );
}

#[test]
fn rejects_anything_that_is_not_a_codex_version() {
    for output in [
        "",
        "codex-cli",
        "codex-cli fake",
        "codex-cli 0.159",
        "codex-cli 0.159.3.1",
        "codex-cli 0.159.x",
        "codex-cli -1.2.3",
        "codex-cli 0.159.3 extra",
        "2.1.281 (Claude Code)",
        "xcodex-cli 0.159.3",
        "codex-cli 99999999999999999999.0.0",
    ] {
        assert!(parse_version(output).is_none(), "{output:?}");
    }
}

#[test]
fn verdicts_follow_the_range_boundaries() {
    for (version, verdict) in [
        ("0.99.99", Verdict::Fail),
        ("1.2.2", Verdict::Fail),
        ("1.2.3-alpha.1", Verdict::Fail),
        ("1.2.3", Verdict::Ok),
        ("1.2.10", Verdict::Ok),
        ("1.3.99", Verdict::Ok),
        ("1.4.0-rc.1", Verdict::Ok),
        ("1.4.0", Verdict::Ok),
        ("1.4.0+build", Verdict::Ok),
        ("1.4.1", Verdict::Warn),
        ("1.5.0-alpha.1", Verdict::Warn),
        ("1.10.0", Verdict::Warn),
        ("2.0.0", Verdict::Warn),
    ] {
        let check = assess_version(0, &format!("codex-cli {version}\n"), &RANGE);
        assert_eq!(check.verdict, verdict, "{version}");
        assert_eq!(check.installed.as_deref(), Some(version));
        assert_eq!(check.minimum, "1.2.3");
        assert_eq!(check.last_tested, "1.4.0");
        assert!(check.message.contains(version), "{}", check.message);
    }
}

#[test]
fn messages_say_what_to_expect() {
    let newer = assess_version(0, "codex-cli 1.5.0", &RANGE);
    assert!(
        newer.message.contains("newer than 1.4.0"),
        "{}",
        newer.message
    );
    assert!(newer.message.contains("not blocked"), "{}", newer.message);
    let older = assess_version(0, "codex-cli 1.2.2", &RANGE);
    assert!(
        older.message.contains("older than 1.2.3"),
        "{}",
        older.message
    );
    assert!(
        older.message.contains("last tested 1.4.0"),
        "{}",
        older.message
    );
}

#[test]
fn unreadable_or_failing_probes_fail_without_an_installed_version() {
    let garbled = assess_version(0, "codex-cli fake\n", &RANGE);
    assert_eq!(garbled.verdict, Verdict::Fail);
    assert_eq!(garbled.installed, None);
    assert!(
        garbled.message.contains("codex-cli fake"),
        "{}",
        garbled.message
    );
    assert!(garbled.message.contains("1.2.3"), "{}", garbled.message);
    let crashed = assess_version(3, "codex-cli 1.3.0\nboom\n", &RANGE);
    assert_eq!(crashed.verdict, Verdict::Fail);
    assert_eq!(crashed.installed, None);
    assert!(
        crashed.message.contains("exited with 3"),
        "{}",
        crashed.message
    );
    let long = "x".repeat(5000);
    assert!(assess_version(0, &long, &RANGE).message.len() < 400);
}

#[test]
fn probe_reads_the_fake_and_keeps_the_raw_line() {
    let probe = probe_version_within(
        &common::fake_codex(&["--fake-version=codex-cli 999.1.2"]),
        Duration::from_secs(30),
    )
    .unwrap();
    assert_eq!(probe.exit_code, 0);
    assert_eq!(probe.last_line.as_deref(), Some("codex-cli 999.1.2"));
    assert_eq!(probe.check.verdict, Verdict::Warn);
    assert_eq!(probe.check.installed.as_deref(), Some("999.1.2"));
    let failing = probe_version_within(
        &common::fake_codex(&["--fake-version=codex-cli 999.1.2", "--fake-version-exit=3"]),
        Duration::from_secs(30),
    )
    .unwrap();
    assert_eq!(failing.exit_code, 3);
    assert_eq!(failing.check.verdict, Verdict::Fail);
}

#[test]
fn a_child_left_holding_the_streams_cannot_hold_the_probe_past_its_timeout() {
    let dir = common::temp();
    let pid_file = dir.path().join("holder.pid");
    let flag = format!("--fake-version-holder={}", pid_file.display());
    let start = Instant::now();
    let probe = probe_version_within(
        &common::fake_codex(&["--fake-version=codex-cli 999.1.2", &flag]),
        Duration::from_secs(2),
    );
    let elapsed = start.elapsed();
    let holder: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while axiom::base::proc::pid_alive(holder) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(elapsed < Duration::from_secs(10), "the probe took {elapsed:?}");
    let error = probe.err().unwrap();
    assert_eq!(error.kind, "probe_failed");
    assert!(error.message.contains("did not answer"), "{}", error.message);
    assert!(
        !axiom::base::proc::pid_alive(holder),
        "pid {holder} still holds the probe's streams"
    );
}

#[test]
fn hung_or_missing_probe_is_a_probe_failure() {
    let start = Instant::now();
    let hung = probe_version_within(
        &common::fake_codex(&["--fake-version-hang"]),
        Duration::from_millis(300),
    )
    .err()
    .unwrap();
    assert_eq!(hung.kind, "probe_failed");
    assert!(hung.message.contains("did not answer"), "{}", hung.message);
    assert!(start.elapsed() < Duration::from_secs(20));
    let dir = common::temp();
    let missing = dir
        .path()
        .join("no-such-codex")
        .to_string_lossy()
        .into_owned();
    let error = probe_version_within(&[missing], Duration::from_secs(5))
        .err()
        .unwrap();
    assert_eq!(error.kind, "probe_failed");
}
