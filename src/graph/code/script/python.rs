use super::lexer::*;
use super::model::{Extraction, Import, ImportName};

pub fn extract(source: &str) -> Extraction {
    let (t, errors) = lex(source, true, false);
    let mut out = Extraction {
        errors,
        ..Default::default()
    };
    let n = t.len();
    let mut skip = vec![false; n];
    let mut types = vec![false; n];
    let mut scope_at = vec![0; n];
    let mut scopes = vec![Scope {
        module: true,
        ..Default::default()
    }];
    let mut stack = vec![(0u32, 0usize)];
    let mut pending: Option<(u32, usize)> = None;
    let mut start = 0;
    while start < n {
        let mut end = start;
        while end < n && t[end].kind != Kind::Newline {
            end += 1
        }
        if start == end {
            start = end + 1;
            continue;
        }
        let indent = t[start].span.col;
        if let Some((base, s)) = pending.take() {
            if indent > base {
                stack.push((indent, s));
            }
        }
        while stack.len() > 1 && indent < stack.last().unwrap().0 {
            stack.pop();
        }
        let s = stack.last().unwrap().1;
        for slot in &mut scope_at[start..end] {
            *slot = s;
        }
        let mut k = start;
        if at(&t, k, "async") && at(&t, k + 1, "def") {
            skip[k] = true;
            k += 1
        }
        if (at(&t, k, "def") || at(&t, k, "class"))
            && (!id(&t, k + 1) || !(k + 2..end).any(|j| at(&t, j, ":")))
        {
            out.errors
                .push(format!("incomplete declaration at line {}", t[k].span.line));
        }
        if (at(&t, k, "def") || at(&t, k, "class")) && id(&t, k + 1) {
            skip[k] = true;
            skip[k + 1] = true;
            let name = k + 1;
            let parent = enclosing(&scopes, s);
            let is_class = at(&t, k, "class");
            let kind = if is_class {
                "class"
            } else if parent.is_some_and(|p| out.defs[p].kind == "class") {
                "method"
            } else {
                "function"
            };
            let d = add_def(
                &mut out,
                source,
                &t[name],
                kind,
                parent,
                s == 0 && !t[name].text.starts_with('_'),
            );
            if s != 0 {
                scopes[s].bindings.insert(t[name].text.clone());
            }
            let child = scopes.len();
            scopes.push(Scope {
                parent: Some(s),
                def: Some(d),
                ..Default::default()
            });
            pending = Some((indent, child));
            if !is_class && at(&t, name + 1, "(") {
                let mut depth = 0i32;
                let mut expect = true;
                let mut j = name + 1;
                while j < end {
                    if at(&t, j, "(") {
                        depth += 1
                    } else if at(&t, j, ")") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else if depth == 1 && at(&t, j, ",") {
                        expect = true
                    } else if depth == 1 && id(&t, j) && expect {
                        scopes[child].bindings.insert(t[j].text.clone());
                        skip[j] = true;
                        expect = false
                    } else if depth == 1 && at(&t, j, ":") {
                        let mut x = j + 1;
                        while x < end && !at(&t, x, ",") && !at(&t, x, "=") && !at(&t, x, ")") {
                            types[x] = true;
                            x += 1
                        }
                    }
                    j += 1;
                }
            }
            let mut j = name + 1;
            while j < end {
                if at(&t, j, "->") {
                    for x in j + 1..end {
                        types[x] = true;
                    }
                    break;
                }
                j += 1;
            }
        } else if at(&t, k, "import") || at(&t, k, "from") {
            let from = at(&t, k, "from");
            let span = t[k].span;
            let mut j = k + 1;
            if from {
                let mut module = String::new();
                while j < end && !at(&t, j, "import") {
                    module.push_str(&t[j].text);
                    j += 1
                }
                if j < end {
                    j += 1
                }
                let mut names = Vec::new();
                let mut glob = false;
                while j < end {
                    if at(&t, j, "*") {
                        glob = true;
                        j += 1
                    } else if id(&t, j) {
                        let name = t[j].text.clone();
                        j += 1;
                        let alias = if at(&t, j, "as") && id(&t, j + 1) {
                            let a = t[j + 1].text.clone();
                            j += 2;
                            Some(a)
                        } else {
                            None
                        };
                        names.push(ImportName { name, alias });
                    } else {
                        j += 1
                    }
                }
                out.imports.push(Import {
                    kind: "from",
                    module,
                    names,
                    glob,
                    default_as: None,
                    namespace_as: None,
                    span,
                });
            } else {
                while j < end {
                    let mut module = String::new();
                    while j < end && ((id(&t, j) && !at(&t, j, "as")) || at(&t, j, ".")) {
                        module.push_str(&t[j].text);
                        j += 1
                    }
                    if module.is_empty() {
                        j += 1;
                        continue;
                    }
                    let alias = if at(&t, j, "as") && id(&t, j + 1) {
                        let a = t[j + 1].text.clone();
                        j += 2;
                        Some(a)
                    } else {
                        None
                    };
                    out.imports.push(Import {
                        kind: "import",
                        module,
                        names: vec![],
                        glob: false,
                        default_as: None,
                        namespace_as: alias,
                        span,
                    });
                    if at(&t, j, ",") {
                        j += 1
                    }
                }
            }
            for x in &mut skip[start..end] {
                *x = true
            }
        } else if at(&t, k, "global") || at(&t, k, "nonlocal") {
            for j in k + 1..end {
                if id(&t, j) {
                    scopes[s].global.insert(t[j].text.clone());
                    skip[j] = true
                }
            }
        } else {
            // Comprehension targets have their own scope, including the expression
            // before the `for`; they do not bind the containing function.
            for open in start..end {
                if t[open].kind != Kind::Punct {
                    continue;
                }
                let close_token = match t[open].text.as_str() {
                    "[" => "]",
                    "{" => "}",
                    "(" => ")",
                    _ => continue,
                };
                let mut depth = 0i32;
                let mut close = None;
                for x in open..end {
                    if t[x].is(&t[open].text) {
                        depth += 1;
                    }
                    if at(&t, x, close_token) {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(x);
                            break;
                        }
                    }
                }
                let Some(close) = close else { continue };
                if !(open + 1..close).any(|x| at(&t, x, "for")) {
                    continue;
                }
                let child = scopes.len();
                scopes.push(Scope {
                    parent: Some(s),
                    ..Default::default()
                });
                for x in open + 1..close {
                    scope_at[x] = child;
                }
                for f in open + 1..close {
                    if !at(&t, f, "for") {
                        continue;
                    }
                    if let Some(in_pos) = (f + 1..close).find(|&x| at(&t, x, "in")) {
                        for x in f + 1..in_pos {
                            if id(&t, x) && !reference_keyword(&t[x].text, true) {
                                scopes[child].bindings.insert(t[x].text.clone());
                                skip[x] = true;
                            }
                        }
                    }
                }
            }
            // Collect binding targets; binding is lexical across the whole function.
            let mut j = start;
            while j < end {
                let target = if (at(&t, j, "for") && scope_at[j] == s) || at(&t, j, "as") {
                    let stop = if at(&t, j, "for") { "in" } else { ":" };
                    let mut e = j + 1;
                    while e < end && !at(&t, e, stop) && (stop == "in" || !at(&t, e, ",")) {
                        e += 1
                    }
                    Some((j + 1, e))
                } else {
                    None
                };
                if let Some((a, b)) = target {
                    for x in a..b {
                        if id(&t, x) && !reference_keyword(&t[x].text, true) {
                            scopes[s].bindings.insert(t[x].text.clone());
                            skip[x] = true
                        }
                    }
                }
                j += 1;
            }
            let mut eq = None;
            let mut depth = 0i32;
            for j in start..end {
                if t[j].kind == Kind::Punct && matches!(t[j].text.as_str(), "(" | "[" | "{") {
                    depth += 1
                } else if t[j].kind == Kind::Punct && matches!(t[j].text.as_str(), ")" | "]" | "}")
                {
                    depth -= 1
                } else if depth == 0 && at(&t, j, "=") {
                    eq = Some(j);
                    break;
                }
            }
            if let Some(e) = eq {
                let target_end = (start..e).find(|&j| at(&t, j, ":")).unwrap_or(e);
                for j in start..target_end {
                    if id(&t, j)
                        && !reference_keyword(&t[j].text, true)
                        && (j == start || !at(&t, j - 1, "."))
                        && !at(&t, j + 1, ".")
                    {
                        if s == 0 {
                            add_def(
                                &mut out,
                                source,
                                &t[j],
                                "variable",
                                None,
                                !t[j].text.starts_with('_'),
                            );
                        } else {
                            scopes[s].bindings.insert(t[j].text.clone());
                        }
                        skip[j] = true;
                    }
                }
            }
            // Variable annotations and return annotations.
            for j in start..end {
                if at(&t, j, ":") && j > start && id(&t, j - 1) {
                    let mut x = j + 1;
                    while x < end && !at(&t, x, "=") && !at(&t, x, ",") {
                        types[x] = true;
                        x += 1
                    }
                }
            }
        }
        start = end + 1;
    }
    refs(&t, &skip, &scope_at, &scopes, &mut out, &types, true);
    out
}
