use crate::{
    base::Res,
    paper::{journals::openalex::WorkMeta, model::ReferenceSeed},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BibEntry {
    pub key: String,
    pub kind: String,
    pub doi: Option<String>,
    pub openalex: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub article_number: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bibliography {
    pub entries: Vec<BibEntry>,
}

fn bib_error() -> crate::base::Failure {
    crate::base::Failure::new("bibtex", "malformed BibTeX")
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}
impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn take(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }
    fn ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }
    fn expect(&mut self, c: char) -> Res<()> {
        self.ws();
        if self.take() == Some(c) {
            Ok(())
        } else {
            Err(bib_error())
        }
    }
    fn ident(&mut self) -> String {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }
    fn value(&mut self) -> Res<String> {
        self.ws();
        let open = self.peek().ok_or_else(bib_error)?;
        if open != '{' && open != '"' {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
            if start == self.pos {
                return Err(bib_error());
            }
            return Ok(self.chars[start..self.pos].iter().collect());
        }
        self.take();
        let mut depth = 0usize;
        let mut out = String::new();
        loop {
            let c = self.take().ok_or_else(bib_error)?;
            if c == '\\' {
                let next = self.take().ok_or_else(bib_error)?;
                if "&%_#${}~^\\".contains(next) {
                    out.push(next);
                } else {
                    out.push('\\');
                    out.push(next);
                }
            } else if open == '{' && c == '{' {
                depth += 1;
                out.push(c);
            } else if open == '{' && c == '}' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                out.push(c);
            } else if open == '"' && c == '"' {
                break;
            } else {
                out.push(c);
            }
        }
        Ok(out)
    }
}

pub fn parse_bibtex(input: &str) -> Res<Bibliography> {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let mut entries = Vec::new();
    loop {
        p.ws();
        if p.peek().is_none() {
            break;
        }
        p.expect('@')?;
        let typ = p.ident().to_lowercase();
        if typ.is_empty() {
            return Err(bib_error());
        }
        p.expect('{')?;
        p.ws();
        let start = p.pos;
        while p
            .peek()
            .is_some_and(|c| c != ',' && c != '}' && !c.is_whitespace())
        {
            p.pos += 1;
        }
        let key: String = p.chars[start..p.pos].iter().collect();
        if key.is_empty() {
            return Err(bib_error());
        }
        p.expect(',')?;
        let mut fields = std::collections::BTreeMap::new();
        loop {
            p.ws();
            if p.peek() == Some('}') {
                p.take();
                break;
            }
            let name = p.ident().to_lowercase();
            if name.is_empty() {
                return Err(bib_error());
            }
            p.expect('=')?;
            let value = p.value()?;
            fields.insert(name, value);
            p.ws();
            match p.peek() {
                Some(',') => {
                    p.take();
                }
                Some('}') => {}
                _ => return Err(bib_error()),
            }
        }
        let get = |names: &[&str]| names.iter().find_map(|n| fields.get(*n).cloned());
        let year = match get(&["year"]) {
            Some(y) => Some(y.trim().parse().map_err(|_| bib_error())?),
            None => None,
        };
        entries.push(BibEntry {
            key,
            kind: match typ.as_str() {
                "article" => "article",
                "conference" | "inproceedings" => "inproceedings",
                "book" => "book",
                "legal" => "legal",
                _ => "misc",
            }
            .into(),
            doi: get(&["doi"]).and_then(|s| super::citations::normalize_doi(&s)),
            openalex: get(&["openalex"]).and_then(|s| super::citations::normalize_openalex(&s)),
            title: get(&["title"]).unwrap_or_default(),
            authors: get(&["author"])
                .map(|s| {
                    s.split(" and ")
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            year,
            venue: get(&["journal", "booktitle", "journaltitle", "howpublished"]),
            volume: get(&["volume"]),
            issue: get(&["number"]),
            pages: get(&["pages"]),
            article_number: get(&["eid", "articleno", "article-number"]),
        });
    }
    Ok(Bibliography { entries })
}
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "&%$#_{}~^\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
pub fn to_bibtex(bib: &Bibliography) -> String {
    let mut entries: Vec<_> = bib.entries.iter().collect();
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("@{}{{{},\n", e.kind, e.key));
        let mut field =
            |name: &str, value: &str| out.push_str(&format!("  {name} = {{{}}},\n", escape(value)));
        if !e.authors.is_empty() {
            field("author", &e.authors.join(" and "));
        }
        field("title", &e.title);
        if let Some(y) = e.year {
            field("year", &y.to_string());
        }
        if let Some(v) = &e.venue {
            field(
                match e.kind.as_str() {
                    "article" => "journal",
                    "inproceedings" => "booktitle",
                    _ => "howpublished",
                },
                v,
            );
        }
        for (name, value) in [
            ("volume", &e.volume),
            ("number", &e.issue),
            ("pages", &e.pages),
            ("eid", &e.article_number),
            ("doi", &e.doi),
            ("openalex", &e.openalex),
        ] {
            if let Some(v) = value {
                field(name, v);
            }
        }
        out.push_str("}\n");
    }
    out
}
pub fn from_seeds(
    seeds: &[ReferenceSeed],
    lookup: &dyn Fn(&ReferenceSeed) -> Res<Option<WorkMeta>>,
) -> Bibliography {
    let entries = seeds
        .iter()
        .map(|seed| {
            let record = lookup(seed).ok().flatten();
            let mut e = BibEntry {
                key: seed.key.clone(),
                kind: "misc".into(),
                doi: None,
                openalex: None,
                title: String::new(),
                authors: vec![],
                year: None,
                venue: None,
                volume: None,
                issue: None,
                pages: None,
                article_number: None,
            };
            if let Some(w) = record {
                e.kind = if w.venue.is_some() { "article" } else { "misc" }.into();
                e.doi = w.doi.as_deref().and_then(super::citations::normalize_doi);
                e.openalex = super::citations::normalize_openalex(&w.id);
                e.title = w.title.unwrap_or_default();
                e.authors = w.authors;
                e.year = w.year;
                e.venue = w.venue;
                e.volume = w.volume;
                e.issue = w.issue;
                e.pages = w.first_page.map(|first| match w.last_page {
                    Some(last) => format!("{first}-{last}"),
                    None => first,
                });
                e.article_number = w.article_number;
            } else {
                e.doi = seed
                    .doi
                    .as_deref()
                    .and_then(super::citations::normalize_doi);
                e.openalex = seed
                    .openalex
                    .as_deref()
                    .and_then(super::citations::normalize_openalex);
            }
            e
        })
        .collect();
    Bibliography { entries }
}
