//! Per-user research session preferences.
use super::index::Filter;
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ResearchRef {
    #[serde(default)]
    pub root: PathBuf,
    #[serde(default)]
    pub store: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TabView {
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub selection: Option<String>,
    #[serde(default)]
    pub node: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ViewState {
    #[serde(default)]
    pub tab: String,
    #[serde(default)]
    pub tabs: BTreeMap<String, TabView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SessionPrefs {
    #[serde(default)]
    pub last: Option<ResearchRef>,
    #[serde(default)]
    pub recent: Vec<ResearchRef>,
    #[serde(default)]
    pub views: BTreeMap<String, ViewState>,
}

pub fn load(research_home: &Path) -> SessionPrefs {
    std::fs::read(research_home.join("session.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(research_home: &Path, p: &SessionPrefs) -> Res<()> {
    let bytes = serde_json::to_vec_pretty(p).map_err(|e| Failure::new("io", e.to_string()))?;
    std::fs::create_dir_all(research_home).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&research_home.join("session.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}

pub fn remember(p: &mut SessionPrefs, r: ResearchRef) {
    p.recent.retain(|item| item.root != r.root);
    p.recent.insert(0, r.clone());
    p.recent.truncate(8);
    p.last = Some(r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn remember_deduplicates_and_limits_in_order() {
        let mut prefs = SessionPrefs::default();
        for n in 0..10 {
            remember(
                &mut prefs,
                ResearchRef {
                    root: n.to_string().into(),
                    store: None,
                },
            );
        }
        let r = ResearchRef {
            root: "5".into(),
            store: Some("store".into()),
        };
        remember(&mut prefs, r.clone());
        assert_eq!(prefs.last, Some(r.clone()));
        assert_eq!(prefs.recent.len(), 8);
        assert_eq!(prefs.recent[0], r);
        assert_eq!(
            prefs
                .recent
                .iter()
                .map(|x| x.root.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["5", "9", "8", "7", "6", "4", "3", "2"]
        );
    }

    #[test]
    fn damaged_and_missing_files_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), SessionPrefs::default());
        for data in ["", "{", r#"{"recent":3}"#] {
            fs::write(dir.path().join("session.json"), data).unwrap();
            assert_eq!(load(dir.path()), SessionPrefs::default());
        }
    }

    #[test]
    fn round_trip_and_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let mut prefs = SessionPrefs::default();
        remember(
            &mut prefs,
            ResearchRef {
                root: "root".into(),
                store: None,
            },
        );
        prefs.views.insert(
            "id".into(),
            ViewState {
                tab: "evidence".into(),
                ..Default::default()
            },
        );
        save(&home, &prefs).unwrap();
        save(&home, &prefs).unwrap();
        assert_eq!(load(&home), prefs);
        assert_eq!(fs::read_dir(home).unwrap().count(), 1);
    }

    #[test]
    fn old_json_missing_fields_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("session.json"),
            r#"{"last":{"root":"old"},"views":{"id":{"tabs":{"map":{}}}}}"#,
        )
        .unwrap();
        let p = load(dir.path());
        assert_eq!(p.last.unwrap().root, std::path::PathBuf::from("old"));
        assert_eq!(p.views["id"].tabs["map"], TabView::default());
    }
}
