use super::seams::ModeKind;
pub use super::ReadFilter;
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadMark {
    pub key: u64,
    pub archived: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UxPrefs {
    pub extreme: bool,
    pub read: BTreeMap<String, ReadMark>,
    pub read_filter: ReadFilter,
    pub usage_wish: Option<String>,
    pub usage_compare: Option<String>,
    pub drafts: BTreeMap<String, String>,
    pub last_mode: ModeKind,
}
impl UxPrefs {
    pub fn parse(v: &Value) -> Self {
        let string = |key| v.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            extreme: v.get("extreme").and_then(Value::as_bool).unwrap_or(false),
            read: v
                .get("read")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| {
                            Some((
                                k.clone(),
                                ReadMark {
                                    key: v.get("key")?.as_u64()?,
                                    archived: v.get("archived")?.as_bool()?,
                                },
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            read_filter: match string("read_filter").as_deref() {
                Some("unread") => ReadFilter::Unread,
                Some("archived") => ReadFilter::Archived,
                _ => ReadFilter::All,
            },
            usage_wish: string("usage_wish"),
            usage_compare: string("usage_compare"),
            drafts: v
                .get("drafts")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
            last_mode: match string("last_mode").as_deref() {
                Some("chat") => ModeKind::Chat,
                Some("quick_task") => ModeKind::QuickTask,
                Some("plaza") => ModeKind::Plaza,
                _ => ModeKind::Wish,
            },
        }
    }
    pub fn to_json(&self) -> Value {
        let read: serde_json::Map<String, Value> = self
            .read
            .iter()
            .map(|(k, v)| (k.clone(), json!({"key":v.key,"archived":v.archived})))
            .collect();
        json!({"extreme":self.extreme,"read":read,"read_filter":match self.read_filter { ReadFilter::All=>"all",ReadFilter::Unread=>"unread",ReadFilter::Archived=>"archived" },"usage_wish":self.usage_wish,"usage_compare":self.usage_compare,"drafts":self.drafts,"last_mode":match self.last_mode { ModeKind::Wish=>"wish",ModeKind::Chat=>"chat",ModeKind::QuickTask=>"quick_task",ModeKind::Plaza=>"plaza" }})
    }
}
