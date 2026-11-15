use super::{
    services::{DecisionReply, DecisionRequest, DecisionRouter, ModelBackend, ModelCall},
    Provider,
};
use crate::{
    base::{Failure, Res},
    laya::router,
    usage::extensions::{self, ExtRecord},
};
use serde::Deserialize;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct BackendRouter {
    pub backend: Arc<dyn ModelBackend>,
    pub model: String,
    pub engine_home: PathBuf,
    pub laya_home: PathBuf,
    pub cwd: PathBuf,
    pub timeout_s: u64,
}

#[derive(Deserialize)]
struct Answer {
    choice: String,
    reason: String,
}

impl DecisionRouter for BackendRouter {
    fn route(&self, module: &str, request: &DecisionRequest) -> Res<DecisionReply> {
        let context = format!(
            "Question: {}\nOptions: {}",
            request.question,
            serde_json::to_string(&request.options).unwrap()
        );
        let call = ModelCall {
            provider: Provider::Claude,
            model: self.model.clone(),
            prompt: format!("{context}\nChoose exactly one option. Reply with a JSON object containing string fields \"choice\" and \"reason\"."),
            cwd: self.cwd.clone(),
            timeout_s: self.timeout_s,
        };
        let reply = self.backend.call(&call, &AtomicBool::new(false))?;
        let record = ExtRecord {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(i64::MAX as u128) as i64,
            module: Some(module.into()),
            provider: "claude".into(),
            model: Some(self.model.clone()),
            input_tokens: reply.input_tokens,
            cached_input_tokens: reply.cached_input_tokens,
            output_tokens: reply.output_tokens,
            usd: reply.usd,
            ..Default::default()
        };
        extensions::append(&self.engine_home, &record)
            .map_err(|e| Failure::new("failed", e.message))?;

        let answer = reply
            .text
            .char_indices()
            .filter(|(_, c)| *c == '{')
            .find_map(|(i, _)| {
                let mut stream =
                    serde_json::Deserializer::from_str(&reply.text[i..]).into_iter::<Answer>();
                stream.next().and_then(Result::ok)
            })
            .ok_or_else(|| Failure::new("invalid_response", "expected a JSON choice and reason"))?;
        if !request.options.contains(&answer.choice) {
            return Err(Failure::new(
                "invalid_response",
                "choice is not an available option",
            ));
        }
        let subject = format!("workshop:{module}");
        router::observe_routing(
            Some(&self.laya_home),
            None,
            Some(&subject),
            &answer.choice,
            &context,
        );
        Ok(DecisionReply {
            choice: answer.choice,
            reason: answer.reason,
        })
    }
}
