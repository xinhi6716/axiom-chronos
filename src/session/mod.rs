pub(crate) mod authority;
pub(crate) mod host;
pub mod render;
pub mod run;
mod tools;
pub use render::*;
pub use run::*;

/// Attach the external caller as the root; no root model thread is started.
pub struct Attachment {
    host: std::sync::Arc<host::Host>,
    id: String,
}
impl Attachment {
    pub fn open(
        config: &Config,
        args: &serde_json::Value,
    ) -> crate::base::Res<(Self, serde_json::Value)> {
        use crate::{
            base::{self, Failure},
            goal,
        };
        use serde_json::json;
        let resume = args["session"].as_str();
        let existing = resume
            .map(|id| goal::load(&config.home, id))
            .transpose()?
            .flatten();
        if resume.is_some() && existing.is_none() {
            return Err(Failure::new("not_found", "session not found"));
        }
        let cwd = existing
            .as_ref()
            .and_then(|g| g.cwd.as_deref())
            .or_else(|| args["project"].as_str())
            .ok_or_else(|| Failure::new("invalid_argument", "project is required"))?;
        let project = base::require_directory(cwd)?;
        let skill = if existing
            .as_ref()
            .is_some_and(|g| g.other.get("workflow").and_then(|v| v.as_str()) == Some("research"))
        {
            "research"
        } else {
            args["skill"].as_str().unwrap_or("wish")
        };
        if !matches!(skill, "wish" | "research") {
            return Err(Failure::new("invalid_argument", "invalid skill"));
        }
        let text = existing
            .as_ref()
            .map(|g| g.wish.as_str())
            .or_else(|| args["text"].as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Failure::new("invalid_argument", "text is required"))?;
        let id = resume.map(str::to_owned).unwrap_or_else(base::unique);
        let logs = config.home.join("logs");
        std::fs::create_dir_all(&logs).map_err(|e| Failure::new("io", e.to_string()))?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join(format!("caller-{id}.jsonl")))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        let host = host::Host::new(
            config,
            &project,
            log,
            args["max_turns"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok()),
        )?;
        let workflow = host.workflow(skill, text)?;
        if let Some(g) = &existing {
            if g.status != "active" && args["preserve_goal"] != true {
                goal::reopen(&config.home, &id)?;
            }
        } else if skill == "research" {
            goal::open_research(&config.home, &id, text, Some(cwd))?;
        } else {
            goal::open(&config.home, &id, text, Some(cwd))?;
        }
        *host.root.lock().unwrap() = id.clone();
        let result = json!({"status":"completed","session":id,"skill":skill,"workflow":workflow,"tools":host::specs("core"),"max_turns":args["max_turns"],"goal":goal::load(&config.home, &id)?});
        Ok((Self { host, id }, result))
    }
    pub fn call(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> crate::base::Res<serde_json::Value> {
        // One in-flight core command per durable session, across connections.
        // Hold this OS lock while dispatcher owns its subtree. Cancellation is
        // still available through session stop/interject and an explicit stop.
        if matches!(name,"goal" | "chronos_goal") && args["classify"]=="stop" {
            return self.host.call("core", &self.id, name, args, &[]);
        }
        let gate=self.host.home.join("authority").join(&self.id).join("caller-command");
        std::fs::create_dir_all(&gate).map_err(|e|crate::base::Failure::new("io",e.to_string()))?;
        crate::orch::with_lock(&gate,false,||self.call_locked(name,args))
    }
    fn call_locked(&self, name: &str, args: &serde_json::Value) -> crate::base::Res<serde_json::Value> {
        if name == "Agent" && args["subagent_type"] == "chronos:dispatcher" {
            authority::update(&self.host.home,&self.id,authority::recover_abandoned_stage)?;
        }
        self.host.call("core", &self.id, name, args, &[])
    }
    /// Durable, idempotent CLI transport. The caller supplies an immutable request
    /// id; replay returns the recorded response without launching another model.
    pub fn call_recorded(&self, request: &str, name: &str, args: &serde_json::Value) -> crate::base::Res<serde_json::Value> {
        use crate::base::{self,Failure};
        use serde_json::{json,Value};
        base::require_slug("request",request)?;
        let dir=self.host.home.join("authority").join(&self.id);
        let gate=dir.join("caller-command");
        std::fs::create_dir_all(&gate).map_err(|e|Failure::new("io",e.to_string()))?;
        crate::orch::with_lock(&gate,false,|| {
            let path=dir.join(format!("request-{request}.json"));
            let input=json!({"tool":name,"arguments":args});
            match std::fs::read(&path) {
                Ok(bytes)=> {
                    let record:Value=serde_json::from_slice(&bytes).map_err(|e|Failure::new("io",e.to_string()))?;
                    if record["input"]!=input { return Err(Failure::new("request_conflict","request id reused with different input")); }
                    if !record["response"].is_null() { return Ok(record["response"].clone()); }
                    let flow=authority::read(&self.host.home,&self.id)?;
                    if name=="Agent" && args["subagent_type"]=="chronos:dispatcher"
                        && matches!(flow.phase,authority::Phase::IntentReview | authority::Phase::ReadySubmit | authority::Phase::Submitted) {
                        let response=json!({"status":"completed","outcome":"awaiting_core","workflow":flow,"replayed":true});
                        base::atomic_write(&path,&serde_json::to_vec_pretty(&json!({"input":input,"response":response})).map_err(|e|Failure::new("io",e.to_string()))?).map_err(|e|Failure::new("io",e.to_string()))?;
                        return Ok(response);
                    }
                    // Only abandoned pre-engineering stages are safely replayable.
                    if !matches!(flow.phase,authority::Phase::ResearchRunning | authority::Phase::PlanningRunning) {
                        return Err(Failure::new("recovery_required","request outcome is unknown; engineering replay refused"));
                    }
                    authority::update(&self.host.home,&self.id,authority::recover_abandoned_stage)?;
                },
                Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
                Err(e)=>return Err(Failure::new("io",e.to_string())),
            }
            base::atomic_write(&path,&serde_json::to_vec_pretty(&json!({"input":input,"response":null})).map_err(|e|Failure::new("io",e.to_string()))?).map_err(|e|Failure::new("io",e.to_string()))?;
            let response=match self.call_locked(name,args) {
                Ok(v)=>v,
                Err(e)=>json!({"status":"bridge_error","error":{"kind":e.kind,"message":e.message}}),
            };
            let record=json!({"input":input,"response":response});
            base::atomic_write(&path,&serde_json::to_vec_pretty(&record).map_err(|e|Failure::new("io",e.to_string()))?).map_err(|e|Failure::new("io",e.to_string()))?;
            Ok(response)
        })
    }
    pub fn status(&self) -> crate::base::Res<serde_json::Value> {
        let gate=self.host.home.join("authority").join(&self.id).join("caller-command");
        std::fs::create_dir_all(&gate).map_err(|e|crate::base::Failure::new("io",e.to_string()))?;
        crate::orch::with_lock(&gate,false,|| {
        let goal = crate::goal::load(&self.host.home, &self.id)?;
        let notices = self.host.pending_core_notices()?;
        Ok(
            serde_json::json!({"status":"completed","session":self.id,"goal":goal,"core_notices":notices}),
        )
        })
    }
    pub fn interject(&self, text: &str) -> crate::base::Res<serde_json::Value> {
        if text.trim().is_empty() {
            return Err(crate::base::Failure::new(
                "invalid_argument",
                "interjection text is required",
            ));
        }
        let _lock = self.host.goal_lock.lock().unwrap();
        Ok(
            serde_json::json!({"status":"completed", "goal":crate::goal::interject(&self.host.home, &self.id, text)?}),
        )
    }
    pub fn stop(&self) {
        self.host.shutdown();
    }
}
impl Drop for Attachment {
    fn drop(&mut self) {
        self.stop();
    }
}
