use super::{
    activity::{ActivitySource, OrchActivitySource},
    council, events, *,
};
use crate::desktop::ux::{
    self, activity::Projection, celestial::CouncilView, events::Notice, usage::UsageSession,
};
use crate::orch::events::Event as OrchEvent;
use std::io::{Read, Seek, SeekFrom};

const READ_BUDGET: u64 = 4 << 20;
const MAX_NOTICES: usize = 2000;

#[cfg(test)]
thread_local! {
    pub(super) static RECEIVED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn note_received(events: &[OrchEvent], after_seq: u64) {
    let mut last_seq = after_seq;
    for event in events {
        if event.seq > last_seq {
            last_seq = event.seq;
            crate::desktop::perf::event_received(event.seq);
            #[cfg(test)]
            RECEIVED.with(|received| received.borrow_mut().push(event.seq));
        }
    }
}

pub(super) struct Row {
    session: String,
    activity: Vec<OrchEvent>,
    notices: Vec<Notice>,
    council: Option<CouncilView>,
    usage: Option<UsageSession>,
    wish_id: Option<String>,
    amendments: Option<Vec<String>>,
}

#[derive(Default)]
struct Log {
    offset: u64,
    partial: Vec<u8>,
    dropping: bool,
    notices: Vec<Notice>,
}

fn merge_notices(to: &mut Vec<Notice>, from: impl IntoIterator<Item = Notice>) {
    for notice in from {
        if let Some(old) = to.iter_mut().find(|n| n.key == notice.key) {
            *old = notice;
        } else if to.len() < MAX_NOTICES {
            to.push(notice);
        }
    }
}

impl Log {
    fn read(&mut self, path: &Path) {
        let Ok(mut file) = std::fs::File::open(path) else {
            return;
        };
        let Ok(meta) = file.metadata() else { return };
        if meta.len() < self.offset {
            *self = Self::default();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if (&mut file)
            .take(READ_BUDGET)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return;
        }
        self.offset += bytes.len() as u64;
        for byte in bytes {
            if byte == b'\n' {
                if !self.dropping {
                    if let Ok(value) = serde_json::from_slice::<Value>(&self.partial) {
                        merge_notices(
                            &mut self.notices,
                            events::notices_from_values([(
                                value.get("ts_ms").and_then(Value::as_i64),
                                &value,
                            )]),
                        );
                    }
                }
                self.partial.clear();
                self.dropping = false;
            } else if !self.dropping {
                if self.partial.len() >= READ_BUDGET as usize {
                    self.partial.clear();
                    self.dropping = true;
                } else {
                    self.partial.push(byte);
                }
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Feed {
    projections: HashMap<String, Projection>,
    logs: HashMap<PathBuf, Log>,
}

impl Feed {
    #[cfg(test)]
    pub(super) fn last_seq(&self, session: &str) -> Option<u64> {
        self.projections.get(session).map(|p| p.last_seq)
    }

    #[cfg(test)]
    pub(super) fn log_offset(&self, path: &Path) -> Option<u64> {
        self.logs.get(path).map(|log| log.offset)
    }

    pub(super) fn rename(&mut self, renames: &[(String, String)]) {
        for (old, new) in renames {
            if let Some(p) = self.projections.remove(old) {
                self.projections.entry(new.clone()).or_insert(p);
            }
        }
    }

    pub(super) fn poll(
        &mut self,
        snapshot: &Snapshot,
        hosted: &HashMap<String, Hosted>,
        ledgers: &HashMap<String, LedgerEntry>,
    ) -> Vec<Row> {
        let mut rows = Vec::new();
        let source = OrchActivitySource;
        let mut paths = HashSet::new();
        for wish in &snapshot.wishes {
            let session = &wish.session;
            let mut activity = Vec::new();
            let mut council_view = None;
            if let Some(store) = &wish.store {
                let projection = self.projections.entry(session.clone()).or_default();
                // The upstream reader loads the file at once; bound that read on this thread.
                if std::fs::metadata(store.join("events.jsonl"))
                    .is_ok_and(|m| m.len() <= READ_BUDGET)
                {
                    if let Ok(events) = source.events(store, projection.last_seq) {
                        note_received(&events, projection.last_seq);
                        projection.apply(&events);
                        activity = events;
                    }
                }
                council_view = council::read_council(&store.join("astra"));
            }
            let mut notices = Vec::new();
            for run in &wish.runs {
                paths.insert(run.log.clone());
                let log = self.logs.entry(run.log.clone()).or_default();
                log.read(&run.log);
                merge_notices(&mut notices, log.notices.clone());
            }
            if let Some(h) = hosted.get(session) {
                merge_notices(&mut notices, events::notices_from_log(&h.log));
            }
            let id = wish
                .goal
                .wish_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(session);
            let usage = usage_of(wish, ledgers.get(session).and_then(|e| e.ledger.as_ref()));
            let complete = usage.total_usd.is_some()
                && !usage.models.is_empty()
                && usage.models.iter().all(|m| {
                    m.usd.is_some()
                        && m.input.is_some()
                        && m.cached.is_some()
                        && m.output.is_some()
                        && m.total.is_some()
                })
                && !wish.runs.is_empty()
                && wish.runs.iter().all(|r| {
                    r.scan.as_ref().is_some_and(|s| {
                        s.complete && s.codex.values().all(|t| !t.missing.iter().any(|m| *m))
                    }) && r.summary.as_ref().is_some_and(|s| {
                        s.total_cost_usd.is_some()
                            && s.models.values().all(|m| {
                                m.input.is_some()
                                    && m.output.is_some()
                                    && m.cache_read.is_some()
                                    && m.cache_creation.is_some()
                                    && m.cost_usd.is_some()
                            })
                    })
                });
            rows.push(Row {
                session: session.clone(),
                activity,
                notices,
                council: council_view,
                usage: Some(UsageSession {
                    wish: id.to_owned().into(),
                    session: session.clone().into(),
                    label: wish.goal.wish.clone().into(),
                    usage,
                    complete,
                    started_at: wish
                        .runs
                        .iter()
                        .filter_map(|r| r.started_unix.and_then(|n| u64::try_from(n).ok()))
                        .min(),
                }),
                wish_id: Some(id.to_owned()),
                amendments: Some(wish.goal.amendments.clone()),
            });
        }
        self.logs.retain(|path, _| paths.contains(path));
        for (session, h) in hosted
            .iter()
            .filter(|(s, _)| !snapshot.wishes.iter().any(|w| &w.session == *s))
        {
            rows.push(Row {
                session: session.clone(),
                activity: vec![],
                notices: events::notices_from_log(&h.log),
                council: None,
                usage: None,
                wish_id: None,
                amendments: None,
            });
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn received_helper_calls_perf() {
        fn calls_perf(body: &str) -> bool {
            let body: String = super::super::source_check::code(body)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            body.contains("crate::desktop::perf::event_received(")
                || body.contains("perf::event_received(")
        }
        let source = super::super::source_check::code(include_str!("feed.rs"));
        let body = super::super::source_check::body(&source, "note_received");
        assert!(calls_perf(body));
        assert!(calls_perf("perf::event_received/* comment */(1)"));
        assert!(!calls_perf("// perf::event_received(1)"));
        assert!(!calls_perf("/* perf::event_received(1) */"));
        assert!(!calls_perf("let x = \"perf::event_received(1)\";"));
        assert!(!calls_perf("let x = r#\"perf::event_received(1)\"#;"));
    }

    #[test]
    fn log_reads_are_bounded_and_recover_after_long_partial_and_truncated_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![b'x'; READ_BUDGET as usize + 10])
            .unwrap();
        file.write_all(b"\n{\"type\":\"chronos\",\"subtype\":\"max_")
            .unwrap();
        let mut log = Log::default();
        log.read(&path);
        assert_eq!(log.offset, READ_BUDGET);
        assert_eq!(log.partial.len(), READ_BUDGET as usize);
        assert!(log.notices.is_empty());
        log.read(&path);
        assert!(log.offset > READ_BUDGET);
        assert!(!log.dropping);
        assert!(!log.partial.is_empty());
        assert!(log.notices.is_empty());
        file.write_all(b"turns\",\"turn_limit\":12}\n").unwrap();
        log.read(&path);
        assert_eq!(log.notices.len(), 1);
        assert_eq!(log.notices[0].key, "max_turns");
        std::fs::write(
            &path,
            b"{\"type\":\"council\",\"subtype\":\"member_degraded\",\"member\":\"fable\"}\n",
        )
        .unwrap();
        log.read(&path);
        assert_eq!(log.offset, std::fs::metadata(&path).unwrap().len());
        assert_eq!(log.notices.len(), 1);
        assert!(log.notices[0].key.starts_with("degraded:fable:"));
    }

    #[test]
    fn apply_ignores_replayed_and_out_of_order_activity() {
        let event = |seq, ts_ms| OrchEvent {
            seq,
            ts: String::new(),
            ts_ms,
            kind: "activity_start".into(),
            wish: "w".into(),
            module: Some("m".into()),
            unit: None,
            data: serde_json::json!({"activity":"a","actor":"core","phase":"brief"}),
        };
        let row = |activity| Row {
            session: "s".into(),
            activity,
            notices: vec![],
            council: None,
            usage: None,
            wish_id: None,
            amendments: None,
        };
        let mut ux = ux::UxState::default();
        apply(&mut ux, vec![row(vec![event(1, 100), event(2, 200)])]);
        let before = ux.activity["s"].clone();
        apply(&mut ux, vec![row(vec![event(2, 999), event(1, 999)])]);
        assert_eq!(ux.activity["s"], before);
    }
}

pub(super) fn apply(ux: &mut ux::UxState, rows: Vec<Row>) {
    let sessions: HashSet<_> = rows
        .iter()
        .filter_map(|row| row.usage.as_ref().map(|u| u.session.clone()))
        .collect();
    let mut seen = HashSet::new();
    ux.usage_sessions
        .retain(|u| sessions.contains(&u.session) && seen.insert(u.session.clone()));
    for row in rows {
        let key: SharedString = row.session.into();
        if !row.activity.is_empty() {
            ux.activity
                .entry(key.clone())
                .or_default()
                .apply(&row.activity);
        }
        merge_notices(ux.notices.entry(key.clone()).or_default(), row.notices);
        if let Some(view) = row.council {
            ux.council.insert(key.clone(), view);
        }
        if let Some(id) = row.wish_id {
            ux.wish_ids.insert(key.clone(), id.into());
        }
        if let Some(amendments) = row.amendments {
            ux.amendments.insert(
                key.clone(),
                amendments.into_iter().map(Into::into).collect(),
            );
        }
        if let Some(usage) = row.usage {
            if let Some(old) = ux.usage_sessions.iter_mut().find(|u| u.session == key) {
                *old = usage;
            } else {
                ux.usage_sessions.push(usage);
            }
        }
    }
}

pub(super) fn rename(ux: &mut ux::UxState, renames: &[(String, String)]) {
    for (old, new) in renames {
        let new_key: SharedString = new.clone().into();
        if let Some(value) = ux.activity.remove(old.as_str()) {
            ux.activity.entry(new_key.clone()).or_insert(value);
        }
        if let Some(value) = ux.notices.remove(old.as_str()) {
            merge_notices(ux.notices.entry(new_key.clone()).or_default(), value);
        }
        if let Some(value) = ux.council.remove(old.as_str()) {
            ux.council.entry(new_key.clone()).or_insert(value);
        }
        if let Some(value) = ux.wish_ids.remove(old.as_str()) {
            ux.wish_ids.entry(new_key.clone()).or_insert(value);
        }
        if let Some(value) = ux.amendments.remove(old.as_str()) {
            ux.amendments.entry(new_key.clone()).or_insert(value);
        }
        for usage in &mut ux.usage_sessions {
            if usage.session.as_ref() == old {
                usage.session = new_key.clone();
            }
        }
        let mut seen = HashSet::new();
        ux.usage_sessions.retain(|u| seen.insert(u.session.clone()));
    }
}
