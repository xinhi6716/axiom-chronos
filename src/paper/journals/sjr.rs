use crate::{
    base::{Failure, Res},
    paper::{
        cache::{Cache, FetchMode, Fetched},
        csv,
        http::Transport,
        model::{Quartile, QuartileScheme, QuartileValue, SourceKind, Sourced},
        provenance,
    },
};
pub struct SjrTable {
    table: csv::CsvTable,
    fetched: Fetched,
    url: String,
    year: i32,
}
#[derive(Debug)]
pub struct SjrRow {
    pub sjr: Sourced<f64>,
    pub quartiles: Vec<Sourced<QuartileValue>>,
    pub publisher: Option<Sourced<String>>,
    pub open_access: Option<Sourced<bool>>,
}
pub fn load_year(cache: &Cache, t: &dyn Transport, mode: FetchMode, year: i32) -> Res<SjrTable> {
    let url = format!("https://www.scimagojr.com/journalrank.php?year={year}&type=j&out=xls");
    let fetched = cache.get(SourceKind::Sjr, &url, mode, t)?;
    let text =
        std::str::from_utf8(&fetched.body).map_err(|e| Failure::new("csv", e.to_string()))?;
    let table = csv::parse(text, b';')?;
    Ok(SjrTable {
        table,
        fetched,
        url,
        year,
    })
}
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}
impl SjrTable {
    pub fn lookup(&self, issns: &[String]) -> Option<SjrRow> {
        let index = |name: &str| self.table.headers.iter().position(|h| h == name);
        let issn_idx = index("Issn")?;
        let sjr_idx = index("SJR")?;
        let wanted: Vec<_> = issns
            .iter()
            .map(|s| normalize(s))
            .filter(|s| !s.is_empty())
            .collect();
        let (rownum, row) = self.table.rows.iter().enumerate().find(|(_, r)| {
            r[issn_idx]
                .split(',')
                .any(|s| wanted.contains(&normalize(s)))
        })?;
        let source = |col: &str| {
            provenance::tag(
                SourceKind::Sjr,
                &self.fetched,
                &self.url,
                &format!(
                    "/row/{rownum}/{}",
                    col.replace('~', "~0").replace('/', "~1")
                ),
                Some(self.year),
            )
        };
        let score = row[sjr_idx]
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())?;
        let quartiles = index("Categories")
            .map(|i| {
                row[i]
                    .split(';')
                    .filter_map(|c| {
                        let (category, q) = c.trim().rsplit_once(" (Q")?;
                        let q = match q.strip_suffix(')')? {
                            "1" => Quartile::Q1,
                            "2" => Quartile::Q2,
                            "3" => Quartile::Q3,
                            "4" => Quartile::Q4,
                            _ => return None,
                        };
                        Some(Sourced {
                            value: QuartileValue {
                                quartile: q,
                                scheme: QuartileScheme::Sjr,
                                category: category.trim().into(),
                                year: self.year,
                            },
                            source: source("Categories"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let publisher = index("Publisher").and_then(|i| {
            let s = row[i].trim();
            if s.is_empty() {
                None
            } else {
                Some(Sourced {
                    value: s.into(),
                    source: source("Publisher"),
                })
            }
        });
        let open_access = index("Open Access").and_then(|i| {
            let s = row[i].trim();
            let value = if s.eq_ignore_ascii_case("yes") {
                true
            } else if s.eq_ignore_ascii_case("no") {
                false
            } else {
                return None;
            };
            Some(Sourced {
                value,
                source: source("Open Access"),
            })
        });
        Some(SjrRow {
            sjr: Sourced {
                value: score,
                source: source("SJR"),
            },
            quartiles,
            publisher,
            open_access,
        })
    }
}
