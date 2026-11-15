use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::graph::types::keys;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
}

pub fn language_of(path: &str) -> Option<Language> {
    let filename = path.rsplit(['/', '\\']).next()?;
    let (_, extension) = filename.rsplit_once('.')?;
    match extension {
        "rs" => Some(Language::Rust),
        "py" | "pyi" => Some(Language::Python),
        "ts" | "mts" | "cts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "js" | "mjs" | "cjs" => Some(Language::JavaScript),
        "jsx" => Some(Language::Jsx),
        _ => None,
    }
}

impl Language {
    pub fn family(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript | Self::Tsx => "ts",
            Self::JavaScript | Self::Jsx => "js",
        }
    }
}

/// A 1-based source span, with columns measured in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

pub const DEF_KINDS: &[&str] = &[
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
];

#[derive(Clone, Debug, Serialize)]
pub struct Def {
    pub kind: &'static str,
    pub name: String,
    pub qualname: String,
    pub span: Span,
    /// A single line of at most 160 characters.
    pub signature: String,
    pub parent: Option<usize>,
    pub exported: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportName {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Import {
    /// One of use, mod, from, import, es, re_export, or require.
    pub kind: &'static str,
    pub module: String,
    pub names: Vec<ImportName>,
    pub glob: bool,
    pub default_as: Option<String>,
    pub namespace_as: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Ref {
    pub name: String,
    pub qualifier: Vec<String>,
    /// One of call, type, value, or macro.
    pub kind: &'static str,
    pub span: Span,
    pub enclosing: Option<usize>,
    pub shadowed: bool,
    pub dynamic: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Extraction {
    pub language: Option<Language>,
    pub defs: Vec<Def>,
    pub imports: Vec<Import>,
    pub refs: Vec<Ref>,
    pub errors: Vec<String>,
}

pub fn symbol_keys(path: &str, defs: &[Def]) -> Vec<String> {
    let mut counts = HashMap::new();
    defs.iter()
        .map(|def| {
            let count = counts
                .entry((def.kind, def.qualname.as_str()))
                .or_insert(0u32);
            *count += 1;
            keys::symbol(path, def.kind, &def.qualname, *count)
        })
        .collect()
}

pub fn extract(language: Language, source: &str) -> Extraction {
    let result = catch_unwind(AssertUnwindSafe(|| match language {
        Language::Rust => super::rust::extract(source),
        _ => super::script::extract(language, source),
    }));
    let mut extraction = result.unwrap_or_else(|_| Extraction {
        errors: vec!["extractor panicked".to_string()],
        ..Extraction::default()
    });
    extraction.language = Some(language);
    extraction
}
