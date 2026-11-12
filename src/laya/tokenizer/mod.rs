//! HuggingFace tokenizer.json inference without runtime dependencies on tokenizers.
//!
//! Bert accent stripping supports ASCII and the built-in Latin accent table
//! (Latin-1, Latin Extended-A, and selected Latin Extended-B letters). Other
//! non-ASCII letters are rejected because their decomposition is not known.
mod normalize;
mod pretokenize;

use crate::base::{Failure, Res};
use normalize::Normalizer;
use pretokenize::PreTokenizer;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("laya_tokenizer_invalid", message)
}
fn unsupported(message: impl Into<String>) -> Failure {
    Failure::new("laya_tokenizer_unsupported", message)
}
fn field<'a>(v: &'a Value, key: &str) -> Res<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing or invalid {key}")))
}
fn flag(v: &Value, key: &str, default: bool) -> Res<bool> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(invalid(format!("invalid {key}"))),
    }
}
fn kind(v: &Value) -> Res<&str> {
    field(v, "type")
}

#[derive(Debug)]
pub struct Specials {
    pub cls: u32,
    pub sep: u32,
    pub pad: u32,
    pub mask: u32,
    pub unk: u32,
}
#[derive(Clone)]
struct Added {
    content: String,
    needle: String,
    id: u32,
    normalized: bool,
    single_word: bool,
    lstrip: bool,
    rstrip: bool,
}
enum Model {
    WordPiece {
        prefix: String,
        max: usize,
        unk: u32,
    },
    Bpe {
        ranks: HashMap<(String, String), usize>,
        byte_fallback: bool,
        ignore_merges: bool,
        unk: Option<u32>,
        prefix: String,
        suffix: String,
        fuse_unk: bool,
    },
}
pub struct Tokenizer {
    vocab: HashMap<String, u32>,
    model: Model,
    normalizer: Normalizer,
    pre: PreTokenizer,
    added: Vec<Added>,
    specials: Specials,
}

