use super::*;
use crate::desktop::ux::usage::UsageSession;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct WishUsage {
    pub wish: Option<SharedString>,
    pub label: SharedString,
    pub total_usd: Option<f64>,
    pub models: Vec<ModelCost>,
    pub sessions: usize,
    pub incomplete: bool,
}

fn wish_id(session: &UsageSession) -> &SharedString {
    if session.wish.is_empty() {
        &session.session
    } else {
        &session.wish
    }
}

pub fn unique<'a>(sessions: &'a [UsageSession], wish: Option<&str>) -> Vec<&'a UsageSession> {
    let mut seen = HashSet::new();
    sessions
        .iter()
        .filter(|s| seen.insert(s.session.clone()))
        .filter(|s| wish.is_none_or(|w| wish_id(s).as_ref() == w))
        .collect()
}

pub(super) fn is_incomplete(s: &UsageSession) -> bool {
    !s.complete || s.usage.total_usd.is_none() || s.usage.models.iter().any(|m| m.usd.is_none())
}

fn sum<T: Copy + std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
    Some(a? + b?)
}

pub fn aggregate(sessions: &[UsageSession], wish: Option<&str>) -> WishUsage {
    let selected = unique(sessions, wish);
    let label = match wish {
        Some(id) => selected
            .first()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| id.to_owned().into()),
        None => "All wishes".into(),
    };
    let mut total = if selected.is_empty() { None } else { Some(0.) };
    let mut models: Vec<ModelCost> = Vec::new();
    let mut incomplete = selected.is_empty();
    for s in &selected {
        let usage = &s.usage;
        let missing = is_incomplete(s);
        incomplete |= missing;
        total = sum(total, if missing { None } else { usage.total_usd });
        for m in &usage.models {
            let usd = if s.complete { m.usd } else { None };
            if let Some(row) = models.iter_mut().find(|r| r.model == m.model) {
                row.usd = sum(row.usd, usd);
                row.input = sum(row.input, m.input);
                row.cached = sum(row.cached, m.cached);
                row.cache_write = sum(row.cache_write, m.cache_write);
                row.output = sum(row.output, m.output);
                row.total = sum(row.total, m.total);
            } else {
                let mut row = m.clone();
                row.usd = usd;
                models.push(row);
            }
        }
    }
    if selected.iter().any(|s| s.usage.models.is_empty()) {
        for row in &mut models {
            row.usd = None;
        }
    }
    models.sort_by(|a, b| a.model.cmp(&b.model));
    WishUsage {
        wish: wish.map(|w| w.to_owned().into()),
        label,
        total_usd: total,
        models,
        sessions: selected.len(),
        incomplete,
    }
}

pub fn wishes(sessions: &[UsageSession]) -> Vec<(SharedString, SharedString)> {
    let mut seen = HashMap::new();
    for s in unique(sessions, None) {
        let entry = seen
            .entry(wish_id(s).clone())
            .or_insert((s.started_at, s.label.clone()));
        if s.started_at > entry.0 {
            *entry = (s.started_at, s.label.clone());
        }
    }
    let mut list: Vec<_> = seen.into_iter().collect();
    list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    list.into_iter()
        .map(|(id, (_, label))| (id, label))
        .collect()
}
pub fn search(list: &[(SharedString, SharedString)], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    list.iter()
        .enumerate()
        .filter(|(_, (id, label))| {
            id.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}
impl AppState {
    pub fn show_usage_for(&mut self, wish: Option<SharedString>, cx: &mut Context<Self>) {
        settings::Store::edit(cx, |s| s.ux.usage_wish = wish.map(|w| w.to_string()));
        self.set_view(Screen::Usage, cx);
    }
    pub fn set_usage_compare(&mut self, other: Option<SharedString>, cx: &mut Context<Self>) {
        settings::Store::edit(cx, |s| s.ux.usage_compare = other.map(|w| w.to_string()));
        cx.notify();
    }
}
