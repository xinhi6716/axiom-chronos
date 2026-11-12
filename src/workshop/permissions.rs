use super::manifest::{data_root, Manifest};
use crate::base::{Failure, Res};
use crate::seams::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    Claude,
    Codex,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    Files,
    Memory,
    Model { provider: Provider, model: String },
    Subprocess,
    Network,
    Env(String),
    DecisionRouter,
}

impl Permission {
    pub fn parse(value: &str) -> Res<Self> {
        let bad = || Failure::new("invalid_permission", format!("invalid permission: {value}"));
        match value {
            "files" => Ok(Self::Files),
            "memory" => Ok(Self::Memory),
            "subprocess" => Ok(Self::Subprocess),
            "network" => Ok(Self::Network),
            "decision_router" => Ok(Self::DecisionRouter),
            _ => {
                if let Some(name) = value.strip_prefix("env:") {
                    let mut bytes = name.bytes();
                    if matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
                        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        return Ok(Self::Env(name.into()));
                    }
                }
                if let Some(rest) = value.strip_prefix("model:") {
                    if let Some((provider, model)) = rest.split_once(':') {
                        let provider = match provider {
                            "claude" => Some(Provider::Claude),
                            "codex" => Some(Provider::Codex),
                            _ => None,
                        };
                        if let Some(provider) = provider {
                            if !model.is_empty()
                                && !model
                                    .chars()
                                    .any(|c| c.is_whitespace() || c.is_control() || c == ':')
                                && (model == "*" || !model.contains('*'))
                            {
                                return Ok(Self::Model {
                                    provider,
                                    model: model.into(),
                                });
                            }
                        }
                    }
                }
                Err(bad())
            }
        }
    }
    pub fn as_string(&self) -> String {
        match self {
            Self::Files => "files".into(),
            Self::Memory => "memory".into(),
            Self::Subprocess => "subprocess".into(),
            Self::Network => "network".into(),
            Self::DecisionRouter => "decision_router".into(),
            Self::Env(name) => format!("env:{name}"),
            Self::Model { provider, model } => format!(
                "model:{}:{model}",
                match provider {
                    Provider::Claude => "claude",
                    Provider::Codex => "codex",
                }
            ),
        }
    }
    pub fn is_high_risk(&self) -> bool {
        matches!(self, Self::Subprocess | Self::Network | Self::Env(_))
    }
}

#[derive(Clone, Debug)]
pub struct Grants {
    permissions: BTreeSet<Permission>,
    max_parallel: u32,
}
impl Grants {
    pub fn from_manifest(manifest: &Manifest) -> Res<Self> {
        Ok(Self {
            permissions: manifest
                .permissions
                .iter()
                .map(|s| Permission::parse(s))
                .collect::<Res<_>>()?,
            max_parallel: manifest.max_parallel,
        })
    }
    pub fn allows(&self, permission: &Permission) -> bool {
        match permission {
            Permission::Model { provider, model } => self.allows_model(provider.clone(), model),
            _ => self.permissions.contains(permission),
        }
    }
    pub fn allows_model(&self, provider: Provider, model: &str) -> bool {
        self.permissions.contains(&Permission::Model {
            provider: provider.clone(),
            model: model.into(),
        }) || self.permissions.contains(&Permission::Model {
            provider,
            model: "*".into(),
        })
    }
    pub fn check(&self, permission: &Permission) -> Res<()> {
        if self.allows(permission) {
            Ok(())
        } else {
            Err(Failure::new(
                "permission_denied",
                format!("permission denied: {}", permission.as_string()),
            ))
        }
    }
    pub fn high_risk(&self) -> Vec<Permission> {
        self.permissions
            .iter()
            .filter(|p| p.is_high_risk())
            .cloned()
            .collect()
    }
    pub fn digest(&self) -> String {
        let mut values: Vec<_> = self.permissions.iter().map(Permission::as_string).collect();
        values.sort();
        sha256_hex(
            serde_json::to_string(&(values, self.max_parallel))
                .expect("serializable permissions")
                .as_bytes(),
        )
    }
    pub fn max_parallel(&self) -> u32 {
        self.max_parallel
    }
}

#[derive(Clone, Debug)]
pub struct DataDirs {
    pub root: PathBuf,
    pub files: PathBuf,
    pub memory: PathBuf,
    pub logs: PathBuf,
    pub receipts: PathBuf,
    pub tmp: PathBuf,
}
impl DataDirs {
    pub fn new(home: &Path, name: &str) -> Self {
        let root = data_root(home, name);
        Self {
            files: root.join("files"),
            memory: root.join("memory"),
            logs: root.join("logs"),
            receipts: root.join("receipts"),
            tmp: root.join("tmp"),
            root,
        }
    }
    pub fn ensure(&self) -> Res<()> {
        for path in [
            &self.root,
            &self.files,
            &self.memory,
            &self.logs,
            &self.receipts,
            &self.tmp,
        ] {
            std::fs::create_dir_all(path).map_err(|e| Failure::new("io", e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
pub const BASE_ENV: &[&str] = &["SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "PATH"];
#[cfg(not(windows))]
pub const BASE_ENV: &[&str] = &["PATH", "LANG"];

pub fn child_env(name: &str, grants: &Grants, dirs: &DataDirs) -> Vec<(String, String)> {
    let mut env = BTreeMap::new();
    for key in BASE_ENV
        .iter()
        .copied()
        .chain(grants.permissions.iter().filter_map(|p| {
            if let Permission::Env(name) = p {
                Some(name.as_str())
            } else {
                None
            }
        }))
    {
        if [
            "HOME",
            "USERPROFILE",
            "TMP",
            "TEMP",
            "TMPDIR",
            "AXIOM_WORKSHOP_MODULE",
            "AXIOM_WORKSHOP_DATA",
            "AXIOM_WORKSHOP_API",
        ]
        .iter()
        .any(|reserved| key == *reserved || (cfg!(windows) && key.eq_ignore_ascii_case(reserved)))
        {
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_owned(), value);
        }
    }
    for (key, value) in [
        ("HOME", dirs.files.to_string_lossy().into_owned()),
        ("USERPROFILE", dirs.files.to_string_lossy().into_owned()),
        ("TMP", dirs.tmp.to_string_lossy().into_owned()),
        ("TEMP", dirs.tmp.to_string_lossy().into_owned()),
        ("TMPDIR", dirs.tmp.to_string_lossy().into_owned()),
        ("AXIOM_WORKSHOP_MODULE", name.into()),
        (
            "AXIOM_WORKSHOP_DATA",
            dirs.files.to_string_lossy().into_owned(),
        ),
        ("AXIOM_WORKSHOP_API", "stdio/1".into()),
    ] {
        env.insert(key.into(), value);
    }
    env.into_iter().collect()
}

pub fn confine(root: &Path, requested: &str) -> Res<PathBuf> {
    let escape = || {
        Failure::new(
            "path_escape",
            format!("path escapes workshop root: {requested}"),
        )
    };
    let bytes = requested.as_bytes();
    if requested.is_empty()
        || requested.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || requested.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(escape());
    }
    let relative = Path::new(requested);
    if relative.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(escape());
    }
    let canonical_root = root.canonicalize().map_err(|_| escape())?;
    let candidate = root.join(relative);
    let mut ancestor = candidate.as_path();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                if !ancestor
                    .canonicalize()
                    .map_err(|_| escape())?
                    .starts_with(&canonical_root)
                {
                    return Err(escape());
                }
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or_else(escape)?
            }
            Err(_) => return Err(escape()),
        }
    }
    Ok(candidate)
}
