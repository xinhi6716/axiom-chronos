use axiom::graph::code::{extract, Extraction, Language};
use std::fs;

fn fixture() -> Extraction {
    let result = extract(
        Language::Rust,
        include_str!("fixtures/graph/code/rust/coverage.rs"),
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
}

#[test]
fn definitions_and_signatures() {
    let r = fixture();
    for (kind, name, parent, exported) in [
        ("module", "inner", None, true),
        ("struct", "inner.Boxed", Some("inner"), true),
        ("enum", "inner.Choice", Some("inner"), true),
        ("trait", "inner.Work", Some("inner"), true),
        ("method", "inner.Work.execute", Some("inner.Work"), false),
        ("type", "inner.Alias", Some("inner"), true),
        ("const", "inner.LIMIT", Some("inner"), true),
        ("static", "inner.GLOBAL", Some("inner"), true),
        ("macro", "inner.sample", Some("inner"), false),
        ("function", "inner.helper", Some("inner"), true),
        ("method", "inner.Boxed.method", Some("inner"), true),
        ("function", "scopes", None, false),
    ] {
        let d = r
            .defs
            .iter()
            .find(|d| d.qualname == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(d.kind, kind, "{name}");
        assert_eq!(
            d.parent.map(|i| r.defs[i].qualname.as_str()),
            parent,
            "{name}"
        );
        assert_eq!(d.exported, exported, "{name}");
        assert!(!d.signature.contains('\n'));
        assert!(d.signature.chars().count() <= 160);
        assert!(!d.signature.contains('{'), "{}", d.signature);
    }
    let r = extract(
        Language::Rust,
        &format!("pub fn {}() {{ body(); }}", "a".repeat(200)),
    );
    assert!(r.errors.is_empty());
    assert_eq!(r.defs[0].signature.chars().count(), 160);
}

#[test]
fn headers_exclude_attributes_and_docs() {
    let source = "/// documentation\n#[inline]\n// comment\npub(crate) async fn multi(\n    arg: usize,\n) -> usize { arg }\n#[repr(C)]\npub struct Record { field: usize }\n#[allow(dead_code)]\nunsafe trait Action {\n    /// method docs\n    #[inline]\n    fn execute(\n        &self,\n    );\n}\n#[allow(dead_code)]\nconst VALUE: usize = 1;\n";
    let r = extract(Language::Rust, source);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    for (name, signature, line, col) in [
        (
            "multi",
            "pub(crate) async fn multi( arg: usize, ) -> usize",
            4,
            1,
        ),
        ("Record", "pub struct Record", 8, 1),
        ("Action", "unsafe trait Action", 10, 1),
        ("Action.execute", "fn execute( &self, )", 13, 5),
        ("VALUE", "const VALUE: usize", 18, 1),
    ] {
        let d = r.defs.iter().find(|d| d.qualname == name).unwrap();
        assert_eq!(d.signature, signature, "{name}");
        assert_eq!((d.span.line, d.span.col), (line, col), "{name}");
    }
}

#[test]
fn impl_self_types_have_stable_owners() {
    let source = "impl &Thing { fn a(&self) {} } impl ((Thing)) { fn b(&self) {} } impl [u8] { fn c(&self) {} } impl (u8, /* comment */ bool) { fn d(&self) {} }";
    let r = extract(Language::Rust, source);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let names: Vec<_> = r.defs.iter().map(|d| d.qualname.as_str()).collect();
    assert_eq!(names, ["Thing.a", "Thing.b", "[u8].c", "(u8,bool).d"]);
}

#[test]
fn nested_items_do_not_capture_function_scopes() {
    let source = r#"
fn outer(param: fn()) {
    let local = make();
    fn inner() { param(); local(); }
    impl Thing { fn method() { param(); local(); } }
    trait Work { fn method() { param(); local(); } }
    param(); local();
}
"#;
    let r = extract(Language::Rust, source);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    for name in ["param", "local"] {
        let flags: Vec<_> = r
            .refs
            .iter()
            .filter(|x| x.kind == "call" && x.name == name)
            .map(|x| x.shadowed)
            .collect();
        assert_eq!(flags, [false, false, false, true], "{name}");
    }
}

#[test]
fn let_conditions_bind_sequentially_and_only_in_scope() {
    let source = r#"
fn test() {
    if let Some(first) = first() && first() && let Some(second) = second() && second() {
        first(); second();
    } else { first(); second(); }
    first(); second();
    while let Some(looped) = looped() && looped() {
        looped();
    }
    looped();
}
"#;
    let r = extract(Language::Rust, source);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    for (name, expected) in [
        ("first", vec![false, true, true, false, false]),
        ("second", vec![false, true, true, false, false]),
        ("looped", vec![false, true, true, false]),
    ] {
        let flags: Vec<_> = r
            .refs
            .iter()
            .filter(|x| x.kind == "call" && x.name == name)
            .map(|x| x.shadowed)
            .collect();
        assert_eq!(flags, expected, "{name}");
    }
}

#[test]
fn imports_are_leaves() {
    let r = fixture();
    let list: Vec<_> = r
        .imports
        .iter()
        .map(|i| {
            (
                i.kind,
                i.module.as_str(),
                i.names
                    .iter()
                    .map(|n| (n.name.as_str(), n.alias.as_deref()))
                    .collect::<Vec<_>>(),
                i.glob,
            )
        })
        .collect();
    assert_eq!(
        list,
        vec![
            ("use", "crate", vec![("alpha", None)], false),
            ("use", "crate::alpha", vec![("beta", Some("bee"))], false),
            ("use", "crate::alpha::nested", vec![("Thing", None)], false),
            ("use", "crate::alpha::nested", vec![], true),
            ("use", "self::local", vec![("item", None)], false),
            ("use", "super", vec![("outer", None)], false),
            ("mod", "external", vec![], false),
        ]
    );
}

#[test]
fn references_spans_and_scope() {
    let r = fixture();
    let call = r
        .refs
        .iter()
        .find(|x| x.kind == "call" && x.name == "run" && x.qualifier == ["crate", "api"])
        .unwrap();
    assert_eq!(
        (
            call.span.line,
            call.span.col,
            call.span.end_line,
            call.span.end_col
        ),
        (13, 30, 13, 45)
    );
    assert_eq!(r.defs[call.enclosing.unwrap()].qualname, "inner.helper");
    let method = r
        .refs
        .iter()
        .find(|x| x.name == "run" && x.qualifier == ["self", "x", "y"])
        .unwrap();
    assert!(!method.dynamic);
    assert_eq!((method.span.line, method.span.col), (16, 22));
    let dynamic = r.refs.iter().find(|x| x.name == "dynamic").unwrap();
    assert!(dynamic.dynamic && dynamic.qualifier.is_empty());
    assert!(r
        .refs
        .iter()
        .any(|x| x.kind == "macro" && x.name == "sample"));
    assert!(r
        .refs
        .iter()
        .any(|x| x.kind == "type" && x.name == "Input" && x.qualifier == ["crate"]));
    for name in ["param", "local", "captured", "matched", "iter"] {
        let reference = r
            .refs
            .iter()
            .find(|x| x.kind == "call" && x.name == name)
            .unwrap();
        assert!(reference.shadowed, "{name}");
    }
    assert!(r
        .refs
        .iter()
        .any(|x| x.kind == "value" && x.name == "local" && x.shadowed));
    let nested: Vec<_> = r
        .refs
        .iter()
        .filter(|x| x.kind == "call" && x.name == "nested")
        .collect();
    assert_eq!(nested.len(), 2);
    assert!(nested[0].shadowed && !nested[1].shadowed);
    let bound: Vec<_> = r
        .refs
        .iter()
        .filter(|x| x.kind == "call" && x.name == "bound")
        .collect();
    assert_eq!(bound.len(), 2);
    assert!(bound[0].shadowed && !bound[1].shadowed);
    assert!(
        !r.refs
            .iter()
            .find(|x| x.name == "unbound")
            .unwrap()
            .shadowed
    );
}

#[test]
fn invalid_input_is_an_error() {
    let r = extract(Language::Rust, "fn broken( {");
    assert!(!r.errors.is_empty());
    assert_eq!(r.language, Some(Language::Rust));
}

#[test]
#[should_panic(expected = "deliberately wrong: fixture has no definitions")]
fn negative_control_wrong_expectation() {
    let result = fixture();
    assert!(
        result.defs.is_empty(),
        "deliberately wrong: fixture has no definitions"
    );
}

#[test]
fn holdout_files_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/graph/holdout/resolution/rust");
    let mut count = 0;
    for case in fs::read_dir(root).unwrap() {
        let files = case.unwrap().path().join("files");
        if !files.is_dir() {
            continue;
        }
        for entry in fs::read_dir(files).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let result = extract(Language::Rust, &fs::read_to_string(&path).unwrap());
            assert!(
                result.errors.is_empty(),
                "{}: {:?}",
                path.display(),
                result.errors
            );
            count += 1;
        }
    }
    assert!(count > 0);
}
