use crate::base::{self, Failure, Res};
use serde_json::{json, Value};
use std::{fs, path::Path};

pub const MAX_FILE_BYTES: usize = 60_000;
pub const MAX_TOTAL_BYTES: usize = 60_000;
pub const CHECKS: [&str; 4] = ["contract_mismatch", "vacuous_check", "untested", "platform"];

const INSTRUCTION: &str = "Every file to review is included in this message; answer from it directly.\n\nReview the files below against the contract. Report every:\n- contract_mismatch: a name, signature, data shape, call to another component or invariant that differs from the contract;\n- vacuous_check: a test, assertion or formal invariant that cannot fail, for example a TLA+ variable that no action ever sets to the value the invariant forbids, a TLA+ invariant that restates the guard of the only action able to break it, or a test whose assertion holds whatever the code does;\n- untested: a promise in the contract that no test exercises;\n- platform: behaviour that depends on the operating system, path separators, or a fixed inventory of files.\nFor each, give the severity (blocker, major or minor), the file and line or name, the evidence quoted from the code, and the fix. The verdict is fail when any finding is a blocker or major.";

fn schema() -> Value {
    json!({"type":"object", "additionalProperties":false, "required":["verdict","findings"], "properties":{
        "verdict":{"type":"string", "enum":["pass","fail"]},
        "findings":{"type":"array", "items":{"type":"object", "additionalProperties":false, "required":["check","severity","location","evidence","fix"], "properties":{
            "check":{"type":"string", "enum":CHECKS}, "severity":{"type":"string", "enum":["blocker","major","minor"]},
            "location":{"type":"string"}, "evidence":{"type":"string"}, "fix":{"type":"string"}
        }}}
    }})
}

fn derive(result: Value, model: &str, total: usize) -> Value {
    let output = &result["structured_output"];
    let verdict = output["verdict"].as_str();
    let findings = output["findings"].as_array();
    if !matches!(verdict, Some("pass" | "fail")) || findings.is_none() {
        return json!({"status":"failed", "verdict":null, "findings":[], "reviewer":model,
            "bytes_reviewed":total, "usage":result["usage"], "error":{"kind":"no_review_output",
            "message":"Review output is missing a valid verdict or findings list", "fallback":true}});
    }
    let findings = findings.unwrap();
    let serious = findings.iter().any(|f| {
        matches!(
            f.get("severity").and_then(Value::as_str),
            Some("blocker" | "major")
        )
    });
    json!({"status":"completed", "verdict":if serious || verdict == Some("fail") {"fail"} else {"pass"},
        "findings":findings, "reviewer":model, "bytes_reviewed":total, "usage":result["usage"], "error":null})
}

pub fn review(
    project_dir: &str,
    paths: &[String],
    contract: &str,
    model: &str,
    timeout_s: u64,
    bridge: &super::server::Bridge,
) -> Res<Value> {
    let root = base::require_directory(project_dir)?;
    if contract.trim().is_empty() {
        return Err(Failure::new(
            "invalid_argument",
            "contract is empty; review against what the code was built to do",
        ));
    }
    if paths.is_empty() {
        return Err(Failure::new("invalid_argument", "paths is empty"));
    }
    let root =
        fs::canonicalize(root).map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    let mut blocks = Vec::new();
    let mut total = 0usize;
    for rel in paths {
        let path = fs::canonicalize(root.join(rel)).map_err(|_| {
            Failure::new(
                "invalid_argument",
                format!("not a file inside project_dir: {rel}"),
            )
        })?;
        if path == root || !path.starts_with(&root) || !path.is_file() {
            return Err(Failure::new(
                "invalid_argument",
                format!("not a file inside project_dir: {rel}"),
            ));
        }
        let data = fs::read(path).map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
        if data.len() > MAX_FILE_BYTES {
            return Ok(
                json!({"status":"failed", "error":{"kind":"too_large", "message":format!("{rel} is {} bytes; split the review (limit {MAX_FILE_BYTES})", data.len()), "fallback":true}}),
            );
        }
        total += data.len();
        blocks.push(format!(
            "=== FILE {}\n{}",
            Path::new(rel).to_string_lossy().replace('\\', "/"),
            String::from_utf8_lossy(&data)
        ));
    }
    if total > MAX_TOTAL_BYTES {
        return Ok(
            json!({"status":"failed", "error":{"kind":"too_large", "message":format!("{total} bytes across {} files; split the review (limit {MAX_TOTAL_BYTES})", paths.len()), "fallback":true}}),
        );
    }
    let prompt = format!(
        "{INSTRUCTION}\n\n=== CONTRACT\n{contract}\n\n{}",
        blocks.join("\n\n")
    );
    let mut result =
        super::review_run::run(bridge, &prompt, None, model, Some(schema()), timeout_s)?;
    if result["status"] != "completed" {
        if result["error"]["kind"] == "invalid_structured_output" {
            return Ok(
                json!({"status":"failed", "verdict":null, "findings":[], "reviewer":model,
                "bytes_reviewed":total, "usage":result["usage"], "error":{"kind":"no_review_output",
                "message":result["error"]["message"], "fallback":true}}),
            );
        }
        result["error"]["fallback"] = json!(true);
        return Ok(result);
    }
    let thread = result["thread_id"].clone();
    let turn = result["turn_id"].clone();
    let mut report = derive(result, model, total);
    report["thread_id"] = thread;
    report["turn_id"] = turn;
    Ok(report)
}
