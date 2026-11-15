//! Engine access for the quota strip.
use crate::desktop::model::facade::usage::{self, PoolRow};
use crate::usage::pools::{Pool, PoolStatus, PoolStrip, Remaining};
use gpui_kit::{App, Global};

impl Global for PoolStrip {}

pub(in crate::desktop) fn unknown_pools() -> Vec<PoolStatus> {
    Pool::ALL.into_iter().map(|pool| PoolStatus {
        pool, used_tokens: None, cost_usd: None, remaining: Remaining::Unknown, exhausted_until_ms: None,
    }).collect()
}

pub(in crate::desktop) fn pool_rows(cx: &App) -> Vec<PoolRow> {
    let fallback = unknown_pools();
    usage::pool_rows(cx.try_global::<PoolStrip>().map(|strip| strip.0.as_slice()).unwrap_or(&fallback))
}

pub(in crate::desktop) fn local_offset_minutes(now: i64) -> i32 {
    crate::session::local_offset_secs(now) / 60
}
