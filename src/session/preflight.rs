//! Executable plan contracts, checked before any engineering registration.
use crate::{base::{self, Failure, Res}, orch};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest,Sha256};
use std::{fs, path::Path};

fn invalid(e:impl std::fmt::Display)->Failure { Failure::new("preflight_failed",e.to_string()) }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    full_test:String,
    capabilities:Vec<String>,
    #[serde(default)]
    frozen_requests:Vec<String>,
}
pub(super) fn probe_request(value:&Value)->Res<Value> {
    let args=if value["method"]=="tools/call" {
        if value["params"]["name"]!="codex_run" {return Err(invalid("installed probe supports codex_run only"));}
        &value["params"]["arguments"]
    } else {value};
    if args["role"]!="astra-verify" || args["sandbox"]!="read-only" || args["task_kind"]!="verification"
        || args.get("effort").is_some_and(|v|v!="high")
        || args.get("model").is_some_and(|v|!v.as_str().is_some_and(|s|crate::codex::role_matches("astra",s)))
        || ["unit","save_to","developer_instructions","session"].iter().any(|k|args.get(k).is_some())
        || args.get("lean").is_some_and(|v|v!=true) || args.get("web_search").is_some_and(|v|v!=false)
        || args.get("keep_plugins").is_some_and(|v|v.as_array().is_none_or(|v|!v.is_empty()))
        || args["brief"].as_str().is_none_or(|s|s.trim().is_empty()) {
        return Err(invalid("installed probe requires astra-verify/high, read-only verification and isolated capabilities; do not mutate a frozen request, revise and record a new contract"));
    }
    Ok(args.clone())
}
pub(super) fn validate(store:&Path)->Res<String> {
    let modules=fs::read(store.join("modules.md")).map_err(invalid)?;
    let map=orch::parse_map(std::str::from_utf8(&modules).map_err(invalid)?).map_err(invalid)?;
    let bytes=fs::read(store.join("preflight.json")).map_err(|_|invalid("planner must write preflight.json: full_test, capabilities, frozen_requests"))?;
    let contract:Contract=serde_json::from_slice(&bytes).map_err(invalid)?;
    if contract.full_test.trim().is_empty() || map.full_test.as_deref().map(str::trim)!=Some(contract.full_test.trim()) {
        return Err(invalid("preflight full_test must exactly match modules.md FULL TEST; select one command before freezing"));
    }
    for capability in &contract.capabilities {
        if !["measurement","installed_probe","core_release","post_submit_content"].contains(&capability.as_str()) {
            return Err(invalid(format!("unsupported capability: {capability}")));
        }
    }
    let mut hash=Sha256::new();hash.update(&modules);hash.update(&bytes);
    for path in &contract.frozen_requests {
        let bytes=fs::read(store.join(path)).map_err(invalid)?;
        probe_request(&serde_json::from_slice(&bytes).map_err(invalid)?)?;
        hash.update(&bytes);
    }
    Ok(format!("{:x}",hash.finalize()))
}
pub(super) fn archive(store:&Path,attempt:u64,result:&Res<String>)->Res<()> {
    fs::create_dir_all(store.join("reports")).map_err(invalid)?;
    let path=store.join("reports").join(format!("preflight-{attempt}-{}.json",base::unique()));
    let value=json!({"attempt":attempt,"modules":fs::read_to_string(store.join("modules.md")).ok(),
        "contract":fs::read_to_string(store.join("preflight.json")).ok(),"hash":result.as_ref().ok(),"error":result.as_ref().err().map(|e|e.to_json())});
    base::atomic_write(&path,&serde_json::to_vec_pretty(&value).map_err(invalid)?).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_freezes_exact_commands_and_supported_probe_roles() {
        let t=tempfile::tempdir().unwrap(); let p=t.path();
        fs::write(p.join("modules.md"),"## module\nPURPOSE CLI\nPATHS src/ tests/\nDEPENDS none\nCONTRACT\n  stable CLI\nACCEPT cargo test\n\nFULL TEST cargo test --locked\n").unwrap();
        let mut c=json!({"full_test":"cargo test --locked --offline","capabilities":[],"frozen_requests":[]});
        fs::write(p.join("preflight.json"),c.to_string()).unwrap(); assert!(validate(p).is_err());
        c["full_test"]=json!("cargo test --locked");
        c["capabilities"]=json!(["installed_probe"]); c["frozen_requests"]=json!(["request.json"]);
        let mut request=json!({"role":"astra","effort":"high","sandbox":"read-only","task_kind":"verification","brief":"check 2+3"});
        fs::write(p.join("request.json"),request.to_string()).unwrap();
        fs::write(p.join("preflight.json"),c.to_string()).unwrap(); assert!(validate(p).is_err());
        request["role"]=json!("astra-verify");fs::write(p.join("request.json"),request.to_string()).unwrap();
        let before=validate(p).unwrap(); request["brief"]=json!("check 3+4");
        fs::write(p.join("request.json"),request.to_string()).unwrap();assert_ne!(before,validate(p).unwrap());
        request["developer_instructions"]=json!("bypass"); assert!(probe_request(&request).is_err());
    }
}
