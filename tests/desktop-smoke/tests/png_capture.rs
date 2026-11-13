#[path = "png_support/mod.rs"]
mod png_support;
#[path = "release_support/mod.rs"]
mod release_support;
#[cfg(windows)]
fn capture_phase(phase: &str) {
    use serde_json::{json, Value};
    use std::fs;
    let repo = release_support::repo();
    let source = release_support::source_id(&repo).expect("clean application source required");
    let commit = release_support::commit(&repo);
    let exe = release_support::release_exe("chronos").expect("release executable");
    let matrix = release_support::load_matrix();
    let pilot = std::env::var("CAPTURE_PILOT").as_deref() == Ok("1");
    let temporary = if pilot {
        Some(tempfile::tempdir().expect("pilot folder"))
    } else {
        None
    };
    let root = temporary
        .as_ref()
        .map(|d| d.path().to_path_buf())
        .unwrap_or_else(|| repo.join("tests/desktop-smoke/captures/png"));
    let manifest_path = root.join("manifest.json");
    let mut manifest: Value = if pilot {
        json!({})
    } else {
        serde_json::from_slice(&fs::read(&manifest_path).expect("manifest")).expect("manifest JSON")
    };
    let folder = root.join("files").join(phase);
    fs::create_dir_all(&folder).expect("capture folder");
    let mut entries = Vec::new();
    for (view, state, lang, theme, size) in png_support::matrix_cases(&matrix, pilot) {
        let home = release_support::isolated_home();
        let app = png_support::launch(
            &exe,
            home.path(),
            &view,
            &state,
            &lang,
            &theme,
            png_support::size(&size).unwrap(),
        );
        let image = png_support::capture(&app)
            .unwrap_or_else(|e| panic!("{view}/{state}/{lang}/{theme}/{size}: {e}"));
        let dpi = png_support::dpi(&app);
        assert!(dpi > 0, "missing window DPI");
        let bytes = image.encode().expect("encode PNG");
        let name = format!("{view}.{state}.{lang}.{theme}.{size}.png");
        let file = format!("files/{phase}/{name}");
        fs::write(root.join(&file), &bytes).expect("write PNG");
        entries.push(json!({"view":view,"state":state,"lang":lang,"theme":theme,"size":size,"dpi":dpi,"source_id":source,"commit":commit,"scenario":format!("{view}/{state}/{lang}/{theme}/{size}"),"file":file,"sha256":png_support::hash(&bytes),"width":image.width,"height":image.height}));
    }
    if !pilot {
        manifest[if phase == "before" {
            "baseline"
        } else {
            "current"
        }] = json!(entries);
        fs::write(
            manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap() + "\n",
        )
        .expect("write manifest");
    }
}
#[cfg(windows)]
#[test]
#[ignore]
fn capture_before() {
    capture_phase("before");
}
#[cfg(windows)]
#[test]
#[ignore]
fn capture_after() {
    capture_phase("after");
}
