use serde_json::{json, Value};

fn size(v: &Value) -> usize {
    serde_json::to_string_pretty(v).map_or(usize::MAX, |s| s.len())
}
fn memory(v: &Value) -> bool {
    !matches!(v["kind"].as_str(), Some("file" | "symbol"))
}
fn memory_size(v: &Value) -> usize {
    v["bundles"]
        .as_array()
        .map_or(0, |a| a.iter().filter(|b| memory(b)).map(size).sum())
}
fn update(v: &mut Value, limit: usize) {
    let m = memory_size(v);
    if let Some(b) = v.get_mut("budget").and_then(Value::as_object_mut) {
        b.insert("limit".into(), json!(limit));
        b.insert("memory_used".into(), json!(m));
        b.insert("used".into(), json!(0));
    }
    for _ in 0..8 {
        let n = size(v);
        if v["budget"]["used"].as_u64() == Some(n as u64) {
            break;
        }
        if let Some(b) = v.get_mut("budget").and_then(Value::as_object_mut) {
            b.insert("used".into(), json!(n));
        } else {
            break;
        }
    }
}
/// Removes snippets, then edges, then complete lowest-ranked bundles, starting
/// from the end (restricted to memory bundles when over the memory budget).
/// Reduces metadata only if necessary.
pub fn fit(mut payload: Value, budget: usize, memory_budget: usize) -> Value {
    let mut omitted = payload["omitted"].as_u64().unwrap_or(0);
    let mut truncated = payload["truncated"].as_u64().unwrap_or(0);
    loop {
        update(&mut payload, budget);
        let mem_over = memory_size(&payload) > memory_budget;
        if !mem_over && size(&payload) <= budget {
            return payload;
        }
        // Keep nodes and their evidence when shedding snippets is sufficient.
        if let Some(bundles) = payload["bundles"].as_array_mut() {
            if let Some(bundle) = bundles
                .iter_mut()
                .rev()
                .find(|b| (!mem_over || memory(b)) && b["snippet"].is_string())
            {
                bundle["snippet"] = Value::Null;
                truncated += 1;
                payload["truncated"] = json!(truncated);
                continue;
            }
            if let Some(bundle) = bundles.iter_mut().rev().find(|b| {
                (!mem_over || memory(b))
                    && b["edges"].as_array().is_some_and(|edges| !edges.is_empty())
            }) {
                bundle["edges"].as_array_mut().unwrap().pop();
                truncated += 1;
                payload["truncated"] = json!(truncated);
                continue;
            }
        }
        let index = payload["bundles"].as_array().and_then(|a| {
            if mem_over {
                a.iter().rposition(memory)
            } else {
                a.len().checked_sub(1)
            }
        });
        if let Some(i) = index {
            payload["bundles"].as_array_mut().unwrap().remove(i);
            omitted += 1;
            truncated += 1;
            payload["omitted"] = json!(omitted);
            payload["truncated"] = json!(truncated);
            continue;
        }
        break;
    }
    // Arbitrarily large query text, paths and seed lists cannot be allowed to exceed the limit.
    for path in ["seeds", "scope", "query", "freshness"] {
        if let Some(obj) = payload.as_object_mut() {
            obj.remove(path);
        }
        update(&mut payload, budget);
        if size(&payload) <= budget {
            return payload;
        }
    }
    for path in [
        "revision",
        "error",
        "truncated",
        "omitted",
        "bundles",
        "status",
        "budget",
    ] {
        if let Some(obj) = payload.as_object_mut() {
            obj.remove(path);
        }
        update(&mut payload, budget);
        if size(&payload) <= budget {
            return payload;
        }
    }
    if size(&payload) <= budget {
        payload
    } else if budget >= 2 {
        json!({})
    } else {
        json!(0)
    }
}
