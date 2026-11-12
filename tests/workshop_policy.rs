use axiom::seams::FakeApproval;
use axiom::workshop::{
    approval::{ensure_approved, receipt_path, ApprovalReceipt},
    manifest::{self, MANIFEST},
    permissions::{child_env, confine, BASE_ENV},
    DataDirs, Grants, Manifest, Panel, Permission, Provider,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::TempDir;

fn module(perms: &[&str]) -> Manifest {
    Manifest {
        name: "demo".into(),
        version: "1".into(),
        entry: vec!["run".into()],
        permissions: perms.iter().map(|p| p.to_string()).collect(),
        max_parallel: 1,
        panel: Panel {
            title: "Title".into(),
            description: "Description".into(),
        },
    }
}
fn write_manifest(dir: &Path, value: &Value) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(MANIFEST), serde_json::to_vec(value).unwrap()).unwrap();
}
fn grants(perms: &[&str]) -> Grants {
    Grants::from_manifest(&module(perms)).unwrap()
}

#[test]
fn manifest_rejects_bad_fields() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("demo");
    let valid = serde_json::to_value(module(&["files"])).unwrap();
    write_manifest(&dir, &valid);
    assert_eq!(manifest::load(&dir).unwrap(), module(&["files"]));
    let mut defaults = valid.clone();
    defaults.as_object_mut().unwrap().remove("permissions");
    defaults.as_object_mut().unwrap().remove("max_parallel");
    write_manifest(&dir, &defaults);
    assert_eq!(manifest::load(&dir).unwrap().max_parallel, 1);
    let mut cases = vec![];
    for (key, value) in [
        ("name", json!("different")),
        ("name", json!("Not-a-slug")),
        ("version", json!("")),
        ("entry", json!([])),
        ("entry", json!([""])),
        ("max_parallel", json!(0)),
        ("max_parallel", json!(9)),
        ("permissions", json!(["bogus"])),
        ("unknown", json!(1)),
    ] {
        let mut item = valid.clone();
        item[key] = value;
        cases.push(item);
    }
    for item in cases {
        write_manifest(&dir, &item);
        assert_eq!(
            manifest::load(&dir).unwrap_err().kind,
            "invalid_manifest",
            "{item}"
        );
    }
}

#[test]
fn discovery_skips_and_reports_invalid() {
    let temp = TempDir::new().unwrap();
    let root = manifest::modules_root(temp.path());
    for name in ["zulu", "alpha", "bad"] {
        let mut m = module(&[]);
        m.name = name.into();
        if name == "bad" {
            m.version.clear();
        }
        write_manifest(&root.join(name), &serde_json::to_value(m).unwrap());
    }
    let found = manifest::discover(temp.path()).unwrap();
    assert_eq!(
        found
            .iter()
            .map(|(_, m)| m.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zulu"]
    );
    let (reported, invalid) = manifest::discover_report(temp.path()).unwrap();
    assert_eq!(reported, found);
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].0, root.join("bad"));
    assert_eq!(invalid[0].1.kind, "invalid_manifest");
}

#[test]
fn permissions_round_trip_and_risk() {
    for (s, risk) in [
        ("files", false),
        ("memory", false),
        ("model:claude:sonnet", false),
        ("model:codex:*", false),
        ("subprocess", true),
        ("network", true),
        ("env:SECRET", true),
        ("decision_router", false),
    ] {
        let p = Permission::parse(s).unwrap();
        assert_eq!(p.as_string(), s);
        assert_eq!(p.is_high_risk(), risk);
    }
    for s in [
        "",
        "model:other:x",
        "model:claude:",
        "env:",
        "env:A=B",
        "model:codex:a*",
    ] {
        assert!(Permission::parse(s).is_err(), "{s}");
    }
}

#[test]
fn undeclared_permission_denied() {
    let g = grants(&[]);
    for s in [
        "files",
        "memory",
        "model:claude:x",
        "subprocess",
        "network",
        "env:SECRET",
        "decision_router",
    ] {
        let p = Permission::parse(s).unwrap();
        assert!(!g.allows(&p));
        let err = g.check(&p).unwrap_err();
        assert_eq!(err.kind, "permission_denied");
        assert!(err.message.contains(s));
    }
}

#[test]
fn model_allowlist_exact_and_star() {
    let g = grants(&["model:claude:sonnet", "model:codex:*"]);
    assert!(g.allows_model(Provider::Claude, "sonnet"));
    assert!(!g.allows_model(Provider::Claude, "other"));
    assert!(g.allows_model(Provider::Codex, "anything"));
    assert!(g.allows(&Permission::parse("model:codex:other").unwrap()));
    let exact = grants(&["model:claude:sonnet"]);
    assert!(!exact.allows_model(Provider::Codex, "sonnet"));
}

#[test]
fn digest_order_and_configuration() {
    let first = grants(&["files", "network"]);
    assert_eq!(first.digest(), grants(&["network", "files"]).digest());
    assert_ne!(first.digest(), grants(&["files"]).digest());
    let mut m = module(&["files", "network"]);
    m.max_parallel = 2;
    let changed = Grants::from_manifest(&m).unwrap();
    assert_eq!(changed.max_parallel(), 2);
    assert_ne!(first.digest(), changed.digest());
}

