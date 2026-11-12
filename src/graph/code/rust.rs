//! Rust extraction placeholder.

pub fn extract(source: &str) -> super::model::Extraction {
    use super::model::{Def, Extraction, Import, ImportName, Ref, Span};
    use proc_macro2::{LineColumn, Span as TokenSpan};
    use std::collections::HashSet;
    use syn::{
        spanned::Spanned,
        visit::{self, Visit},
        *,
    };

    fn span(s: TokenSpan) -> Span {
        let a = s.start();
        let b = s.end();
        Span {
            line: a.line as u32,
            col: a.column as u32 + 1,
            end_line: b.line as u32,
            end_col: b.column as u32 + 1,
        }
    }
    fn offset(src: &str, pos: LineColumn) -> usize {
        let start = src
            .split_inclusive('\n')
            .take(pos.line.saturating_sub(1))
            .map(str::len)
            .sum::<usize>();
        let line = src.get(start..).unwrap_or("");
        start
            + line
                .char_indices()
                .nth(pos.column)
                .map(|(i, _)| i)
                .unwrap_or_else(|| line.find('\n').unwrap_or(line.len()))
    }
    fn bindings(pat: &Pat) -> HashSet<String> {
        struct Binder(HashSet<String>);
        impl<'a> Visit<'a> for Binder {
            fn visit_pat_ident(&mut self, p: &'a PatIdent) {
                self.0.insert(p.ident.to_string());
                visit::visit_pat_ident(self, p);
            }
        }
        let mut b = Binder(HashSet::new());
        b.visit_pat(pat);
        b.0
    }
    fn exported(v: &Visibility) -> bool {
        matches!(v, Visibility::Public(_) | Visibility::Restricted(_))
    }
    struct Walker<'a> {
        src: &'a str,
        out: Extraction,
        parent: Option<usize>,
        modules: Vec<String>,
        owner: Option<String>,
        scopes: Vec<HashSet<String>>,
    }
    impl Walker<'_> {
        fn header_start(&self, item: TokenSpan, attrs: &[Attribute]) -> LineColumn {
            let mut start = offset(
                self.src,
                attrs.last().map_or(item.start(), |a| a.span().end()),
            );
            // Comments between the last attribute and the item are not represented in the AST.
            loop {
                let rest = &self.src[start..];
                if let Some(c) = rest.chars().next().filter(|c| c.is_whitespace()) {
                    start += c.len_utf8();
                } else if rest.starts_with("//") {
                    start += rest.find('\n').unwrap_or(rest.len());
                } else if rest.starts_with("/*") {
                    let mut depth = 1;
                    start += 2;
                    while depth > 0 && start < self.src.len() {
                        let rest = &self.src[start..];
                        if rest.starts_with("/*") {
                            depth += 1;
                            start += 2;
                        } else if rest.starts_with("*/") {
                            depth -= 1;
                            start += 2;
                        } else {
                            start += rest.chars().next().unwrap().len_utf8();
                        }
                    }
                } else {
                    break;
                }
            }
            let prefix = &self.src[..start];
            LineColumn {
                line: prefix.bytes().filter(|&b| b == b'\n').count() + 1,
                column: prefix.rsplit('\n').next().unwrap().chars().count(),
            }
        }
        fn signature(&self, start: LineColumn, item: TokenSpan, end: Option<TokenSpan>) -> String {
            let start = offset(self.src, start);
            let finish = offset(
                self.src,
                end.map(|s| s.start()).unwrap_or_else(|| item.end()),
            );
            self.src
                .get(start..finish)
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .trim_end_matches(';')
                .trim()
                .chars()
                .take(160)
                .collect()
        }
        fn add_def(
            &mut self,
            kind: &'static str,
            name: &Ident,
            item: TokenSpan,
            attrs: &[Attribute],
            end: Option<TokenSpan>,
            vis: bool,
            owner: Option<&str>,
        ) -> Option<usize> {
            let name = name.to_string();
            let local = owner
                .map(|o| format!("{o}.{name}"))
                .unwrap_or_else(|| name.clone());
            let qualname = if self.modules.is_empty() {
                local
            } else {
                format!("{}.{local}", self.modules.join("."))
            };
            let prev = self.parent;
            let index = self.out.defs.len();
            let start = self.header_start(item, attrs);
            let mut item_span = span(item);
            item_span.line = start.line as u32;
            item_span.col = start.column as u32 + 1;
            self.out.defs.push(Def {
                kind,
                name,
                qualname,
                span: item_span,
                signature: self.signature(start, item, end),
                parent: prev,
                exported: vis,
            });
            self.parent = Some(index);
            prev
        }
        fn reference(&mut self, path: &Path, kind: &'static str) {
            let parts: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            if let Some(name) = parts.last() {
                let shadowed = parts.len() == 1
                    && (kind == "call" || kind == "value")
                    && self.scopes.iter().rev().any(|scope| scope.contains(name));
                self.out.refs.push(Ref {
                    name: name.clone(),
                    qualifier: parts[..parts.len() - 1].to_vec(),
                    kind,
                    span: span(path.span()),
                    enclosing: self.parent,
                    shadowed,
                    dynamic: false,
                });
            }
        }
        fn imports(&mut self, tree: &UseTree, path: &mut Vec<String>, s: TokenSpan) {
            match tree {
                UseTree::Path(p) => {
                    path.push(p.ident.to_string());
                    self.imports(&p.tree, path, s);
                    path.pop();
                }
                UseTree::Group(g) => {
                    for t in &g.items {
                        self.imports(t, path, s);
                    }
                }
                UseTree::Glob(_) => self.out.imports.push(Import {
                    kind: "use",
                    module: path.join("::"),
                    names: vec![],
                    glob: true,
                    default_as: None,
                    namespace_as: None,
                    span: span(s),
                }),
                UseTree::Name(n) => self.import_leaf(path, &n.ident, None, s),
                UseTree::Rename(n) => {
                    self.import_leaf(path, &n.ident, Some(n.rename.to_string()), s)
                }
            }
        }
        fn import_leaf(
            &mut self,
            path: &[String],
            ident: &Ident,
            alias: Option<String>,
            s: TokenSpan,
        ) {
            let mut module = path.to_vec();
            let mut name = ident.to_string();
            if name == "self" && !module.is_empty() {
                name = module.pop().unwrap();
            }
            self.out.imports.push(Import {
                kind: "use",
                module: module.join("::"),
                names: vec![ImportName { name, alias }],
                glob: false,
                default_as: None,
                namespace_as: None,
                span: span(s),
            });
        }
        fn parameters(&mut self, sig: &Signature) {
            let mut scope = HashSet::new();
            for arg in &sig.inputs {
                match arg {
                    FnArg::Typed(p) => scope.extend(bindings(&p.pat)),
                    FnArg::Receiver(_) => {
                        scope.insert("self".into());
                    }
                }
            }
            self.scopes.push(scope);
        }
        fn bind(&mut self, pat: &Pat) {
            if let Some(scope) = self.scopes.last_mut() {
                scope.extend(bindings(pat));
            }
        }
        fn condition(&mut self, expr: &Expr) {
            match expr {
                Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
                    self.condition(&b.left);
                    self.condition(&b.right);
                }
                Expr::Paren(p) => self.condition(&p.expr),
                Expr::Group(g) => self.condition(&g.expr),
                Expr::Let(l) => {
                    self.visit_expr(&l.expr);
                    self.visit_pat(&l.pat);
                    self.bind(&l.pat);
                }
                _ => self.visit_expr(expr),
            }
        }
        fn self_type_name(&self, ty: &Type) -> String {
            match ty {
                Type::Reference(t) => self.self_type_name(&t.elem),
                Type::Paren(t) => self.self_type_name(&t.elem),
                Type::Group(t) => self.self_type_name(&t.elem),
                Type::Path(t) => t
                    .path
                    .segments
                    .last()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_else(|| "_".into()),
                _ => {
                    let s = ty.span();
                    let source = &self.src[offset(self.src, s.start())..offset(self.src, s.end())];
                    let printed: String = source
                        .parse::<proc_macro2::TokenStream>()
                        .map(|tokens| tokens.to_string())
                        .unwrap_or_else(|_| source.to_owned())
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    if printed.is_empty() {
                        "_".into()
                    } else {
                        printed
                    }
                }
            }
        }
        fn receiver(expr: &Expr) -> Option<Vec<String>> {
            match expr {
                Expr::Path(p) if p.qself.is_none() => Some(
                    p.path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect(),
                ),
                Expr::Field(f) => {
                    let mut p = Self::receiver(&f.base)?;
                    if let Member::Named(i) = &f.member {
                        p.push(i.to_string());
                        Some(p)
                    } else {
                        None
                    }
                }
                Expr::Paren(p) => Self::receiver(&p.expr),
                _ => None,
            }
        }
    }
    impl<'a> Visit<'a> for Walker<'_> {
        fn visit_item(&mut self, item: &'a Item) {
            match item {
                Item::Use(i) => self.imports(&i.tree, &mut Vec::new(), i.span()),
                Item::Mod(i) if i.content.is_none() => self.out.imports.push(Import {
                    kind: "mod",
                    module: i.ident.to_string(),
                    names: vec![],
                    glob: false,
                    default_as: None,
                    namespace_as: None,
                    span: span(i.span()),
                }),
                Item::Mod(i) => {
                    let old = self.add_def(
                        "module",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        i.content.as_ref().map(|(b, _)| b.span.open()),
                        exported(&i.vis),
                        None,
                    );
                    self.modules.push(i.ident.to_string());
                    visit::visit_item_mod(self, i);
                    self.modules.pop();
                    self.parent = old;
                }
                Item::Fn(i) => {
                    let old = self.add_def(
                        "function",
                        &i.sig.ident,
                        i.span(),
                        &i.attrs,
                        Some(i.block.brace_token.span.open()),
                        exported(&i.vis),
                        None,
                    );
                    let scopes = std::mem::take(&mut self.scopes);
                    self.parameters(&i.sig);
                    visit::visit_item_fn(self, i);
                    self.scopes = scopes;
                    self.parent = old;
                }
                Item::Impl(i) => {
                    let old = self.owner.take();
                    self.owner = Some(self.self_type_name(&i.self_ty));
                    visit::visit_item_impl(self, i);
                    self.owner = old;
                }
                Item::Trait(i) => {
                    let old = self.add_def(
                        "trait",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        Some(i.brace_token.span.open()),
                        exported(&i.vis),
                        None,
                    );
                    let owner = self.owner.replace(i.ident.to_string());
                    visit::visit_item_trait(self, i);
                    self.owner = owner;
                    self.parent = old;
                }
                Item::Struct(i) => {
                    let end = match &i.fields {
                        Fields::Named(f) => Some(f.brace_token.span.open()),
                        Fields::Unnamed(f) => Some(f.paren_token.span.open()),
                        Fields::Unit => None,
                    };
                    let old = self.add_def(
                        "struct",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        end,
                        exported(&i.vis),
                        None,
                    );
                    visit::visit_item_struct(self, i);
                    self.parent = old;
                }
                Item::Enum(i) => {
                    let old = self.add_def(
                        "enum",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        Some(i.brace_token.span.open()),
                        exported(&i.vis),
                        None,
                    );
                    visit::visit_item_enum(self, i);
                    self.parent = old;
                }
                Item::Type(i) => {
                    let old = self.add_def(
                        "type",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        None,
                        exported(&i.vis),
                        None,
                    );
                    visit::visit_item_type(self, i);
                    self.parent = old;
                }
                Item::Const(i) => {
                    let old = self.add_def(
                        "const",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        Some(i.eq_token.span),
                        exported(&i.vis),
                        None,
                    );
                    visit::visit_item_const(self, i);
                    self.parent = old;
                }
                Item::Static(i) => {
                    let old = self.add_def(
                        "static",
                        &i.ident,
                        i.span(),
                        &i.attrs,
                        Some(i.eq_token.span),
                        exported(&i.vis),
                        None,
                    );
                    visit::visit_item_static(self, i);
                    self.parent = old;
                }
                Item::Macro(i) if i.mac.path.is_ident("macro_rules") && i.ident.is_some() => {
                    let old = self.add_def(
                        "macro",
                        i.ident.as_ref().unwrap(),
                        i.span(),
                        &i.attrs,
                        Some(i.mac.delimiter.span().open()),
                        false,
                        None,
                    );
                    self.parent = old;
                }
                _ => visit::visit_item(self, item),
            }
        }
        fn visit_impl_item_fn(&mut self, i: &'a ImplItemFn) {
            let owner = self.owner.clone().unwrap_or_default();
            let old = self.add_def(
                "method",
                &i.sig.ident,
                i.span(),
                &i.attrs,
                Some(i.block.brace_token.span.open()),
                exported(&i.vis),
                Some(&owner),
            );
            let scopes = std::mem::take(&mut self.scopes);
            self.parameters(&i.sig);
            visit::visit_impl_item_fn(self, i);
            self.scopes = scopes;
            self.parent = old;
        }
        fn visit_trait_item_fn(&mut self, i: &'a TraitItemFn) {
            let owner = self.owner.clone().unwrap_or_default();
            let old = self.add_def(
                "method",
                &i.sig.ident,
                i.span(),
                &i.attrs,
                i.default.as_ref().map(|b| b.brace_token.span.open()),
                false,
                Some(&owner),
            );
            let scopes = std::mem::take(&mut self.scopes);
            self.parameters(&i.sig);
            visit::visit_trait_item_fn(self, i);
            self.scopes = scopes;
            self.parent = old;
        }
        fn visit_block(&mut self, block: &'a Block) {
            self.scopes.push(HashSet::new());
            for stmt in &block.stmts {
                self.visit_stmt(stmt);
            }
            self.scopes.pop();
        }
        fn visit_local(&mut self, local: &'a Local) {
            for attr in &local.attrs {
                self.visit_attribute(attr);
            }
            if let Some(init) = &local.init {
                self.visit_expr(&init.expr);
                if let Some((_, e)) = &init.diverge {
                    self.visit_expr(e);
                }
            }
            self.visit_pat(&local.pat);
            self.bind(&local.pat);
        }
        fn visit_expr_closure(&mut self, e: &'a ExprClosure) {
            let mut scope = HashSet::new();
            for pat in &e.inputs {
                scope.extend(bindings(pat));
            }
            self.scopes.push(scope);
            visit::visit_expr_closure(self, e);
            self.scopes.pop();
        }
        fn visit_expr_for_loop(&mut self, e: &'a ExprForLoop) {
            self.visit_expr(&e.expr);
            self.visit_pat(&e.pat);
            self.scopes.push(bindings(&e.pat));
            self.visit_block(&e.body);
            self.scopes.pop();
        }
        fn visit_arm(&mut self, arm: &'a Arm) {
            self.visit_pat(&arm.pat);
            self.scopes.push(bindings(&arm.pat));
            if let Some((_, e)) = &arm.guard {
                self.visit_expr(e);
            }
            self.visit_expr(&arm.body);
            self.scopes.pop();
        }
        fn visit_expr_if(&mut self, e: &'a ExprIf) {
            self.scopes.push(HashSet::new());
            self.condition(&e.cond);
            self.visit_block(&e.then_branch);
            self.scopes.pop();
            if let Some((_, other)) = &e.else_branch {
                self.visit_expr(other);
            }
        }
        fn visit_expr_while(&mut self, e: &'a ExprWhile) {
            self.scopes.push(HashSet::new());
            self.condition(&e.cond);
            self.visit_block(&e.body);
            self.scopes.pop();
        }
        fn visit_expr_call(&mut self, e: &'a ExprCall) {
            if let Expr::Path(p) = &*e.func {
                self.reference(&p.path, "call");
                visit::visit_path(self, &p.path);
            } else {
                self.visit_expr(&e.func);
            }
            for arg in &e.args {
                self.visit_expr(arg);
            }
        }
        fn visit_expr_method_call(&mut self, e: &'a ExprMethodCall) {
            let parts = Self::receiver(&e.receiver);
            self.out.refs.push(Ref {
                name: e.method.to_string(),
                qualifier: parts.clone().unwrap_or_default(),
                kind: "call",
                span: span(e.method.span()),
                enclosing: self.parent,
                shadowed: false,
                dynamic: parts.is_none(),
            });
            self.visit_expr(&e.receiver);
            if let Some(args) = &e.turbofish {
                self.visit_angle_bracketed_generic_arguments(args);
            }
            for arg in &e.args {
                self.visit_expr(arg);
            }
        }
        fn visit_expr_path(&mut self, e: &'a ExprPath) {
            self.reference(&e.path, "value");
            visit::visit_expr_path(self, e);
        }
        fn visit_type_path(&mut self, t: &'a TypePath) {
            self.reference(&t.path, "type");
            visit::visit_type_path(self, t);
        }
        fn visit_macro(&mut self, m: &'a Macro) {
            self.reference(&m.path, "macro");
        }
    }
    match syn::parse_file(source) {
        Ok(file) => {
            let mut w = Walker {
                src: source,
                out: Extraction::default(),
                parent: None,
                modules: Vec::new(),
                owner: None,
                scopes: Vec::new(),
            };
            w.visit_file(&file);
            w.out
        }
        Err(err) => Extraction {
            errors: vec![err.to_string()],
            ..Extraction::default()
        },
    }
}
