use super::{field, flag, invalid, kind, unsupported};
use crate::base::Res;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

pub(super) enum Normalizer {
    Identity,
    Sequence(Vec<Self>),
    Lowercase,
    Nfc,
    Replace(String, String),
    Prepend(String),
    Bert {
        clean: bool,
        chinese: bool,
        strip: bool,
        lower: bool,
    },
}
impl Normalizer {
    pub fn parse(v: Option<&Value>) -> Res<Self> {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return Ok(Self::Identity);
        };
        Ok(match kind(v)? {
            "Sequence" => Self::Sequence(
                v.get("normalizers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("invalid normalizers"))?
                    .iter()
                    .map(|v| Self::parse(Some(v)))
                    .collect::<Res<_>>()?,
            ),
            "Lowercase" => Self::Lowercase,
            "NFC" => Self::Nfc,
            "Prepend" => Self::Prepend(field(v, "prepend")?.to_owned()),
            "Replace" => {
                let p = v
                    .get("pattern")
                    .and_then(|x| x.get("String"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("normalizer Replace requires literal pattern"))?;
                if p.is_empty() {
                    return Err(invalid("empty Replace pattern"));
                }
                Self::Replace(p.to_owned(), field(v, "content")?.to_owned())
            }
            "BertNormalizer" => {
                let lower = flag(v, "lowercase", true)?;
                Self::Bert {
                    clean: flag(v, "clean_text", true)?,
                    chinese: flag(v, "handle_chinese_chars", true)?,
                    strip: flag(v, "strip_accents", lower)?,
                    lower,
                }
            }
            x => return Err(unsupported(format!("normalizer {x}"))),
        })
    }
    pub fn apply(&self, text: &str) -> Res<String> {
        Ok(match self {
            Self::Identity => text.to_owned(),
            Self::Sequence(ns) => {
                let mut s = text.to_owned();
                for n in ns {
                    s = n.apply(&s)?;
                }
                s
            }
            Self::Lowercase => text.to_lowercase(),
            Self::Nfc => {
                if text.chars().any(|c| {
                    is_mark(c) || matches!(c as u32, 0x1100..=0x11ff | 0x2126 | 0x212a | 0x212b)
                }) {
                    return Err(invalid(
                        "NFC cannot verify combining marks, Hangul jamo, or non-NFC singletons",
                    ));
                }
                text.to_owned()
            }
            Self::Replace(a, b) => text.replace(a, b),
            Self::Prepend(p) => format!("{p}{text}"),
            Self::Bert {
                clean,
                chinese,
                strip,
                lower,
            } => {
                let mut s = String::new();
                for mut c in text.chars() {
                    if *clean {
                        if c == '\0'
                            || c == '\u{fffd}'
                            || ((c.is_control() || category_match(c, r"^\p{Cf}$", &FORMAT))
                                && !matches!(c, '\t' | '\n' | '\r'))
                        {
                            continue;
                        }
                        if c.is_whitespace() {
                            c = ' ';
                        }
                    }
                    if *chinese && is_chinese(c) {
                        s.push(' ');
                        s.push(c);
                        s.push(' ');
                    } else {
                        s.push(c);
                    }
                }
                if *lower {
                    s = s.to_lowercase();
                }
                if *strip {
                    let mut stripped = String::new();
                    for c in s.chars() {
                        if category_match(c, r"^\p{Mn}$", &NONSPACING_MARK) {
                            continue;
                        }
                        if let Some(base) = base_letter(c) {
                            stripped.push(base);
                        } else if !c.is_ascii() && category_match(c, r"^\p{L}$", &LETTER) {
                            return Err(unsupported(format!(
                                "Bert accent stripping cannot decompose {c}"
                            )));
                        } else {
                            stripped.push(c);
                        }
                    }
                    s = stripped;
                }
                s
            }
        })
    }
}
pub(super) fn is_mark(c: char) -> bool {
    matches!(c as u32,0x0300..=0x036f|0x1ab0..=0x1aff|0x1dc0..=0x1dff|0x20d0..=0x20ff|0xfe20..=0xfe2f)
}
fn is_chinese(c: char) -> bool {
    matches!(c as u32,0x4e00..=0x9fff|0x3400..=0x4dbf|0x20000..=0x2a6df|0x2a700..=0x2b73f|0x2b740..=0x2b81f|0x2b820..=0x2ceaf|0xf900..=0xfaff|0x2f800..=0x2fa1f)
}
fn base_letter(c: char) -> Option<char> {
    const GROUPS: &[(&str, char)] = &[
        ("ÀÁÂÃÄÅĀĂĄǍ", 'A'),
        ("àáâãäåāăąǎ", 'a'),
        ("ÇĆĈĊČ", 'C'),
        ("çćĉċč", 'c'),
        ("Ď", 'D'),
        ("ď", 'd'),
        ("ÈÉÊËĒĔĖĘĚ", 'E'),
        ("èéêëēĕėęě", 'e'),
        ("ĜĞĠĢ", 'G'),
        ("ĝğġģ", 'g'),
        ("Ĥ", 'H'),
        ("ĥ", 'h'),
        ("ÌÍÎÏĨĪĬĮİ", 'I'),
        ("ìíîïĩīĭį", 'i'),
        ("Ĵ", 'J'),
        ("ĵ", 'j'),
        ("Ķ", 'K'),
        ("ķ", 'k'),
        ("ĹĻĽ", 'L'),
        ("ĺļľ", 'l'),
        ("ÑŃŅŇ", 'N'),
        ("ñńņň", 'n'),
        ("ÒÓÔÕÖŌŎŐ", 'O'),
        ("òóôõöōŏő", 'o'),
        ("ŔŖŘ", 'R'),
        ("ŕŗř", 'r'),
        ("ŚŜŞŠ", 'S'),
        ("śŝşš", 's'),
        ("ŢŤ", 'T'),
        ("ţť", 't'),
        ("ÙÚÛÜŨŪŬŮŰŲ", 'U'),
        ("ùúûüũūŭůűų", 'u'),
        ("Ŵ", 'W'),
        ("ŵ", 'w'),
        ("ÝŶŸ", 'Y'),
        ("ýÿŷ", 'y'),
        ("ŹŻŽ", 'Z'),
        ("źżž", 'z'),
    ];
    GROUPS.iter().find_map(|(s, b)| s.contains(c).then_some(*b))
}

static FORMAT: OnceLock<Regex> = OnceLock::new();
static NONSPACING_MARK: OnceLock<Regex> = OnceLock::new();
static LETTER: OnceLock<Regex> = OnceLock::new();
fn category_match(c: char, pattern: &str, cache: &OnceLock<Regex>) -> bool {
    let mut bytes = [0; 4];
    cache
        .get_or_init(|| Regex::new(pattern).unwrap())
        .is_match(c.encode_utf8(&mut bytes))
}
