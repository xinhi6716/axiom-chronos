// Baseline 2026-11-13: violation counts (tokens, deps) per group.
// root: 3, 0
// shell: 67, 0
// overlays: 40, 230
// run: 88, 318
// mesh: 85, 131
// wishes: 40, 388
// panels: 48, 678
// mini: 31, 54
// sessions_settings: 4, 55
// research: 45, 417
// paper: 1, 103
// modes: 18, 24
// data: 18, 17
// layout: 5, 0
// components: 31, 0
// motion: 3, 0
// viz: 27, 0
// Unlisted files inherit ux/mod.rs (root): ux/accept.rs, ux/activity.rs,
// ux/events.rs, ux/prefs.rs, ux/usage.rs.
use proc_macro2::Span;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
    Attribute, Expr, Item, UseTree,
};

const GROUPS: &[&str] = &[
    "root",
    "shell",
    "overlays",
    "run",
    "mesh",
    "wishes",
    "panels",
    "mini",
    "sessions_settings",
    "research",
    "paper",
    "modes",
    "data",
    "layout",
    "components",
    "motion",
    "viz",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layer {
    Foundation,
    Tokens,
    Seam,
    Adapter,
    Root,
    View,
}
fn in_dir(p: &str, d: &str) -> bool {
    p.strip_prefix(d).is_some_and(|s| s.starts_with('/'))
}
fn file_or_dir(p: &str, n: &str) -> bool {
    p == format!("{n}.rs") || in_dir(p, n)
}
fn direct_group(p: &str) -> Option<&'static str> {
    if in_dir(p, "layout") {
        Some("layout")
    } else if file_or_dir(p, "motion") {
        Some("motion")
    } else if in_dir(p, "viz") {
        Some("viz")
    } else if in_dir(p, "data") {
        Some("data")
    } else if in_dir(p, "components")
        || file_or_dir(p, "theme")
        || ["i18n.rs", "ornament.rs"].contains(&p)
    {
        Some("components")
    } else if [
        "mod.rs",
        "shell/pages.rs",
        "shell/background.rs",
        "shell/logic.rs",
        "perf.rs",
        "demo.rs",
        "ux/mod.rs",
    ]
    .contains(&p)
        || in_dir(p, "targets")
    {
        Some("root")
    } else if in_dir(p, "paper")
        || [
            "ux/seams/paper.rs",
            "ux/real/paper.rs",
            "ux/real/paper_accept.rs",
        ]
        .contains(&p)
    {
        Some("paper")
    } else if in_dir(p, "shell/modes")
        || [
            "composer/modes.rs",
            "ux/seams/modes.rs",
            "ux/real/modes.rs",
            "ux/real/modes_accept.rs",
            "ux/real/routines.rs",
            "ux/seams/routines.rs",
        ]
        .contains(&p)
    {
        Some("modes")
    } else if file_or_dir(p, "shell") || p == "launch.rs" {
        Some("shell")
    } else if file_or_dir(p, "palette") || file_or_dir(p, "composer") || p == "ux/composer.rs" {
        Some("overlays")
    } else if file_or_dir(p, "run_view") || file_or_dir(p, "todo") {
        Some("run")
    } else if file_or_dir(p, "mesh") || p == "ux/celestial.rs" {
        Some("mesh")
    } else if p == "detail/sessions.rs"
        || p == "detail/sessions_accept.rs"
        || file_or_dir(p, "settings")
    {
        Some("sessions_settings")
    } else if file_or_dir(p, "wishes") || file_or_dir(p, "detail") || file_or_dir(p, "control") {
        Some("wishes")
    } else if ["memory", "usage", "doctor"]
        .iter()
        .any(|n| file_or_dir(p, n))
        || ["quota.rs", "evolve.rs", "plain_text.rs"].contains(&p)
    {
        Some("panels")
    } else if ["mini.rs", "superswitch.rs"].contains(&p) {
        Some("mini")
    } else if in_dir(p, "research") {
        Some("research")
    } else if in_dir(p, "ux/seams")
        || in_dir(p, "ux/real")
        || file_or_dir(p, "live")
        || file_or_dir(p, "model")
        || p == "concurrency.rs"
    {
        Some("data")
    } else {
        None
    }
}
fn group(p: &str) -> Option<&'static str> {
    if let Some(g) = direct_group(p) {
        return Some(g);
    }
    let mut parent = p.rsplit_once('/').map(|(dir, _)| dir);
    while let Some(dir) = parent {
        if let Some(g) =
            direct_group(&format!("{dir}/mod.rs")).or_else(|| direct_group(&format!("{dir}.rs")))
        {
            return Some(g);
        }
        parent = dir.rsplit_once('/').map(|(up, _)| up);
    }
    None
}