#[test]
fn env_allowlist_drops_undeclared() {
    let temp = TempDir::new().unwrap();
    let dirs = DataDirs::new(temp.path(), "demo");
    dirs.ensure().unwrap();
    for path in [
        &dirs.root,
        &dirs.files,
        &dirs.memory,
        &dirs.logs,
        &dirs.receipts,
        &dirs.tmp,
    ] {
        assert!(path.is_dir());
    }
    // These names are unique to this test and are not used elsewhere.
    std::env::set_var("AXIOM_TEST_DECLARED_WORKSHOP", "visible");
    std::env::set_var("AXIOM_TEST_UNDECLARED_WORKSHOP", "secret");
    let env: BTreeMap<_, _> = child_env(
        "demo",
        &grants(&["env:AXIOM_TEST_DECLARED_WORKSHOP", "env:HOME"]),
        &dirs,
    )
    .into_iter()
    .collect();
    assert_eq!(env.get("AXIOM_TEST_DECLARED_WORKSHOP").unwrap(), "visible");
    assert!(!env.contains_key("AXIOM_TEST_UNDECLARED_WORKSHOP"));
    for key in BASE_ENV {
        if let Ok(value) = std::env::var(key) {
            assert_eq!(env.get(*key), Some(&value));
        }
    }
    for key in ["HOME", "USERPROFILE", "AXIOM_WORKSHOP_DATA"] {
        assert_eq!(env.get(key).unwrap(), &dirs.files.to_string_lossy());
    }
    for key in ["TMP", "TEMP", "TMPDIR"] {
        assert_eq!(env.get(key).unwrap(), &dirs.tmp.to_string_lossy());
    }
    assert_eq!(env.get("AXIOM_WORKSHOP_MODULE").unwrap(), "demo");
    assert_eq!(env.get("AXIOM_WORKSHOP_API").unwrap(), "stdio/1");
    std::env::remove_var("AXIOM_TEST_DECLARED_WORKSHOP");
    std::env::remove_var("AXIOM_TEST_UNDECLARED_WORKSHOP");
}

#[test]
fn confine_rejects_traversal_and_symlink_escape() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    for s in [
        "",
        "..",
        "../outside",
        "nested/../outside",
        "/absolute",
        "C:\\escape",
        "C:relative",
        "\\\\server\\share",
        "a\\..\\escape",
    ] {
        assert_eq!(confine(&root, s).unwrap_err().kind, "path_escape", "{s}");
    }
    assert_eq!(
        confine(&root, "nested/file").unwrap(),
        root.join("nested/file")
    );
    #[cfg(unix)]
    let link_created = std::os::unix::fs::symlink(&outside, root.join("link")).is_ok();
    #[cfg(windows)]
    let link_created = std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let link_created = false;
    #[cfg(unix)]
    assert!(link_created);
    if link_created {
        assert_eq!(
            confine(&root, "link/escape").unwrap_err().kind,
            "path_escape"
        );
    }
}

#[test]
fn denied_approval_refuses() {
    let temp = TempDir::new().unwrap();
    let dirs = DataDirs::new(temp.path(), "demo");
    let approver = FakeApproval::denying();
    assert!(
        ensure_approved(&dirs, &module(&[]), &grants(&[]), &approver)
            .unwrap()
            .is_none()
    );
    assert!(approver.asked().is_empty());
    let m = module(&["network"]);
    assert_eq!(
        ensure_approved(&dirs, &m, &Grants::from_manifest(&m).unwrap(), &approver)
            .unwrap_err()
            .kind,
        "approval_denied"
    );
    assert_eq!(approver.asked().len(), 1);
    assert!(!receipt_path(&dirs).exists());
}

#[test]
fn widened_set_needs_reconfirmation() {
    let temp = TempDir::new().unwrap();
    let dirs = DataDirs::new(temp.path(), "demo");
    let approver = FakeApproval::approving();
    let m = module(&["network"]);
    let g = Grants::from_manifest(&m).unwrap();
    let receipt = ensure_approved(&dirs, &m, &g, &approver).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<ApprovalReceipt>(&fs::read(receipt_path(&dirs)).unwrap()).unwrap(),
        receipt
    );
    assert_eq!(approver.asked()[0].subject, "workshop:demo");
    assert_eq!(approver.asked()[0].action, "grant_permissions");
    assert!(approver.asked()[0].summary.contains("network"));
    assert_eq!(approver.asked()[0].digest, g.digest());
    ensure_approved(&dirs, &m, &g, &approver).unwrap();
    assert_eq!(approver.asked().len(), 1);
    let widened = module(&["network", "subprocess"]);
    ensure_approved(
        &dirs,
        &widened,
        &Grants::from_manifest(&widened).unwrap(),
        &approver,
    )
    .unwrap();
    assert_eq!(approver.asked().len(), 2);
}

#[test]
fn narrowed_set_reuses_receipt() {
    let temp = TempDir::new().unwrap();
    let dirs = DataDirs::new(temp.path(), "demo");
    let approver = FakeApproval::approving();
    let m = module(&["network", "subprocess"]);
    let old = ensure_approved(&dirs, &m, &Grants::from_manifest(&m).unwrap(), &approver)
        .unwrap()
        .unwrap();
    let narrowed = module(&["network"]);
    let reused = ensure_approved(
        &dirs,
        &narrowed,
        &Grants::from_manifest(&narrowed).unwrap(),
        &approver,
    )
    .unwrap()
    .unwrap();
    assert_eq!(reused, old);
    assert_eq!(approver.asked().len(), 1);
}
