use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub const HELDOUT_ROWS: usize = 60_000;
pub const HELDOUT_SEED: u64 = 7919;
pub const REGRESSION_ROWS: usize = 50_000;
pub const REGRESSION_SEED: u64 = 1;

pub struct Workload {
    pub root: PathBuf,
    pub rows: usize,
    pub seed: u64,
    pub sha256: String,
}

pub fn workload(home: &Path, rows: usize, seed: u64) -> Workload {
    let root = home.join("research");
    fs::create_dir_all(root.join("data")).expect("create research data");
    fs::write(root.join("research-manifest.json"), r#"{"manifest_version":1,"research_id":"perf","research_version":"1","title":"Performance","capabilities":["evidence","dataset"],"settings":{},"versions":{},"artifacts":[{"id":"evidence","role":"evidence","path":"data/evidence.csv","format":"csv"},{"id":"measurements","role":"measurements","path":"data/measurements.csv","format":"csv"}]}"#).unwrap();
    // Split total rows in the same 4:1 ratio as research_gates::synthetic.
    let evidence_rows = rows - rows / 5;
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    };
    let mut evidence = BufWriter::new(File::create(root.join("data/evidence.csv")).unwrap());
    writeln!(evidence, "evidence_id,paper_id,sha256,physical_page,quote").unwrap();
    for n in 0..evidence_rows {
        let r = next();
        writeln!(
            evidence,
            "E{n},P{},hash{:064x},{},experimental evidence group {}",
            r % 2000,
            r,
            r % 30 + 1,
            r % 20
        )
        .unwrap();
    }
    evidence.flush().unwrap();
    let mut measurements =
        BufWriter::new(File::create(root.join("data/measurements.csv")).unwrap());
    writeln!(
        measurements,
        "row_id,paper_id,variable,value_original,unit_original,value_canonical"
    )
    .unwrap();
    for n in 0..rows - evidence_rows {
        let r = next();
        let canonical = if r % 7 == 0 {
            String::new()
        } else {
            (r % 347).to_string()
        };
        writeln!(
            measurements,
            "M{n},P{},variable{},{},mV,{canonical}",
            r % 2000,
            r % 12,
            r % 347
        )
        .unwrap();
    }
    measurements.flush().unwrap();
    let mut hash = Sha256::new();
    for path in [
        "data/evidence.csv",
        "data/measurements.csv",
        "research-manifest.json",
    ] {
        let bytes = fs::read(root.join(path)).unwrap();
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Workload {
        root,
        rows,
        seed,
        sha256: format!("{:x}", hash.finalize()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sha256() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(
            workload(a.path(), 300, 7919).sha256,
            workload(b.path(), 300, 7919).sha256
        );
    }

    #[test]
    fn different_seed_different_sha256() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_ne!(
            workload(a.path(), 300, 7919).sha256,
            workload(b.path(), 300, 1).sha256
        );
    }

    #[test]
    fn generated_row_count() {
        let dir = tempfile::tempdir().unwrap();
        let work = workload(dir.path(), 300, 1);
        let count: usize = ["data/evidence.csv", "data/measurements.csv"]
            .iter()
            .map(|p| {
                fs::read_to_string(work.root.join(p))
                    .unwrap()
                    .lines()
                    .count()
                    - 1
            })
            .sum();
        assert_eq!(count, 300);
    }
}
