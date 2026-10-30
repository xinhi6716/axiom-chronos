use axiom::orch;
use std::{fs, process::Command, time::Instant};
#[test]
#[ignore = "short local performance probe; compare same machine and build profile"]
fn begin_abort_16_dirty_files() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap()
            .status
            .success())
    };
    git(&["init", "-q"]);
    for i in 0..16 {
        fs::write(repo.join(format!("file{i}.txt")), "original").unwrap();
    }
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Audit",
        "-c",
        "user.email=audit@example.invalid",
        "commit",
        "-qm",
        "fixture",
    ]);
    for i in 0..16 {
        fs::write(repo.join(format!("file{i}.txt")), "existing dirty content").unwrap();
    }
    let store = orch::store_for(temp.path(), "bench").unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL benchmark\nSCOPE file0.txt\nVERIFY echo ok\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let mut samples = vec![];
    for _ in 0..11 {
        let start = Instant::now();
        orch::begin_unit(&store, "u", "terra", &repo, false).unwrap();
        orch::abort_unit(&store, "u");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.remove(0);
    samples.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"benchmark":"begin_abort_16_dirty_files","samples":10,"median_ms":(samples[4]+samples[5])/2.0,"max_ms":samples[9],"sample_ms":samples})
    );
}
