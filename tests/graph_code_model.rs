use axiom::graph::{
    code::{extract, language_of, symbol_keys, Def, Language, Span, DEF_KINDS},
    types::keys::split_symbol,
};

#[test]
fn extensions_and_families() {
    for (path, language) in [
        ("a.rs", Language::Rust),
        ("a.py", Language::Python),
        ("a.pyi", Language::Python),
        ("a.ts", Language::TypeScript),
        ("a.mts", Language::TypeScript),
        ("a.cts", Language::TypeScript),
        ("a.d.ts", Language::TypeScript),
        ("a.tsx", Language::Tsx),
        ("a.js", Language::JavaScript),
        ("a.mjs", Language::JavaScript),
        ("a.cjs", Language::JavaScript),
        ("a.jsx", Language::Jsx),
        (r"dir.name\file.rs", Language::Rust),
    ] {
        assert_eq!(language_of(path), Some(language), "{path}");
    }
    for path in ["a.RS", "a.txt", "no_extension", "dir.rs/file"] {
        assert_eq!(language_of(path), None, "{path}");
    }
    for (language, family) in [
        (Language::Rust, "rust"),
        (Language::Python, "python"),
        (Language::TypeScript, "ts"),
        (Language::Tsx, "ts"),
        (Language::JavaScript, "js"),
        (Language::Jsx, "js"),
    ] {
        assert_eq!(language.family(), family);
    }
}

#[test]
fn def_kinds_are_exact() {
    assert_eq!(
        DEF_KINDS,
        &[
            "function",
            "method",
            "class",
            "struct",
            "enum",
            "trait",
            "interface",
            "type",
            "const",
            "static",
            "variable",
            "module",
            "macro",
            "namespace",
        ]
    );
}

fn def(kind: &'static str) -> Def {
    Def {
        kind,
        name: "item".into(),
        qualname: "module::item".into(),
        span: Span {
            line: 1,
            col: 1,
            end_line: 1,
            end_col: 5,
        },
        signature: String::new(),
        parent: None,
        exported: false,
    }
}

#[test]
fn duplicate_symbol_keys_and_round_trip() {
    let keys = symbol_keys(
        "src/a.rs",
        &[
            def("function"),
            def("function"),
            def("class"),
            def("function"),
        ],
    );
    assert_eq!(
        keys,
        [
            "sym:src/a.rs#function:module::item",
            "sym:src/a.rs#function:module::item~2",
            "sym:src/a.rs#class:module::item",
            "sym:src/a.rs#function:module::item~3",
        ]
    );
    for (key, kind, occurrence) in [
        (&keys[0], "function", 1),
        (&keys[1], "function", 2),
        (&keys[2], "class", 1),
        (&keys[3], "function", 3),
    ] {
        assert_eq!(
            split_symbol(key),
            Some((
                "src/a.rs".into(),
                kind.into(),
                "module::item".into(),
                occurrence
            ))
        );
    }
}

#[test]
fn real_extractors_set_language_and_report_invalid_source() {
    for (language, errors, refs) in [
        (Language::Rust, vec!["expected `!`"], vec![]),
        (Language::Python, vec![], vec![("source", "value")]),
    ] {
        let result = extract(language, "source");
        assert_eq!(result.language, Some(language));
        assert_eq!(result.errors, errors);
        assert!(
            result.defs.is_empty() && result.imports.is_empty(),
            "{language:?}: {result:?}"
        );
        assert_eq!(
            result
                .refs
                .iter()
                .map(|reference| (reference.name.as_str(), reference.kind))
                .collect::<Vec<_>>(),
            refs,
            "{language:?}: {result:?}"
        );
    }
}

#[test]
#[should_panic]
fn negative_control_wrong_extension_expectation() {
    assert_eq!(language_of("a.txt"), Some(Language::Rust));
}

#[test]
fn attribution_files() {
    let license = include_str!("../src/graph/code/LICENSE-codegraph.txt");
    let notice = include_str!("../src/graph/code/NOTICE.md");
    let commit = "f72c9d2efb14454ff4f9fd7b8460abd82f19a287";
    for text in [
        "Copyright (c) Colby McHenry",
        commit,
        "Permission is hereby granted",
    ] {
        assert!(license.contains(text));
    }
    for text in [commit, "tree-sitter", "syn", "hand-written lexer"] {
        assert!(notice.contains(text));
    }
}
