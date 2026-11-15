use super::lexer::*;
use super::model::{Extraction, Import, ImportName};
use std::collections::HashMap;

fn matching(t: &[Token], start: usize, open: &str, close: &str) -> Option<usize> {
    let mut depth = 0i32;
    for i in start..t.len() {
        if at(t, i, open) {
            depth += 1
        } else if at(t, i, close) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}
fn statement_end(t: &[Token], start: usize) -> usize {
    let mut depth = 0i32;
    let mut i = start;
    while i < t.len() {
        if depth == 0 && (at(t, i, ";") || t[i].kind == Kind::Newline) {
            break;
        }
        if t[i].kind == Kind::Punct && matches!(t[i].text.as_str(), "(" | "[" | "{") {
            depth += 1
        } else if t[i].kind == Kind::Punct && matches!(t[i].text.as_str(), ")" | "]" | "}") {
            depth -= 1;
            if depth < 0 {
                break;
            }
        }
        i += 1
    }
    i
}
fn names(t: &[Token], a: usize, b: usize) -> Vec<ImportName> {
    let mut result = Vec::new();
    let mut i = a;
    while i < b {
        if id(t, i) && !keyword(&t[i].text) {
            let name = t[i].text.clone();
            i += 1;
            let alias = if at(t, i, "as") && id(t, i + 1) {
                let s = t[i + 1].text.clone();
                i += 2;
                Some(s)
            } else {
                None
            };
            result.push(ImportName { name, alias })
        } else {
            i += 1
        }
    }
    result
}
fn params(
    t: &[Token],
    a: usize,
    b: usize,
    scope: &mut Scope,
    skip: &mut [bool],
    types: &mut [bool],
) {
    let mut expect = true;
    let mut typepos = false;
    let mut i = a;
    while i < b {
        if expect && (at(t, i, "{") || at(t, i, "[")) {
            let close = if at(t, i, "{") { "}" } else { "]" };
            let end = matching(t, i, &t[i].text, close).unwrap_or(b).min(b);
            for j in i + 1..end {
                if id(t, j) && !keyword(&t[j].text) {
                    skip[j] = true;
                    if !at(t, j + 1, ":") {
                        scope.bindings.insert(t[j].text.clone());
                    }
                }
            }
            i = end.saturating_add(1);
            expect = false;
            continue;
        }
        if at(t, i, ",") {
            expect = true;
            typepos = false
        } else if at(t, i, ":") {
            typepos = true
        } else if at(t, i, "=") {
            typepos = false;
            expect = false
        } else if id(t, i) && expect && !keyword(&t[i].text) {
            scope.bindings.insert(t[i].text.clone());
            skip[i] = true;
            expect = false
        } else if typepos {
            types[i] = true
        }
        i += 1;
    }
}

pub fn extract(source: &str, jsx: bool) -> Extraction {
    let (t, errors) = lex(source, false, jsx);
    let n = t.len();
    let mut out = Extraction {
        errors,
        ..Default::default()
    };
    let mut skip = vec![false; n];
    let mut types = vec![false; n];
    let mut scope_at = vec![0; n];
    let mut scopes = vec![Scope {
        module: true,
        ..Default::default()
    }];
    let mut stack = vec![0usize];
    let mut opening: HashMap<usize, usize> = HashMap::new();
    let mut expression_scopes: HashMap<usize, usize> = HashMap::new();
    let mut i = 0;
    while i < n {
        let s = expression_scopes
            .get(&i)
            .copied()
            .unwrap_or(*stack.last().unwrap());
        scope_at[i] = s;
        if at(&t, i, "{") {
            let child = opening.remove(&i).unwrap_or_else(|| {
                let k = scopes.len();
                scopes.push(Scope {
                    parent: Some(s),
                    ..Default::default()
                });
                k
            });
            stack.push(child);
            i += 1;
            continue;
        }
        if at(&t, i, "}") {
            if stack.len() > 1 {
                stack.pop();
            }
            i += 1;
            continue;
        }
        if at(&t, i, "for") && at(&t, i + 1, "(") {
            if let Some(end) = matching(&t, i + 1, "(", ")") {
                if (i + 2..end).any(|j| at(&t, j, "let") || at(&t, j, "const")) {
                    let child = scopes.len();
                    scopes.push(Scope {
                        parent: Some(s),
                        ..Default::default()
                    });
                    for x in i + 2..end {
                        expression_scopes.insert(x, child);
                    }
                    if at(&t, end + 1, "{") {
                        opening.insert(end + 1, child);
                    } else {
                        let body_end = statement_end(&t, end + 1);
                        for x in end + 1..body_end {
                            expression_scopes.insert(x, child);
                        }
                    }
                }
            }
        }
        let exported = (i > 0 && at(&t, i - 1, "export"))
            || (i > 1
                && at(&t, i - 2, "export")
                && (at(&t, i - 1, "default") || at(&t, i - 1, "async")))
            || (i > 2
                && at(&t, i - 3, "export")
                && at(&t, i - 2, "default")
                && at(&t, i - 1, "async"));
        if (at(&t, i, "import") && !at(&t, i + 1, "("))
            || (at(&t, i, "export") && (at(&t, i + 1, "{") || at(&t, i + 1, "*")))
        {
            let end = statement_end(&t, i);
            let is_export = at(&t, i, "export");
            let from = (i + 1..end).find(|&j| at(&t, j, "from"));
            let literal = from
                .and_then(|j| t.get(j + 1))
                .filter(|x| x.kind == Kind::String)
                .or_else(|| {
                    if !is_export {
                        t.get(i + 1).filter(|x| x.kind == Kind::String)
                    } else {
                        None
                    }
                });
            if let Some(lit) = literal {
                let mut imp = Import {
                    kind: if is_export { "re_export" } else { "es" },
                    module: lit.text.clone(),
                    names: vec![],
                    glob: false,
                    default_as: None,
                    namespace_as: None,
                    span: t[i].span,
                };
                let limit = from.unwrap_or(end);
                let mut j = i + 1;
                if !is_export && j < limit && id(&t, j) && !at(&t, j, "type") {
                    imp.default_as = Some(t[j].text.clone());
                    j += 1;
                }
                while j < limit {
                    if at(&t, j, "*") {
                        if at(&t, j + 1, "as") && id(&t, j + 2) {
                            imp.namespace_as = Some(t[j + 2].text.clone());
                            j += 3
                        } else {
                            imp.glob = true;
                            j += 1
                        }
                    } else if at(&t, j, "{") {
                        let e = matching(&t, j, "{", "}").unwrap_or(limit).min(limit);
                        imp.names = names(&t, j + 1, e);
                        j = e + 1
                    } else {
                        j += 1
                    }
                }
                out.imports.push(imp);
            }
            for x in i..end {
                skip[x] = true;
                scope_at[x] = s
            }
            i = end;
            continue;
        }
        if t[i].kind == Kind::Id
            && matches!(
                t[i].text.as_str(),
                "function" | "class" | "interface" | "type" | "enum" | "namespace"
            )
            && id(&t, i + 1)
        {
            let kind = match t[i].text.as_str() {
                "function" => "function",
                "class" => "class",
                "interface" => "interface",
                "type" => "type",
                "enum" => "enum",
                _ => "namespace",
            };
            let name = i + 1;
            skip[i] = true;
            skip[name] = true;
            let parent = enclosing(&scopes, s);
            let d = add_def(&mut out, source, &t[name], kind, parent, exported);
            if s != 0 {
                scopes[s].bindings.insert(t[name].text.clone());
            }
            if kind == "function" && at(&t, name + 1, "(") {
                if let Some(end) = matching(&t, name + 1, "(", ")") {
                    let child = scopes.len();
                    scopes.push(Scope {
                        parent: Some(s),
                        def: Some(d),
                        ..Default::default()
                    });
                    params(&t, name + 2, end, &mut scopes[child], &mut skip, &mut types);
                    let mut b = end + 1;
                    while b < n && !at(&t, b, "{") && !at(&t, b, ";") {
                        if at(&t, b, ":") {
                            let mut x = b + 1;
                            while x < n && !at(&t, x, "{") && !at(&t, x, ";") {
                                types[x] = true;
                                x += 1
                            }
                        }
                        b += 1
                    }
                    if at(&t, b, "{") {
                        opening.insert(b, child);
                    }
                }
            } else if matches!(kind, "class" | "interface" | "namespace" | "enum") {
                let mut b = name + 1;
                while b < n && !at(&t, b, "{") && !at(&t, b, ";") {
                    b += 1
                }
                if at(&t, b, "{") {
                    let child = scopes.len();
                    scopes.push(Scope {
                        parent: Some(s),
                        def: Some(d),
                        ..Default::default()
                    });
                    opening.insert(b, child);
                    if kind == "interface" {
                        let end = matching(&t, b, "{", "}").unwrap_or(n);
                        let mut typepos = false;
                        for x in b + 1..end {
                            if at(&t, x, ":") {
                                typepos = true;
                            } else if at(&t, x, ";") || t[x].kind == Kind::Newline {
                                typepos = false;
                            } else if typepos {
                                types[x] = true;
                            }
                        }
                    }
                }
            } else if kind == "type" {
                let end = statement_end(&t, i);
                for x in name + 1..end {
                    types[x] = true
                }
            }
        }
        // Class methods.
        if id(&t, i)
            && !skip[i]
            && at(&t, i + 1, "(")
            && scopes[s]
                .def
                .is_some_and(|d| matches!(out.defs[d].kind, "class" | "interface"))
            && (i == 0 || !at(&t, i - 1, "."))
        {
            if let Some(end) = matching(&t, i + 1, "(", ")") {
                let mut b = end + 1;
                let mut return_type = false;
                while b < n && !at(&t, b, "{") && !at(&t, b, ";") && t[b].kind != Kind::Newline {
                    if at(&t, b, ":") {
                        return_type = true;
                    } else if return_type {
                        types[b] = true;
                    }
                    b += 1
                }
                if at(&t, b, "{") {
                    let d = add_def(
                        &mut out,
                        source,
                        &t[i],
                        "method",
                        enclosing(&scopes, s),
                        false,
                    );
                    skip[i] = true;
                    let child = scopes.len();
                    scopes.push(Scope {
                        parent: Some(s),
                        def: Some(d),
                        ..Default::default()
                    });
                    params(&t, i + 2, end, &mut scopes[child], &mut skip, &mut types);
                    opening.insert(b, child);
                }
            }
        }
        if t[i].kind == Kind::Id && matches!(t[i].text.as_str(), "const" | "let" | "var") {
            let end = statement_end(&t, i);
            let mut eq = i + 1;
            while eq < end && !at(&t, eq, "=") {
                eq += 1
            }
            let mut bound = Vec::new();
            if at(&t, i + 1, "{") || at(&t, i + 1, "[") {
                for j in i + 2..eq {
                    if id(&t, j) && !keyword(&t[j].text) && !at(&t, j + 1, ":") {
                        bound.push(j);
                        skip[j] = true
                    } else if id(&t, j) && at(&t, j + 1, ":") {
                        skip[j] = true
                    }
                }
            } else if id(&t, i + 1) {
                bound.push(i + 1);
                skip[i + 1] = true
            }
            let arrow = (eq + 1..end).find(|&j| at(&t, j, "=>"));
            if arrow.is_none() {
                let mut depth = 0i32;
                for j in eq + 1..end {
                    if t[j].kind == Kind::Punct && matches!(t[j].text.as_str(), "(" | "[" | "{") {
                        depth += 1;
                    } else if t[j].kind == Kind::Punct
                        && matches!(t[j].text.as_str(), ")" | "]" | "}")
                    {
                        depth -= 1;
                    } else if depth == 0 && at(&t, j, ",") && id(&t, j + 1) {
                        bound.push(j + 1);
                        skip[j + 1] = true;
                    }
                }
            }
            for &j in &bound {
                if s != 0 {
                    scopes[s].bindings.insert(t[j].text.clone());
                }
                if s == 0 || arrow.is_some() {
                    let kind = if arrow.is_some() {
                        "function"
                    } else {
                        "variable"
                    };
                    let d = add_def(
                        &mut out,
                        source,
                        &t[j],
                        kind,
                        enclosing(&scopes, s),
                        exported,
                    );
                    if let Some(a) = arrow {
                        let child = scopes.len();
                        scopes.push(Scope {
                            parent: Some(s),
                            def: Some(d),
                            ..Default::default()
                        });
                        if at(&t, eq + 1, "(") {
                            params(
                                &t,
                                eq + 2,
                                a.saturating_sub(1),
                                &mut scopes[child],
                                &mut skip,
                                &mut types,
                            )
                        } else {
                            params(&t, eq + 1, a, &mut scopes[child], &mut skip, &mut types)
                        }
                        if at(&t, a + 1, "{") {
                            opening.insert(a + 1, child);
                        } else {
                            for x in a + 1..end {
                                expression_scopes.insert(x, child);
                            }
                        }
                    }
                }
            }
            if at(&t, eq + 1, "require")
                && at(&t, eq + 2, "(")
                && t.get(eq + 3).is_some_and(|x| x.kind == Kind::String)
                && at(&t, eq + 4, ")")
            {
                let mut imp = Import {
                    kind: "require",
                    module: t[eq + 3].text.clone(),
                    names: vec![],
                    glob: false,
                    default_as: None,
                    namespace_as: None,
                    span: t[eq + 1].span,
                };
                if at(&t, i + 1, "{") {
                    let mut j = i + 2;
                    while j < eq && !at(&t, j, "}") {
                        if id(&t, j) {
                            let name = t[j].text.clone();
                            let alias = if at(&t, j + 1, ":") && id(&t, j + 2) {
                                let a = t[j + 2].text.clone();
                                j += 2;
                                Some(a)
                            } else {
                                None
                            };
                            imp.names.push(ImportName { name, alias })
                        }
                        j += 1
                    }
                } else if let Some(&j) = bound.first() {
                    imp.default_as = Some(t[j].text.clone())
                }
                out.imports.push(imp);
                for x in eq + 1..=(eq + 4).min(n - 1) {
                    skip[x] = true
                }
            }
            if at(&t, i + 2, ":") {
                for x in i + 3..eq {
                    types[x] = true
                }
            }
        }
        if at(&t, i, "catch") && at(&t, i + 1, "(") {
            if let Some(e) = matching(&t, i + 1, "(", ")") {
                if at(&t, e + 1, "{") {
                    let child = scopes.len();
                    scopes.push(Scope {
                        parent: Some(s),
                        ..Default::default()
                    });
                    params(&t, i + 2, e, &mut scopes[child], &mut skip, &mut types);
                    opening.insert(e + 1, child);
                }
            }
        }
        // JSX opening component tags are value references; closing tags and intrinsic names are not references.
        if at(&t, i, "<") && id(&t, i + 1) {
            if t[i + 1]
                .text
                .chars()
                .next()
                .is_some_and(|c| c.is_lowercase())
            {
                skip[i + 1] = true
            }
        } else if at(&t, i, "<") && at(&t, i + 1, "/") && id(&t, i + 2) {
            skip[i + 2] = true
        }
        i += 1;
    }
    refs(&t, &skip, &scope_at, &scopes, &mut out, &types, false);
    out
}
