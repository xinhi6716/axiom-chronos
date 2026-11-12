use super::presets::Preset;
use crate::base::{Failure, Res};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub trait TextMeasure: Send + Sync {
    fn measure(&self, text: &str, family: &str, pt: f64) -> Res<(f64, f64)>;
    fn covers(&self, text: &str, family: &str) -> Res<Vec<char>>;
}
#[derive(Debug, Default)]
struct FontCache {
    files: Option<Vec<PathBuf>>,
    families: HashMap<String, Arc<Vec<Vec<u8>>>>,
}
#[derive(Debug, Clone)]
pub struct FontMeasure {
    _preset: Preset,
    _dirs: Vec<PathBuf>,
    cache: Arc<Mutex<FontCache>>,
}
impl PartialEq for FontMeasure {
    fn eq(&self, other: &Self) -> bool {
        self._preset == other._preset && self._dirs == other._dirs
    }
}
impl FontMeasure {
    pub fn new(preset: &Preset) -> Self {
        let mut dirs = Vec::new();
        #[cfg(target_os = "windows")]
        {
            dirs.push(PathBuf::from("C:/Windows/Fonts"));
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
            }
        }
        #[cfg(target_os = "linux")]
        {
            dirs.extend([
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
            ]);
            if let Some(home) = std::env::var_os("HOME") {
                let home = PathBuf::from(home);
                dirs.push(home.join(".fonts"));
                dirs.push(home.join(".local/share/fonts"));
            }
        }
        #[cfg(target_os = "macos")]
        {
            dirs.extend([
                PathBuf::from("/Library/Fonts"),
                PathBuf::from("/System/Library/Fonts"),
            ]);
            if let Some(home) = std::env::var_os("HOME") {
                dirs.push(PathBuf::from(home).join("Library/Fonts"));
            }
        }
        if let Some(extra) = std::env::var_os("AXIOM_FONT_DIRS") {
            dirs.extend(std::env::split_paths(&extra));
        }
        Self::with_dirs(preset, dirs)
    }
    pub fn with_dirs(preset: &Preset, dirs: Vec<PathBuf>) -> Self {
        Self {
            _preset: preset.clone(),
            _dirs: dirs,
            cache: Arc::new(Mutex::new(FontCache::default())),
        }
    }

    fn fonts(&self, family: &str) -> Res<Arc<Vec<Vec<u8>>>> {
        let names = if family == self._preset.font_family {
            &self._preset.font_files
        } else if self._preset.cjk_font_family.as_deref() == Some(family) {
            &self._preset.cjk_font_files
        } else {
            return Err(Failure::new(
                "font_missing",
                format!("unknown font family: {family}"),
            ));
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(fonts) = cache.families.get(family) {
            return if fonts.is_empty() {
                Err(Failure::new(
                    "font_missing",
                    format!("no usable font for family: {family}"),
                ))
            } else {
                Ok(Arc::clone(fonts))
            };
        }
        if names.iter().any(|name| !Path::new(name).is_absolute()) {
            cache.files.get_or_insert_with(|| {
                let mut files = Vec::new();
                for dir in &self._dirs {
                    collect_files(dir, &mut files);
                }
                files
            });
        }
        let mut fonts = Vec::new();
        for name in names {
            let path = Path::new(name);
            let candidates: Vec<&Path> = if path.is_absolute() {
                vec![path]
            } else if path.components().count() == 1 {
                cache
                    .files
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|file| {
                        file.file_name().is_some_and(|filename| {
                            filename.to_string_lossy().to_lowercase() == name.to_lowercase()
                        })
                    })
                    .map(PathBuf::as_path)
                    .collect()
            } else {
                Vec::new()
            };
            for candidate in candidates {
                if let Ok(bytes) = std::fs::read(candidate) {
                    if ttf_parser::Face::parse(&bytes, 0).is_ok() {
                        fonts.push(bytes);
                    }
                }
            }
        }
        let fonts = Arc::new(fonts);
        cache.families.insert(family.to_owned(), Arc::clone(&fonts));
        if fonts.is_empty() {
            Err(Failure::new(
                "font_missing",
                format!("no usable font for family: {family}"),
            ))
        } else {
            Ok(fonts)
        }
    }
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if let Ok(kind) = entry.file_type() {
            if kind.is_dir() {
                collect_files(&entry.path(), files);
            } else if kind.is_file() || kind.is_symlink() && entry.path().is_file() {
                files.push(entry.path());
            }
        }
    }
}
impl TextMeasure for FontMeasure {
    fn measure(&self, text: &str, family: &str, pt: f64) -> Res<(f64, f64)> {
        let fonts = self.fonts(family)?;
        let face = ttf_parser::Face::parse(&fonts[0], 0).expect("font was validated");
        let advances: f64 = text
            .chars()
            .map(|ch| {
                let glyph = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                f64::from(face.glyph_hor_advance(glyph).unwrap_or(0))
            })
            .sum();
        let scale = pt * 25.4 / 72.0 / f64::from(face.units_per_em());
        Ok((
            advances * scale,
            (f64::from(face.ascender()) - f64::from(face.descender())) * scale,
        ))
    }
    fn covers(&self, text: &str, family: &str) -> Res<Vec<char>> {
        let fonts = self.fonts(family)?;
        let faces: Vec<_> = fonts
            .iter()
            .map(|bytes| ttf_parser::Face::parse(bytes, 0).expect("font was validated"))
            .collect();
        Ok(text
            .chars()
            .filter(|ch| {
                !ch.is_whitespace()
                    && !ch.is_control()
                    && faces.iter().all(|face| face.glyph_index(*ch).is_none())
            })
            .collect())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct FixedMeasure {
    pub em_ratio: f64,
}
impl TextMeasure for FixedMeasure {
    fn measure(&self, text: &str, _family: &str, pt: f64) -> Res<(f64, f64)> {
        Ok((
            text.chars().count() as f64 * pt * self.em_ratio * 25.4 / 72.0,
            pt * 25.4 / 72.0,
        ))
    }
    fn covers(&self, _text: &str, _family: &str) -> Res<Vec<char>> {
        Ok(vec![])
    }
}
