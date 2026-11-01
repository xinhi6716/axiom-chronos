//! Host-owned contract repair and installed model probes. No worker gets
//! delegation authority or permission to mutate lifecycle state.
use super::*;
use sha2::{Digest,Sha256};
use crate::session::preflight;
impl Host {
    pub(super) fn replan(&self,caller:&str,args:&Value)->Res<Value> {
        let root=self.root.lock().unwrap().clone();
        let state=authority::read(&self.home,&root)?;
        if caller!="dispatcher" || !matches!(state.phase,authority::Phase::Planning|authority::Phase::Execution) || self.producers.load(Ordering::Acquire)>1 {
            return Err(Failure::new("permission_denied","replan requires dispatcher before engineering; running or unknown work is never reset"));
        }
        let reason=args["reason"].as_str().filter(|s|!s.trim().is_empty() && s.chars().count()<=160).ok_or_else(||invalid("short replan reason required"))?;
        authority::check_research(&state)?;
        let g=goal::load(&self.home,&root)?.ok_or_else(||invalid("missing goal"))?;
        let store=orch::store_for(&self.home,g.project.as_deref().ok_or_else(||invalid("missing project"))?)?;
        orch::with_lock(&store,false,|| {
            if store.join(".worker.claim").exists() || store.join("quarantine").exists()
                || (store.join("units.tsv").exists() && orch::read_units(&store)?.iter().any(|u|u.state!="pending"))
                || std::fs::read_dir(&store).map_err(io)?.filter_map(Result::ok).any(|e|e.file_name().to_string_lossy().ends_with(".evidence.tsv"))
                || !orch::wait::read(&store)?.events.is_empty() {
                return Err(Failure::new("recovery_required","engineering evidence or ownership exists; automatic replan refused"));
            }
            authority::update(&self.home,&root,|s| {
                if s.phase!=state.phase || s.plan_revision>=8 {return Err(invalid("stale or exhausted replan"));}
                // Persist a non-executing phase before changing store identity.
                // A crash between files can only leave planning on the old store.
                s.plan_hash=None;s.phase=authority::Phase::Planning;s.reason=reason.into();s.plan_revision+=1;
                Ok(s.plan_revision)
            }).and_then(|revision| {
                let revised=goal::revise_store(&self.home,&root,revision)?;
                let initialized=bridge::tools::dispatch(&self.bridge,"orch",&json!({"project":revised.project,"args":["init"]}))?;
                if initialized["status"]!="completed" {return Err(Failure::new("orch_init",initialized.to_string()));}
                Ok(json!({"status":"completed","phase":"planning","project":revised.project,"previous_store":store,"reason":reason}))
            })
        })
    }
    pub(super) fn installed_probe(&self,caller:&str,args:&Value)->Res<Value> {
        let root=self.root.lock().unwrap().clone();
        let state=authority::read(&self.home,&root)?;
        let g=goal::load(&self.home,&root)?.ok_or_else(||invalid("missing goal"))?;
        if caller!="dispatcher" || !matches!(state.phase,authority::Phase::Planning|authority::Phase::Execution)
            || g.mode.as_deref()!=Some("change") || g.interjection.is_some() {
            return Err(Failure::new("permission_denied","installed probe requires active dispatcher, approved research, change mode and no interjection"));
        }
        authority::check_research(&state)?;
        let request=tools::resolved(&tools::path(&self.cwd,string(args,"request_file")))?;
        let store=orch::store_for(&self.home,g.project.as_deref().ok_or_else(||invalid("missing project"))?)?;
        if !request.starts_with(tools::resolved(&self.home.join("artifacts").join(&root))?) && !request.starts_with(tools::resolved(&store)?) {
            return Err(Failure::new("permission_denied","probe request must belong to this session"));
        }
        let bytes=std::fs::read(&request).map_err(io)?;
        let input:Value=serde_json::from_slice(&bytes).map_err(io)?;
        let params=preflight::probe_request(&input)?;
        let bin=std::env::current_exe().map_err(io)?;
        let binary_hash=format!("{:x}",Sha256::digest(std::fs::read(&bin).map_err(io)?));
        let source=orch::snapshot(&self.cwd,&[".".into()]);
        if source.is_empty() {return Err(invalid("probe requires identifiable project source"));}
        let request_hash=format!("{:x}",Sha256::digest(&bytes));
        let id=format!("{:x}",Sha256::digest(format!("{request_hash}:{binary_hash}:{source}")));
        let dir=self.home.join("artifacts").join(&root).join("installed-probes").join(id);
        std::fs::create_dir_all(&dir).map_err(io)?;
        orch::with_lock(&dir,false,|| {
            let result_path=dir.join("result.json");
            if result_path.exists() {return serde_json::from_slice(&std::fs::read(&result_path).map_err(io)?).map_err(io);}
            if dir.join("request.json").exists() {return Err(Failure::new("recovery_required","probe outcome unknown; do not duplicate the model task"));}
            base::atomic_write(&dir.join("request.json"),&bytes).map_err(io)?;
            let identity=json!({"binary":bin,"sha256":binary_hash,"version":env!("CARGO_PKG_VERSION"),"pid":std::process::id(),"session":root,"project":g.project,"source":source,"request_sha256":request_hash,"transport":"Host dynamic callback on installed bridge"});
            base::atomic_write(&dir.join("identity.json"),&serde_json::to_vec_pretty(&identity).map_err(io)?).map_err(io)?;
            let result=bridge::tools::dispatch(&self.bridge,"codex_run",&params);
            let after=orch::snapshot(&self.cwd,&[".".into()]);
            let report=match result {Ok(v)=>v,Err(e)=>json!({"status":"failed","error":e.to_json()})};
            let value=json!({"status":if base::tool_succeeded(&report) && after==source {"completed"}else{"failed"},"identity":identity,"source_after":after,"report":report,"receipt":result_path});
            base::atomic_write(&result_path,&serde_json::to_vec_pretty(&value).map_err(io)?).map_err(io)?;
            Ok(value)
        })
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_probe_replays_sealed_result_and_refuses_unknown_attempt() {
        let t=tempfile::tempdir().unwrap();let repo=t.path().join("repo");let home=t.path().join("home");
        std::fs::create_dir(&repo).unwrap();assert!(std::process::Command::new("git").args(["init","-q"]).current_dir(&repo).status().unwrap().success());std::fs::write(repo.join("input.txt"),"fixture").unwrap();
        let python=std::env::var("PYTHON").unwrap_or_else(|_|if cfg!(windows){"python".into()}else{"python3".into()});
        let config=crate::session::Config {home:home.clone(),bin:std::env::current_exe().unwrap(),codex:Ok(vec![python,"-I".into(),format!("{}/tests/fakes/fake_codex.py",env!("CARGO_MANIFEST_DIR"))])};
        let (a,opened)=crate::session::Attachment::open(&config,&json!({"project":repo,"text":"probe"})).unwrap();let root=opened["session"].as_str().unwrap();
        a.call("goal",&json!({"mode":"change"})).unwrap();
        let store=orch::store_for(&home,opened["goal"]["project"].as_str().unwrap()).unwrap();std::fs::create_dir_all(&store).unwrap();
        let research=store.join("research.json");std::fs::write(&research,"approved").unwrap();
        authority::update(&home,root,|s| {s.phase=authority::Phase::Planning;s.research=Some(json!({"artifact":research,"sha256":format!("{:x}",Sha256::digest(b"approved"))}));Ok(())}).unwrap();
        let request=store.join("probe.json");std::fs::write(&request,json!({"role":"astra-verify","effort":"high","sandbox":"read-only","task_kind":"verification","brief":"verify fixture","cwd":repo}).to_string()).unwrap();
        let args=json!({"request_file":request});
        assert!(a.host.installed_probe("core",&args).is_err());
        let first=a.host.installed_probe("dispatcher",&args).unwrap();assert_eq!(first["status"],"completed","{first}");
        let repeated=a.host.installed_probe("dispatcher",&args).unwrap();assert_eq!(first,repeated);
        let receipt=Path::new(first["receipt"].as_str().unwrap());let raw=std::fs::read(receipt).unwrap();std::fs::remove_file(receipt).unwrap();
        assert_eq!(a.host.installed_probe("dispatcher",&args).unwrap_err().kind,"recovery_required");
        std::fs::write(receipt,raw).unwrap();
    }
    #[test]
    fn session_stores_and_replan_preserve_evidence_without_nested_lock() {
        let temp=tempfile::tempdir().unwrap(); let home=temp.path().join("home");
        let config=crate::session::Config {home:home.clone(),bin:std::env::current_exe().unwrap(),codex:Ok(vec!["unused".into()])};
        let args=json!({"project":temp.path(),"text":"tiny"});
        let (a,aa)=crate::session::Attachment::open(&config,&args).unwrap();
        let (b,bb)=crate::session::Attachment::open(&config,&args).unwrap();
        assert_ne!(aa["goal"]["project"],bb["goal"]["project"]);
        let root=aa["session"].as_str().unwrap();
        let old=orch::store_for(&home,aa["goal"]["project"].as_str().unwrap()).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("modules.md"),"old contract").unwrap();
        let research=temp.path().join("research.json");std::fs::write(&research,"approved").unwrap();
        authority::update(&home,root,|s| {s.phase=authority::Phase::Execution;s.research=Some(json!({"artifact":research,"sha256":format!("{:x}",Sha256::digest(b"approved"))}));Ok(())}).unwrap();
        // Dispatcher itself is one producer; it must not block its own replan.
        let _dispatcher=Producer::new(&a.host.producers);
        let result=a.host.replan("dispatcher",&json!({"reason":"infeasible baseline"})).unwrap();
        assert_eq!(result["phase"],"planning");assert_ne!(result["project"],aa["goal"]["project"]);
        assert_eq!(std::fs::read_to_string(old.join("modules.md")).unwrap(),"old contract");
        assert_eq!(b.status().unwrap()["goal"]["project"],bb["goal"]["project"]);
        let store=orch::store_for(&home,result["project"].as_str().unwrap()).unwrap();
        std::fs::create_dir_all(&store).unwrap();std::fs::write(store.join("run.evidence.tsv"),"evidence").unwrap();
        assert_eq!(a.host.replan("dispatcher",&json!({"reason":"cannot reset started work"})).unwrap_err().kind,"recovery_required");
        assert!(a.host.replan("core",&json!({"reason":"bypass"})).is_err());
    }
}
