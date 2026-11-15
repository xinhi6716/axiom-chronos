//! Choice inference. Sequence construction, batching, and softmax adapted from
//! laya-rust v0.4.31 (sequence_builder.rs, collator.rs, calibration.rs), Apache-2.0.
use crate::{
    base::{Failure, Res},
    laya::{
        ort::{self, Runtime, Session, Tensor},
        tokenizer::Tokenizer,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct LoadSpec {
    pub model_dir: PathBuf,
    pub calibration: Option<PathBuf>,
    pub runtime: PathBuf,
    pub intra_threads: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TypeCalibration {
    pub labels: Vec<String>,
    pub temperature: f64,
    pub abstain_below: f64,
    pub fitted_on: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Calibration {
    pub format: u32,
    pub model_version: String,
    pub types: BTreeMap<String, TypeCalibration>,
}
pub fn load_calibration(path: &Path) -> Res<Calibration> {
    let invalid = |e: String| {
        Failure::new(
            "laya_calibration_invalid",
            format!("{}: {e}", path.display()),
        )
    };
    let data = fs::read(path).map_err(|e| invalid(e.to_string()))?;
    let cal: Calibration = serde_json::from_slice(&data).map_err(|e| invalid(e.to_string()))?;
    if cal.format != 1 {
        return Err(invalid("unsupported format".into()));
    }
    for (name, t) in &cal.types {
        if !t.temperature.is_finite()
            || t.temperature <= 0.0
            || t.temperature > 100.0
            || !t.abstain_below.is_finite()
            || !(0.0..=1.0).contains(&t.abstain_below)
            || t.fitted_on != "dev"
        {
            return Err(invalid(format!("invalid calibration for {name}")));
        }
    }
    Ok(cal)
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Abstain {
    Answer,
    Abstain,
    Uncalibrated { reason: String },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Prediction {
    pub labels: Vec<String>,
    pub probs: Vec<f64>,
    pub top: usize,
    pub confidence: f64,
    pub tokens: u64,
    pub abstain: Abstain,
}
pub trait Classifier: Send + Sync {
    fn model_version(&self) -> String;
    fn predict(
        &self,
        decision_type: &str,
        instructions: &str,
        labels: &[String],
        state: &str,
    ) -> Res<Prediction>;
}
pub struct FakeClassifier {
    pub version: String,
    pub table: BTreeMap<String, Prediction>,
}
impl Classifier for FakeClassifier {
    fn model_version(&self) -> String {
        self.version.clone()
    }
    fn predict(&self, decision_type: &str, _: &str, _: &[String], _: &str) -> Res<Prediction> {
        self.table.get(decision_type).cloned().ok_or_else(|| {
            Failure::new(
                "invalid_argument",
                format!("unknown decision type: {decision_type}"),
            )
        })
    }
}

#[derive(Deserialize)]
struct Config {
    max_len: usize,
    head_max_len: usize,
    temperature: Vec<f64>,
}
enum Model {
    Fused(Session),
    Split { encoder: Session, head: Session },
}
pub struct Laya {
    version: String,
    tokenizer: Tokenizer,
    config: Config,
    calibration: Option<Calibration>,
    model: Model,
}
impl Laya {
    pub fn load(spec: &LoadSpec) -> Res<Self> {
        let manifest = ort::verify_dir(&spec.model_dir)?;
        let config_path = spec.model_dir.join("rl_agent_config.json");
        let invalid_config = |e: String| {
            Failure::new(
                "laya_config_invalid",
                format!("{}: {e}", config_path.display()),
            )
        };
        let config: Config = serde_json::from_slice(
            &fs::read(&config_path).map_err(|e| invalid_config(e.to_string()))?,
        )
        .map_err(|e| invalid_config(e.to_string()))?;
        if config.max_len == 0
            || config.temperature.len() < 3
            || config
                .temperature
                .iter()
                .any(|t| !t.is_finite() || *t <= 0.0)
        {
            return Err(invalid_config("invalid lengths or temperature".into()));
        }
        let nested = spec.model_dir.join("tokenizer/tokenizer.json");
        let tokenizer_path = if nested.is_file() {
            nested
        } else {
            spec.model_dir.join("tokenizer.json")
        };
        let tokenizer = Tokenizer::from_file(&tokenizer_path)?;
        let default_cal = spec.model_dir.join("calibration.json");
        let cal_path = spec
            .calibration
            .as_deref()
            .or_else(|| default_cal.is_file().then_some(default_cal.as_path()));
        let calibration = cal_path.map(load_calibration).transpose()?;
        if calibration
            .as_ref()
            .is_some_and(|c| c.model_version != manifest.model_version)
        {
            return Err(Failure::new(
                "laya_version_mismatch",
                "calibration and model versions differ",
            ));
        }
        let rt = Runtime::load(&spec.runtime)?;
        let fused = spec.model_dir.join("model.onnx");
        let model = if fused.is_file() {
            Model::Fused(Session::new(&rt, &fused, spec.intra_threads)?)
        } else {
            let encoder = spec.model_dir.join("encoder.onnx");
            let head = spec.model_dir.join("head.onnx");
            if !encoder.is_file() || !head.is_file() {
                return Err(Failure::new(
                    "laya_artifact_missing",
                    "missing fused model or split encoder/head",
                ));
            }
            Model::Split {
                encoder: Session::new(&rt, &encoder, spec.intra_threads)?,
                head: Session::new(&rt, &head, spec.intra_threads)?,
            }
        };
        Ok(Self {
            version: manifest.model_version,
            tokenizer,
            config,
            calibration,
            model,
        })
    }
}

// Upstream builds choice fragments from label text, capping each at 48 tokens.
fn sequence(
    tok: &Tokenizer,
    cfg: &Config,
    instructions: &str,
    labels: &[String],
    state: &str,
) -> Res<(Vec<i64>, Vec<i64>)> {
    let s = tok.special();
    let mask = if tok.token_to_id("[MASK]") == Some(s.mask) {
        "[MASK]"
    } else {
        "<mask>"
    };
    let scrub = |text: &str| text.replace(mask, " ");
    let mut header = tok.encode(&format!("choice question: {}", scrub(instructions)))?;
    let mut options = Vec::with_capacity(labels.len());
    for label in labels {
        let mut ids = tok.encode(&format!(" {}", scrub(label)))?;
        ids.truncate(48);
        ids.insert(0, s.mask);
        options.push(ids);
    }
    let bound = i64::try_from(cfg.head_max_len).unwrap_or(i64::MAX);
    let option_sum = |o: &[Vec<u32>]| {
        o.iter().fold(0i64, |n, v| {
            n.saturating_add(i64::try_from(v.len()).unwrap_or(i64::MAX))
        })
    };
    let mut budget = bound.saturating_sub(option_sum(&options));
    if budget < 16 {
        let per = 4.max(
            bound
                .saturating_sub(16)
                .div_euclid(i64::try_from(options.len()).unwrap_or(i64::MAX).max(1)),
        ) as usize;
        for o in &mut options {
            o.truncate(per);
        }
        budget = bound.saturating_sub(option_sum(&options));
    }
    header.truncate(usize::try_from(budget.max(8)).unwrap_or(usize::MAX));
    let mut ids = vec![s.cls];
    ids.extend(header);
    ids.push(s.sep);
    let mut markers = Vec::with_capacity(options.len());
    for o in options {
        markers.push(ids.len());
        ids.extend(o);
    }
    ids.push(s.sep);
    let room = cfg.max_len.saturating_sub(ids.len().saturating_add(1));
    let mut state_ids = tok.encode(&scrub(state))?;
    state_ids.truncate(room);
    ids.extend(state_ids);
    ids.push(s.sep);
    ids.truncate(cfg.max_len);
    if markers
        .iter()
        .any(|&m| m.checked_add(1).is_none_or(|n| n >= ids.len()))
    {
        return Err(Failure::new(
            "invalid_argument",
            "sequence budget truncates choice markers",
        ));
    }
    Ok((
        ids.into_iter().map(i64::from).collect(),
        markers.into_iter().map(|x| x as i64).collect(),
    ))
}

fn run(
    session: &Session,
    ids: &[i64],
    markers: &[i64],
    hidden: Option<Tensor>,
    outputs: &[&str],
) -> Res<Vec<Tensor>> {
    let mut inputs = Vec::new();
    for info in session.inputs() {
        let tensor = match info.name.as_str() {
            "input_ids" => Tensor::I64 {
                shape: vec![1, ids.len() as i64],
                data: ids.to_vec(),
            },
            "attention_mask" => Tensor::I64 {
                shape: vec![1, ids.len() as i64],
                data: vec![1; ids.len()],
            },
            "marker_pos" => Tensor::I64 {
                shape: vec![1, markers.len() as i64],
                data: markers.to_vec(),
            },
            "marker_mask" => Tensor::Bool {
                shape: vec![1, markers.len() as i64],
                data: vec![true; markers.len()],
            },
            "qtype" => Tensor::I64 {
                shape: match info.dims.len() {
                    1 => vec![1],
                    2 => vec![1, 1],
                    _ => return Err(Failure::new("laya_ort", "unsupported qtype rank")),
                },
                data: vec![0],
            },
            "hidden_states" => hidden
                .clone()
                .ok_or_else(|| Failure::new("laya_ort", "missing hidden states"))?,
            name => return Err(Failure::new("laya_ort", format!("unknown input {name}"))),
        };
        inputs.push((info.name.as_str(), tensor));
    }
    session.run(&inputs, outputs)
}
fn probabilities(logits: &[f32], temperature: f64) -> Res<Vec<f64>> {
    if logits.iter().any(|x| !x.is_finite()) {
        return Err(Failure::new("laya_nonfinite", "non-finite logits"));
    }
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mut p: Vec<f64> = logits
        .iter()
        .map(|&x| ((f64::from(x) - max) / temperature).exp())
        .collect();
    let sum: f64 = p.iter().sum();
    for x in &mut p {
        *x /= sum;
    }
    Ok(p)
}
impl Classifier for Laya {
    fn model_version(&self) -> String {
        self.version.clone()
    }
    fn predict(
        &self,
        decision_type: &str,
        instructions: &str,
        labels: &[String],
        state: &str,
    ) -> Res<Prediction> {
        if labels.len() < 2 {
            return Err(Failure::new(
                "invalid_argument",
                "at least two labels are required",
            ));
        }
        let (ids, markers) = sequence(&self.tokenizer, &self.config, instructions, labels, state)?;
        let output = match &self.model {
            Model::Fused(s) => run(s, &ids, &markers, None, &["logits"])?,
            Model::Split { encoder, head } => {
                let hidden = run(encoder, &ids, &markers, None, &["last_hidden_state"])?.remove(0);
                run(head, &ids, &markers, Some(hidden), &["logits"])?
            }
        };
        let logits = match &output[0] {
            Tensor::F32 { shape, data }
                if shape.as_slice() == [1, labels.len() as i64] && data.len() == labels.len() =>
            {
                data
            }
            _ => return Err(Failure::new("laya_ort", "invalid logits shape or type")),
        };
        let matched = self
            .calibration
            .as_ref()
            .and_then(|c| c.types.get(decision_type))
            .filter(|t| t.labels == labels);
        let probs = probabilities(logits, matched.map_or(1.0, |t| t.temperature))?;
        let mut top = 0;
        for i in 1..probs.len() {
            if probs[i] > probs[top] {
                top = i;
            }
        }
        let confidence = probs[top];
        let abstain = match matched {
            Some(c) if confidence < c.abstain_below => Abstain::Abstain,
            Some(_) => Abstain::Answer,
            None => Abstain::Uncalibrated {
                reason: if self.calibration.is_none() {
                    "no calibration"
                } else if self
                    .calibration
                    .as_ref()
                    .unwrap()
                    .types
                    .contains_key(decision_type)
                {
                    "labels differ"
                } else {
                    "missing type"
                }
                .into(),
            },
        };
        Ok(Prediction {
            labels: labels.to_vec(),
            probs,
            top,
            confidence,
            tokens: ids.len() as u64,
            abstain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonfinite_logits_are_rejected() {
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                probabilities(&[0.0, x], 1.0).unwrap_err().kind,
                "laya_nonfinite"
            );
        }
    }
}
