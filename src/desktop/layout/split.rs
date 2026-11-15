use super::{grid, Breakpoint};
use crate::desktop::theme::Density;

#[derive(Clone, Debug)]
pub struct Pane {
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub collapsible: bool,
}
#[derive(Clone, Debug)]
pub struct Split {
    pub id: &'static str,
    pub panes: Vec<Pane>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    Side,
    Stacked,
    Tabs,
}

impl Split {
    pub fn sizes(&self, available: f32, saved: Option<&[f32]>) -> Vec<f32> {
        let available = if available.is_finite() {
            available.max(0.)
        } else {
            0.
        };
        let valid = saved.filter(|s| {
            s.len() == self.panes.len()
                && s.iter()
                    .zip(&self.panes)
                    .all(|(&v, p)| v.is_finite() && v >= p.min && v <= p.max)
        });
        let mut result: Vec<f32> = self
            .panes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let min = if p.min.is_finite() { p.min.max(0.) } else { 0. };
                let max = if p.max.is_finite() {
                    p.max.max(min)
                } else {
                    min
                };
                valid.map(|s| s[i]).unwrap_or(if p.default.is_finite() {
                    p.default.clamp(min, max)
                } else {
                    min
                })
            })
            .collect();
        let mins: Vec<f32> = self
            .panes
            .iter()
            .map(|p| if p.min.is_finite() { p.min.max(0.) } else { 0. })
            .collect();
        let sum: f32 = result.iter().sum();
        if sum > available {
            let capacity: f32 = result.iter().zip(&mins).map(|(v, m)| (v - m).max(0.)).sum();
            let reduction = (sum - available).min(capacity);
            if capacity > 0. {
                for (v, m) in result.iter_mut().zip(&mins) {
                    *v -= reduction * (*v - m).max(0.) / capacity;
                }
            }
            // If minima do not fit, collapse collapsible panes before reducing any other minimum.
            if result.iter().sum::<f32>() > available {
                for (v, p) in result.iter_mut().zip(&self.panes) {
                    if p.collapsible {
                        *v = 0.;
                    }
                }
                let remaining: f32 = result.iter().sum();
                if remaining > available && remaining > 0. {
                    for v in &mut result {
                        *v *= available / remaining;
                    }
                }
            }
        }
        // Avoid a floating-point rounding overshoot.
        let excess = result.iter().sum::<f32>() - available;
        if excess > 0. {
            if let Some(v) = result.iter_mut().max_by(|a, b| a.total_cmp(b)) {
                *v = (*v - excess).max(0.);
            }
        }
        result
    }
    pub fn resize(&self, sizes: &[f32], handle: usize, delta: f32, available: f32) -> Vec<f32> {
        // Preserve already-fitted collapsed or undersized panes when the available
        // space cannot satisfy all minima; they are outputs of sizes(), not saved data.
        let mins_do_not_fit = self.panes.iter().map(|p| p.min.max(0.)).sum::<f32>() > available;
        let fitted = sizes.len() == self.panes.len()
            && available.is_finite()
            && available >= 0.
            && sizes.iter().zip(&self.panes).all(|(&v, p)| {
                v.is_finite() && v >= 0. && v <= p.max && (v >= p.min || mins_do_not_fit)
            })
            && sizes.iter().sum::<f32>() <= available;
        let mut result = if fitted {
            sizes.to_vec()
        } else {
            self.sizes(available, Some(sizes))
        };
        if !delta.is_finite() || handle >= result.len().saturating_sub(1) {
            return result;
        }
        let a = &self.panes[handle];
        let b = &self.panes[handle + 1];
        if result[handle] < a.min || result[handle + 1] < b.min {
            return result;
        }
        let lower = (a.min - result[handle]).max(result[handle + 1] - b.max);
        let upper = (a.max - result[handle]).min(result[handle + 1] - b.min);
        if lower <= upper {
            let d = delta.clamp(lower, upper);
            result[handle] += d;
            result[handle + 1] -= d;
        }
        result
    }
}
pub fn arrange(bp: Breakpoint, split: &Split) -> Arrangement {
    match bp {
        Breakpoint::Narrow => {
            if split.panes.len() > 2 {
                Arrangement::Tabs
            } else {
                Arrangement::Stacked
            }
        }
        Breakpoint::Compact
            if split.panes.iter().map(|p| p.min.max(0.)).sum::<f32>()
                > 960. - 2. * f32::from(grid::margin(bp, Density::Comfortable)) =>
        {
            Arrangement::Stacked
        }
        _ => Arrangement::Side,
    }
}