fn layer(p: &str) -> Option<Layer> {
    group(p)?;
    Some(
        if file_or_dir(p, "theme")
            || ["components/tokens.rs", "motion/tokens.rs", "layout/grid.rs"].contains(&p)
        {
            Layer::Tokens
        } else if in_dir(p, "layout")
            || in_dir(p, "components")
            || file_or_dir(p, "motion")
            || in_dir(p, "viz")
            || ["i18n.rs", "ornament.rs"].contains(&p)
        {
            Layer::Foundation
        } else if in_dir(p, "ux/seams") {
            Layer::Seam
        } else if in_dir(p, "ux/real")
            || in_dir(p, "data")
            || file_or_dir(p, "live")
            || file_or_dir(p, "model")
            || p == "concurrency.rs"
        {
            Layer::Adapter
        } else if [
            "mod.rs",
            "shell.rs",
            "shell/pages.rs",
            "shell/background.rs",
            "shell/logic.rs",
            "perf.rs",
            "demo.rs",
            "ux/mod.rs",
        ]
        .contains(&p)
            || in_dir(p, "targets")
        {
            Layer::Root
        } else {
            Layer::View
        },
    )
}
fn walk(dir: &Path, result: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            walk(&p, result)
        } else if p.extension().is_some_and(|e| e == "rs") {
            result.push(p);
        }
    }
}
fn files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/desktop");
    let mut paths = Vec::new();
    walk(&root, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}
