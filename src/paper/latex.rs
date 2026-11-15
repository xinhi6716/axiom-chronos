use crate::{
    base::{Failure, Res},
    paper::{
        bibliography::{self, Bibliography},
        model::{tokens, Disclosure, EvidenceBundle, Manuscript, RenderedTable, Token},
        quantities,
        templates::{TemplateEntry, TemplateStatus},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LatexProject {
    pub dir: PathBuf,
    pub main: PathBuf,
    pub files: Vec<PathBuf>,
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '_' | '%' => {
                out.push('\\');
                out.push(c);
            }
            '^' => out.push_str("\\textasciicircum{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            _ => out.push(c),
        }
    }
    out
}

fn render(text: &str, bundle: &EvidenceBundle) -> Res<String> {
    let mut out = String::new();
    let mut end = 0;
    for (range, token) in tokens(text) {
        out.push_str(&escape(&text[end..range.start]));
        match token {
            Token::Quantity(_) => out.push_str(&quantities::fill(&text[range.clone()], bundle)?),
            Token::Cite(keys) => out.push_str(&format!("\\cite{{{}}}", keys.join(","))),
            Token::Fig(id) => out.push_str(&format!("\\ref{{fig:{id}}}")),
            Token::Tab(id) => out.push_str(&format!("\\ref{{tab:{id}}}")),
            Token::Sec(id) => out.push_str(&format!("\\ref{{sec:{id}}}")),
        }
        end = range.end;
    }
    out.push_str(&escape(&text[end..]));
    Ok(out)
}

pub fn assemble(
    dir: &Path,
    ms: &Manuscript,
    bundle: &EvidenceBundle,
    template: &TemplateEntry,
    figures: &[(String, PathBuf)],
    tables: &[RenderedTable],
    bib: &Bibliography,
    disclosure: Option<&Disclosure>,
) -> Res<LatexProject> {
    if let TemplateStatus::Unconfirmed { reason } = &template.status {
        return Err(Failure::new("template_unconfirmed", reason));
    }
    if !matches!(
        template.id.as_str(),
        "elsarticle" | "IEEEtran" | "sn-jnl" | "achemso"
    ) || template.id != template.class
        || template.bib_style.trim().is_empty()
    {
        return Err(Failure::new(
            "template_mismatch",
            "Invalid template class or bibliography style",
        ));
    }
    let io = |e: std::io::Error| Failure::new("io", e.to_string());
    fs::create_dir_all(dir).map_err(io)?;
    let mut files = Vec::new();
    let mut body = String::new();
    for section in &ms.sections {
        body.push_str(&format!(
            "\\section{{{}}}\\label{{sec:{}}}\n",
            render(&section.heading, bundle)?,
            section.id
        ));
        for para in &section.paragraphs {
            body.push_str(&format!("{}\n\n", render(&para.text, bundle)?));
        }
    }
    for (index, (id, source)) in figures.iter().enumerate() {
        if id.is_empty()
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
        {
            return Err(Failure::new("latex", format!("Invalid figure id {id}")));
        }
        let spec = ms
            .figures
            .iter()
            .find(|f| &f.id == id)
            .ok_or_else(|| Failure::new("dangling", format!("Unknown figure {id}")))?;
        let name = format!("figure-{index}.pdf");
        let target = dir.join(&name);
        if source != &target {
            fs::copy(source, &target).map_err(io)?;
        }
        files.push(target);
        body.push_str(&format!("\\begin{{figure}}\n\\centering\n\\includegraphics[width=\\columnwidth]{{{name}}}\n\\caption{{{}}}\\label{{fig:{id}}}\n\\end{{figure}}\n", render(&spec.caption.text, bundle)?));
    }
    for table in tables {
        let spec = ms
            .tables
            .iter()
            .find(|t| t.id == table.id)
            .ok_or_else(|| Failure::new("dangling", format!("Unknown table {}", table.id)))?;
        body.push_str(&format!(
            "\\begin{{table}}\n\\centering\n\\caption{{{}}}\\label{{tab:{}}}\n{}\n\\end{{table}}\n",
            render(&spec.caption.text, bundle)?,
            table.id,
            table.latex
        ));
    }
    let mut front = match template.class.as_str() {
        "elsarticle" => format!("\\begin{{frontmatter}}\n\\title{{{}}}\n\\author{{Anonymous author(s)}}\n\\begin{{abstract}}\n{}\n\\end{{abstract}}\n\\end{{frontmatter}}", render(&ms.title, bundle)?, render(&ms.abstract_text.text, bundle)?),
        "IEEEtran" => format!("\\title{{{}}}\n\\author{{Anonymous author(s)}}\n\\maketitle\n\\begin{{abstract}}\n{}\n\\end{{abstract}}", render(&ms.title, bundle)?, render(&ms.abstract_text.text, bundle)?),
        _ => return Err(Failure::new("template_unconfirmed", "Front matter for this class is not confirmed")),
    };
    if !ms.keywords.is_empty() {
        front.push_str(&format!(
            "\n\\noindent\\textbf{{Keywords:}} {}\n",
            ms.keywords
                .iter()
                .map(|s| escape(s))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut after_bibliography = String::new();
    if let Some(d) = disclosure {
        let placement = d.placement.to_ascii_lowercase();
        let title = placement
            .find("titled ")
            .map(|pos| &d.placement[pos + 7..])
            .unwrap_or("Disclosure");
        let statement = format!(
            "\\section*{{{}}}\n{}\n",
            escape(title),
            render(&d.text, bundle)?
        );
        if placement.contains("after the references") || placement.contains("after references") {
            after_bibliography = statement;
        } else if placement.contains("before the references")
            || placement.contains("above the references")
            || placement.contains("before references")
            || placement.contains("above references")
            || placement.contains("end of the manuscript")
            || placement.contains("end of manuscript")
        {
            body.push_str(&statement);
        } else if placement.contains("beginning") || placement.contains("after the abstract") {
            body.insert_str(0, &statement);
        } else {
            return Err(Failure::new(
                "disclosure_placement",
                format!("Unsupported disclosure placement: {}", d.placement),
            ));
        }
    }
    let class_options = if template.class_options.is_empty() {
        String::new()
    } else {
        format!("[{}]", template.class_options.join(","))
    };
    let main_text = include_str!("../../engine/paper/templates/main.tex")
        .replace(
            "{{DOCUMENTCLASS}}",
            &format!("\\documentclass{class_options}{{{}}}", template.class),
        )
        .replace("{{PREAMBLE}}", &template.preamble.join("\n"))
        .replace("{{FRONTMATTER}}", &front)
        .replace("{{BODY}}", &body)
        .replace(
            "{{BIBLIOGRAPHY}}",
            &format!(
                "\\bibliographystyle{{{}}}\n\\bibliography{{refs}}\n{}",
                template.bib_style, after_bibliography
            ),
        );
    let main = dir.join("main.tex");
    let refs = dir.join("refs.bib");
    fs::write(&main, main_text).map_err(io)?;
    fs::write(&refs, bibliography::to_bibtex(bib)).map_err(io)?;
    files.push(main.clone());
    files.push(refs);
    Ok(LatexProject {
        dir: dir.to_path_buf(),
        main,
        files,
    })
}
