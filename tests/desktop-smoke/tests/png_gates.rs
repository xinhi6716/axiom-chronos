#[path = "png_support/mod.rs"]
mod png_support;
#[path = "release_support/mod.rs"]
mod release_support;

#[cfg(test)]
mod rules {
    use super::*;
    use serde_json::{json, Value};
    use std::{fs, path::Path};
    struct Fixture {
        dir: tempfile::TempDir,
        manifest: Value,
        cases: Vec<(String, String, String, String, String)>,
    }
    fn image(invert: bool) -> png_support::Png {
        let mut rgba = Vec::new();
        for i in 0..400 {
            let v = if (i % 20 < 10) ^ invert { 40 } else { 200 };
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        png_support::Png {
            width: 20,
            height: 20,
            rgba,
        }
    }
    fn entry(root: &Path, phase: &str, view: &str, png: &png_support::Png) -> Value {
        let bytes = png.encode().unwrap();
        let file = format!("files/{phase}/{view}.png");
        fs::write(root.join(&file), &bytes).unwrap();
        json!({"view":view,"state":"live","lang":"en","theme":"dark","size":"20x20","dpi":96,"source_id":"new","commit":"abc","scenario":"test","file":file,"sha256":png_support::hash(&bytes),"width":png.width,"height":png.height})
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        for p in ["before", "after"] {
            fs::create_dir_all(dir.path().join("files").join(p)).unwrap();
        }
        let before = entry(dir.path(), "before", "run", &image(false));
        let after = entry(dir.path(), "after", "run", &image(false));
        Fixture {
            dir,
            manifest: json!({"thresholds":{"run":0.02},"baseline":[before],"current":[after],"approvals":[]}),
            cases: vec![(
                "run".into(),
                "live".into(),
                "en".into(),
                "dark".into(),
                "20x20".into(),
            )],
        }
    }
    impl Fixture {
        fn check(&self) -> Result<(), String> {
            png_support::validate(self.dir.path(), &self.manifest, &self.cases, "new")
        }
    }
    #[test]
    fn passing_case() {
        assert!(fixture().check().is_ok());
    }
    #[test]
    fn wrong_size() {
        let mut f = fixture();
        f.manifest["current"][0]["width"] = json!(21);
        assert!(f.check().unwrap_err().contains("size"));
    }
    #[test]
    fn blank_or_near_uniform() {
        for v in [0, 5] {
            let mut f = fixture();
            let mut p = image(false);
            for (i, c) in p.rgba.chunks_exact_mut(4).enumerate() {
                c[..3].fill(if i % 2 == 0 { 0 } else { v });
            }
            f.manifest["current"][0] = entry(f.dir.path(), "after", "run", &p);
            assert!(f.check().is_err());
        }
    }
    #[test]
    fn duplicate_hash_across_targets() {
        let mut f = fixture();
        let b = entry(f.dir.path(), "before", "other", &image(false));
        let a = entry(f.dir.path(), "after", "other", &image(false));
        f.manifest["baseline"].as_array_mut().unwrap().push(b);
        f.manifest["current"].as_array_mut().unwrap().push(a);
        f.manifest["thresholds"]["other"] = json!(0.02);
        f.cases.push((
            "other".into(),
            "live".into(),
            "en".into(),
            "dark".into(),
            "20x20".into(),
        ));
        assert!(f.check().is_err());
    }
    #[test]
    fn missing_file() {
        let f = fixture();
        fs::remove_file(f.dir.path().join("files/after/run.png")).unwrap();
        assert!(f.check().is_err());
    }
    #[test]
    fn hash_mismatch() {
        let mut f = fixture();
        f.manifest["current"][0]["sha256"] = json!("bad");
        assert!(f.check().is_err());
    }
    #[test]
    fn old_source_id() {
        let mut f = fixture();
        f.manifest["current"][0]["source_id"] = json!("old");
        assert!(f.check().is_err());
    }
    #[test]
    fn diff_over_threshold_without_approval() {
        let mut f = fixture();
        f.manifest["current"][0] = entry(f.dir.path(), "after", "run", &image(true));
        assert!(f.check().is_err());
        f.manifest["approvals"] =
            json!([{"view":"run","reason":"changed","source_id":"new","commit":"abc"}]);
        assert!(f.check().is_ok());
    }
    #[test]
    fn missing_folder() {
        let f = fixture();
        fs::remove_dir_all(f.dir.path().join("files/after")).unwrap();
        assert!(f.check().is_err());
    }
    #[test]
    fn pilot_filter_selects_four_views() {
        let matrix = release_support::load_matrix();
        let mut ids: Vec<_> = png_support::matrix_cases(&matrix, true)
            .into_iter()
            .map(|c| c.0)
            .collect();
        ids.sort();
        assert_eq!(ids, ["dialog.composer", "focus.graph", "mini", "run"]);
    }
    #[test]
    fn diff_channel_tolerance_and_size() {
        let a = image(false);
        let mut b = image(false);
        b.rgba[0] += 8;
        assert_eq!(a.diff(&b), 0.0);
        b.rgba[0] += 1;
        assert_eq!(a.diff(&b), 1.0 / 400.0);
        b.width = 10;
        assert_eq!(a.diff(&b), 1.0);
    }
}
#[cfg(windows)]
mod evidence {
    use super::*;
    #[test]
    #[ignore = "3.0.0-beta.1: the final PNG captures (captures-final) have not run; the official 3.0.0 re-enables this test"]
    fn complete_current_matrix() {
        let repo = release_support::repo();
        let source = release_support::source_id(&repo).expect("clean application source required");
        let root = repo.join("tests/desktop-smoke/captures/png");
        let manifest =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json")).expect("manifest"))
                .expect("manifest JSON");
        png_support::validate(
            &root,
            &manifest,
            &png_support::matrix_cases(&release_support::load_matrix(), false),
            &source,
        )
        .expect("PNG evidence");
    }
}
