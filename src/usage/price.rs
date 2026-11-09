//! Claude API list prices in USD per million tokens, keyed by model family.
//!
//! Source: https://platform.claude.com/docs/en/about-claude/pricing, read 2026-11-09.
//! Claude Code's own cost-state records agree with these rates to the cent for Opus 5.5
//! (5-minute and 1-hour cache writes) and Haiku 5.5 (both prompt-length tiers).

pub const PRICES_AS_OF: &str = "2026-11-09";
pub const PRICES_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing";

const LONG_PROMPT_TOKENS: u64 = 100_000;
const FAST_MULTIPLIER: f64 = 2.0;
const US_ONLY_MULTIPLIER: f64 = 1.1;
const WEB_SEARCH_USD: f64 = 0.01;

#[derive(Clone, Copy)]
struct Rates {
    input: f64,
    write_5m: f64,
    write_1h: f64,
    read: f64,
    output: f64,
}

struct Family {
    id: &'static str,
    rates: Rates,
    over_100k: Option<Rates>,
    fast: bool,
}

const fn rates(input: f64, write_5m: f64, write_1h: f64, read: f64, output: f64) -> Rates {
    Rates {
        input,
        write_5m,
        write_1h,
        read,
        output,
    }
}
const fn family(
    id: &'static str,
    input: f64,
    write_5m: f64,
    write_1h: f64,
    read: f64,
    output: f64,
) -> Family {
    Family {
        id,
        rates: rates(input, write_5m, write_1h, read, output),
        over_100k: None,
        fast: false,
    }
}
impl Family {
    const fn with_fast_mode(mut self) -> Self {
        self.fast = true;
        self
    }
    const fn over_100k(mut self, rates: Rates) -> Self {
        self.over_100k = Some(rates);
        self
    }
}

#[rustfmt::skip]
const FAMILIES: &[Family] = &[
    //     family               input  5m write  1h write  read   output
    family("claude-fable-5-1",  10.0,  12.5,     20.0,     0.25,  50.0),
    family("claude-mythos-5-1", 10.0,  12.5,     20.0,     0.25,  50.0),
    family("claude-fable-5",    10.0,  12.5,     20.0,     1.0,   50.0),
    family("claude-mythos-5",   10.0,  12.5,     20.0,     1.0,   50.0),
    family("claude-opus-5-5",    4.0,   5.0,      8.0,     0.20,  20.0).with_fast_mode(),
    family("claude-opus-5",      5.0,   6.25,    10.0,     0.50,  25.0).with_fast_mode(),
    family("claude-opus-4-8",    5.0,   6.25,    10.0,     0.50,  25.0).with_fast_mode(),
    family("claude-opus-4-7",    5.0,   6.25,    10.0,     0.50,  25.0),
    family("claude-opus-4-6",    5.0,   6.25,    10.0,     0.50,  25.0),
    family("claude-opus-4-5",    5.0,   6.25,    10.0,     0.50,  25.0),
    family("claude-opus-4-1",   15.0,  18.75,    30.0,     1.50,  75.0),
    family("claude-opus-4",     15.0,  18.75,    30.0,     1.50,  75.0),
    family("claude-sonnet-5-5",  2.0,   2.50,     4.0,     0.10,  10.0),
    family("claude-sonnet-5",    2.0,   2.50,     4.0,     0.20,  10.0),
    family("claude-sonnet-4-6",  3.0,   3.75,     6.0,     0.30,  15.0),
    family("claude-sonnet-4-5",  3.0,   3.75,     6.0,     0.30,  15.0),
    family("claude-sonnet-4",    3.0,   3.75,     6.0,     0.30,  15.0),
    family("claude-haiku-5-5",   0.10,  0.125,    0.20,    0.01,  0.50)
        .over_100k(rates(        0.50,  0.625,    1.0,     0.05,  2.50)),
    family("claude-haiku-4-5",   1.0,   1.25,     2.0,     0.10,  5.0),
];

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CallUsage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    pub web_searches: u64,
    pub fast: bool,
    pub us_only: bool,
    pub non_standard_tier: bool,
}
impl CallUsage {
    pub fn cache_write_5m(&self) -> u64 {
        self.cache_write.saturating_sub(self.cache_write_1h)
    }
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
    pub fn is_zero(&self) -> bool {
        self.input == 0 && self.cache_read == 0 && self.cache_write == 0 && self.output == 0
    }
    pub fn merge(&mut self, other: &CallUsage) {
        self.input = self.input.max(other.input);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.cache_write_1h = self.cache_write_1h.max(other.cache_write_1h);
        self.cache_read = self.cache_read.max(other.cache_read);
        self.output = self.output.max(other.output);
        self.web_searches = self.web_searches.max(other.web_searches);
        self.fast |= other.fast;
        self.us_only |= other.us_only;
        self.non_standard_tier |= other.non_standard_tier;
    }
}

impl Rates {
    fn scaled(self, k: f64) -> Self {
        rates(
            self.input * k,
            self.write_5m * k,
            self.write_1h * k,
            self.read * k,
            self.output * k,
        )
    }
    fn write_cost(&self, u: &CallUsage) -> f64 {
        (u.cache_write_5m() as f64 * self.write_5m + u.cache_write_1h as f64 * self.write_1h) / 1e6
    }
    fn cost(&self, u: &CallUsage) -> f64 {
        (u.input as f64 * self.input
            + u.cache_read as f64 * self.read
            + u.output as f64 * self.output)
            / 1e6
            + self.write_cost(u)
    }
}

fn family_of(model: &str) -> Option<&'static Family> {
    let undated = match model.rsplit_once('-') {
        Some((base, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => model,
    };
    FAMILIES.iter().find(|f| f.id == undated)
}

fn rates_for(model: &str, u: &CallUsage) -> Option<Rates> {
    let family = family_of(model)?;
    if u.non_standard_tier || (u.fast && !family.fast) {
        return None;
    }
    let mut rates = match family.over_100k {
        Some(long) if u.context() > LONG_PROMPT_TOKENS => long,
        _ => family.rates,
    };
    if u.fast {
        rates = rates.scaled(FAST_MULTIPLIER);
    }
    if u.us_only {
        rates = rates.scaled(US_ONLY_MULTIPLIER);
    }
    Some(rates)
}

pub fn usd(model: &str, usage: &CallUsage) -> Option<f64> {
    let rates = rates_for(model, usage)?;
    Some(rates.cost(usage) + usage.web_searches as f64 * WEB_SEARCH_USD)
}

pub fn cache_write_usd(model: &str, usage: &CallUsage) -> Option<f64> {
    Some(rates_for(model, usage)?.write_cost(usage))
}