fn module_path(file: &str) -> Vec<String> {
    let mut parts: Vec<_> = file
        .trim_end_matches(".rs")
        .split('/')
        .map(str::to_owned)
        .collect();
    if parts.last().is_some_and(|s| s == "mod") {
        parts.pop();
    }
    let mut p = vec!["crate".into(), "desktop".into()];
    p.extend(parts);
    p
}
fn cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Path>()
                .is_ok_and(|p| p.is_ident("test"))
    })
}
fn item_attrs(i: &Item) -> &[Attribute] {
    match i {
        Item::Const(x) => &x.attrs,
        Item::Enum(x) => &x.attrs,
        Item::ExternCrate(x) => &x.attrs,
        Item::Fn(x) => &x.attrs,
        Item::ForeignMod(x) => &x.attrs,
        Item::Impl(x) => &x.attrs,
        Item::Macro(x) => &x.attrs,
        Item::Mod(x) => &x.attrs,
        Item::Static(x) => &x.attrs,
        Item::Struct(x) => &x.attrs,
        Item::Trait(x) => &x.attrs,
        Item::TraitAlias(x) => &x.attrs,
        Item::Type(x) => &x.attrs,
        Item::Union(x) => &x.attrs,
        Item::Use(x) => &x.attrs,
        _ => &[],
    }
}
#[derive(Clone, Debug)]
struct Import {
    path: Vec<String>,
    name: Option<String>,
    glob: bool,
    span: Span,
    public: bool,
}
fn flatten(tree: &UseTree, prefix: Vec<String>, public: bool, out: &mut Vec<Import>) {
    match tree {
        UseTree::Path(x) => {
            let mut p = prefix;
            p.push(x.ident.to_string());
            flatten(&x.tree, p, public, out);
        }
        UseTree::Name(x) => {
            let mut p = prefix;
            let n = x.ident.to_string();
            if n != "self" {
                p.push(n.clone());
            }
            let name = if n == "self" {
                p.last().cloned()
            } else {
                Some(n)
            };
            out.push(Import {
                path: p,
                name,
                glob: false,
                span: x.span(),
                public,
            });
        }
        UseTree::Rename(x) => {
            let mut p = prefix;
            if x.ident != "self" {
                p.push(x.ident.to_string());
            }
            out.push(Import {
                path: p,
                name: Some(x.rename.to_string()),
                glob: false,
                span: x.span(),
                public,
            });
        }
        UseTree::Glob(x) => out.push(Import {
            path: prefix,
            name: None,
            glob: true,
            span: x.span(),
            public,
        }),
        UseTree::Group(x) => {
            for t in &x.items {
                flatten(t, prefix.clone(), public, out);
            }
        }
    }
}
#[derive(Default)]
struct Index {
    modules: BTreeMap<Vec<String>, String>,
    exports: BTreeMap<Vec<String>, Vec<Import>>,
    members: BTreeMap<Vec<String>, BTreeSet<String>>,
    test_modules: BTreeSet<Vec<String>>,
}
fn collect_declarations(items: &[Item], module: &[String], idx: &mut Index) {
    for item in items {
        let name = match item {
            Item::Const(x) => Some(&x.ident),
            Item::Enum(x) => Some(&x.ident),
            Item::Fn(x) => Some(&x.sig.ident),
            Item::Mod(x) => Some(&x.ident),
            Item::Static(x) => Some(&x.ident),
            Item::Struct(x) => Some(&x.ident),
            Item::Trait(x) => Some(&x.ident),
            Item::TraitAlias(x) => Some(&x.ident),
            Item::Type(x) => Some(&x.ident),
            Item::Union(x) => Some(&x.ident),
            _ => None,
        };
        if let Some(name) = name {
            idx.members
                .entry(module.to_vec())
                .or_default()
                .insert(name.to_string());
        }
        if let Item::Mod(m) = item {
            let mut path = module.to_vec();
            path.push(m.ident.to_string());
            if cfg_test(&m.attrs) {
                idx.test_modules.insert(path.clone());
            }
            if let Some((_, inner)) = &m.content {
                collect_declarations(inner, &path, idx);
            }
        }
        if cfg_test(item_attrs(item)) {
            continue;
        }
        if let Item::Use(u) = item {
            let mut imports = Vec::new();
            flatten(
                &u.tree,
                Vec::new(),
                !matches!(u.vis, syn::Visibility::Inherited),
                &mut imports,
            );
            idx.exports
                .entry(module.to_vec())
                .or_default()
                .extend(imports);
        }
    }
}
fn index(files: &[(String, String)]) -> Index {
    let mut idx = Index::default();
    for (p, source) in files {
        let module = module_path(p);
        idx.modules.insert(module.clone(), p.clone());
        let syntax = syn::parse_file(source).unwrap_or_else(|e| panic!("{p}: {e}"));
        collect_declarations(&syntax.items, &module, &mut idx);
    }
    idx
}
fn absolute(
    path: &[String],
    module: &[String],
    aliases: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    if path.is_empty() {
        return vec![];
    }
    if path[0] == "crate" || path[0] == "gpui_kit" || path[0] == "gpui" || path[0] == "std" {
        return path.to_vec();
    }
    let mut i = 0;
    let mut result = module.to_vec();
    if path[0] == "self" {
        i = 1;
    } else if path[0] == "super" {
        while i < path.len() && path[i] == "super" {
            result.pop();
            i += 1;
        }
    } else if let Some(alias) = aliases.get(&path[0]) {
        result = alias.clone();
        i = 1;
    } else {
        result.push(path[0].clone());
        i = 1;
    }
    result.extend_from_slice(&path[i..]);
    result
}
fn can_export(
    module: &[String],
    name: &str,
    idx: &Index,
    seen: &mut BTreeSet<Vec<String>>,
) -> bool {
    if !seen.insert(module.to_vec()) {
        return false;
    }
    if idx.members.get(module).is_some_and(|m| m.contains(name)) {
        return true;
    }
    if let Some(exports) = idx.exports.get(module) {
        for e in exports {
            if e.name.as_deref() == Some(name) {
                return true;
            }
            if e.glob && e.public {
                let target = absolute(&e.path, module, &BTreeMap::new());
                if !idx.modules.contains_key(&target) || can_export(&target, name, idx, seen) {
                    return true;
                }
            }
        }
    }
    false
}
fn expand(path: Vec<String>, idx: &Index, _depth: usize) -> Vec<Vec<String>> {
    // An export edge can be followed only once per chain; this both preserves
    // distinct re-export routes and terminates on mutually recursive globs.
    fn follow(
        path: Vec<String>,
        idx: &Index,
        edges: &mut BTreeSet<(Vec<String>, usize)>,
        out: &mut BTreeSet<Vec<String>>,
    ) {
        let mut expanded = false;
        for n in (2..path.len()).rev() {
            let module = &path[..n];
            if let Some(exports) = idx.exports.get(module) {
                for (i, e) in exports.iter().enumerate() {
                    if !e.glob && e.name.as_deref() != Some(path[n].as_str()) {
                        continue;
                    }
                    if e.glob {
                        if !e.public {
                            continue;
                        }
                        if idx
                            .members
                            .get(module)
                            .is_some_and(|m| m.contains(&path[n]))
                            || exports
                                .iter()
                                .any(|x| x.name.as_deref() == Some(path[n].as_str()))
                        {
                            continue;
                        }
                        let glob_target = absolute(&e.path, module, &BTreeMap::new());
                        if idx.modules.contains_key(&glob_target)
                            && !can_export(&glob_target, &path[n], idx, &mut BTreeSet::new())
                        {
                            continue;
                        }
                    }
                    let edge = (module.to_vec(), i);
                    if !edges.insert(edge.clone()) {
                        continue;
                    }
                    let mut target = absolute(&e.path, module, &BTreeMap::new());
                    if !(e.glob && path.starts_with(&target)) {
                        target.extend_from_slice(if e.glob { &path[n..] } else { &path[n + 1..] });
                        if target != path {
                            expanded = true;
                            follow(target, idx, edges, out);
                        }
                    }
                    edges.remove(&edge);
                }
            }
        }
        if !expanded {
            out.insert(path);
        }
    }
    let mut out = BTreeSet::new();
    follow(path, idx, &mut BTreeSet::new(), &mut out);
    out.into_iter().collect()
}
fn literal(e: &Expr) -> bool {
    match e {
        Expr::Lit(x) => matches!(x.lit, syn::Lit::Int(_) | syn::Lit::Float(_)),
        Expr::Unary(x) => literal(&x.expr),
        Expr::Paren(x) => literal(&x.expr),
        Expr::Group(x) => literal(&x.expr),
        _ => false,
    }
}
fn contains_literal(e: &Expr) -> bool {
    struct Find(bool);
    impl<'ast> Visit<'ast> for Find {
        fn visit_expr(&mut self, e: &'ast Expr) {
            if literal(e) {
                self.0 = true;
            } else {
                visit::visit_expr(self, e);
            }
        }
    }
    let mut f = Find(false);
    f.visit_expr(e);
    f.0
}
#[derive(Debug)]
struct Violation {
    line: usize,
    column: usize,
    rule: &'static str,
}
// Layout helpers, theme accessors, traits and metadata are not widgets.
fn component_widget(path: &[&str]) -> bool {
    if !path.starts_with(&["gpui_kit", "component"]) {
        return false;
    }
    let tail = &path[2..];
    if tail.is_empty() {
        return true;
    } // A glob can include widgets.
    let non_widgets = [
        "h_flex",
        "v_flex",
        "ActiveTheme",
        "Disableable",
        "Selectable",
        "Sizable",
        "WindowExt",
        "StyledExt",
        "ElementExt",
        "IconName",
        "ButtonVariants",
        "ButtonVariant",
        "ButtonCustomVariant",
        "InputEvent",
        "InputState",
        "MoveDown",
        "OutdentInline",
        "IndentInline",
        "ListDelegate",
        "ListEvent",
        "ListState",
        "ContextMenuExt",
        "DropdownMenu",
        "ScrollableElement",
        "ResizablePanelEvent",
        "ResizableState",
        "TITLE_BAR_HEIGHT",
    ];
    let name = tail
        .iter()
        .find(|part| part.chars().next().is_some_and(char::is_uppercase))
        .copied()
        .unwrap_or_else(|| tail.last().copied().unwrap());
    !non_widgets.contains(&name)
}
struct Scan<'a> {
    module: Vec<String>,
    layer: Layer,
    idx: &'a Index,
    aliases: BTreeMap<String, Vec<String>>,
    violations: Vec<Violation>,
    tokens: bool,
}
impl Scan<'_> {
    fn add(&mut self, span: Span, rule: &'static str) {
        self.violations.push(Violation {
            line: span.start().line,
            column: span.start().column,
            rule,
        });
    }
    fn resolvable(&self, parts: &[String]) -> bool {
        let Some(first) = parts.first() else {
            return false;
        };
        if ["crate", "self", "super", "gpui", "gpui_kit"].contains(&first.as_str())
            || self.aliases.contains_key(first)
        {
            return true;
        }
        let mut local = self.module.clone();
        local.push(first.clone());
        self.idx.modules.contains_key(&local)
    }
    fn check_path(&mut self, path: Vec<String>, span: Span) {
        for p in expand(path, self.idx, 0) {
            let s: Vec<&str> = p.iter().map(String::as_str).collect();
            if self.layer == Layer::View && component_widget(&s) {
                self.add(span, "view widget");
            }
            if s.first() != Some(&"crate") {
                continue;
            }
            if self.layer == Layer::Seam && s.get(1) != Some(&"base") {
                self.add(span, "seam dependency");
            }
            let foundation = s.starts_with(&["crate", "desktop"])
                && s.get(2).is_some_and(|x| {
                    [
                        "components",
                        "theme",
                        "motion",
                        "layout",
                        "i18n",
                        "ornament",
                        "viz",
                    ]
                    .contains(x)
                });
            if matches!(self.layer, Layer::Foundation | Layer::Tokens)
                && (self
                    .module
                    .get(2)
                    .is_some_and(|x| x == "components" || x == "viz"))
                && !foundation
            {
                self.add(span, "foundation dependency");
            }
            if self.layer == Layer::View {
                let permitted = foundation
                    || s.starts_with(&["crate", "base"])
                        && s.get(2).is_some_and(|x| ["Failure", "Res"].contains(x))
                    || s.starts_with(&["crate", "desktop"])
                        && (s
                            .get(2)
                            .is_some_and(|x| ["model", "data", "targets"].contains(x))
                            || s.get(2) == Some(&"ux") && s.get(3) != Some(&"real"));
                if !permitted {
                    self.add(span, "view dependency");
                }
                for n in (2..=p.len()).rev() {
                    if let Some(file) = self.idx.modules.get(&p[..n]) {
                        if layer(file) == Some(Layer::View) && p[..n] != self.module[..] {
                            self.add(span, "view imports view");
                        }
                        break;
                    }
                }
            }
        }
    }
    fn scan_items(&mut self, items: &[Item]) {
        // Resolve local aliases before use, independent of declaration order.
        let mut imports = Vec::new();
        for item in items {
            if let Item::Use(u) = item {
                if !cfg_test(&u.attrs) {
                    flatten(
                        &u.tree,
                        Vec::new(),
                        !matches!(u.vis, syn::Visibility::Inherited),
                        &mut imports,
                    );
                }
            }
        }
        for _ in 0..imports.len().min(16) {
            for imp in &imports {
                if let Some(name) = &imp.name {
                    let target = absolute(&imp.path, &self.module, &self.aliases);
                    if name != "_" {
                        self.aliases.insert(name.clone(), target);
                    }
                }
            }
        }
        for imp in imports {
            let target = absolute(&imp.path, &self.module, &self.aliases);
            if self.resolvable(&imp.path) {
                self.check_path(target.clone(), imp.span);
            }
            if imp.glob {
                for p in expand(target, self.idx, 0) {
                    if let Some(exports) = self.idx.exports.get(&p) {
                        for export in exports {
                            if !export.public {
                                continue;
                            }
                            let t = absolute(&export.path, &p, &BTreeMap::new());
                            self.check_path(t, imp.span);
                        }
                    }
                }
            }
            let _ = imp.public;
        }
        for item in items {
            if !matches!(item, Item::Use(_)) {
                self.visit_item(item);
            }
        }
    }
}
impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_item(&mut self, i: &'ast Item) {
        if cfg_test(item_attrs(i)) {
            return;
        }
        if let Item::Mod(m) = i {
            if let Some((_, items)) = &m.content {
                let old = self.aliases.clone();
                self.module.push(m.ident.to_string());
                self.scan_items(items);
                self.module.pop();
                self.aliases = old;
            }
            return;
        }
        visit::visit_item(self, i);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        use syn::parse::Parser;
        // Many expression macros (for example vec![px(4)]) contain normal Rust
        // expressions; syn leaves their token streams opaque unless parsed here.
        let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
        if let Ok(expressions) = parser.parse2(mac.tokens.clone()) {
            let mut nested = Scan {
                module: self.module.clone(),
                layer: self.layer,
                idx: self.idx,
                aliases: self.aliases.clone(),
                violations: Vec::new(),
                tokens: self.tokens,
            };
            for expression in &expressions {
                nested.visit_expr(expression);
            }
            self.violations.extend(nested.violations);
        }
    }
    fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
        let attrs = match i {
            syn::ImplItem::Const(x) => &x.attrs,
            syn::ImplItem::Fn(x) => &x.attrs,
            syn::ImplItem::Type(x) => &x.attrs,
            syn::ImplItem::Macro(x) => &x.attrs,
            _ => return,
        };
        if !cfg_test(attrs) {
            visit::visit_impl_item(self, i);
        }
    }
    fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
        let attrs = match i {
            syn::TraitItem::Const(x) => &x.attrs,
            syn::TraitItem::Fn(x) => &x.attrs,
            syn::TraitItem::Type(x) => &x.attrs,
            syn::TraitItem::Macro(x) => &x.attrs,
            _ => return,
        };
        if !cfg_test(attrs) {
            visit::visit_trait_item(self, i);
        }
    }
    fn visit_path(&mut self, p: &'ast syn::Path) {
        if p.segments.len() > 1 {
            let parts: Vec<_> = p.segments.iter().map(|s| s.ident.to_string()).collect();
            if self.resolvable(&parts) {
                let path = absolute(&parts, &self.module, &self.aliases);
                self.check_path(path, p.span());
            }
        }
        visit::visit_path(self, p);
    }
    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        if self.tokens {
            if let Expr::Path(f) = &*e.func {
                let p = &f.path;
                let parts: Vec<_> = p.segments.iter().map(|s| s.ident.to_string()).collect();
                let full = absolute(&parts, &self.module, &self.aliases);
                let original: Vec<&str> = parts.iter().map(String::as_str).collect();
                let name = full.last().unwrap().clone();
                let numeric = [
                    "px", "rems", "size", "point", "relative", "hsla", "rgb", "rgba",
                ]
                .contains(&name.as_str())
                    && e.args.iter().any(contains_literal);
                let duration = name.starts_with("from_")
                    && (parts.iter().any(|x| x == "Duration")
                        || full.iter().any(|x| x == "Duration"))
                    && e.args.iter().any(literal);
                let black_white = ["black", "white"].contains(&name.as_str())
                    && (parts.len() == 1
                        || original.first() == Some(&"gpui")
                        || full.iter().any(|x| x == "gpui"));
                if numeric || duration || black_white {
                    self.add(e.span(), "token literal");
                }
            }
        }
        visit::visit_expr_call(self, e);
    }
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        if self.tokens
            && [
                "opacity",
                "font_weight",
                "px",
                "rems",
                "size",
                "point",
                "relative",
                "hsla",
                "rgb",
                "rgba",
            ]
            .contains(&e.method.to_string().as_str())
            && e.args.iter().any(contains_literal)
        {
            self.add(e.span(), "token literal");
        }
        visit::visit_expr_method_call(self, e);
    }
}
fn scan(file: &str, source: &str, idx: &Index, token_rules: bool) -> Vec<Violation> {
    let module = module_path(file);
    if idx.test_modules.iter().any(|p| module.starts_with(p)) {
        return vec![];
    }
    let tree = syn::parse_file(source).unwrap_or_else(|e| panic!("{file}: {e}"));
    if cfg_test(&tree.attrs) {
        return vec![];
    }
    let layer = layer(file).expect("unclassified file");
    let mut scan = Scan {
        module,
        layer,
        idx,
        aliases: BTreeMap::new(),
        violations: vec![],
        tokens: token_rules && layer != Layer::Tokens,
    };
    scan.scan_items(&tree.items);
    scan.violations
        .retain(|v| (v.rule == "token literal") == token_rules);
    scan.violations.sort_by_key(|v| (v.line, v.column, v.rule));
    scan.violations.dedup_by_key(|v| (v.line, v.column, v.rule));
    scan.violations
}
fn check_group(g: &str, token_rules: bool) {
    let files = files();
    let idx = index(&files);
    let mut errors = Vec::new();
    for (p, src) in &files {
        if group(p) == Some(g) {
            for v in scan(p, src, &idx, token_rules) {
                errors.push(format!("src/desktop/{p}:{}: {}", v.line, v.rule));
            }
        }
    }
    assert!(
        errors.is_empty(),
        "{} violations in {g}:\n{}",
        errors.len(),
        errors.join("\n")
    );
}
macro_rules! group_tests { ($($name:ident),*) => { $(mod $name { #[test] fn violations() { super::super::check_group(stringify!($name), super::TOKEN); } })* }; }
mod tokens {
    const TOKEN: bool = true;
    group_tests!(
        root,
        shell,
        overlays,
        run,
        mesh,
        wishes,
        panels,
        mini,
        sessions_settings,
        research,
        paper,
        modes,
        data,
        layout,
        components,
        motion,
        viz
    );
}
mod deps {
    const TOKEN: bool = false;
    group_tests!(
        root,
        shell,
        overlays,
        run,
        mesh,
        wishes,
        panels,
        mini,
        sessions_settings,
        research,
        paper,
        modes,
        data,
        layout,
        components,
        motion,
        viz
    );
}
mod classify {
    use super::*;
    #[test]
    fn all_files_and_unknown() {
        let fs = files();
        assert!(!fs.is_empty());
        for (p, _) in &fs {
            assert!(GROUPS.contains(&group(p).unwrap_or_else(|| panic!("unclassified group: {p}"))));
            assert!(layer(p).is_some(), "unclassified layer: {p}");
        }
        for p in ["unknown.rs", "unknown/file.rs"] {
            assert_eq!(group(p), None);
            assert_eq!(layer(p), None);
        }
    }
    #[test]
    fn priorities() {
        assert_eq!(group("detail/sessions.rs"), Some("sessions_settings"));
        assert_eq!(group("shell/modes/chat.rs"), Some("modes"));
        assert_eq!(layer("components/tokens.rs"), Some(Layer::Tokens));
        assert_eq!(layer("ux/seams/paper.rs"), Some(Layer::Seam));
    }
}
mod rules {
    use super::*;
    fn check(file: &str, source: &str, token: bool, rule: &str) -> bool {
        let fs = vec![
            (file.to_string(), source.to_string()),
            ("wishes.rs".into(), "pub struct Other;".into()),
        ];
        let idx = index(&fs);
        scan(file, source, &idx, token)
            .iter()
            .any(|v| v.rule == rule)
    }
    #[test]
    fn token_literals_and_clean() {
        for e in [
            "px(1)",
            "rems(1.)",
            "size(1, 2)",
            "point(1, 2)",
            "relative(1.)",
            "hsla(1., 0., 0., 1.)",
            "rgb(1)",
            "rgba(1)",
            "gpui::black()",
            "gpui::white()",
            "Duration::from_secs(1)",
            "x.opacity(0.5)",
            "x.font_weight(700)",
            "x.size(5)",
        ] {
            assert!(
                check(
                    "mini.rs",
                    &format!("fn f() {{ {e}; }}"),
                    true,
                    "token literal"
                ),
                "{e}"
            );
        }
        assert!(!check(
            "mini.rs",
            "fn f() { px(SPACE); x.opacity(OPACITY); x.font_weight(theme::WEIGHT); }",
            true,
            "token literal"
        ));
        assert!(!check(
            "components/tokens.rs",
            "fn f() { px(1); }",
            true,
            "token literal"
        ));
    }
    #[test]
    fn expression_macro_arguments() {
        assert!(check(
            "mini.rs",
            "fn f() { vec![px(2)]; }",
            true,
            "token literal"
        ));
        assert!(check(
            "mini.rs",
            "fn f() { vec![crate::orch::Thing::new()]; }",
            false,
            "view dependency"
        ));
    }
    #[test]
    fn view_to_view_alias_glob_and_reexport() {
        for use_ in [
            "use crate::desktop::wishes::Other as Alias;",
            "use crate::desktop::wishes::*;",
            "pub use crate::desktop::wishes::Other;",
        ] {
            assert!(check("mini.rs", use_, false, "view imports view"), "{use_}");
        }
        assert!(!check(
            "mini.rs",
            "use crate::desktop::theme::Tokens;",
            false,
            "view imports view"
        ));
    }
    #[test]
    fn indirect_aliases_globs_and_reexports() {
        let cases = [
            "use crate::desktop::wishes as w; use w::Other;",
            "use super::wishes::*;",
            "use crate::desktop::ux::Alias;",
            "use crate::desktop::ux::*;",
        ];
        for source in cases {
            let fs = vec![
                ("mini.rs".into(), source.into()),
                ("wishes.rs".into(), "pub struct Other;".into()),
                (
                    "ux/mod.rs".into(),
                    "pub use crate::desktop::wishes::Other as Alias;".into(),
                ),
            ];
            let idx = index(&fs);
            assert!(
                scan("mini.rs", source, &idx, false)
                    .iter()
                    .any(|v| v.rule == "view imports view"),
                "{source}"
            );
        }
    }
    #[test]
    fn foundation_dependencies() {
        assert!(check(
            "components/panel.rs",
            "use crate::orch::Thing;",
            false,
            "foundation dependency"
        ));
        assert!(!check(
            "viz/mod.rs",
            "use crate::desktop::theme::Tokens;",
            false,
            "foundation dependency"
        ));
    }
    #[test]
    fn view_dependencies() {
        for u in [
            "use crate::orch::Thing;",
            "use crate::desktop::ux::real::Thing;",
            "use crate::desktop::live::*;",
            "use crate::base::other;",
        ] {
            assert!(check("mini.rs", u, false, "view dependency"), "{u}");
        }
        assert!(!check("mini.rs", "use crate::base::{Failure, Res}; use crate::desktop::{model::Thing, ux::seams::Thing};", false, "view dependency"));
    }
    #[test]
    fn parent_private_foundation_alias() {
        let fs = vec![
            ("mesh.rs".into(), "use super::theme;".into()),
            (
                "mesh/council.rs".into(),
                "use super::theme; fn f() { theme::font(); }".into(),
            ),
            ("theme.rs".into(), "pub fn font() {}".into()),
        ];
        let idx = index(&fs);
        assert!(scan(&fs[1].0, &fs[1].1, &idx, false).is_empty());
    }
    #[test]
    fn local_and_external_types_are_not_crate_imports() {
        for rule in ["view dependency", "view imports view"] {
            assert!(!check(
                "mini.rs",
                "fn f() { Self::method(); Local::new(); external::Thing::new(); }",
                false,
                rule
            ));
        }
    }
    #[test]
    fn widget_dependencies() {
        assert!(check(
            "mini.rs",
            "use gpui_kit::component::button::Button as B;",
            false,
            "view widget"
        ));
        assert!(!check(
            "mini.rs",
            "use crate::desktop::components::Button; use gpui_kit::component::{v_flex, ActiveTheme, IconName};",
            false,
            "view widget"
        ));
    }
    #[test]
    fn seam_dependencies() {
        assert!(check(
            "ux/seams/optimizer.rs",
            "use crate::orch::Thing;",
            false,
            "seam dependency"
        ));
        assert!(!check(
            "ux/seams/optimizer.rs",
            "use crate::base::Failure;",
            false,
            "seam dependency"
        ));
    }
    #[test]
    fn cfg_test_exemption() {
        assert!(!check("mini.rs", "#[cfg(test)] mod tests { use crate::orch::Bad; fn f() { px(2); } } #[cfg(test)] fn f() { px(3); }", true, "token literal"));
        assert!(!check(
            "mini.rs",
            "#[cfg(test)] mod tests { use crate::orch::Bad; }",
            false,
            "view dependency"
        ));
    }
}
