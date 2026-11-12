use super::model::{Def, Extraction, Ref, Span};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub span: Span,
    pub kind: Kind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Id,
    String,
    Opaque,
    Punct,
    Newline,
}
impl Token {
    pub fn is(&self, s: &str) -> bool {
        matches!(self.kind, Kind::Id | Kind::Punct) && self.text == s
    }
}
pub fn at(t: &[Token], i: usize, s: &str) -> bool {
    t.get(i).is_some_and(|x| x.is(s))
}
pub fn id(t: &[Token], i: usize) -> bool {
    t.get(i).is_some_and(|x| x.kind == Kind::Id)
}
pub fn signature(source: &str, line: u32) -> String {
    source
        .lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim()
        .chars()
        .take(160)
        .collect()
}
pub fn add_def(
    out: &mut Extraction,
    source: &str,
    t: &Token,
    kind: &'static str,
    parent: Option<usize>,
    exported: bool,
) -> usize {
    let qualname = parent
        .and_then(|p| out.defs.get(p))
        .map(|p| format!("{}.{}", p.qualname, t.text))
        .unwrap_or_else(|| t.text.clone());
    let i = out.defs.len();
    out.defs.push(Def {
        kind,
        name: t.text.clone(),
        qualname,
        span: t.span,
        signature: signature(source, t.span.line),
        parent,
        exported,
    });
    i
}
#[derive(Default, Clone)]
pub struct Scope {
    pub parent: Option<usize>,
    pub def: Option<usize>,
    pub bindings: HashSet<String>,
    pub global: HashSet<String>,
    pub module: bool,
}
pub fn shadow(scopes: &[Scope], mut s: usize, name: &str) -> bool {
    loop {
        let Some(scope) = scopes.get(s) else {
            return false;
        };
        if scope.module {
            return false;
        }
        if scope.global.contains(name) {
            return false;
        }
        if scope.bindings.contains(name) {
            return true;
        }
        match scope.parent {
            Some(p) => s = p,
            None => return false,
        }
    }
}
pub fn enclosing(scopes: &[Scope], mut s: usize) -> Option<usize> {
    loop {
        let scope = scopes.get(s)?;
        if scope.def.is_some() {
            return scope.def;
        }
        s = scope.parent?;
    }
}
pub fn keyword(s: &str) -> bool {
    matches!(
        s,
        "def"
            | "class"
            | "async"
            | "await"
            | "return"
            | "yield"
            | "if"
            | "else"
            | "elif"
            | "for"
            | "while"
            | "in"
            | "not"
            | "and"
            | "or"
            | "is"
            | "None"
            | "True"
            | "False"
            | "pass"
            | "break"
            | "continue"
            | "try"
            | "except"
            | "finally"
            | "raise"
            | "with"
            | "as"
            | "from"
            | "import"
            | "global"
            | "nonlocal"
            | "lambda"
            | "del"
            | "assert"
            | "function"
            | "export"
            | "default"
            | "const"
            | "let"
            | "var"
            | "new"
            | "this"
            | "super"
            | "extends"
            | "implements"
            | "interface"
            | "type"
            | "enum"
            | "namespace"
            | "public"
            | "private"
            | "protected"
            | "static"
            | "readonly"
            | "abstract"
            | "get"
            | "set"
            | "true"
            | "false"
            | "null"
            | "undefined"
            | "throw"
            | "catch"
            | "switch"
            | "case"
            | "of"
            | "typeof"
            | "instanceof"
            | "void"
            | "delete"
            | "do"
            | "constructor"
    )
}
pub fn reference_keyword(s: &str, python: bool) -> bool {
    if python {
        matches!(
            s,
            "False"
                | "None"
                | "True"
                | "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "try"
                | "while"
                | "with"
                | "yield"
        )
    } else {
        keyword(s) && !matches!(s, "this" | "super" | "get" | "set" | "constructor")
    }
}
pub fn refs(
    t: &[Token],
    skip: &[bool],
    scope_at: &[usize],
    scopes: &[Scope],
    out: &mut Extraction,
    types: &[bool],
    python: bool,
) {
    let mut i = 0;
    while i < t.len() {
        if skip[i]
            || t[i].kind != Kind::Id
            || (reference_keyword(&t[i].text, python)
                && !(!python && t[i].is("import") && at(t, i + 1, "(")))
            || (i > 1 && at(t, i - 1, ".") && id(t, i - 2))
        {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while at(t, end + 1, ".") && id(t, end + 2) && !skip[end + 2] {
            end += 2;
        }
        let chain: Vec<String> = (start..=end)
            .step_by(2)
            .map(|j| t[j].text.clone())
            .collect();
        let call = at(t, end + 1, "(");
        let computed = at(t, end + 1, "[") && {
            let mut depth = 0i32;
            let mut j = end + 1;
            let mut found = false;
            while j < t.len() {
                if at(t, j, "[") {
                    depth += 1
                } else if at(t, j, "]") {
                    depth -= 1;
                    if depth == 0 {
                        found = at(t, j + 1, "(");
                        break;
                    }
                }
                if t[j].kind == Kind::Newline {
                    break;
                }
                j += 1;
            }
            found
        };
        let dynamic = if python {
            chain[0] == "getattr" || chain[0] == "__import__" || chain[0] == "importlib"
        } else {
            computed
                || chain[0] == "import"
                || (chain[0] == "require"
                    && call
                    && !(t.get(end + 2).is_some_and(|x| x.kind == Kind::String)
                        && at(t, end + 3, ")")))
        };
        let s = scope_at.get(start).copied().unwrap_or(0);
        out.refs.push(Ref {
            name: chain.last().cloned().unwrap_or_default(),
            qualifier: chain[..chain.len() - 1].to_vec(),
            kind: if call || computed {
                "call"
            } else if types.get(start).copied().unwrap_or(false) {
                "type"
            } else {
                "value"
            },
            span: t[end].span,
            enclosing: enclosing(scopes, s),
            shadowed: shadow(scopes, s, &chain[0]),
            dynamic,
        });
        i = end + 1;
    }
}

pub fn lex(source: &str, python: bool, jsx: bool) -> (Vec<Token>, Vec<String>) {
    lex_inner(source, python, jsx, 0)
}
fn lex_inner(source: &str, python: bool, jsx: bool, level: usize) -> (Vec<Token>, Vec<String>) {
    let c: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut brackets: Vec<char> = Vec::new();
    let mut jsx_depth = 0usize;
    let mut jsx_text = false;
    let mut jsx_tag: Option<bool> = None;
    let mut jsx_expression: Option<usize> = None;
    while i < c.len() {
        let ch = c[i];
        if jsx_text && ch != '<' && ch != '{' {
            if ch == '\n' {
                line += 1;
                col = 1;
            } else if ch != '\r' {
                col += 1;
            }
            i += 1;
            continue;
        }
        if jsx_text && ch == '{' {
            jsx_text = false;
            jsx_expression = Some(brackets.len());
        }
        if jsx
            && ch == '<'
            && (jsx_text
                || (jsx_tag.is_none()
                    && c.get(i + 1).is_some_and(|x| x.is_alphabetic())
                    && out.last().is_some_and(|p: &Token| {
                        matches!(p.kind, Kind::Id | Kind::Punct)
                            && matches!(p.text.as_str(), "=" | "return" | "=>" | "(" | "{" | ",")
                    })))
        {
            jsx_tag = Some(c.get(i + 1) == Some(&'/'));
            jsx_text = false;
        }
        if ch == '>' {
            if let Some(closing) = jsx_tag.take() {
                if closing || (i > 0 && c[i - 1] == '/') {
                    if closing {
                        jsx_depth = jsx_depth.saturating_sub(1);
                    }
                } else {
                    jsx_depth += 1;
                }
                jsx_text = jsx_depth > 0;
            }
        }
        if ch == '}'
            && jsx_expression == Some(brackets.len().saturating_sub(1))
            && jsx_tag.is_none()
        {
            jsx_expression = None;
            jsx_text = jsx_depth > 0;
        }
        if ch == '\r' {
            i += 1;
            continue;
        }
        if ch == '\n' {
            if !python || brackets.is_empty() {
                out.push(Token {
                    text: "\n".into(),
                    span: Span {
                        line,
                        col,
                        end_line: line,
                        end_col: col + 1,
                    },
                    kind: Kind::Newline,
                });
            }
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        if (python && ch == '#') || (!python && ch == '/' && c.get(i + 1) == Some(&'/')) {
            while i < c.len() && c[i] != '\n' {
                i += 1;
                col += 1
            }
            continue;
        }
        if !python && ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            col += 2;
            let mut closed = false;
            while i < c.len() {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    i += 2;
                    col += 2;
                    closed = true;
                    break;
                }
                if c[i] == '\n' {
                    line += 1;
                    col = 1
                } else {
                    col += 1
                }
                i += 1;
            }
            if !closed {
                errors.push("unterminated comment".into())
            }
            continue;
        }
        if ch == '\0' || ch == '\u{fffd}' || (ch.is_control() && !ch.is_whitespace()) {
            errors.push(format!("invalid character at line {line}"));
        }
        let start = i;
        let sl = line;
        let sc = col;
        // Python string prefixes are consumed as part of the literal.
        let mut q = i;
        if python && ch.is_ascii_alphabetic() {
            while q < c.len()
                && q - i < 3
                && matches!(c[q], 'r' | 'R' | 'f' | 'F' | 'b' | 'B' | 'u' | 'U')
            {
                q += 1
            }
            if !matches!(c.get(q), Some('\'') | Some('"')) {
                q = i
            }
        }
        if matches!(c.get(q), Some('\'') | Some('"')) || (!python && ch == '`') {
            let quote = c[q];
            let triple = python && c.get(q + 1) == Some(&quote) && c.get(q + 2) == Some(&quote);
            let n = if triple { 3 } else { 1 };
            col += (q - i + n) as u32;
            i = q + n;
            let mut closed = false;
            while i < c.len() {
                if c[i] == '\\' {
                    i += 1;
                    col += 1;
                    if i < c.len() {
                        if c[i] == '\n' {
                            line += 1;
                            col = 1
                        } else {
                            col += 1
                        }
                        i += 1
                    }
                    continue;
                }
                if c[i] == quote
                    && (!triple || (c.get(i + 1) == Some(&quote) && c.get(i + 2) == Some(&quote)))
                {
                    i += n;
                    col += n as u32;
                    closed = true;
                    break;
                }
                if c[i] == '\n' {
                    if !triple && quote != '`' {
                        break;
                    }
                    line += 1;
                    col = 1
                } else {
                    col += 1
                }
                i += 1;
            }
            if !closed {
                errors.push("unterminated string".into())
            }
            let content_start = q + n;
            let content_end = if closed { i - n } else { i };
            let value = if content_start <= content_end {
                c[content_start..content_end].iter().collect()
            } else {
                String::new()
            };
            out.push(Token {
                text: value,
                span: Span {
                    line: sl,
                    col: sc,
                    end_line: line,
                    end_col: col,
                },
                kind: if quote == '`' {
                    Kind::Opaque
                } else {
                    Kind::String
                },
            });
            let interpolated =
                quote == '`' || (python && c[start..q].iter().any(|x| *x == 'f' || *x == 'F'));
            if interpolated && level < 32 {
                let mut x = content_start;
                while x < content_end {
                    if python && c[x] == '{' && c.get(x + 1) == Some(&'{') {
                        x += 2;
                        continue;
                    }
                    let marker = if python {
                        c[x] == '{'
                    } else {
                        c[x] == '$' && c.get(x + 1) == Some(&'{')
                    };
                    if !marker {
                        x += 1;
                        continue;
                    }
                    let begin = x + if python { 1 } else { 2 };
                    let mut y = begin;
                    let mut depth = 1usize;
                    let mut inner_quote = None;
                    while y < content_end {
                        let ch = c[y];
                        if ch == '\\' {
                            y = (y + 2).min(content_end);
                            continue;
                        }
                        if let Some(q) = inner_quote {
                            if ch == q {
                                inner_quote = None;
                            }
                        } else if ch == '\'' || ch == '"' {
                            inner_quote = Some(ch);
                        } else if ch == '{' {
                            depth += 1;
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        y += 1;
                    }
                    if depth != 0 {
                        errors.push("unterminated interpolation".into());
                        break;
                    }
                    let expression: String = c[begin..y].iter().collect();
                    let (mut embedded, mut embedded_errors) =
                        lex_inner(&expression, python, false, level + 1);
                    let mut base_line = sl;
                    let mut base_col = sc;
                    for ch in &c[start..begin] {
                        if *ch == '\n' {
                            base_line += 1;
                            base_col = 1;
                        } else {
                            base_col += 1;
                        }
                    }
                    for token in &mut embedded {
                        if token.span.line == 1 {
                            token.span.col += base_col - 1;
                        }
                        if token.span.end_line == 1 {
                            token.span.end_col += base_col - 1;
                        }
                        token.span.line += base_line - 1;
                        token.span.end_line += base_line - 1;
                    }
                    out.extend(embedded);
                    errors.append(&mut embedded_errors);
                    x = y + 1;
                }
            }
            continue;
        }
        if !python
            && ch == '/'
            && c.get(i + 1) != Some(&'=')
            && out.last().is_none_or(|p: &Token| {
                p.kind == Kind::Newline
                    || (matches!(p.kind, Kind::Id | Kind::Punct)
                        && matches!(
                            p.text.as_str(),
                            "=" | "(" | "[" | "{" | ":" | "," | "return" | "=>"
                        ))
            })
        {
            i += 1;
            col += 1;
            let mut closed = false;
            let mut class = false;
            while i < c.len() && c[i] != '\n' {
                if c[i] == '\\' {
                    i += 1;
                    col += 1;
                    if i < c.len() {
                        i += 1;
                        col += 1
                    }
                    continue;
                }
                if c[i] == '[' {
                    class = true
                }
                if c[i] == ']' {
                    class = false
                }
                if c[i] == '/' && !class {
                    i += 1;
                    col += 1;
                    closed = true;
                    break;
                }
                i += 1;
                col += 1;
            }
            while i < c.len() && c[i].is_ascii_alphabetic() {
                i += 1;
                col += 1
            }
            if !closed {
                errors.push("unterminated regex".into())
            }
            out.push(Token {
                text: String::new(),
                span: Span {
                    line: sl,
                    col: sc,
                    end_line: line,
                    end_col: col,
                },
                kind: Kind::Opaque,
            });
            continue;
        }
        if ch == '_' || ch == '$' || ch.is_alphabetic() {
            i += 1;
            col += 1;
            while i < c.len() && (c[i] == '_' || c[i] == '$' || c[i].is_alphanumeric()) {
                i += 1;
                col += 1
            }
            out.push(Token {
                text: c[start..i].iter().collect(),
                span: Span {
                    line: sl,
                    col: sc,
                    end_line: line,
                    end_col: col,
                },
                kind: Kind::Id,
            });
            continue;
        }
        if ch.is_ascii_digit() {
            i += 1;
            col += 1;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '.' || c[i] == '_') {
                i += 1;
                col += 1
            }
            out.push(Token {
                text: c[start..i].iter().collect(),
                span: Span {
                    line: sl,
                    col: sc,
                    end_line: line,
                    end_col: col,
                },
                kind: Kind::Opaque,
            });
            continue;
        }
        if matches!(ch, '(' | '[' | '{') {
            brackets.push(ch)
        }
        if matches!(ch, ')' | ']' | '}') {
            let expected = match ch {
                ')' => '(',
                ']' => '[',
                _ => '{',
            };
            if brackets.pop() != Some(expected) {
                errors.push("unbalanced brackets".into())
            }
        }
        i += 1;
        col += 1;
        if i < c.len()
            && matches!(
                (ch, c[i]),
                ('=', '>') | ('-', '>') | ('=', '=') | ('!', '=') | ('<', '=') | ('>', '=')
            )
        {
            i += 1;
            col += 1
        }
        out.push(Token {
            text: c[start..i].iter().collect(),
            span: Span {
                line: sl,
                col: sc,
                end_line: line,
                end_col: col,
            },
            kind: Kind::Punct,
        });
    }
    if !brackets.is_empty() {
        errors.push("unbalanced brackets".into())
    }
    (out, errors)
}
