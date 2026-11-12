use super::{field, flag, invalid, kind, unsupported};
use crate::base::Res;
use regex::Regex;
use serde_json::Value;

pub(super) enum PreTokenizer {
    Identity,
    Sequence(Vec<Self>),
    Bert,
    Whitespace,
    WhitespaceSplit,
    Metaspace {
        replacement: char,
        scheme: String,
        split: bool,
    },
    ByteLevel {
        add_prefix: bool,
        use_regex: bool,
    },
    Split {
        pattern: Regex,
        behavior: String,
        invert: bool,
    },
}
impl PreTokenizer {
    pub fn parse(v: Option<&Value>) -> Res<Self> {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return Ok(Self::Identity);
        };
        Ok(match kind(v)? {
            "Sequence" => Self::Sequence(
                v.get("pretokenizers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("invalid pretokenizers"))?
                    .iter()
                    .map(|x| Self::parse(Some(x)))
                    .collect::<Res<_>>()?,
            ),
            "BertPreTokenizer" => Self::Bert,
            "Whitespace" => Self::Whitespace,
            "WhitespaceSplit" => Self::WhitespaceSplit,
            "Metaspace" => {
                let r = field(v, "replacement")?;
                let mut chars = r.chars();
                let c = chars
                    .next()
                    .ok_or_else(|| invalid("empty Metaspace replacement"))?;
                if chars.next().is_some() {
                    return Err(invalid("Metaspace replacement must be one character"));
                }
                let scheme = v.get("prepend_scheme").and_then(Value::as_str).unwrap_or(
                    if flag(v, "add_prefix_space", true)? {
                        "always"
                    } else {
                        "never"
                    },
                );
                if !["always", "never", "first"].contains(&scheme) {
                    return Err(unsupported(format!("Metaspace prepend_scheme {scheme}")));
                }
                Self::Metaspace {
                    replacement: c,
                    scheme: scheme.to_owned(),
                    split: flag(v, "split", true)?,
                }
            }
            "ByteLevel" => Self::ByteLevel {
                add_prefix: flag(v, "add_prefix_space", true)?,
                use_regex: flag(v, "use_regex", true)?,
            },
            "Split" => {
                let p = v
                    .get("pattern")
                    .ok_or_else(|| invalid("missing Split pattern"))?;
                let pattern = if let Some(s) = p.get("String").and_then(Value::as_str) {
                    regex::escape(s)
                } else if let Some(s) = p.get("Regex").and_then(Value::as_str) {
                    s.to_owned()
                } else {
                    return Err(invalid("invalid Split pattern"));
                };
                let behavior = field(v, "behavior")?;
                if ![
                    "Removed",
                    "Isolated",
                    "MergedWithPrevious",
                    "MergedWithNext",
                    "Contiguous",
                ]
                .contains(&behavior)
                {
                    return Err(unsupported(format!("Split behavior {behavior}")));
                }
                Self::Split {
                    pattern: Regex::new(&pattern)
                        .map_err(|e| unsupported(format!("Split regex: {e}")))?,
                    behavior: behavior.to_owned(),
                    invert: flag(v, "invert", false)?,
                }
            }
            x => return Err(unsupported(format!("pre_tokenizer {x}"))),
        })
    }
    pub fn apply(&self, inputs: Vec<String>) -> Res<Vec<String>> {
        if let Self::Sequence(ns) = self {
            let mut x = inputs;
            for n in ns {
                x = n.apply(x)?;
            }
            return Ok(x);
        }
        let mut out = Vec::new();
        for (index, s) in inputs.into_iter().enumerate() {
            match self {
                Self::Identity => out.push(s),
                Self::WhitespaceSplit => out.extend(s.split_whitespace().map(str::to_owned)),
                Self::Whitespace => {
                    let re = Regex::new(r"\w+|[^\w\s]+").unwrap();
                    out.extend(re.find_iter(&s).map(|m| m.as_str().to_owned()));
                }
                Self::Bert => {
                    let re=Regex::new(r"[\p{P}\x21-\x2F\x3A-\x40\x5B-\x60\x7B-\x7E]|[^\s\p{P}\x21-\x2F\x3A-\x40\x5B-\x60\x7B-\x7E]+").unwrap();
                    out.extend(re.find_iter(&s).map(|m| m.as_str().to_owned()));
                }
                Self::Metaspace {
                    replacement,
                    scheme,
                    split,
                } => {
                    let mut t = s.replace(' ', &replacement.to_string());
                    if (scheme == "always" || (scheme == "first" && index == 0))
                        && !t.starts_with(*replacement)
                    {
                        t.insert(0, *replacement);
                    }
                    if *split {
                        let mut starts: Vec<usize> = t
                            .char_indices()
                            .filter_map(|(i, c)| (c == *replacement).then_some(i))
                            .collect();
                        if starts.first().copied() != Some(0) {
                            starts.insert(0, 0);
                        }
                        for (i, start) in starts.iter().enumerate() {
                            let end = starts.get(i + 1).copied().unwrap_or(t.len());
                            if *start < end {
                                out.push(t[*start..end].to_owned());
                            }
                        }
                    } else if !t.is_empty() {
                        out.push(t);
                    }
                }
                Self::ByteLevel {
                    add_prefix,
                    use_regex,
                } => {
                    let mut t = s;
                    if *add_prefix && !t.starts_with(' ') {
                        t.insert(0, ' ');
                    }
                    let chunks: Vec<String> = if *use_regex { byte_chunks(&t) } else { vec![t] };
                    out.extend(
                        chunks
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .map(|s| s.as_bytes().iter().map(|b| byte_char(*b)).collect()),
                    );
                }
                Self::Split {
                    pattern,
                    behavior,
                    invert,
                } => out.extend(split(&s, pattern, behavior, *invert)),
                Self::Sequence(_) => unreachable!(),
            }
        }
        Ok(out)
    }
}
fn byte_chunks(s: &str) -> Vec<String> {
    // GPT-2's lookahead whitespace alternative is handled separately below.
    let re =
        Regex::new(r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+").unwrap();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < s.len() {
        let m = re.find_at(s, pos).unwrap();
        let mut end = m.end();
        let chunk = &s[pos..end];
        if chunk.chars().all(char::is_whitespace) && end < s.len() && chunk.chars().count() > 1 {
            let last = chunk.chars().next_back().unwrap();
            end -= last.len_utf8();
        }
        out.push(s[pos..end].to_owned());
        pos = end;
    }
    out
}
fn byte_char(b: u8) -> char {
    if (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b) {
        return char::from_u32(b as u32).unwrap();
    }
    let mut n = 0;
    for x in 0..b {
        if !((33..=126).contains(&x) || (161..=172).contains(&x) || (174..=255).contains(&x)) {
            n += 1;
        }
    }
    char::from_u32(256 + n).unwrap()
}
fn split(s: &str, re: &Regex, behavior: &str, invert: bool) -> Vec<String> {
    let mut parts = Vec::<(String, bool)>::new();
    let mut pos = 0;
    for m in re.find_iter(s) {
        if m.start() > pos {
            parts.push((s[pos..m.start()].to_owned(), invert));
        }
        if m.end() > m.start() {
            parts.push((m.as_str().to_owned(), !invert));
        }
        pos = m.end();
    }
    if pos < s.len() {
        parts.push((s[pos..].to_owned(), invert));
    }
    let mut out: Vec<String> = Vec::new();
    let mut pending = String::new();
    let mut prev_match = false;
    for (p, matched) in parts {
        match behavior {
            "Removed" => {
                if !matched {
                    out.push(p)
                }
            }
            "Isolated" => out.push(p),
            "Contiguous" => {
                if matched && prev_match {
                    out.last_mut().unwrap().push_str(&p);
                } else {
                    out.push(p);
                }
                prev_match = matched;
            }
            "MergedWithPrevious" => {
                if matched && !out.is_empty() {
                    out.last_mut().unwrap().push_str(&p);
                } else {
                    out.push(p);
                }
            }
            "MergedWithNext" => {
                if matched {
                    pending.push_str(&p);
                } else {
                    pending.push_str(&p);
                    out.push(std::mem::take(&mut pending));
                }
            }
            _ => unreachable!(),
        }
    }
    if !pending.is_empty() {
        out.push(pending);
    }
    out
}
