use super::server::Bridge;
use crate::{
    base::{Failure, Res},
    engine::{
        adjudicator::{self, Case, Outcome},
        approvals, dual_core, finding_verification, interjections, policy,
    },
    goal, graph, linux,
    memory::{Memory, Record},
    orch,
};
use serde_json::{json, Value};
use std::path::Path;

fn dual(
    b: &Bridge,
    store: &Path,
    cwd: &Path,
    kind: dual_core::Kind,
    choice: &str,
    reason: &str,
) -> Res<Option<Value>> {
    if !policy::load(store)?.dual_core {
        return Ok(None);
    }
    let decision = dual_core::Decision {
        kind,
        id: format!("{}-{}", kind.name(), crate::base::now_iso()),
        question: reason.into(),
        inputs: vec![],
        core_choice: choice.into(),
        core_reasoning: reason.into(),
        irreversible: false,
        reserved: adjudicator::is_reserved(reason),
    };
    let verdict = dual_core::judge(b.codex()?.as_ref(), store, cwd, &decision)?;
    Ok(Some(json!(format!("{verdict:?}"))))
}

fn adjudicate(b: &Bridge, store: &Path, cwd: &Path, case: &Case) -> Res<Outcome> {
    let mut evidence = adjudicator::evidence_for(store, cwd, case)?;
    if let Case::CloseAsk { question } = case {
        evidence.push(adjudicator::Evidence {
            id: "E1".into(),
            path: None,
            text: question.clone(),
        });
    }
    let result = adjudicator::adjudicate(b.codex()?.as_ref(), store, cwd, case, evidence, false)?;
    if let Outcome::Ruled(r) = &result {
        adjudicator::apply(store, case, r)?;
    }
    Ok(result)
}
fn ruling(r: &adjudicator::Ruling) -> Value {
    json!({"id":r.id,"decision":r.decision,"reason":r.reason,"evidence":r.evidence,"answer":r.answer})
}
fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}
fn opt<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}
fn arr(a: &Value, k: &str) -> Res<Vec<String>> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str().map(str::to_owned).ok_or_else(|| {
                    Failure::new("invalid_argument", format!("{k} must contain strings"))
                })
            })
            .collect(),
        _ => Err(Failure::new(
            "invalid_argument",
            format!("{k} must be an array"),
        )),
    }
}
fn timeout(a: &Value, d: u64) -> u64 {
    a.get("timeout_s")
        .and_then(Value::as_u64)
        .filter(|x| *x > 0)
        .unwrap_or(d)
}
fn store(b: &Bridge, project: &str) -> Res<std::path::PathBuf> {
    orch::store_for(b.home()?, project)
}
fn batch(a: &Value) -> Res<Option<goal::Batch>> {
    let given = ["progress_file", "marker_file", "pid"]
        .iter()
        .filter(|k| a.get(**k).is_some_and(|v| !v.is_null()))
        .count();
    if given == 0 {
        return Ok(None);
    }
    let pid = a
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    match (opt(a, "progress_file"), opt(a, "marker_file"), pid) {
        (Some(progress_file), Some(marker_file), Some(pid)) => Ok(Some(goal::Batch {
            progress_file: progress_file.into(),
            marker_file: marker_file.into(),
            pid,
        })),
        _ => Err(Failure::new(
            "invalid_argument",
            "progress_file, marker_file and pid go together: two paths and a process id",
        )),
    }
}

fn protected(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "kind"
            | "unit"
            | "state"
            | "next"
            | "exit_code"
            | "path"
            | "pack"
            | "report"
            | "report_json"
            | "verify_output"
            | "output_file"
    ) || key.ends_with("_file")
}

fn contains_protected(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| protected(k) || contains_protected(v)),
        Value::Array(values) => values.iter().any(contains_protected),
        _ => false,
    }
}

