use crate::graph::code::{Extraction, Language, Ref};
use std::collections::{HashMap, HashSet};

pub struct FileInput<'a> {
    pub path: &'a str,
    pub extraction: &'a Extraction,
    pub source: Option<&'a str>,
}
impl<'a> FileInput<'a> {
    pub fn new(path: &'a str, extraction: &'a Extraction) -> Self {
        Self {
            path,
            extraction,
            source: None,
        }
    }
    pub fn with_source(mut self, source: &'a str) -> Self {
        self.source = Some(source);
        self
    }
}
#[derive(Clone, Debug)]
pub struct RefResolution {
    pub resolution: &'static str,
    pub target: Option<(usize, usize)>,
    pub candidates: Vec<(usize, usize)>,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct Resolution {
    pub refs: Vec<Vec<RefResolution>>,
    pub imports: Vec<Vec<Option<usize>>>,
}

type Target = (usize, usize);
struct Resolver<'a> {
    files: &'a [FileInput<'a>],
    paths: HashMap<String, usize>,
    imports: Vec<Vec<Option<usize>>>,
}
fn normalize(path: &str) -> String {
    let mut parts = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}
impl<'a> Resolver<'a> {
    fn module(&self, f: usize, module: &str) -> Option<usize> {
        let path = self.files[f].path;
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
        let lang = self.files[f].extraction.language?;
        let base = match lang {
            Language::Rust => {
                let m = module.replace("::", "/");
                if let Some(rest) = m.strip_prefix("crate/") {
                    rest.to_owned()
                } else if m == "crate" {
                    "".into()
                } else if let Some(rest) = m.strip_prefix("super/") {
                    format!("{dir}/{rest}")
                } else if let Some(rest) = m.strip_prefix("self/") {
                    format!("{dir}/{rest}")
                } else {
                    format!("{dir}/{m}")
                }
            }
            Language::Python => {
                let dots = module.chars().take_while(|c| *c == '.').count();
                if dots == 0 {
                    module.replace('.', "/")
                } else {
                    let mut d = dir.to_owned();
                    for _ in 1..dots {
                        d = d.rsplit_once('/').map_or("", |x| x.0).to_owned();
                    }
                    format!("{d}/{}", module[dots..].replace('.', "/"))
                }
            }
            _ => {
                if !module.starts_with('.') {
                    return None;
                }
                format!("{dir}/{module}")
            }
        };
        let base = normalize(&base);
        let mut choices = vec![base.clone()];
        match lang {
            Language::Rust => {
                choices.push(format!("{base}.rs"));
                choices.push(format!("{base}/mod.rs"));
            }
            Language::Python => {
                choices.push(format!("{base}.py"));
                choices.push(format!("{base}.pyi"));
                choices.push(format!("{base}/__init__.py"));
            }
            _ => {
                for ext in ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"] {
                    choices.push(format!("{base}.{ext}"));
                    choices.push(format!("{base}/index.{ext}"));
                }
            }
        }
        choices.iter().find_map(|p| self.paths.get(p).copied())
    }
    fn exported(&self, f: usize, name: &str, seen: &mut HashSet<(usize, String)>) -> Vec<Target> {
        if !seen.insert((f, name.into())) {
            return vec![];
        }
        let ex = self.files[f].extraction;
        let mut found: Vec<_> = ex
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.name == name
                    && d.parent.is_none()
                    && (ex.language == Some(Language::Python) || d.exported)
            })
            .map(|(i, _)| (f, i))
            .collect();
        for (i, import) in ex.imports.iter().enumerate() {
            // Rust's Extraction does not record use visibility, so a pub use
            // cannot be distinguished from a private use without source text.
            let reexport = match ex.language {
                Some(Language::Python) => import.kind == "from" || import.kind == "import",
                Some(Language::Rust) => false,
                _ => import.kind == "re_export",
            };
            if !reexport {
                continue;
            }
            let Some(to) = self.imports[f][i] else {
                continue;
            };
            for n in &import.names {
                if n.alias.as_deref().unwrap_or(&n.name) == name {
                    found.extend(self.exported(to, &n.name, seen));
                }
            }
            if import.glob {
                found.extend(self.exported(to, name, seen));
            }
        }
        found.sort();
        found.dedup();
        found
    }
    fn all(&self, name: &str) -> Vec<Target> {
        self.files
            .iter()
            .enumerate()
            .flat_map(|(f, file)| {
                file.extraction
                    .defs
                    .iter()
                    .enumerate()
                    .filter(move |(_, d)| d.name == name)
                    .map(move |(d, _)| (f, d))
            })
            .collect()
    }
    fn require_member(&self, f: usize, name: &str) -> Vec<Target> {
        let file = &self.files[f];
        let candidates: Vec<_> = file
            .extraction
            .defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.parent.is_none() && d.name == name)
            .map(|(i, _)| (f, i))
            .collect();
        let exported: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|&(_, d)| file.extraction.defs[d].exported)
            .collect();
        if !exported.is_empty() {
            return exported;
        }
        // Accept only a simple, unambiguous shorthand object assignment.  The
        // extractor does not retain CommonJS exports; absent source is not proof.
        let Some(source) = file.source else {
            return vec![];
        };
        if candidates.len() != 1
            || source.contains(['/', '\'', '"', '`'])
            || source.matches("exports").count() != 1
            || file
                .extraction
                .defs
                .iter()
                .any(|d| d.parent.is_none() && (d.name == "module" || d.name == "exports"))
        {
            return vec![];
        }
        let mut depth = 0i32;
        let mut assignment = None;
        for line in source.lines() {
            if depth == 0 {
                assignment = assignment.or_else(|| {
                    let line = line.trim().strip_prefix("module.exports")?.trim_start();
                    let object = line.strip_prefix('=')?.trim().strip_suffix(';')?.trim();
                    object.strip_prefix('{')?.strip_suffix('}')
                });
            }
            for c in line.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => (),
                }
                if depth < 0 {
                    return vec![];
                }
            }
        }
        if depth != 0 {
            return vec![];
        }
        if assignment.is_some_and(|object| {
            let fields: Vec<_> = object
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            !fields.is_empty()
                && fields.iter().all(|s| {
                    s.chars().enumerate().all(|(i, c)| {
                        c == '_'
                            || c == '$'
                            || c.is_ascii_alphabetic()
                            || (i > 0 && c.is_ascii_digit())
                    })
                })
                && fields.contains(&name)
        }) {
            candidates
        } else {
            vec![]
        }
    }

    fn rust_parameter_method(&self, f: usize, r: &Ref) -> Vec<Target> {
        let ex = self.files[f].extraction;
        if ex.language != Some(Language::Rust) || r.kind != "call" || r.qualifier.len() != 1 {
            return vec![];
        }
        let Some(scope) = r.enclosing.and_then(|i| ex.defs.get(i)) else {
            return vec![];
        };
        let Ok(sig) = syn::parse_str::<syn::Signature>(&scope.signature) else {
            return vec![];
        };
        let mut types = sig.inputs.iter().filter_map(|arg| {
            let syn::FnArg::Typed(arg) = arg else {
                return None;
            };
            let syn::Pat::Ident(pat) = &*arg.pat else {
                return None;
            };
            if pat.ident != r.qualifier[0] || pat.mutability.is_some() || pat.subpat.is_some() {
                return None;
            }
            let ty = match &*arg.ty {
                syn::Type::Reference(ty) => &*ty.elem,
                ty => ty,
            };
            let syn::Type::Path(ty) = ty else { return None };
            if ty.qself.is_some() || ty.path.segments.len() != 1 {
                return None;
            }
            let part = ty.path.segments.first()?;
            if !matches!(part.arguments, syn::PathArguments::None) {
                return None;
            }
            Some(part.ident.to_string())
        });
        let Some(ty) = types.next() else {
            return vec![];
        };
        if types.next().is_some()
            || ex
                .defs
                .iter()
                .filter(|d| d.name == ty && d.kind == "struct" && d.parent.is_none())
                .count()
                != 1
        {
            return vec![];
        }
        ex.defs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                d.kind == "method"
                    && d.name == r.name
                    && d.parent.is_none()
                    && d.qualname == format!("{ty}.{}", r.name)
            })
            .map(|(i, _)| (f, i))
            .collect()
    }
    fn class_receiver_method(&self, f: usize, r: &Ref) -> Vec<Target> {
        if r.kind != "call" || r.qualifier.len() != 1 {
            return vec![];
        }
        let ex = self.files[f].extraction;
        let receiver = r.qualifier[0].as_str();
        let Some(method) = r.enclosing.and_then(|i| ex.defs.get(i)) else {
            return vec![];
        };
        if method.kind != "method" {
            return vec![];
        }
        if matches!(ex.language, Some(Language::Python)) && receiver == "self"
            || matches!(
                ex.language,
                Some(Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx)
            ) && receiver == "this"
        {
            let Some(owner) = method.parent.filter(|&i| ex.defs[i].kind == "class") else {
                return vec![];
            };
            return ex
                .defs
                .iter()
                .enumerate()
                .filter(|(_, d)| d.parent == Some(owner) && d.kind == "method" && d.name == r.name)
                .map(|(i, _)| (f, i))
                .collect();
        }
        if ex.language == Some(Language::Rust) && receiver == "self" {
            let Ok(sig) = syn::parse_str::<syn::Signature>(&method.signature) else {
                return vec![];
            };
            if !sig
                .inputs
                .iter()
                .any(|p| matches!(p, syn::FnArg::Receiver(_)))
            {
                return vec![];
            }
            let Some((owner, _)) = method.qualname.rsplit_once('.') else {
                return vec![];
            };
            if ex
                .defs
                .iter()
                .filter(|d| d.kind == "struct" && d.qualname == owner)
                .count()
                != 1
            {
                return vec![];
            }
            return ex
                .defs
                .iter()
                .enumerate()
                .filter(|(_, d)| d.kind == "method" && d.qualname == format!("{owner}.{}", r.name))
                .map(|(i, _)| (f, i))
                .collect();
        }
        vec![]
    }
    fn result(&self, f: usize, r: &Ref) -> RefResolution {
        let candidates = self.all(&r.name);
        let make = |resolution, targets: Vec<Target>, reason: &str| RefResolution {
            resolution,
            target: if resolution == "resolved" {
                targets.first().copied()
            } else {
                None
            },
            candidates: targets,
            reason: reason.into(),
        };
        if r.dynamic {
            return make("unresolved", candidates, "dynamic reference");
        }
        let ex = self.files[f].extraction;
        if r.qualifier.is_empty() {
            let mut scope = r.enclosing;
            loop {
                if r.shadowed && scope.is_none() {
                    break;
                }
                if scope.is_some_and(|s| ex.defs[s].kind == "class") {
                    scope = ex.defs[scope.unwrap()].parent;
                    if r.shadowed {
                        break;
                    }
                    continue;
                }
                let local: Vec<_> = ex
                    .defs
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| {
                        d.name == r.name
                            && d.parent == scope
                            && !(d.kind == "variable"
                                && ex.imports.iter().any(|imp| {
                                    imp.kind == "require"
                                        && imp.span.line == d.span.line
                                        && imp.names.iter().any(|n| {
                                            n.alias.as_deref().unwrap_or(&n.name) == r.name
                                        })
                                }))
                    })
                    .map(|(i, _)| (f, i))
                    .collect();
                if local.len() == 1 {
                    return make("resolved", local, "lexical scope");
                }
                if local.len() > 1 {
                    return make("ambiguous", local, "multiple lexical definitions");
                }
                if r.shadowed {
                    break;
                }
                if let Some(s) = scope {
                    scope = ex.defs[s].parent;
                } else {
                    break;
                }
            }
        }
        let mut methods = self.class_receiver_method(f, r);
        if !methods.is_empty()
            && ex.language == Some(Language::Rust)
            && r.qualifier.len() == 1
            && r.qualifier[0] == "self"
        {
            let free: Vec<_> = ex
                .defs
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.kind == "function"
                        && d.name == r.name
                        && d.parent.is_none_or(|p| ex.defs[p].kind == "module")
                })
                .map(|(i, _)| (f, i))
                .collect();
            if !free.is_empty() {
                methods.extend(free);
                return make("ambiguous", methods, "self receiver or module path");
            }
        }
        if methods.is_empty() && !r.shadowed {
            methods = self.rust_parameter_method(f, r);
        }
        if methods.len() == 1 {
            return make("resolved", methods, "typed receiver");
        }
        if methods.len() > 1 {
            return make("ambiguous", methods, "multiple receiver methods");
        }
        if r.shadowed {
            return make("unresolved", candidates, "shadowed by local binding");
        }
        if ex.language == Some(Language::Rust) && !r.qualifier.is_empty() {
            let mut parent = None;
            let mut valid = true;
            for part in &r.qualifier {
                if part == "crate" || part == "self" {
                    continue;
                }
                let modules: Vec<_> = ex
                    .defs
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.kind == "module" && d.parent == parent && d.name == *part)
                    .collect();
                if modules.len() != 1 {
                    valid = false;
                    break;
                }
                parent = Some(modules[0].0);
            }
            if valid && parent.is_some() {
                let local: Vec<_> = ex
                    .defs
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.parent == parent && d.name == r.name)
                    .map(|(i, _)| (f, i))
                    .collect();
                if local.len() == 1 {
                    return make("resolved", local, "module path");
                }
                if local.len() > 1 {
                    return make("ambiguous", local, "multiple module definitions");
                }
            }
        }
        let mut imported = Vec::new();
        let mut external = false;
        for (i, imp) in ex.imports.iter().enumerate() {
            // Extracted imports have spans but no parent; infer their lexical container.
            let container = ex
                .defs
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.span.line <= imp.span.line
                        && imp.span.line <= d.span.end_line
                        && matches!(d.kind, "module" | "function" | "method" | "class")
                })
                .min_by_key(|(_, d)| d.span.end_line - d.span.line)
                .map(|(i, _)| i);
            if let Some(container) = container {
                let mut scope = r.enclosing;
                let mut visible = false;
                while let Some(i) = scope {
                    if i == container {
                        visible = true;
                        break;
                    }
                    scope = ex.defs[i].parent;
                }
                if !visible {
                    continue;
                }
            }
            let to = self.imports[f][i];
            let is_external = match ex.language {
                Some(Language::Rust) => {
                    !imp.module.starts_with("crate::")
                        && !imp.module.starts_with("self::")
                        && !imp.module.starts_with("super::")
                        && imp.kind != "mod"
                }
                Some(Language::Python) => !imp.module.starts_with('.'),
                _ => !imp.module.starts_with('.'),
            };
            if r.qualifier.is_empty() {
                for n in &imp.names {
                    if n.alias.as_deref().unwrap_or(&n.name) == r.name {
                        if let Some(to) = to {
                            if imp.kind == "require" {
                                imported.extend(self.require_member(to, &n.name));
                            } else {
                                imported.extend(self.exported(to, &n.name, &mut HashSet::new()));
                            }
                        } else if is_external {
                            external = true;
                        }
                    }
                }
                if imp.glob {
                    if let Some(to) = to {
                        imported.extend(self.exported(to, &r.name, &mut HashSet::new()));
                    }
                }
                if imp.default_as.as_deref() == Some(&r.name) {
                    if let Some(to) = to {
                        imported.extend(self.exported(to, "default", &mut HashSet::new()));
                    } else if is_external {
                        external = true;
                    }
                }
            } else {
                let q = &r.qualifier[0];
                let matches = imp.namespace_as.as_deref() == Some(q)
                    || (imp.kind == "require" && imp.default_as.as_deref() == Some(q))
                    || (imp.kind == "mod" && imp.module == *q)
                    || imp
                        .names
                        .iter()
                        .any(|n| n.alias.as_deref().unwrap_or(&n.name) == q)
                    || (imp.kind == "import" && imp.module.split('.').next() == Some(q));
                if matches {
                    if let Some(to) = to {
                        if imp.kind == "require" && r.qualifier.len() == 1 {
                            imported.extend(self.require_member(to, &r.name));
                        } else if imp.kind != "require" {
                            imported.extend(self.exported(to, &r.name, &mut HashSet::new()));
                        }
                    } else if is_external {
                        external = true;
                    }
                }
            }
        }
        imported.sort();
        imported.dedup();
        if imported.len() == 1 {
            return make("resolved", imported, "import path");
        }
        if imported.len() > 1 {
            return make("ambiguous", imported, "multiple imported definitions");
        }
        if external {
            return make("external", candidates, "non-project import");
        }
        if !r.qualifier.is_empty() {
            return make(
                "unresolved",
                candidates,
                "undetermined qualifier or receiver",
            );
        }
        if candidates.is_empty() {
            make("unresolved", candidates, "no visible definition")
        } else {
            make("ambiguous", candidates, "no proven path to definition")
        }
    }
}
pub fn resolve(files: &[FileInput]) -> Resolution {
    let mut r = Resolver {
        files,
        paths: files
            .iter()
            .enumerate()
            .map(|(i, f)| (normalize(f.path), i))
            .collect(),
        imports: Vec::new(),
    };
    r.imports = files
        .iter()
        .enumerate()
        .map(|(f, file)| {
            file.extraction
                .imports
                .iter()
                .map(|imp| r.module(f, &imp.module))
                .collect()
        })
        .collect();
    let refs = files
        .iter()
        .enumerate()
        .map(|(f, file)| {
            file.extraction
                .refs
                .iter()
                .map(|reference| r.result(f, reference))
                .collect()
        })
        .collect();
    Resolution {
        refs,
        imports: r.imports,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::code::{extract, language_of, symbol_keys};
    use serde::Deserialize;
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    #[derive(Deserialize)]
    struct Case {
        id: String,
        refs: Vec<ExpectedRef>,
    }
    #[derive(Deserialize)]
    struct ExpectedRef {
        file: String,
        line: u32,
        name: String,
        nth: usize,
        expect: Expect,
    }
    #[derive(Deserialize)]
    struct Expect {
        resolution: String,
        target: Option<String>,
    }
    #[derive(Default, Debug)]
    struct Score {
        expected: usize,
        correct: usize,
        wrong: usize,
    }
    impl Score {
        fn add(
            &mut self,
            expected: &str,
            target: Option<&str>,
            actual: &str,
            actual_target: Option<&str>,
        ) -> bool {
            if expected == "resolved" {
                self.expected += 1;
            }
            let correct = expected == "resolved" && actual == "resolved" && target == actual_target;
            if correct {
                self.correct += 1;
            }
            if actual == "resolved" && (expected != "resolved" || target != actual_target) {
                self.wrong += 1;
            }
            correct
        }
        fn passes(&self) -> bool {
            self.wrong == 0 && self.expected > 0 && self.correct * 10 >= self.expected * 7
        }
    }
    fn gather(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                gather(&path, root, out);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    path,
                ));
            }
        }
    }
    #[test]
    fn shadowed_and_scoped_imports_are_conservative() {
        let main = extract(Language::Rust, "mod lib; mod inner { use crate::lib::work; fn inside() { work(); } } fn outside() { work(); } fn shadow(work: fn()) { work(); }");
        let lib = extract(Language::Rust, "pub fn work() {}");
        let files = [
            FileInput::new("main.rs", &main),
            FileInput::new("lib.rs", &lib),
        ];
        let result = resolve(&files);
        let calls: Vec<_> = main
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name == "work" && r.kind == "call")
            .collect();
        assert_eq!(calls.len(), 3);
        assert_eq!(result.refs[0][calls[0].0].target, Some((1, 0)));
        assert_ne!(result.refs[0][calls[1].0].resolution, "resolved");
        assert_eq!(result.refs[0][calls[2].0].resolution, "unresolved");
    }
    #[test]
    fn shadowed_refs_do_not_escape_their_own_scope() {
        let source = "class A:\n    def compute(self):\n        pass\n    def run(self, compute):\n        compute()\ndef outer():\n    compute = 1\n    def inner(compute):\n        compute()\ndef local():\n    def compute():\n        pass\n    compute()\n";
        let ex = extract(Language::Python, source);
        let result = resolve(&[FileInput::new("main.py", &ex)]);
        let calls: Vec<_> = ex
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name == "compute" && r.kind == "call")
            .collect();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|(_, r)| r.shadowed));
        assert_eq!(result.refs[0][calls[0].0].resolution, "unresolved");
        assert_eq!(result.refs[0][calls[1].0].resolution, "unresolved");
        let target = result.refs[0][calls[2].0].target.unwrap().1;
        assert_eq!(ex.defs[target].qualname, "local.compute");
    }
    #[test]
    fn rust_self_may_be_a_module_path() {
        let ex = extract(Language::Rust, "fn helper() {} struct Owner; impl Owner { fn helper(&self) {} fn run(&self) { self.helper(); self::helper(); } }");
        let result = resolve(&[FileInput::new("main.rs", &ex)]);
        let calls: Vec<_> = ex
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name == "helper" && r.kind == "call")
            .collect();
        assert_eq!(calls.len(), 2);
        for (i, _) in calls {
            assert_eq!(result.refs[0][i].resolution, "ambiguous");
            assert_eq!(result.refs[0][i].candidates.len(), 2);
        }
    }
    #[test]
    fn ordinary_import_is_not_an_ecma_reexport() {
        let main = extract(
            Language::TypeScript,
            "import { work } from './api'; work();",
        );
        let api = extract(Language::TypeScript, "import { work } from './lib';");
        let lib = extract(Language::TypeScript, "export function work() {} ");
        let files = [
            FileInput::new("main.ts", &main),
            FileInput::new("api.ts", &api),
            FileInput::new("lib.ts", &lib),
        ];
        let result = resolve(&files);
        let call = main
            .refs
            .iter()
            .position(|r| r.name == "work" && r.kind == "call")
            .unwrap();
        assert_ne!(result.refs[0][call].resolution, "resolved");
    }
    #[test]
    fn require_members_and_ambiguous_exports() {
        let main = extract(
            Language::JavaScript,
            "const lib = require('./lib'); lib.work(); lib.dupe();",
        );
        let lib = extract(
            Language::JavaScript,
            "function work() {}\nfunction dupe() {}\nfunction dupe() {}",
        );
        let files = [
            FileInput::new("main.js", &main),
            FileInput::new("lib.js", &lib),
        ];
        let result = resolve(&files);
        let call = |name: &str| {
            main.refs
                .iter()
                .position(|r| r.name == name && r.kind == "call")
                .unwrap()
        };
        assert_ne!(result.refs[0][call("work")].resolution, "resolved");
        assert_ne!(result.refs[0][call("dupe")].resolution, "resolved");
        let source = "function work() {}\nfunction dupe() {}\nfunction dupe() {}\nmodule.exports = { work, dupe };";
        let files = [
            FileInput::new("main.js", &main),
            FileInput::new("lib.js", &lib).with_source(source),
        ];
        let result = resolve(&files);
        assert_eq!(result.refs[0][call("work")].target, Some((1, 0)));
        assert_ne!(result.refs[0][call("dupe")].resolution, "resolved");
        let source = "function work() {}\nmodule.exports = { other };";
        let files = [
            FileInput::new("main.js", &main),
            FileInput::new("lib.js", &lib).with_source(source),
        ];
        assert_ne!(resolve(&files).refs[0][call("work")].resolution, "resolved");
        let source = "function work() {}\nfunction setup() {\nmodule.exports = { work };\n}";
        let files = [
            FileInput::new("main.js", &main),
            FileInput::new("lib.js", &lib).with_source(source),
        ];
        assert_ne!(resolve(&files).refs[0][call("work")].resolution, "resolved");
    }
    #[test]
    fn receiver_types_and_unknowns() {
        let main = extract(Language::Rust, "struct Worker; impl Worker { fn work(&self) {} fn run(&self) { self.work(); } } fn typed(w: Worker) { w.work(); } fn unknown(w: &dyn Trait) { w.work(); }");
        let files = [FileInput::new("main.rs", &main)];
        let result = resolve(&files);
        let calls: Vec<_> = main
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.name == "work" && r.kind == "call")
            .collect();
        assert_eq!(calls.len(), 3);
        assert_eq!(result.refs[0][calls[0].0].resolution, "resolved");
        assert_eq!(result.refs[0][calls[1].0].resolution, "resolved");
        assert_ne!(result.refs[0][calls[2].0].resolution, "resolved");
        for (language, source) in [
            (
                Language::Python,
                "class Worker:\n    def work(self):\n        pass\n    def run(self):\n        self.work()\n",
            ),
            (
                Language::JavaScript,
                "class Worker { work() {} run() { this.work(); } }",
            ),
        ] {
            let extraction = extract(language, source);
            let files = [FileInput::new("main", &extraction)];
            let result = resolve(&files);
            let call = extraction
                .refs
                .iter()
                .position(|r| r.name == "work" && r.kind == "call")
                .unwrap();
            assert_eq!(result.refs[0][call].resolution, "resolved", "{language:?}: {extraction:?}");
        }
    }
    #[test]
    fn scoring_negative_controls() {
        let mut wrong = Score::default();
        wrong.add("resolved", Some("a"), "resolved", Some("b"));
        assert_eq!(wrong.wrong, 1);
        assert!(!wrong.passes());
        let mut empty = Score::default();
        empty.add("resolved", Some("a"), "unresolved", None);
        assert!(!empty.passes());
    }
    #[test]
    fn holdout_resolution() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph/holdout/resolution");
        let mut scores = Vec::new();
        for lang in ["rust", "python", "ts", "js"] {
            let mut score = Score::default();
            let mut cases: Vec<_> = fs::read_dir(root.join(lang))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.is_dir())
                .collect();
            cases.sort();
            for dir in cases {
                let case: Case =
                    serde_json::from_slice(&fs::read(dir.join("case.json")).unwrap()).unwrap();
                let tree = dir.join("files");
                let mut paths = Vec::new();
                gather(&tree, &tree, &mut paths);
                paths.sort();
                let sources: Vec<_> = paths
                    .iter()
                    .map(|(_, abs)| fs::read_to_string(abs).unwrap())
                    .collect();
                let extracted: Vec<_> = paths
                    .iter()
                    .zip(&sources)
                    .map(|((p, _), source)| extract(language_of(p).unwrap(), source))
                    .collect();
                let inputs: Vec<_> = paths
                    .iter()
                    .zip(&extracted)
                    .zip(&sources)
                    .map(|(((p, _), e), source)| FileInput::new(p, e).with_source(source))
                    .collect();
                let result = resolve(&inputs);
                for expected in case.refs {
                    let fi = paths.iter().position(|(p, _)| *p == expected.file).unwrap();
                    let mut matching: Vec<_> = extracted[fi]
                        .refs
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| r.span.line == expected.line && r.name == expected.name)
                        .collect();
                    matching.sort_by_key(|(_, r)| r.span.col);
                    let outcome = matching
                        .get(expected.nth - 1)
                        .map(|(i, _)| &result.refs[fi][*i]);
                    let actual = outcome.map_or("missing", |o| o.resolution);
                    let target = outcome.and_then(|o| o.target).map(|(f, d)| {
                        symbol_keys(&paths[f].0, &extracted[f].defs)[d]
                            .trim_start_matches("sym:")
                            .to_owned()
                    });
                    let correct = score.add(
                        &expected.expect.resolution,
                        expected.expect.target.as_deref(),
                        actual,
                        target.as_deref(),
                    );
                    if (expected.expect.resolution == "resolved" && !correct)
                        || (expected.expect.resolution != "resolved"
                            && actual != expected.expect.resolution)
                    {
                        eprintln!(
                            "{} {}:{} {} #{} expected {} {:?}, got {} {:?} ({:?})",
                            case.id,
                            expected.file,
                            expected.line,
                            expected.name,
                            expected.nth,
                            expected.expect.resolution,
                            expected.expect.target,
                            actual,
                            target,
                            outcome.map(|o| &o.reason)
                        );
                    }
                }
            }
            eprintln!(
                "{lang}: correct {}/{} wrong {}",
                score.correct, score.expected, score.wrong
            );
            scores.push((lang, score));
        }
        assert!(
            scores.iter().all(|(_, s)| s.passes()),
            "holdout thresholds not met: {scores:?}"
        );
    }
}