impl Tokenizer {
    pub fn from_file(path: &Path) -> Res<Self> {
        let s =
            fs::read_to_string(path).map_err(|e| invalid(format!("cannot read tokenizer: {e}")))?;
        Self::from_str(&s)
    }
    pub fn from_str(json: &str) -> Res<Self> {
        let root: Value = serde_json::from_str(json.strip_prefix('\u{feff}').unwrap_or(json))
            .map_err(|e| invalid(format!("invalid JSON: {e}")))?;
        let m = root.get("model").ok_or_else(|| invalid("missing model"))?;
        let model_kind = kind(m)?;
        if model_kind != "WordPiece" && model_kind != "BPE" {
            return Err(unsupported(format!("model {model_kind}")));
        }
        let normalizer = Normalizer::parse(root.get("normalizer"))?;
        let pre = PreTokenizer::parse(root.get("pre_tokenizer"))?;
        let vocab_obj = m
            .get("vocab")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("missing model vocab"))?;
        let mut vocab = HashMap::new();
        for (s, id) in vocab_obj {
            let id = id
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalid("invalid vocabulary id"))?;
            vocab.insert(s.clone(), id);
        }
        let mut added = Vec::new();
        for a in root
            .get("added_tokens")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("missing added_tokens"))?
        {
            let content = field(a, "content")?.to_string();
            if content.is_empty() {
                return Err(invalid("empty added token"));
            }
            let id = a
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| invalid("invalid added token id"))?;
            let special = flag(a, "special", false)?;
            if vocab.get(&content).is_some_and(|old| *old != id) {
                return Err(invalid("conflicting added token id"));
            }
            let normalized = flag(a, "normalized", !special)?;
            let needle = if normalized {
                normalizer.apply(&content)?
            } else {
                content.clone()
            };
            added.push(Added {
                content,
                needle,
                id,
                normalized,
                single_word: flag(a, "single_word", false)?,
                lstrip: flag(a, "lstrip", false)?,
                rstrip: flag(a, "rstrip", false)?,
            });
        }
        let special_id = |names: &[&str]| -> Res<u32> {
            added
                .iter()
                .find(|a| names.contains(&a.content.as_str()))
                .map(|a| a.id)
                .ok_or_else(|| invalid(format!("missing special token {}", names.join("/"))))
        };
        let p = root
            .get("post_processor")
            .ok_or_else(|| invalid("missing post_processor"))?;
        if kind(p)? != "TemplateProcessing" {
            return Err(unsupported(format!("post_processor {}", kind(p)?)));
        }
        let single = p
            .get("single")
            .ok_or_else(|| invalid("missing single template"))?;
        let parts: Vec<String> = if let Some(s) = single.as_str() {
            s.split_whitespace()
                .map(|x| x.split(':').next().unwrap_or(x).to_string())
                .collect()
        } else if let Some(a) = single.as_array() {
            a.iter()
                .map(|x| {
                    if let Some(s) = x
                        .get("SpecialToken")
                        .and_then(|y| y.get("id"))
                        .and_then(Value::as_str)
                    {
                        Ok(s.to_owned())
                    } else if let Some(s) = x
                        .get("Sequence")
                        .and_then(|y| y.get("id"))
                        .and_then(Value::as_str)
                    {
                        Ok(format!("${s}"))
                    } else {
                        Err(invalid("invalid template element"))
                    }
                })
                .collect::<Res<_>>()?
        } else {
            return Err(invalid("invalid single template"));
        };
        if parts.len() != 3 || parts[1] != "$A" {
            return Err(unsupported(
                "post_processor TemplateProcessing: unsupported single template",
            ));
        }
        let template_id = |name: &str| -> Res<u32> {
            if let Some(st) = p.get("special_tokens").and_then(|v| v.get(name)) {
                let ids = st
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("invalid template special ids"))?;
                if ids.len() != 1 {
                    return Err(unsupported("template special must have exactly one id"));
                }
                return ids[0]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .ok_or_else(|| invalid("invalid template special id"));
            }
            added
                .iter()
                .find(|a| a.content == name)
                .map(|a| a.id)
                .or_else(|| vocab.get(name).copied())
                .ok_or_else(|| invalid(format!("missing template special {name}")))
        };
        let specials = Specials {
            cls: template_id(&parts[0])?,
            sep: template_id(&parts[2])?,
            pad: special_id(&["[PAD]", "<pad>"])?,
            mask: special_id(&["[MASK]", "<mask>"])?,
            unk: special_id(&["[UNK]", "<unk>"])?,
        };
        let lookup_unk = |v: &Value| -> Res<Option<u32>> {
            match v.get("unk_token") {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => vocab
                    .get(s)
                    .copied()
                    .map(Some)
                    .ok_or_else(|| invalid("unknown unk_token")),
                _ => Err(invalid("invalid unk_token")),
            }
        };
        let model = match kind(m)? {
            "WordPiece" => Model::WordPiece {
                prefix: m
                    .get("continuing_subword_prefix")
                    .and_then(Value::as_str)
                    .unwrap_or("##")
                    .to_owned(),
                max: m
                    .get("max_input_chars_per_word")
                    .and_then(Value::as_u64)
                    .unwrap_or(100)
                    .try_into()
                    .map_err(|_| invalid("invalid max_input_chars_per_word"))?,
                unk: lookup_unk(m)?.ok_or_else(|| invalid("missing WordPiece unk_token"))?,
            },
            "BPE" => {
                if m.get("dropout")
                    .is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0))
                {
                    return Err(unsupported("BPE dropout"));
                }
                let mut ranks = HashMap::new();
                for (i, pair) in m
                    .get("merges")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("missing BPE merges"))?
                    .iter()
                    .enumerate()
                {
                    let (a, b) = if let Some(s) = pair.as_str() {
                        s.split_once(' ')
                            .ok_or_else(|| invalid("invalid BPE merge"))?
                    } else {
                        let a = pair
                            .as_array()
                            .filter(|a| a.len() == 2)
                            .ok_or_else(|| invalid("invalid BPE merge"))?;
                        (
                            a[0].as_str().ok_or_else(|| invalid("invalid BPE merge"))?,
                            a[1].as_str().ok_or_else(|| invalid("invalid BPE merge"))?,
                        )
                    };
                    if a.is_empty()
                        || b.is_empty()
                        || ranks.insert((a.to_owned(), b.to_owned()), i).is_some()
                    {
                        return Err(invalid("invalid or duplicate BPE merge"));
                    }
                }
                Model::Bpe {
                    ranks,
                    byte_fallback: flag(m, "byte_fallback", false)?,
                    ignore_merges: flag(m, "ignore_merges", false)?,
                    unk: lookup_unk(m)?,
                    prefix: m
                        .get("continuing_subword_prefix")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    suffix: m
                        .get("end_of_word_suffix")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    fuse_unk: flag(m, "fuse_unk", false)?,
                }
            }
            other => return Err(unsupported(format!("model {other}"))),
        };
        Ok(Self {
            vocab,
            model,
            normalizer,
            pre,
            added,
            specials,
        })
    }
    pub fn token_to_id(&self, token: &str) -> Option<u32> {
        self.added
            .iter()
            .find(|a| a.content == token)
            .map(|a| a.id)
            .or_else(|| self.vocab.get(token).copied())
    }
    pub fn special(&self) -> &Specials {
        &self.specials
    }
    pub fn encode(&self, text: &str) -> Res<Vec<u32>> {
        let mut out = Vec::new();
        for segment in self.extract(text, false) {
            match segment {
                Segment::Token(id) => out.push(id),
                Segment::Text(s) => {
                    let normalized = self.normalizer.apply(&s)?;
                    for segment in self.extract(&normalized, true) {
                        match segment {
                            Segment::Token(id) => out.push(id),
                            Segment::Text(s) => {
                                for word in self.pre.apply(vec![s])? {
                                    self.model_encode(&word, &mut out)?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }
    fn extract(&self, text: &str, normalized: bool) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut cursor = 0;
        while cursor < text.len() {
            let mut best: Option<(usize, usize, &Added)> = None;
            for a in self.added.iter().filter(|a| a.normalized == normalized) {
                let needle = &a.needle;
                if needle.is_empty() {
                    continue;
                }
                for (off, _) in text[cursor..].match_indices(needle.as_str()) {
                    let start = cursor + off;
                    let end = start + needle.len();
                    if a.single_word
                        && (text[..start].chars().next_back().is_some_and(is_word)
                            || text[end..].chars().next().is_some_and(is_word))
                    {
                        continue;
                    }
                    if best.as_ref().is_none_or(|(s, e, _)| {
                        start < *s || (start == *s && end - start > *e - *s)
                    }) {
                        best = Some((start, end, a));
                    }
                    break;
                }
            }
            if let Some((mut start, mut end, a)) = best {
                if a.lstrip {
                    while start > cursor {
                        let c = text[..start].chars().next_back().unwrap();
                        if !c.is_whitespace() {
                            break;
                        }
                        start -= c.len_utf8();
                    }
                }
                if a.rstrip {
                    while end < text.len() {
                        let c = text[end..].chars().next().unwrap();
                        if !c.is_whitespace() {
                            break;
                        }
                        end += c.len_utf8();
                    }
                }
                if start > cursor {
                    out.push(Segment::Text(text[cursor..start].to_owned()));
                }
                out.push(Segment::Token(a.id));
                cursor = end;
            } else {
                out.push(Segment::Text(text[cursor..].to_owned()));
                break;
            }
        }
        out
    }
    fn model_encode(&self, s: &str, out: &mut Vec<u32>) -> Res<()> {
        if s.is_empty() {
            return Ok(());
        }
        match &self.model {
            Model::WordPiece { prefix, max, unk } => {
                let chars: Vec<usize> = s
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(s.len()))
                    .collect();
                if chars.len() - 1 > *max {
                    out.push(*unk);
                    return Ok(());
                }
                let mut pos = 0;
                let mut ids = Vec::new();
                while pos < chars.len() - 1 {
                    let mut found = None;
                    for end in (pos + 1..chars.len()).rev() {
                        let token = if pos == 0 {
                            s[chars[pos]..chars[end]].to_string()
                        } else {
                            format!("{prefix}{}", &s[chars[pos]..chars[end]])
                        };
                        if let Some(id) = self.vocab.get(&token) {
                            found = Some((end, *id));
                            break;
                        }
                    }
                    if let Some((end, id)) = found {
                        ids.push(id);
                        pos = end;
                    } else {
                        out.push(*unk);
                        return Ok(());
                    }
                }
                out.extend(ids);
            }
            Model::Bpe {
                ranks,
                byte_fallback,
                ignore_merges,
                unk,
                prefix,
                suffix,
                fuse_unk,
            } => {
                if *ignore_merges {
                    if let Some(id) = self.vocab.get(s) {
                        out.push(*id);
                        return Ok(());
                    }
                }
                let chars: Vec<char> = s.chars().collect();
                let mut pieces: Vec<String> = Vec::new();
                let mut unknown = false;
                for (i, c) in chars.iter().enumerate() {
                    let piece = format!(
                        "{}{}{}",
                        if i > 0 { prefix.as_str() } else { "" },
                        c,
                        if i + 1 == chars.len() {
                            suffix.as_str()
                        } else {
                            ""
                        }
                    );
                    if self.vocab.contains_key(&piece) {
                        pieces.push(piece);
                        unknown = false;
                    } else {
                        let mut bytes = [0u8; 4];
                        let fb: Vec<String> = c
                            .encode_utf8(&mut bytes)
                            .as_bytes()
                            .iter()
                            .map(|b| format!("<0x{b:02X}>"))
                            .collect();
                        if *byte_fallback && fb.iter().all(|p| self.vocab.contains_key(p)) {
                            pieces.extend(fb);
                            unknown = false;
                        } else if let Some(id) = unk {
                            if !*fuse_unk || !unknown {
                                pieces.push(format!("\0{id}"));
                            }
                            unknown = true;
                        }
                    }
                }
                while pieces.len() > 1 {
                    let best = pieces
                        .windows(2)
                        .enumerate()
                        .filter_map(|(i, p)| {
                            ranks.get(&(p[0].clone(), p[1].clone())).map(|r| (i, *r))
                        })
                        .min_by_key(|x| x.1);
                    if let Some((i, _)) = best {
                        let right = pieces.remove(i + 1);
                        let trimmed = if !prefix.is_empty() {
                            right.strip_prefix(prefix).unwrap_or(&right)
                        } else {
                            &right
                        };
                        pieces[i].push_str(trimmed);
                    } else {
                        break;
                    }
                }
                for p in pieces {
                    if let Some(id) = self.vocab.get(&p) {
                        out.push(*id)
                    } else if let Some(id) = p.strip_prefix('\0').and_then(|s| s.parse().ok()) {
                        out.push(id)
                    } else {
                        return Err(invalid(format!("missing BPE merged token {p}")));
                    }
                }
            }
        }
        Ok(())
    }
}
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
enum Segment {
    Text(String),
    Token(u32),
}