fn shrink(value: &mut Value, limit: usize, keep: bool, level: u8) {
    match value {
        Value::String(s) if !keep && s.len() > limit => {
            let mut end = limit.min(s.len());
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        Value::Array(values) => {
            let count = limit / 16;
            values.truncate(count);
            for v in values {
                shrink(v, limit, keep, level);
            }
        }
        Value::Object(map) => {
            let count = limit / 16;
            let mut i = 0;
            map.retain(|k, v| {
                i += 1;
                i <= count || protected(k) || contains_protected(v)
            });
            for (k, v) in map {
                if k == "stdout" {
                    if let Some(s) = v.as_str() {
                        if let Ok(mut inner) = serde_json::from_str::<Value>(s) {
                            shrink(&mut inner, limit, false, level);
                            *v = json!(inner.to_string());
                            continue;
                        }
                    }
                }
                let keep = protected(k)
                    && (level == 0
                        || (level == 1
                            && matches!(
                                k.as_str(),
                                "status" | "kind" | "unit" | "state" | "next" | "exit_code"
                            )));
                shrink(v, limit, keep, level);
            }
        }
        _ => {}
    }
}

fn reply_size(payload: &Value) -> usize {
    let body = super::server::call_body(|| Ok(payload.clone()));
    serde_json::to_vec(&body).expect("JSON payload").len()
}

/// Bound the complete bridge response, including its duplicated pretty-printed content.
pub fn bound_reply(payload: Value) -> Value {
    use crate::orch::summary::MAX_REPLY_BYTES;
    if reply_size(&payload) <= MAX_REPLY_BYTES {
        return payload;
    }
    let payload = if payload.is_object() {
        payload
    } else {
        json!({"value":payload})
    };
    for level in 0..=2 {
        let mut low = 0;
        let mut high = MAX_REPLY_BYTES;
        let mut best = None;
        while low <= high {
            let middle = (low + high) / 2;
            let mut candidate = payload.clone();
            shrink(&mut candidate, middle, false, level);
            candidate["truncated"] = json!(true);
            if reply_size(&candidate) <= MAX_REPLY_BYTES {
                best = Some(candidate);
                low = middle + 1;
            } else if middle == 0 {
                break;
            } else {
                high = middle - 1;
            }
        }
        if let Some(best) = best {
            return best;
        }
    }
    let mut candidate = payload;
    shrink(&mut candidate, 0, false, 2);
    candidate["truncated"] = json!(true);
    candidate
}

fn unit_reply(mut value: Value, b: &Bridge, args: &Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.remove("verify_tail");
        if let Some(accept) = map.get_mut("accept").and_then(Value::as_object_mut) {
            if accept.remove("output").is_some() {
                if let (Ok(root), Some(module)) = (
                    store(b, s(args, "project")),
                    accept.get("module").and_then(Value::as_str),
                ) {
                    accept.insert(
                        "output_file".into(),
                        json!(orch::accept_output_path(&root, module)),
                    );
                }
            }
        }
    }
    bound_reply(value)
}
pub fn dispatch(b: &Bridge, n: &str, a: &Value) -> Res<Value> {
    match n {
        "codex_run" => {
            let result = super::codex_tool::codex_run(b, a);
            if result["status"] == "completed" {
                b.note_project_cwd(s(a, "project"), s(a, "cwd"));
            }
            Ok(if a.get("unit").is_some() {
                unit_reply(result, b, a)
            } else {
                result
            })
        }
        "council" => super::council_tool::council(b, a),
        "approval" => {
            let op = s(a, "op");
            if matches!(op, "approve" | "deny") {
                return Err(Failure::new(
                    "person_only",
                    "only a person may approve or deny",
                ));
            }
            let path = store(b, s(a, "project"))?;
            match op {
                "request" => {
                    let action = &a["action"];
                    let command = arr(action, "command")?;
                    let (program, args) = command.split_first().ok_or_else(|| {
                        Failure::new("invalid_argument", "action command is required")
                    })?;
                    let cwd = opt(action, "cwd")
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            Failure::new("invalid_argument", "action cwd is required")
                        })?;
                    let result = approvals::request(
                        &path,
                        approvals::Action {
                            command: program.clone(),
                            args: args.to_vec(),
                            cwd: cwd.into(),
                        },
                        s(a, "reason"),
                        approvals::Origin::Model,
                    )?;
                    Ok(json!({"status":"requested","id":result.id,"digest":result.digest}))
                }
                "status" => approvals::status(&path, s(a, "id")),
                "execute" => {
                    let result = approvals::execute(&path, s(a, "id"))?;
                    Ok(
                        json!({"status":"completed","exit_code":result.exit_code,"output_tail":result.output_tail}),
                    )
                }
                _ => Err(Failure::new(
                    "invalid_argument",
                    "invalid approval operation",
                )),
            }
        }
        "orch" => {
            let path = store(b, s(a, "project"))?;
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| Failure::new("io", e.to_string()))?;
            let mut args = arr(a, "args")?;
            let json_flag = args.first().map_or(false, |x| x == "--json");
            if json_flag {
                args.remove(0);
            }
            let bounded = matches!(
                args.first().map(String::as_str),
                Some("verify" | "accept" | "schedule")
            );
            let force = args.iter().any(|x| x == "--force");
            args.retain(|x| x != "--force");
            if args.first().map(String::as_str) == Some("review") {
                if let Some(repo) = args
                    .iter()
                    .position(|x| x == "--repo")
                    .and_then(|i| args.get(i + 1))
                {
                    b.note_project_cwd(s(a, "project"), repo);
                }
            }
            let active = goal::find_active(b.home()?, s(a, "project"))?;
            let known_cwd = b.project_cwd(s(a, "project"));
            let explicit_cwd = args
                .iter()
                .position(|x| x == "--repo")
                .and_then(|i| args.get(i + 1))
                .map(|s| Path::new(s));
            let cwd = active
                .as_ref()
                .and_then(|(_, g)| g.cwd.as_deref())
                .map(Path::new)
                .or(explicit_cwd)
                .or(known_cwd.as_deref());
            if args.first().map(String::as_str) == Some("unit")
                && args.get(1).map(String::as_str) == Some("add")
            {
                if let Some(old) = args
                    .iter()
                    .position(|x| x == "--fixes")
                    .and_then(|i| args.get(i + 1))
                {
                    let gate_cwd = match active.as_ref() {
                        Some((_, g)) => g.cwd.as_deref().map(Path::new),
                        None => cwd,
                    };
                    let Some(cwd) = gate_cwd else {
                        return Ok(
                            json!({"status":"failed","error":{"kind":"findings_rejected","message":"repository is unavailable for finding verification"}}),
                        );
                    };
                    if let Err(e) = b.codex().and_then(|codex| {
                        finding_verification::fix_gate(codex.as_ref(), &path, old, cwd)
                    }) {
                        return Ok(
                            json!({"status":"failed","error":{"kind":"findings_rejected","message":e.message}}),
                        );
                    }
                }
            }
            let dual_kind = match args.first().map(String::as_str) {
                Some("map") => Some(dual_core::Kind::ModuleMap),
                Some("waive") if args.iter().any(|x| x.contains("fix_round")) => {
                    Some(dual_core::Kind::FixRounds)
                }
                Some("waive") => Some(dual_core::Kind::ScopeReduction),
                _ => None,
            };
            if dual_kind.is_some() && cwd.is_none() && policy::load(&path)?.dual_core {
                return Err(Failure::new(
                    "invalid_argument",
                    "repository is unavailable for dual-core judgment",
                ));
            }
            let dual_result = if let (Some(kind), Some(cwd)) = (dual_kind, cwd) {
                dual(b, &path, cwd, kind, &args.join(" "), &args.join(" "))?
            } else {
                None
            };
            if dual_result.as_ref().is_some_and(|v| {
                v.as_str().is_some_and(|s| {
                    s.starts_with("Overruled") || s.starts_with("ToPerson") || s == "Reserved"
                })
            }) {
                return Ok(
                    json!({"status":"failed","error":{"kind":"dual_core","message":"decision not upheld"},"dual_core":dual_result}),
                );
            }
            let r = orch::run(&path, &args, json_flag, force);
            let schedule_kind = (args.first().map(String::as_str) == Some("schedule"))
                .then(|| {
                    r.stderr
                        .strip_prefix("error: ")
                        .and_then(|s| s.split_once(": "))
                })
                .flatten()
                .map(|(kind, _)| kind)
                .filter(|kind| {
                    [
                        "invalid_argument",
                        "not_ready",
                        "already_claimed",
                        "already_verified",
                        "held",
                        "not_found",
                        "not_claimed",
                        "no_map",
                    ]
                    .contains(kind)
                });
            let err = if r.exit_code == 0 {
                Value::Null
            } else {
                json!({"kind":schedule_kind.unwrap_or("orch_failed"),"message":if r.stderr.is_empty(){&r.stdout}else{&r.stderr}.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>()})
            };
            let mut result = json!({"status":if r.exit_code==0 {"completed"}else{"failed"},"store":path,"exit_code":r.exit_code,"stdout":r.stdout,"stderr":r.stderr,"error":err});
            if let Some(v) = dual_result {
                result["dual_core"] = v;
            }
            if r.exit_code != 0
                && (r.stderr.contains("fix_rounds") || r.stdout.contains("fix_rounds"))
            {
                if let Some(cwd) = cwd {
                    if let Some(v) = dual(
                        b,
                        &path,
                        cwd,
                        dual_core::Kind::FixRounds,
                        "adjudicate",
                        &r.stderr,
                    )? {
                        let rejected = v.as_str().is_some_and(|s| {
                            s.starts_with("Overruled")
                                || s.starts_with("ToPerson")
                                || s == "Reserved"
                        });
                        result["dual_core"] = v;
                        if rejected {
                            return Ok(result);
                        }
                    }
                }
                if policy::load(&path)?.adjudicator.enabled {
                    if let (Some(cwd), Some(unit)) = (
                        cwd,
                        args.iter()
                            .position(|x| x == "--fixes")
                            .and_then(|i| args.get(i + 1)),
                    ) {
                        match adjudicate(b, &path, cwd, &Case::FixRounds { unit: unit.clone() })? {
                            Outcome::Ruled(r) => result["adjudication"] = ruling(&r),
                            Outcome::Person { reason } => {
                                result["person_requests"] = json!([reason])
                            }
                        }
                    }
                }
            }
            Ok(if bounded { bound_reply(result) } else { result })
        }
        "run_on_linux" => linux::run(
            b.config.wsl.clone(),
            b.config.on_windows,
            s(a, "project_dir"),
            &arr(a, "args")?,
            timeout(a, 1800),
        ),
        "memory_record" => {
            if opt(a, "mode").unwrap_or("unit") == "source" {
                let source = a.get("source").ok_or_else(|| Failure::new("invalid_argument", "source is required"))?;
                let artifact = source.get("artifact").and_then(Value::as_str).ok_or_else(|| Failure::new("invalid_argument", "source.artifact is required"))?;
                let links: Vec<graph::ingest::SourceLink> = serde_json::from_value(source.get("links").cloned().unwrap_or(json!([])))
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                let report = graph::ingest::import_source(&graph::GraphStore::open(b.home()?)?, std::path::Path::new(s(a, "project_dir")), artifact, &links)?;
                return Ok(json!({"status":"completed","source":report}));
            }
            if opt(a, "mode").unwrap_or("unit") != "unit" {
                return Err(Failure::new("invalid_argument", "mode must be unit or source"));
            }
            let record = Record {
            project: s(a, "project").into(),
            project_dir: s(a, "project_dir").into(),
            unit: s(a, "unit").into(),
            title: s(a, "title").into(),
            summary: s(a, "summary").into(),
            body: s(a, "body").into(),
            symbols: arr(a, "symbols")?,
            files: arr(a, "files")?,
            wish: s(a, "wish").into(),
            };
            let memory = Memory::open(b.home()?)?;
            if a.get("nodes").is_some() || a.get("links").is_some() {
                let typed: graph::TypedWrite = serde_json::from_value(json!({"nodes":a.get("nodes").cloned().unwrap_or(json!([])),"links":a.get("links").cloned().unwrap_or(json!([]))}))
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                memory.record_typed(&record, &typed)
            } else {
                memory.record(&record)
            }
        }
        "memory_search" => Memory::open(b.home()?)?.search(
            s(a, "query"),
            opt(a, "project"),
            a["limit"].as_i64().unwrap_or(5),
        ),
        "memory_get" => Memory::open(b.home()?)?.get(
            a["id"]
                .as_i64()
                .ok_or_else(|| Failure::new("invalid_argument", "id must be an integer"))?,
        ),
        "memory_digest" => Memory::open(b.home()?)?.digest(s(a, "project"), opt(a, "add")),
        "graph_explore" => {
            let req: graph::query::ExploreRequest = serde_json::from_value(a.clone())
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            graph::query::explore(b.home()?, &req)
        }
        "goal" => {
            let keys = ["status", "classify", "mode", "interjection"];
            if keys
                .iter()
                .filter(|k| a.get(**k).is_some_and(|v| !v.is_null()))
                .count()
                != 1
            {
                return Err(Failure::new(
                    "invalid_argument",
                    "exactly one of status, classify, mode, interjection is required",
                ));
            }
            let home = b.home()?;
            let session = s(a, "session");
            b.record_provenance(home, session)?;
            let mut adjudications = Vec::new();
            let mut person_requests = Vec::new();
            let mut dual_result = None;
            let g = if let Some(v) = opt(a, "status") {
                let current = goal::load(home, session)?
                    .ok_or_else(|| Failure::new("not_found", "goal not found"))?;
                let mut status = v.to_owned();
                let mut note = opt(a, "note").map(str::to_owned);
                let mut keep_active = false;
                if matches!(v, "done" | "blocked") && current.project.is_some() {
                    let path = orch::store_for_goal(home, &current)?;
                    let cwd = Path::new(current.cwd.as_deref().unwrap_or("."));
                    dual_result = dual(
                        b,
                        &path,
                        cwd,
                        dual_core::Kind::GoalClose,
                        v,
                        note.as_deref().unwrap_or(v),
                    )?;
                    if dual_result.as_ref().is_some_and(|v| {
                        v.as_str().is_some_and(|s| {
                            s.starts_with("Overruled")
                                || s.starts_with("ToPerson")
                                || s == "Reserved"
                        })
                    }) {
                        status = "blocked".into();
                        note = Some("dual-core decision requires a person".into());
                        person_requests.push(json!("dual-core decision requires a person"));
                    } else if policy::load(&path)?.adjudicator.enabled {
                        if v == "done" && path.join("units.tsv").exists() {
                            let units = orch::read_units(&path)?;
                            let events = orch::events::read_events(&path, 0)?;
                            for unit in &units {
                                let Some(verdict) = orch::read_verdict(&path, &unit.id)? else {
                                    continue;
                                };
                                let waived = orch::waivers(&orch::evidence(&path, &unit.id)?);
                                for finding in verdict.findings.iter().filter(|f| {
                                    f.needs_check() && !waived.iter().any(|w| w.number == f.number)
                                }) {
                                    let subject = format!("{}#{}", unit.id, finding.number);
                                    if events.iter().any(|e| {
                                        e.kind == "adjudication"
                                            && e.data["case"] == "needs_check"
                                            && e.data["subject"] == subject
                                            && e.data["decision"] == "pass"
                                            && !events.iter().any(|later| {
                                                later.kind == "adjudication_overturned"
                                                    && later.data["id"] == e.data["id"]
                                            })
                                    }) {
                                        continue;
                                    }
                                    let case = Case::NeedsCheck {
                                        unit: unit.id.clone(),
                                        finding: finding.number,
                                        needs: format!("{:?}", finding.needs),
                                    };
                                    match adjudicate(b, &path, cwd, &case)? {
                                        Outcome::Ruled(r) => {
                                            if r.decision == "block" {
                                                person_requests.push(json!(format!(
                                                    "{subject}: {}",
                                                    r.reason
                                                )));
                                            }
                                            adjudications.push(ruling(&r));
                                        }
                                        Outcome::Person { reason } => person_requests
                                            .push(json!(format!("{subject}: {reason}"))),
                                    }
                                }
                            }
                            if !person_requests.is_empty() {
                                status = "blocked".into();
                                note = Some(format!(
                                    "Needs person: {}",
                                    person_requests
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                ));
                            }
                        } else if v == "blocked"
                            && current.status != "waiting"
                            && !note.as_deref().unwrap_or("").trim().is_empty()
                        {
                            let case = Case::CloseAsk {
                                question: note.clone().unwrap_or_default(),
                            };
                            match adjudicate(b, &path, cwd, &case)? {
                                Outcome::Ruled(r) => {
                                    keep_active = r.decision == "answer"
                                        && r.answer
                                            .as_deref()
                                            .is_some_and(|s| !s.trim().is_empty());
                                    adjudications.push(ruling(&r));
                                }
                                Outcome::Person { reason } => person_requests.push(json!(reason)),
                            }
                        }
                    }
                }
                if keep_active {
                    if current.status == "active" {
                        json!(current)
                    } else {
                        json!(goal::reopen(home, session)?)
                    }
                } else {
                    goal::set_status_with_batch(home, session, &status, note.as_deref(), batch(a)?)?
                        ["goal"]
                        .clone()
                }
            } else if let Some(v) = opt(a, "mode") {
                json!(goal::set_mode(home, session, v)?)
            } else if let Some(v) = opt(a, "classify") {
                crate::laya::router::observe_interjection(home, session, v); // laya-shadow
                let g = if goal::load(home, session)?.is_some_and(|g| g.project.is_some()) {
                    interjections::classify(home, session, v)?
                } else {
                    goal::classify(home, session, v)?
                };
                if v == "stop" {
                    b.cancel();
                }
                json!(g)
            } else if let Some(v) = a.get("interjection").filter(|v| v.is_object()) {
                let applied = v.get("applied").filter(|x| !x.is_null());
                let declined = v.get("declined").filter(|x| !x.is_null());
                if applied.is_some() == declined.is_some() || s(v, "id").is_empty() {
                    return Err(Failure::new(
                        "invalid_argument",
                        "interjection requires id and exactly one of applied or declined",
                    ));
                }
                if let Some(links) = applied {
                    if !links.is_object() {
                        return Err(Failure::new(
                            "invalid_argument",
                            "applied must be an object",
                        ));
                    }
                    interjections::apply(
                        home,
                        session,
                        s(v, "id"),
                        interjections::Links {
                            plan_revision: opt(links, "plan_revision").map(str::to_owned),
                            units: arr(links, "units")?,
                            scope: arr(links, "scope")?,
                        },
                    )?;
                } else {
                    let reason = declined.and_then(Value::as_str).ok_or_else(|| {
                        Failure::new("invalid_argument", "declined must be a reason")
                    })?;
                    interjections::decline(home, session, s(v, "id"), reason)?;
                }
                json!(goal::load(home, session)?
                    .ok_or_else(|| Failure::new("not_found", "goal not found"))?)
            } else {
                return Err(Failure::new("invalid_argument", "invalid goal operation"));
            };
            let mut reply = json!({"status":g["status"],"mode":g["mode"],"reason":g["reason"]});
            for key in ["batch", "provenance"] {
                if !g[key].is_null() {
                    reply[key] = g[key].clone();
                }
            }
            let mut result = json!({"status":"completed","goal":reply});
            if !adjudications.is_empty() {
                result["adjudication"] = if adjudications.len() == 1 {
                    adjudications.remove(0)
                } else {
                    json!(adjudications)
                };
            }
            if !person_requests.is_empty() {
                result["person_requests"] = json!(person_requests);
            }
            if let Some(v) = dual_result {
                result["dual_core"] = v;
            }
            Ok(result)
        }
        "bridge_status" => {
            let mut result = json!({"status":"completed","codex_app_server_running":b.running()});
            for (key, cmd) in [("codex", &b.config.codex)] {
                result[key] = match cmd {
                    Err(e) => json!({"error":e.to_json()}),
                    Ok(c) => match crate::codex::probe_version(c) {
                        Ok(p) => {
                            json!({"command":c,"version":p.last_line.map(|x|vec![x]),"exit_code":p.exit_code,"version_check":p.check})
                        }
                        Err(e) => json!({"error":e.to_json()}),
                    },
                };
            }
            Ok(result)
        }
        _ => Err(Failure::new("unknown_tool", format!("unknown tool {n}"))),
    }
}
