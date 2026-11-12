use super::wish_scope::{aggregate, WishUsage};
use crate::desktop::ux::usage::UsageSession;

pub fn compare(
    sessions: &[UsageSession],
    a: Option<&str>,
    b: Option<&str>,
) -> (WishUsage, WishUsage, Option<f64>) {
    let a = aggregate(sessions, a);
    let b = aggregate(sessions, b);
    let delta = a.total_usd.zip(b.total_usd).map(|(a, b)| a - b);
    (a, b, delta)
}
