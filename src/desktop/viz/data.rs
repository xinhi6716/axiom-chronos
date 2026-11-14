//! Bounded display data, independent of the UI.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Bounded<K: Ord + Clone, T> {
    cap: usize,
    items: BTreeMap<K, T>,
}

impl<K: Ord + Clone, T> Bounded<K, T> {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            items: BTreeMap::new(),
        }
    }
    /// Replaces an equal key and discards the smallest key when over capacity.
    pub fn insert(&mut self, key: K, item: T) {
        self.items.insert(key, item);
        if self.items.len() > self.cap {
            self.items.pop_first();
        }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&K, &T)> {
        self.items.iter()
    }
    pub fn get(&self, key: &K) -> Option<&T> {
        self.items.get(key)
    }
}

#[derive(Clone, Copy, Debug)]
struct Ordered(f64);
impl PartialEq for Ordered {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0).is_eq()
    }
}
impl Eq for Ordered {}
impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// A chart sample whose value may be unavailable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartValue {
    Known(f64),
    Unknown,
}

/// Aggregates for known values in the retained window. Unknown samples occupy
/// window capacity but do not contribute to aggregates or `iter`/`window`.
/// Floating-point arithmetic follows IEEE 754.
/// When the window contains a non-finite value, `sum` and `mean` recompute the
/// sum over the window in O(cap) time.
#[derive(Clone, Debug)]
pub struct Series {
    window: Bounded<i64, f64>,
    unknown: BTreeSet<i64>,
    bins: Vec<i128>,
    nonfinite: usize,
    values: BTreeMap<Ordered, usize>,
}
impl Series {
    pub fn new(cap: usize) -> Self {
        Self {
            window: Bounded::new(cap),
            unknown: BTreeSet::new(),
            bins: vec![0; 2046],
            nonfinite: 0,
            values: BTreeMap::new(),
        }
    }
    // Every finite value is an integer multiple of the smallest subnormal.
    // Bins hold exact signed significands until the result is rounded to f64.
    fn add(&mut self, value: f64, direction: i128) {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 2047) as usize;
        let fraction = bits & ((1u64 << 52) - 1);
        let significand = if exponent == 0 {
            fraction
        } else {
            fraction | (1u64 << 52)
        };
        let sign = if bits >> 63 == 0 { 1 } else { -1 };
        let bin = exponent.saturating_sub(1);
        self.bins[bin] += direction * sign * significand as i128;
    }
    fn finite_sum(&self) -> f64 {
        // Enough bits for any f64 significand, exponent and usize-sized window.
        let mut words = [0u64; 35];
        for (shift, &bin) in self.bins.iter().enumerate() {
            if bin == 0 {
                continue;
            }
            let magnitude = bin.unsigned_abs();
            let low = magnitude as u64;
            let high = (magnitude >> 64) as u64;
            let offset = shift % 64;
            let parts = if offset == 0 {
                [low, high, 0]
            } else {
                [
                    low << offset,
                    (low >> (64 - offset)) | (high << offset),
                    high >> (64 - offset),
                ]
            };
            for (part, word) in parts.into_iter().enumerate() {
                let mut index = shift / 64 + part;
                let mut operand = word;
                while operand != 0 && index < words.len() {
                    let (result, carry) = if bin > 0 {
                        words[index].overflowing_add(operand)
                    } else {
                        words[index].overflowing_sub(operand)
                    };
                    words[index] = result;
                    operand = u64::from(carry);
                    index += 1;
                }
            }
        }
        let negative = words[words.len() - 1] >> 63 != 0;
        if negative {
            for word in &mut words {
                *word = !*word;
            }
            for word in &mut words {
                let (value, carry) = word.overflowing_add(1);
                *word = value;
                if !carry {
                    break;
                }
            }
        }
        let Some(top_word) = words.iter().rposition(|word| *word != 0) else {
            return 0.;
        };
        let top = top_word * 64 + 63 - words[top_word].leading_zeros() as usize;
        let sign = if negative { 1u64 << 63 } else { 0 };
        if top < 52 {
            return f64::from_bits(sign | words[0]);
        }
        let bit = |index: usize| (words[index / 64] >> (index % 64)) & 1;
        let mut significand = 0u64;
        for index in (top - 52..=top).rev() {
            significand = (significand << 1) | bit(index);
        }
        let discarded = top - 52;
        if discarded != 0 {
            let guard = bit(discarded - 1);
            let sticky = (0..discarded - 1).any(|index| bit(index) != 0);
            if guard != 0 && (sticky || significand & 1 != 0) {
                significand += 1;
            }
        }
        let mut exponent = top - 51;
        if significand == 1u64 << 53 {
            significand >>= 1;
            exponent += 1;
        }
        if exponent >= 2047 {
            return f64::from_bits(sign | (2047u64 << 52));
        }
        f64::from_bits(sign | ((exponent as u64) << 52) | (significand & ((1u64 << 52) - 1)))
    }
    fn remove_value(&mut self, value: f64) {
        if value.is_finite() {
            self.add(value, -1);
        } else {
            self.nonfinite -= 1;
        }
        let key = Ordered(value);
        let count = self.values.get_mut(&key).expect("value in window");
        *count -= 1;
        if *count == 0 {
            self.values.remove(&key);
        }
    }
    pub fn push(&mut self, ts: i64, value: f64) {
        self.push_value(ts, ChartValue::Known(value));
    }
    /// Insert or replace a sample; known and unknown samples share the capacity.
    pub fn push_value(&mut self, ts: i64, value: ChartValue) {
        if self.window.cap == 0 {
            return;
        }
        let first = self
            .window
            .items
            .keys()
            .next()
            .copied()
            .into_iter()
            .chain(self.unknown.first().copied())
            .min();
        if self.window.len() + self.unknown.len() == self.window.cap
            && first.is_some_and(|first| ts < first)
        {
            return;
        }
        if let Some(old) = self.window.items.remove(&ts) {
            self.remove_value(old);
        }
        self.unknown.remove(&ts);
        match value {
            ChartValue::Known(value) => {
                self.window.items.insert(ts, value);
                if value.is_finite() {
                    self.add(value, 1);
                } else {
                    self.nonfinite += 1;
                }
                *self.values.entry(Ordered(value)).or_default() += 1;
            }
            ChartValue::Unknown => {
                self.unknown.insert(ts);
            }
        }
        if self.window.len() + self.unknown.len() > self.window.cap {
            let first_known = self.window.items.keys().next().copied();
            let first_unknown = self.unknown.first().copied();
            if first_unknown.is_some_and(|u| first_known.is_none_or(|k| u < k)) {
                self.unknown.pop_first();
            } else {
                let (_, old) = self.window.items.pop_first().expect("over capacity");
                self.remove_value(old);
            }
        }
    }
    /// All retained samples in timestamp order, including unknown values.
    pub fn iter_points(&self) -> impl Iterator<Item = (i64, ChartValue)> {
        let mut points: Vec<_> = self
            .iter()
            .map(|(ts, value)| (*ts, ChartValue::Known(*value)))
            .chain(self.unknown.iter().map(|ts| (*ts, ChartValue::Unknown)))
            .collect();
        points.sort_unstable_by_key(|(ts, _)| *ts);
        points.into_iter()
    }
    pub fn window(&self) -> &Bounded<i64, f64> {
        &self.window
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&i64, &f64)> {
        self.window.iter()
    }
    pub fn sum(&self) -> f64 {
        if self.nonfinite != 0 {
            self.window.iter().map(|(_, v)| *v).sum()
        } else {
            self.finite_sum()
        }
    }
    pub fn count(&self) -> usize {
        self.window.len()
    }
    pub fn min(&self) -> Option<f64> {
        self.values
            .keys()
            .find(|v| !v.0.is_nan())
            .or_else(|| self.values.keys().next())
            .map(|v| v.0)
    }
    pub fn max(&self) -> Option<f64> {
        self.values
            .keys()
            .rev()
            .find(|v| !v.0.is_nan())
            .or_else(|| self.values.keys().next())
            .map(|v| v.0)
    }
    pub fn mean(&self) -> Option<f64> {
        (self.count() != 0).then(|| self.sum() / self.count() as f64)
    }
    #[cfg(test)]
    pub(super) fn corrupt_sum_for_test(&mut self) {
        self.add(1., 1);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionLost<K> {
    pub key: K,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection<K> {
    key: Option<K>,
}
impl<K: Ord + Clone> Selection<K> {
    pub fn new(key: Option<K>) -> Self {
        Self { key }
    }
    pub fn key(&self) -> Option<&K> {
        self.key.as_ref()
    }
    pub fn select(&mut self, key: Option<K>) {
        self.key = key;
    }
    /// Call after updating the window or changing the filter.
    pub fn reconcile<T>(
        &mut self,
        window: &Bounded<K, T>,
        visible: impl Fn(&K, &T) -> bool,
    ) -> Option<SelectionLost<K>> {
        let key = self.key.as_ref()?;
        if window.get(key).is_some_and(|item| visible(key, item)) {
            return None;
        }
        Some(SelectionLost {
            key: self.key.take().expect("selected key"),
        })
    }
}

/// A snapshot of matching items and their aggregates, rebuilt in O(window size).
pub struct Filtered<'a, K: Ord + Clone, T> {
    items: Vec<(&'a K, &'a T)>,
}
impl<'a, K: Ord + Clone, T> Filtered<'a, K, T> {
    pub fn new(window: &'a Bounded<K, T>, predicate: impl Fn(&K, &T) -> bool) -> Self {
        Self {
            items: window
                .iter()
                .filter(|(key, item)| predicate(key, item))
                .collect(),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&'a K, &'a T)> + '_ {
        self.items.iter().copied()
    }
    pub fn count(&self) -> usize {
        self.items.len()
    }
}
impl<K: Ord + Clone, T: Copy + Into<f64>> Filtered<'_, K, T> {
    pub fn sum(&self) -> f64 {
        self.items.iter().map(|(_, v)| (**v).into()).sum()
    }
    pub fn min(&self) -> Option<f64> {
        self.items
            .iter()
            .map(|(_, v)| (**v).into())
            .reduce(f64::min)
    }
    pub fn max(&self) -> Option<f64> {
        self.items
            .iter()
            .map(|(_, v)| (**v).into())
            .reduce(f64::max)
    }
    pub fn mean(&self) -> Option<f64> {
        (self.count() != 0).then(|| self.sum() / self.count() as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Bad,
}
#[derive(Clone, Debug)]
pub struct TimelineItem {
    pub id: String,
    pub at_ms: i64,
    pub label: String,
    pub tone: Tone,
}
#[derive(Clone, Debug)]
pub struct FunnelStage {
    pub label: String,
    pub count: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbitState {
    Idle,
    Speaking,
    Waiting,
    Degraded,
    Done,
}
#[derive(Clone, Debug)]
pub struct OrbitMember {
    pub id: String,
    pub label: String,
    pub state: OrbitState,
    pub weight: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarPhase {
    Apart,
    Approaching,
    Agreed,
    Disagreed,
}
#[derive(Clone, Debug)]
pub struct BinaryStar {
    pub a_label: String,
    pub b_label: String,
    pub phase: StarPhase,
    pub agreement: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKind {
    Bar,
    Line,
    Area,
}
#[derive(Clone, Debug)]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub series: Vec<(String, Series)>,
}
