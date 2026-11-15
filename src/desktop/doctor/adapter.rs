//! Background sources for the settings and storage panels.
use crate::desktop::{data::{Cursor, Delta, Source}, ux::seams::{StorageItem, StorageService}};
use std::sync::Arc;

pub(super) fn ensure_folder(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[derive(Clone)]
pub(super) struct SettingsSnapshot {
    pub argon: bool,
    pub fast_cost: Option<crate::desktop::ux::seams::FastCost>,
}
pub(super) struct SettingsSource(pub std::path::PathBuf, pub Option<Arc<dyn crate::desktop::ux::seams::TurnPolicy>>);
impl Source<SettingsSnapshot> for SettingsSource {
    fn poll(&mut self, cursor: Cursor) -> crate::base::Res<(Delta<SettingsSnapshot>, Cursor)> {
        Ok((Delta::Replace(SettingsSnapshot {
            argon: crate::desktop::model::facade::usage::argon_every_round(&self.0),
            fast_cost: self.1.as_ref().and_then(|policy| policy.fast_cost()),
        }), cursor))
    }
}

pub(super) fn set_plain_words(on: bool, cx: &mut gpui_kit::App) -> crate::desktop::model::facade::settings::SettingPrior {
    use crate::desktop::model::facade::{settings, work};
    if !cx.has_global::<settings::PlainWordsApply>() {
        cx.set_global(settings::PlainWordsApply(work::apply_plain_words));
    }
    settings::set_plain_words(on, cx)
}

pub(super) fn set_argon(home: &std::path::Path, on: bool) -> crate::base::Res<()> {
    crate::desktop::model::facade::usage::set_argon_every_round(home, on).map(|_| ())
}

pub(super) struct StorageSource(pub Option<Arc<dyn StorageService>>);
impl Source<Vec<StorageItem>> for StorageSource {
    fn poll(&mut self, cursor: Cursor) -> crate::base::Res<(Delta<Vec<StorageItem>>, Cursor)> {
        let items = match &self.0 {
            Some(service) => service.report()?.items,
            None => Vec::new(),
        };
        Ok((Delta::Replace(items), cursor))
    }
}

impl super::storage::Flow {
    pub fn preview(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Selecting(ids) if !ids.is_empty() => match service.preview(&ids) {
                Ok(p) => Self::Previewed(p),
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
    pub fn confirm(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Previewed(plan) => match service.clean(&plan.id, plan.version, true) {
                Ok(job) => Self::Running { plan, job },
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
    pub fn poll(self, service: &dyn StorageService) -> Self {
        match self {
            Self::Running { plan, job } => match service.progress(&job) {
                Ok(progress) if progress.finished => Self::Finished { plan, progress },
                Ok(_) => Self::Running { plan, job },
                Err(e) => Self::Failed(e.to_string()),
            },
            other => other,
        }
    }
}

pub(super) fn progress(service: &dyn StorageService, job: &str) -> crate::base::Res<crate::desktop::ux::seams::CleanupProgress> {
    service.progress(job)
}
