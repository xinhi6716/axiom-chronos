#![allow(dead_code)]
use crate::release_support::Matrix;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path},
};

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{capture, dpi, launch};

pub struct Png {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Png {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, self.width, self.height);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .map_err(|e| e.to_string())?
                .write_image_data(&self.rgba)
                .map_err(|e| e.to_string())?;
        }
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut d = png::Decoder::new(bytes);
        d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = d.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let b = &buf[..info.buffer_size()];
        let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(b),
            png::ColorType::Rgb => {
                for p in b.chunks_exact(3) {
                    rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
            png::ColorType::Grayscale => {
                for &v in b {
                    rgba.extend_from_slice(&[v, v, v, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for p in b.chunks_exact(2) {
                    rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
                }
            }
            _ => return Err("unsupported PNG color type".into()),
        }
        Ok(Self {
            width: info.width,
            height: info.height,
            rgba,
        })
    }
    pub fn nonblank(&self) -> bool {
        if self.width == 0 || self.height == 0 || self.rgba.len() < 4 {
            return false;
        }
        let first = &self.rgba[..3];
        self.rgba
            .chunks_exact(4)
            .filter(|p| (0..3).any(|i| p[i].abs_diff(first[i]) > 8))
            .count()
            * 100
            > (self.width as usize * self.height as usize)
    }
    pub fn diff(&self, other: &Self) -> f64 {
        if self.width != other.width || self.height != other.height {
            return 1.0;
        }
        let count = self
            .rgba
            .chunks_exact(4)
            .zip(other.rgba.chunks_exact(4))
            .filter(|(a, b)| (0..4).any(|i| a[i].abs_diff(b[i]) > 8))
            .count();
        count as f64 / (self.width as f64 * self.height as f64)
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn pilot_view(id: &str) -> bool {
    matches!(id, "mini" | "focus.graph" | "dialog.composer" | "run")
}
pub fn matrix_cases(matrix: &Matrix, pilot: bool) -> Vec<(String, String, String, String, String)> {
    let mut result = Vec::new();
    for v in &matrix.views {
        if pilot {
            if pilot_view(&v.id) {
                result.push((
                    v.id.clone(),
                    "live".into(),
                    "zh-TW".into(),
                    "dark".into(),
                    "1100x700".into(),
                ));
            }
            continue;
        }
        for state in v.states.as_array().expect("states array") {
            for lang in matrix.capture["langs"].as_array().expect("langs array") {
                for theme in matrix.capture["themes"].as_array().expect("themes array") {
                    for size in matrix.capture["sizes"].as_array().expect("sizes array") {
                        result.push((
                            v.id.clone(),
                            state.as_str().unwrap().into(),
                            lang.as_str().unwrap().into(),
                            theme.as_str().unwrap().into(),
                            size.as_str().unwrap().into(),
                        ));
                    }
                }
            }
        }
    }
    result
}
pub fn size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("invalid size")?;
    Ok((
        w.parse().map_err(|_| "invalid width")?,
        h.parse().map_err(|_| "invalid height")?,
    ))
}
fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {k}"))
}
fn number(v: &Value, k: &str) -> Result<u32, String> {
    v.get(k)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| format!("invalid {k}"))
}
fn key(e: &Value) -> Result<(String, String, String, String, String), String> {
    Ok((
        string(e, "view")?.into(),
        string(e, "state")?.into(),
        string(e, "lang")?.into(),
        string(e, "theme")?.into(),
        string(e, "size")?.into(),
    ))
}
fn read_entry(root: &Path, e: &Value, phase: &str) -> Result<Png, String> {
    let file = Path::new(string(e, "file")?);
    if file
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || !file.starts_with(Path::new("files").join(phase))
    {
        return Err("invalid file path".into());
    }
    let bytes = fs::read(root.join(file)).map_err(|e| e.to_string())?;
    if hash(&bytes) != string(e, "sha256")? {
        return Err("hash mismatch".into());
    }
    let png = Png::decode(&bytes)?;
    let (w, h) = (number(e, "width")?, number(e, "height")?);
    if (png.width, png.height) != (w, h) {
        return Err("recorded size mismatch".into());
    }
    let view = string(e, "view")?;
    let dpi = number(e, "dpi")?;
    let (rw, rh) = size(string(e, "size")?)?;
    if !matches!(view, "mini" | "focus.graph" | "focus.checklist")
        && ((u64::from(rw) * u64::from(dpi) + 48) / 96 != u64::from(w)
            || (u64::from(rh) * u64::from(dpi) + 48) / 96 != u64::from(h))
    {
        return Err("requested size mismatch".into());
    }
    string(e, "source_id")?;
    string(e, "commit")?;
    string(e, "scenario")?;
    if !png.nonblank() {
        return Err("blank or near-uniform PNG".into());
    }
    Ok(png)
}
pub fn validate(
    root: &Path,
    manifest: &Value,
    expected: &[(String, String, String, String, String)],
    source: &str,
) -> Result<(), String> {
    if !root.join("files/before").is_dir() || !root.join("files/after").is_dir() {
        return Err("missing PNG folder".into());
    }
    let current = manifest["current"].as_array().ok_or("missing current")?;
    let baseline = manifest["baseline"].as_array().ok_or("missing baseline")?;
    let approvals = manifest["approvals"]
        .as_array()
        .ok_or("missing approvals")?;
    let mut bases = HashMap::new();
    for e in baseline {
        if bases.insert(key(e)?, e).is_some() {
            return Err("duplicate baseline case".into());
        }
    }
    let mut seen = HashSet::new();
    let mut hashes: HashMap<String, String> = HashMap::new();
    for e in current {
        let k = key(e)?;
        if !seen.insert(k.clone()) {
            return Err("duplicate current case".into());
        }
        if string(e, "source_id")? != source || source.is_empty() {
            return Err("old source_id".into());
        }
        let image = read_entry(root, e, "after")?;
        let digest = string(e, "sha256")?.to_owned();
        if hashes.get(&digest).is_some_and(|v| v != &k.0) {
            return Err("duplicate hash across targets".into());
        }
        hashes.insert(digest, k.0.clone());
        let base = read_entry(
            root,
            bases.get(&k).ok_or("missing baseline case")?,
            "before",
        )?;
        let threshold = manifest["thresholds"]
            .get(&k.0)
            .and_then(Value::as_f64)
            .filter(|v| (0.0..=1.0).contains(v))
            .ok_or("invalid threshold")?;
        let approved = approvals.iter().any(|a| {
            a["view"].as_str() == Some(&k.0)
                && a["source_id"].as_str() == Some(source)
                && a["reason"].as_str().is_some_and(|s| !s.trim().is_empty())
                && a["commit"].as_str().is_some_and(|s| !s.trim().is_empty())
        });
        if image.diff(&base) > threshold && !approved {
            return Err("diff exceeds threshold".into());
        }
    }
    let expected: HashSet<_> = expected.iter().cloned().collect();
    if seen != expected {
        return Err("current matrix incomplete or has extra cases".into());
    }
    Ok(())
}
