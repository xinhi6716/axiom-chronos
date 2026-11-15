use super::model::{self, Language};
mod ecma;
mod lexer;
mod python;

pub fn extract(language: Language, source: &str) -> model::Extraction {
    match language {
        Language::Python => python::extract(source),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            ecma::extract(source, matches!(language, Language::Tsx | Language::Jsx))
        }
        Language::Rust => model::Extraction {
            errors: vec!["Rust is not a script language".into()],
            ..Default::default()
        },
    }
}
