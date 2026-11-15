#![allow(dead_code)]
use crate::desktop::{
    i18n::Lang,
    model::LogLine,
    ux::events::{EventSummary, Notice, NoticeAction, NoticeKind, Tone},
};
use crate::orch::events::Event;
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_RAW_CHARS: usize = 4000;
fn raw(v: &Value) -> Option<String> {
    serde_json::to_string_pretty(v)
        .ok()
        .map(|s| s.chars().take(MAX_RAW_CHARS).collect())
}
fn summary(tone: Tone, en: impl Into<String>, zh: impl Into<String>, v: &Value) -> EventSummary {
    EventSummary {
        tone,
        en: en.into(),
        zh: zh.into(),
        raw: raw(v),
    }
}
fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn content_size(v: &Value) -> usize {
    match v {
        Value::String(s) => s.chars().count(),
        Value::Null => 0,
        _ => v.to_string().chars().count(),
    }
}
fn orch_summary(kind: &str, v: &Value) -> Option<EventSummary> {
    let (tone, en, zh) = match kind {
        "heartbeat" => (Tone::Info, "Heartbeat received", "已收到心跳訊號"),
        "activity_start" => (Tone::Info, "Activity started", "活動已開始"),
        "activity_finish" => (Tone::Info, "Activity finished", "活動已結束"),
        "module_claimed" => (Tone::Info, "Module claimed", "模組已認領"),
        "module_released" => (Tone::Info, "Module released", "模組已釋放"),
        "module_verified" => (Tone::Ok, "Module verified", "模組已驗證"),
        "unit_state" => (Tone::Info, "Unit state changed", "工作單元狀態已變更"),
        _ => return None,
    };
    Some(summary(tone, en, zh, v))
}
pub fn summarize_value(v: &Value) -> EventSummary {
    let kind = v
        .get("type")
        .or_else(|| v.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if let Some(s) = orch_summary(kind, v) {
        return s;
    }
    match kind {
        "system" if v["subtype"] == "init" => {
            summary(Tone::Info, "Session started", "工作階段已開始", v)
        }
        "assistant" => {
            let blocks = v["message"]["content"].as_array();
            if let Some(b) = blocks
                .into_iter()
                .flatten()
                .find(|b| b["type"] == "tool_use")
            {
                let name = str_field(b, "name");
                summary(
                    Tone::Info,
                    format!("Using tool: {name}"),
                    format!("使用工具：{name}"),
                    v,
                )
            } else if blocks.into_iter().flatten().any(|b| b["type"] == "text") {
                summary(Tone::Info, "Assistant responded", "助理已回覆", v)
            } else {
                summary(
                    Tone::Unknown,
                    "Unrecognised event: assistant",
                    "無法辨識的事件：assistant",
                    v,
                )
            }
        }
        "user"
            if v["message"]["content"]
                .as_array()
                .is_some_and(|b| b.iter().any(|b| b["type"] == "tool_result")) =>
        {
            let b = v["message"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["type"] == "tool_result")
                .unwrap();
            let name = b["name"].as_str().unwrap_or("tool");
            let n = content_size(&b["content"]);
            summary(
                if b["is_error"] == true {
                    Tone::Error
                } else {
                    Tone::Info
                },
                format!("Tool {name} returned {n} characters"),
                format!("工具 {name} 回傳 {n} 字元"),
                v,
            )
        }
        "tool_result" | "codex_tool_result" => {
            let name = v["name"].as_str().unwrap_or("Codex");
            let n = content_size(&v["content"]);
            summary(
                if !v["error"].is_null() || v["is_error"] == true {
                    Tone::Error
                } else {
                    Tone::Info
                },
                format!("Tool {name} returned {n} characters"),
                format!("工具 {name} 回傳 {n} 字元"),
                v,
            )
        }
        "result" => {
            if v["is_error"] == true {
                summary(
                    Tone::Error,
                    "Session ended with an error",
                    "工作階段因錯誤結束",
                    v,
                )
            } else {
                summary(Tone::Ok, "Session finished", "工作階段已結束", v)
            }
        }
        "chronos" if v["subtype"] == "limit_wait" => summary(
            Tone::Warn,
            "Waiting for the usage limit",
            "等待用量限制解除",
            v,
        ),
        "chronos" if v["subtype"] == "max_turns" => summary(
            Tone::Warn,
            "Stopped at the turn limit",
            "已達回合上限而停止",
            v,
        ),
        "council" if v["subtype"] == "member_degraded" => summary(
            Tone::Warn,
            format!("Council continued without {}", str_field(v, "member")),
            format!("議會在缺少 {} 的情況下繼續", str_field(v, "member")),
            v,
        ),
        _ if (v.get("type").is_none()
            && v.get("kind").is_none()
            && v.get("unit").is_some()
            && v.get("status").is_some())
            || (kind == "codex" && v["subtype"] == "tool_result") =>
        {
            summary(
                if v["status"] == "completed" {
                    Tone::Ok
                } else {
                    Tone::Error
                },
                format!("Codex tool returned {} characters", content_size(v)),
                format!("Codex 工具回傳 {} 字元", content_size(v)),
                v,
            )
        }
        _ => summary(
            Tone::Unknown,
            format!("Unrecognised event: {kind}"),
            format!("無法辨識的事件：{kind}"),
            v,
        ),
    }
}
pub fn summarize_text(text: &str) -> EventSummary {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => summarize_value(&v),
        Err(_) => EventSummary {
            tone: Tone::Info,
            en: text.to_owned(),
            zh: text.to_owned(),
            raw: None,
        },
    }
}
pub fn summarize_orch(event: &Event) -> EventSummary {
    let v = serde_json::to_value(event).unwrap_or(Value::Null);
    orch_summary(&event.kind, &v).unwrap_or_else(|| summarize_value(&v))
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    }
}
fn is_scope_result(v: &Value) -> bool {
    v["error"]["kind"] == "outside_scope" || !strings(&v["outside_scope"]).is_empty()
}
fn notice(v: &Value, at_ms: Option<i64>) -> Option<Notice> {
    let kind = if v["type"] == "chronos" && v["subtype"] == "max_turns" {
        NoticeKind::MaxTurns {
            turn_limit: v["turn_limit"].as_u64().and_then(|n| u32::try_from(n).ok()),
        }
    } else if v["type"] == "council" && v["subtype"] == "member_degraded" {
        NoticeKind::DegradedCouncil {
            member: str_field(v, "member").into(),
            phase: str_field(v, "phase").into(),
            round: v["round"].as_u64().unwrap_or(0),
            status: str_field(v, "status").into(),
            reason: str_field(v, "reason").into(),
        }
    } else if is_scope_result(v) {
        let allowed = strings(
            v.get("allowed")
                .or_else(|| v.get("scope"))
                .unwrap_or(&Value::Null),
        );
        let outside = strings(&v["outside_scope"]);
        let outside_count = v["outside_scope_count"]
            .as_u64()
            .or_else(|| v["outside_count"].as_u64())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(outside.len());
        let ignored = v["scope_ignored"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                Some((
                    x["path"].as_str()?.into(),
                    x["reason"].as_str().unwrap_or("").into(),
                ))
            })
            .collect();
        NoticeKind::ScopeFailure {
            unit: str_field(v, "unit").into(),
            allowed,
            outside,
            outside_count,
            ignored,
        }
    } else {
        return None;
    };
    let key = match &kind {
        NoticeKind::MaxTurns { .. } => "max_turns".into(),
        NoticeKind::DegradedCouncil {
            member,
            phase,
            round,
            ..
        } => format!("degraded:{member}:{phase}:{round}"),
        NoticeKind::ScopeFailure { unit, .. } => format!("scope:{unit}"),
    };
    Some(Notice { key, kind, at_ms })
}
fn collect_notices(
    v: &Value,
    at: Option<i64>,
    out: &mut Vec<Notice>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    if let Some(n) = notice(v, at) {
        if seen.insert(n.key.clone()) {
            out.push(n);
        }
    }
    match v {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "message" | "content" | "result" | "output") {
                    collect_notices(child, at, out, seen, depth + 1);
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                collect_notices(child, at, out, seen, depth + 1);
            }
        }
        Value::String(s) => {
            if let Ok(child) = serde_json::from_str::<Value>(s) {
                collect_notices(&child, at, out, seen, depth + 1);
            }
        }
        _ => {}
    }
}
pub fn notices_from_values<'a>(
    values: impl IntoIterator<Item = (Option<i64>, &'a Value)>,
) -> Vec<Notice> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (at, v) in values {
        collect_notices(v, at, &mut out, &mut seen, 0);
    }
    out
}
pub fn notices_from_log(lines: &[LogLine]) -> Vec<Notice> {
    let values: Vec<Value> = lines
        .iter()
        .filter_map(|l| serde_json::from_str(l.text.as_ref()).ok())
        .collect();
    notices_from_values(
        values
            .iter()
            .map(|v| (v.get("ts_ms").and_then(Value::as_i64), v)),
    )
}
pub struct NoticeCopy {
    pub title: String,
    pub cause: String,
    pub effect: String,
    pub action: Option<(NoticeAction, String)>,
}
pub fn notice_copy(n: &Notice, lang: Lang) -> NoticeCopy {
    match (&n.kind, lang) {
        (NoticeKind::MaxTurns { turn_limit }, l) => {
            let (title, cause, effect, action) = match l {
                Lang::En => (
                    "Stopped at the turn limit",
                    turn_limit.map_or("The turn limit was reached.".into(), |n| {
                        format!("The limit was {n} turns.")
                    }),
                    "The session stopped.",
                    "Resume with more turns",
                ),
                Lang::ZhTw => (
                    "已達回合上限而停止",
                    turn_limit.map_or("已達回合上限。".into(), |n| {
                        format!("上限為 {n} 回合。")
                    }),
                    "工作階段已停止。",
                    "增加回合後接續",
                ),
            };
            NoticeCopy {
                title: title.into(),
                cause,
                effect: effect.into(),
                action: Some((NoticeAction::Resume, action.into())),
            }
        }
        (
            NoticeKind::DegradedCouncil {
                member,
                status,
                reason,
                ..
            },
            l,
        ) => {
            let (title, cause, effect, action) = match l {
                Lang::En => (
                    format!("Council continued without {member}"),
                    format!("Status: {status}; reason: {reason}"),
                    "The remaining members continued.",
                    "Open council",
                ),
                Lang::ZhTw => (
                    format!("議會在缺少 {member} 的情況下繼續"),
                    format!("狀態：{status}；原因：{reason}"),
                    "其餘成員繼續討論。",
                    "開啟議會",
                ),
            };
            NoticeCopy {
                title,
                cause,
                effect: effect.into(),
                action: Some((NoticeAction::OpenCouncil, action.into())),
            }
        }
        (
            NoticeKind::ScopeFailure {
                unit,
                allowed,
                outside,
                outside_count,
                ignored,
            },
            l,
        ) => {
            let ignored = ignored
                .iter()
                .map(|(p, r)| format!("{p} ({r})"))
                .collect::<Vec<_>>()
                .join(", ");
            let (title, cause, effect, action) = match l {
                Lang::En => (
                    "Outside the allowed paths",
                    format!(
                        "Allowed: {}; outside ({outside_count}): {}; ignored: {ignored}",
                        allowed.join(", "),
                        outside.join(", ")
                    ),
                    "The unit needs attention.",
                    "Open unit",
                ),
                Lang::ZhTw => (
                    "超出允許路徑",
                    format!(
                        "允許：{}；超出（{outside_count}）：{}；已忽略：{ignored}",
                        allowed.join(", "),
                        outside.join(", ")
                    ),
                    "工作單元需要處理。",
                    "開啟工作單元",
                ),
            };
            NoticeCopy {
                title: title.into(),
                cause,
                effect: effect.into(),
                action: Some((NoticeAction::OpenUnit(unit.clone()), action.into())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn summaries_and_privacy() {
        for v in [
            json!({"type":"system","subtype":"init"}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"secret"}]}}),
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"content":"secret"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","content":"secret"}]}}),
            json!({"type":"result"}),
            json!({"type":"chronos","subtype":"limit_wait"}),
            json!({"type":"chronos","subtype":"max_turns"}),
            json!({"type":"council","subtype":"member_degraded"}),
            json!({"type":"codex_tool_result","content":"secret"}),
            json!({"kind":"unit_state"}),
        ] {
            let s = summarize_value(&v);
            assert_ne!(s.tone, Tone::Unknown);
            assert!(!s.en.is_empty() && !s.zh.is_empty());
            assert!(!s.en.contains("secret") && !s.zh.contains("secret"));
        }
        assert_eq!(
            summarize_value(&json!({"type":"mystery"})).tone,
            Tone::Unknown
        );
        assert_eq!(summarize_text("plain").raw, None);
        assert!(
            summarize_value(&json!({"type":"x","x":"字".repeat(5000)}))
                .raw
                .unwrap()
                .chars()
                .count()
                <= MAX_RAW_CHARS
        );
    }
    #[test]
    fn notices_and_nested_results() {
        let values = [
            json!({"type":"chronos","subtype":"max_turns","turn_limit":3}),
            json!({"type":"council","subtype":"member_degraded","member":"argon","round":1}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","content":json!({"unit":"u","error":{"kind":"outside_scope"},"allowed":["src"],"outside_scope":["bad"],"scope_ignored":[{"path":"x","reason":"generated"}]}).to_string()}]}}),
        ];
        let n = notices_from_values(values.iter().chain(values.iter()).map(|v| (Some(42), v)));
        assert_eq!(n.len(), 3);
        assert!(notice_copy(&n[2], Lang::ZhTw).cause.contains("generated"));
        assert_eq!(n[0].at_ms, Some(42));
        let lines: Vec<LogLine> = values
            .iter()
            .map(|v| LogLine {
                time: "".into(),
                source: "".into(),
                text: v.to_string().into(),
                level: crate::desktop::model::LogLevel::Info,
            })
            .collect();
        assert_eq!(notices_from_log(&lines).len(), 3);
        for item in &n {
            for lang in [Lang::En, Lang::ZhTw] {
                let c = notice_copy(item, lang);
                assert!(
                    !c.title.is_empty()
                        && !c.cause.is_empty()
                        && !c.effect.is_empty()
                        && c.action.is_some()
                );
            }
        }
    }
    #[test]
    fn all_orchestration_kinds_are_recognised() {
        for kind in [
            "heartbeat",
            "activity_start",
            "activity_finish",
            "module_claimed",
            "module_released",
            "module_verified",
            "unit_state",
        ] {
            let event = Event {
                seq: 1,
                ts: String::new(),
                ts_ms: 1,
                kind: kind.into(),
                wish: "w".into(),
                module: None,
                unit: None,
                data: json!({}),
            };
            let s = summarize_orch(&event);
            assert_ne!(s.tone, Tone::Unknown);
            assert!(!s.en.is_empty() && !s.zh.is_empty());
        }
    }
}
