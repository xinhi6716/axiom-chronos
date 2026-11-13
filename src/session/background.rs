use serde_json::Value;
use std::collections::HashSet;

#[derive(Default)]
pub struct BackgroundTasks {
    open: HashSet<String>,
    seen: HashSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TaskChange {
    Started(String),
    Finished { id: String, status: String },
}

impl BackgroundTasks {
    pub fn observe(&mut self, event: &Value) -> Option<TaskChange> {
        if event["type"] != "system" {
            return None;
        }
        let id = event["task_id"].as_str()?;
        match event["subtype"].as_str()? {
            "task_started" if event["is_backgrounded"] == true => {
                if self.seen.insert(id.to_owned()) {
                    self.open.insert(id.to_owned());
                    Some(TaskChange::Started(id.to_owned()))
                } else {
                    None
                }
            }
            "task_notification" => {
                let status = event["status"].as_str()?;
                if self.open.remove(id) {
                    Some(TaskChange::Finished {
                        id: id.to_owned(),
                        status: status.to_owned(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn open(&self) -> usize {
        self.open.len()
    }
}
