use axiom::graph::code::{extract, language_of, symbol_keys, Language};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

fn files(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        if p.is_dir() {
            files(&p, base, out)
        } else {
            let name = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((name, fs::read_to_string(p).unwrap()));
        }
    }
}
#[test]
fn frozen_holdout() {
    let root = Path::new("tests/fixtures/graph/holdout/resolution");
    let mut cases = 0;
    for family in ["python", "ts", "js"] {
        for entry in fs::read_dir(root.join(family)).unwrap() {
            let dir = entry.unwrap().path();
            let json: Value =
                serde_json::from_str(&fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
            let base = dir.join("files");
            let mut sources = Vec::new();
            files(&base, &base, &mut sources);
            let extracted: HashMap<_, _> = sources
                .iter()
                .map(|(path, src)| (path.clone(), extract(language_of(path).unwrap(), src)))
                .collect();
            for r in json["refs"].as_array().unwrap() {
                let file = r["file"].as_str().unwrap();
                let line = r["line"].as_u64().unwrap() as u32;
                let name = r["name"].as_str().unwrap();
                let nth = r["nth"].as_u64().unwrap() as usize;
                let found = extracted[file]
                    .refs
                    .iter()
                    .filter(|x| x.span.line == line && x.name == name)
                    .nth(nth - 1);
                assert!(
                    found.is_some(),
                    "{}: {file}:{line} {name} #{nth}",
                    json["id"]
                );
                let found = found.unwrap();
                for expected in [r, &r["expect"]] {
                    if let Some(qualifier) = expected["qualifier"].as_array() {
                        let qualifier: Vec<_> =
                            qualifier.iter().map(|v| v.as_str().unwrap()).collect();
                        assert_eq!(found.qualifier, qualifier, "{}", json["id"]);
                    }
                    if let Some(kind) = expected["kind"].as_str() {
                        assert_eq!(found.kind, kind, "{}", json["id"]);
                    }
                    if let Some(shadowed) = expected["shadowed"].as_bool() {
                        assert_eq!(found.shadowed, shadowed, "{}", json["id"]);
                    }
                }
                if let Some(target) = r["expect"]["target"].as_str() {
                    let (target_file, _) = target.split_once('#').unwrap();
                    {
                        let result = extracted.get(target_file).unwrap_or_else(|| {
                            panic!("{} missing target file {target_file}", json["id"])
                        });
                        let key = format!("sym:{target}");
                        assert!(
                            symbol_keys(target_file, &result.defs).contains(&key),
                            "{} missing {target}; defs {:?}",
                            json["id"],
                            result.defs
                        );
                    }
                }
            }
            cases += 1;
        }
    }
    assert!(cases > 0);
}
#[test]
fn literals_and_comments_hide_definitions() {
    let p = extract(
        Language::Python,
        "s = 'def f(): pass'\n# def g():\nx = f\"import bogus\"\n",
    );
    assert!(p.defs.iter().all(|d| d.name != "f" && d.name != "g"));
    assert!(p.imports.is_empty());
    let j=extract(Language::JavaScript,"const s = 'function fake() {}'; // import x from 'bad'\nconst t = `class Hidden {}`; const re = /function fake2\\(\\)/;\n");
    assert!(j
        .defs
        .iter()
        .all(|d| !d.name.starts_with("fake") && d.name != "Hidden"));
    assert!(j.imports.is_empty());
}
#[test]
fn python_features() {
    let x=extract(Language::Python,"import a.b as c\nfrom ..pkg import a as b, *\nclass C:\n    async def m(self, p: T):\n        local = p\n        local.run()\n        getattr(p, 'x')\n        importlib.load()\n\nasync def run():\n    def compute():\n        pass\n    compute()\n");
    assert!(x
        .defs
        .iter()
        .any(|d| d.kind == "method" && d.qualname == "C.m"));
    assert!(x.defs.iter().any(|d| d.qualname == "run.compute"));
    assert_eq!(x.imports[0].module, "a.b");
    assert_eq!(x.imports[0].namespace_as.as_deref(), Some("c"));
    assert_eq!(x.imports[1].module, "..pkg");
    assert!(x.imports[1].glob);
    assert!(x.refs.iter().any(|r| r.name == "run" && r.shadowed));
    assert!(x.refs.iter().any(|r| r.name == "getattr" && r.dynamic));
    assert!(x.refs.iter().any(|r| r.name == "load" && r.dynamic));
    assert!(x.refs.iter().any(|r| r.name == "T" && r.kind == "type"));
}
#[test]
fn ecma_features() {
    let x=extract(Language::Tsx,"import d, {a as b} from 'm';\nimport * as ns from 'n';\nexport {a as b} from 'r';\nexport * from 's';\nconst {a, b: c} = require('q');\nexport class C { m(p: T) { let local = p; local.go(); } }\nexport const run = (p: T) => { p.go(); obj[key](); import(path); require(path); return <Comp />; };\ninterface I {}\ntype Alias = I;\nenum E {}\nnamespace N {}\n");
    assert!(x.defs.iter().any(|d| d.qualname == "C.m"));
    assert!(x
        .defs
        .iter()
        .any(|d| d.name == "run" && d.kind == "function" && d.exported));
    for k in ["interface", "type", "enum", "namespace"] {
        assert!(x.defs.iter().any(|d| d.kind == k), "{k}")
    }
    assert_eq!(x.imports.len(), 5);
    assert_eq!(x.imports[0].default_as.as_deref(), Some("d"));
    assert_eq!(x.imports[1].namespace_as.as_deref(), Some("ns"));
    assert!(x.imports[3].glob);
    assert_eq!(x.imports[4].names.len(), 2);
    assert!(x.refs.iter().any(|r| r.name == "go" && r.shadowed));
    assert!(x.refs.iter().any(|r| r.name == "obj" && r.dynamic));
    assert!(x.refs.iter().any(|r| r.name == "import" && r.dynamic));
    assert!(x.refs.iter().any(|r| r.name == "require" && r.dynamic));
    assert!(x.refs.iter().any(|r| r.name == "Comp" && r.kind == "value"));
}
#[test]
fn robustness() {
    let root = Path::new("tests/fixtures/graph/holdout/resolution");
    for family in ["python", "ts", "js"] {
        for entry in fs::read_dir(root.join(family)).unwrap() {
            let base = entry.unwrap().path().join("files");
            let mut all = Vec::new();
            files(&base, &base, &mut all);
            for (path, source) in all {
                let lang = language_of(&path).unwrap();
                let mut offsets: Vec<_> = source.char_indices().map(|(i, _)| i).collect();
                offsets.push(source.len());
                for i in offsets.into_iter().step_by(3) {
                    let r = std::panic::catch_unwind(|| {
                        axiom::graph::code::script::extract(lang, &source[..i])
                    });
                    assert!(r.is_ok(), "{path} prefix {i}");
                }
            }
        }
    }
    for lang in [Language::Python, Language::JavaScript] {
        for s in ["([{'\"", "\0💥\n\\/ /* ]", "def f(\n", "`unterminated"] {
            assert!(
                std::panic::catch_unwind(|| axiom::graph::code::script::extract(lang, s)).is_ok()
            );
        }
    }
    assert!(!axiom::graph::code::script::extract(Language::Rust, "")
        .errors
        .is_empty());
}

#[test]
fn scopes_interpolation_and_positions() {
    let p = extract(Language::Python, "def run():\n    for a, b in pairs:\n        a()\n        b()\n    with manager() as item:\n        item()\n    try:\n        fail()\n    except Error as err:\n        err()\n    result = [item() for item in items]\n    item()\n    global outside\n    outside = 1\n    outside()\n    text = f'hi {a()}'\n");
    for name in ["a", "b", "item", "err"] {
        assert!(
            p.refs.iter().any(|r| r.name == name && r.shadowed),
            "{name}"
        );
    }
    assert!(p.refs.iter().any(|r| r.name == "outside" && !r.shadowed));
    assert!(p
        .refs
        .iter()
        .any(|r| r.name == "a" && r.span.line == 16 && r.span.col == 18));
    let comp = extract(Language::Python, "value = [x() for x in items]\nx()\n");
    assert!(comp
        .refs
        .iter()
        .any(|r| r.name == "x" && r.span.line == 1 && r.shadowed));
    assert!(comp
        .refs
        .iter()
        .any(|r| r.name == "x" && r.span.line == 2 && !r.shadowed));
    let j = extract(Language::Tsx, "function outer({a, b: c}: T) {\n  const nested = (p: T) => p.run();\n  { let local = a; local(); c(); }\n  try {} catch (err) { err(); }\n  return <Comp>function fake() {} import bad from 'fake'<Inner />{a()}</Comp>;\n}\nconst message = `hello ${call()}`;\nrequire(123);\n");
    assert!(j
        .defs
        .iter()
        .any(|d| d.qualname == "outer.nested" && d.kind == "function"));
    assert!(!j.defs.iter().any(|d| d.name == "fake"));
    assert!(j.imports.is_empty());
    for name in ["local", "c", "err", "run"] {
        assert!(
            j.refs.iter().any(|r| r.name == name && r.shadowed),
            "{name}: {:?}",
            j.refs
        );
    }
    for name in ["Comp", "Inner", "call"] {
        assert!(j.refs.iter().any(|r| r.name == name), "{name}");
    }
    assert!(j.refs.iter().any(|r| r.name == "require" && r.dynamic));
}

#[test]
fn imports_definitions_and_language_specific_names() {
    let py = extract(Language::Python, "import a.b, c.d as cd\nfrom .. import (one as first, two)\npublic = 1\n_hidden = 2\ndef outer(x):\n    nonlocal y\n    y = 2\n    y()\n    type(x)\n    __import__('x')\n    return x.field()\n");
    assert_eq!(py.imports.len(), 3);
    assert_eq!(py.imports[0].module, "a.b");
    assert_eq!(py.imports[1].namespace_as.as_deref(), Some("cd"));
    assert_eq!(py.imports[2].module, "..");
    assert_eq!(py.imports[2].names[0].alias.as_deref(), Some("first"));
    assert!(py
        .defs
        .iter()
        .any(|d| d.name == "public" && d.exported && d.kind == "variable"));
    assert!(py.defs.iter().any(|d| d.name == "_hidden" && !d.exported));
    assert!(py.refs.iter().any(|r| r.name == "y" && !r.shadowed));
    assert!(py.refs.iter().any(|r| r.name == "type" && r.kind == "call"));
    assert!(py.refs.iter().any(|r| r.name == "__import__" && r.dynamic));
    assert!(py
        .refs
        .iter()
        .any(|r| r.name == "field" && r.qualifier == ["x"] && r.shadowed));
    let js = extract(Language::JavaScript, "const x = require('m');\nexport function run(p) { this.work(); return p(); }\nconst n = require(`name`); require('prefix' + x);\n");
    assert_eq!(js.imports.len(), 1);
    assert_eq!(js.imports[0].default_as.as_deref(), Some("x"));
    assert!(js.defs.iter().any(|d| d.name == "run" && d.exported));
    assert!(js
        .refs
        .iter()
        .any(|r| r.name == "work" && r.qualifier == ["this"]));
    assert_eq!(
        js.refs
            .iter()
            .filter(|r| r.name == "require" && r.dynamic)
            .count(),
        2
    );
}

#[test]
fn malformed_reports_errors_and_unicode_columns_are_characters() {
    for (lang, source) in [
        (Language::Python, "def f(\n"),
        (Language::JavaScript, "const x = 'oops"),
        (Language::Jsx, "function f() { /*"),
    ] {
        assert!(!axiom::graph::code::script::extract(lang, source)
            .errors
            .is_empty());
    }
    let p = extract(Language::Python, "é = 1; fn()\n");
    assert_eq!(p.refs.iter().find(|r| r.name == "fn").unwrap().span.col, 8);
}

#[test]
fn multiple_bindings() {
    let r = extract(
        Language::JavaScript,
        "export const a = 1, b = 2; function f() { let x = 1, y = 2; y(); }\n",
    );
    assert!(r.defs.iter().any(|d| d.name == "a" && d.exported));
    assert!(r.defs.iter().any(|d| d.name == "b" && d.exported));
    assert!(r.refs.iter().any(|r| r.name == "y" && r.shadowed));
}

#[test]
fn loop_scopes() {
    let py = extract(
        Language::Python,
        "v = [(x(), y()) for x in xs for y in ys]\ny()\n",
    );
    assert!(py
        .refs
        .iter()
        .any(|r| r.name == "y" && r.span.line == 1 && r.shadowed));
    assert!(py
        .refs
        .iter()
        .any(|r| r.name == "y" && r.span.line == 2 && !r.shadowed));
    let js = extract(
        Language::JavaScript,
        "function run() { for (let item of items) { item(); } item(); }\n",
    );
    let refs: Vec<_> = js.refs.iter().filter(|r| r.name == "item").collect();
    assert_eq!(refs.len(), 2);
    assert!(refs[0].shadowed && !refs[1].shadowed);
}

#[test]
fn types_reexports_and_literal_dynamic_import() {
    let x = extract(Language::TypeScript, "function f(p: Param): Result { return p; }\ntype Alias = Body;\ninterface I { field: Field; }\nclass C { method(p: MethodParam): MethodResult { return p; } }\nexport {a as b} from 'm';\nexport {a};\nimport('lit');\n");
    for name in [
        "Param",
        "Result",
        "Body",
        "Field",
        "MethodParam",
        "MethodResult",
    ] {
        assert!(
            x.refs.iter().any(|r| r.name == name && r.kind == "type"),
            "{name}: {:?}",
            x.refs
        );
    }
    assert_eq!(x.imports.len(), 1);
    let imp = &x.imports[0];
    assert_eq!((imp.kind, imp.module.as_str()), ("re_export", "m"));
    assert_eq!(imp.names.len(), 1);
    assert_eq!(
        (imp.names[0].name.as_str(), imp.names[0].alias.as_deref()),
        ("a", Some("b"))
    );
    assert!(x
        .refs
        .iter()
        .any(|r| r.name == "import" && r.kind == "call" && r.dynamic));
}

#[test]
fn literal_token_text_does_not_change_scopes() {
    let py = extract(Language::Python, "def outer(local):\n    \"\"\"import\"\"\"\n    'def'\n    foo(')')\n    x.join(',')\n    values = ['{', '}', '(', ')', ',', '.']\n    local()\n    def inner():\n        local()\n    inner()\nafter()\n");
    assert!(py.errors.is_empty(), "{:?}", py.errors);
    assert!(py.imports.is_empty());
    assert_eq!(
        py.defs
            .iter()
            .filter(|d| d.kind == "function")
            .map(|d| d.qualname.as_str())
            .collect::<Vec<_>>(),
        ["outer", "outer.inner"]
    );
    for (name, line, shadowed, enclosing) in [
        ("local", 7, true, Some("outer")),
        ("local", 9, true, Some("outer.inner")),
        ("inner", 10, true, Some("outer")),
        ("after", 11, false, None),
    ] {
        let r = py
            .refs
            .iter()
            .find(|r| r.name == name && r.span.line == line)
            .unwrap();
        assert_eq!(
            (
                r.shadowed,
                r.enclosing.map(|i| py.defs[i].qualname.as_str())
            ),
            (shadowed, enclosing)
        );
    }
    let js = extract(Language::JavaScript, "function outer(local) {\n const a = '{', b = '}', c = '(', d = ')', e = ',', dot = '.';\n const t = `{`, u = `}`, v = `(`, w = `)`, z = `,`;\n 'import'; 'def'; `import`; `def`;\n foo(')'); x.join(','); local(); b(); dot();\n function inner() { local(); }\n inner();\n}\nafter();\n");
    assert!(js.errors.is_empty(), "{:?}", js.errors);
    assert!(js.imports.is_empty());
    assert_eq!(
        js.defs
            .iter()
            .filter(|d| d.kind == "function")
            .map(|d| d.qualname.as_str())
            .collect::<Vec<_>>(),
        ["outer", "outer.inner"]
    );
    for (name, line, shadowed, enclosing) in [
        ("local", 5, true, Some("outer")),
        ("b", 5, true, Some("outer")),
        ("dot", 5, true, Some("outer")),
        ("local", 6, true, Some("outer.inner")),
        ("inner", 7, true, Some("outer")),
        ("after", 9, false, None),
    ] {
        let r = js
            .refs
            .iter()
            .find(|r| r.name == name && r.span.line == line)
            .unwrap();
        assert_eq!(
            (
                r.shadowed,
                r.enclosing.map(|i| js.defs[i].qualname.as_str())
            ),
            (shadowed, enclosing)
        );
    }
}
