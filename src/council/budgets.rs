//! Shared budget validation for the engine, CLI and desktop.
use crate::base::{Failure, Res};
pub const FABLE_CAP: &str = "AXIOM_FABLE_CAP_USD";
pub const ASTRA_CAP: &str = "AXIOM_ASTRA_TOKEN_CAP";
pub const ARGON_CAP: &str = "AXIOM_ARGON_TOKEN_CAP";
pub const ROUNDS: &str = "AXIOM_COUNCIL_ROUNDS";
pub const POSITIVE_DOLLARS: &str =
    "Fable must be a positive dollar amount, at most $200; a larger cap is probably a typo.";
pub const POSITIVE_INTEGER: &str = "Tokens and rounds must be positive whole numbers.";
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetError {
    pub variable: &'static str,
    pub value: String,
    pub reason: &'static str,
}
impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} got {:?}: {}", self.variable, self.value, self.reason)
    }
}
impl From<BudgetError> for Failure {
    fn from(e: BudgetError) -> Self {
        Failure::new("invalid_argument", e.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budgets {
    pub fable_cap_usd: f64,
    pub astra_token_cap: u64,
    pub argon_token_cap: u64,
    pub rounds: u64,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            fable_cap_usd: super::DEFAULT_FABLE_CAP_USD,
            astra_token_cap: 200_000,
            argon_token_cap: 400_000,
            rounds: 3,
        }
    }
}
impl Budgets {
    pub fn parse(mut get: impl FnMut(&str) -> Option<String>) -> Result<Self, BudgetError> {
        let mut b = Self::default();
        if let Some(value) = get(FABLE_CAP) {
            b.fable_cap_usd = value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n > 0.0 && *n <= 200.0)
                .ok_or_else(|| BudgetError {
                    variable: FABLE_CAP,
                    value,
                    reason: POSITIVE_DOLLARS,
                })?;
        }
        for (key, slot) in [
            (ASTRA_CAP, &mut b.astra_token_cap),
            (ARGON_CAP, &mut b.argon_token_cap),
            (ROUNDS, &mut b.rounds),
        ] {
            if let Some(value) = get(key) {
                *slot = value
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| BudgetError {
                        variable: key,
                        value,
                        reason: POSITIVE_INTEGER,
                    })?;
            }
        }
        Ok(b)
    }
    pub fn from_env() -> Res<Self> {
        Self::parse(env_value).map_err(Into::into)
    }
    pub fn env(self) -> [(&'static str, String); 4] {
        [
            (FABLE_CAP, self.fable_cap_usd.to_string()),
            (ASTRA_CAP, self.astra_token_cap.to_string()),
            (ARGON_CAP, self.argon_token_cap.to_string()),
            (ROUNDS, self.rounds.to_string()),
        ]
    }
    pub fn validate(self) -> Res<Self> {
        Self::parse(|k| {
            self.env()
                .into_iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v)
        })
        .map_err(Into::into)
    }
    pub fn notice(self) -> String {
        format!("{}Fable is capped at ${:.2}, Astra at {} tokens, Argon at {} tokens for one review, up to {} rounds per phase", super::NOTICE, self.fable_cap_usd, tokens(self.astra_token_cap), tokens(self.argon_token_cap), self.rounds)
    }
}
pub fn env_value(key: &str) -> Option<String> {
    std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
}
pub fn tokens(n: u64) -> String {
    if n % 1000 == 0 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_overrides_and_ceiling() {
        assert_eq!(Budgets::parse(|_| None).unwrap(), Budgets::default());
        let b = Budgets::parse(|k| {
            Some(
                match k {
                    FABLE_CAP => "20",
                    ASTRA_CAP => "600000",
                    ARGON_CAP => "400000",
                    _ => "5",
                }
                .into(),
            )
        })
        .unwrap();
        assert_eq!(b.rounds, 5);
        assert!(b.notice().contains("$20.00, Astra at 600k tokens, Argon at 400k tokens for one review, up to 5 rounds per phase"));
        assert!(Budgets::parse(|k| (k == FABLE_CAP).then(|| "200".into())).is_ok());
    }
    #[test]
    fn invalid_values_preserve_the_key_and_value() {
        for name in [FABLE_CAP, ASTRA_CAP, ARGON_CAP, ROUNDS] {
            for value in ["", "text", "0", "-1", "NaN", "inf", "18446744073709551616"] {
                let e = Budgets::parse(|k| (k == name).then(|| value.into())).unwrap_err();
                assert_eq!(e.variable, name);
                assert_eq!(e.value, value);
            }
        }
        assert!(
            Budgets::parse(|k| (k == FABLE_CAP).then(|| "200.01".into()))
                .unwrap_err()
                .reason
                .contains("typo")
        );
        assert!(Budgets::parse(|k| (k == ROUNDS).then(|| "1.5".into())).is_err());
    }
}
