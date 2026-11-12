use crate::desktop::{i18n::Lang, model::{AppState, Run, RunStatus, UnitState}, settings::Store, ux::prefs::{ReadFilter, ReadMark, UxPrefs}};
use gpui_kit::Context;

/// FNV-1a over a versioned, length-prefixed canonical UTF-8 representation.
/// Sorting makes the attention key independent of source ordering.
pub fn attention_key(run: &Run) -> u64 {
    let mut canonical = String::from("attention-v1;");
    fn field(s: &mut String, value: &str) { s.push_str(&format!("{}:{};", value.len(), value)); }
    field(&mut canonical, match run.status { RunStatus::Active => "active", RunStatus::Waiting => "waiting", RunStatus::LimitWait => "limit_wait", RunStatus::Interrupted => "interrupted", RunStatus::Blocked => "blocked", RunStatus::Done => "done" });
    let mut needs: Vec<_> = run.needs.iter().map(|n| n.title.as_ref()).collect(); needs.sort_unstable();
    field(&mut canonical, &needs.len().to_string()); for title in needs { field(&mut canonical, title); }
    let mut failed: Vec<_> = run.units.iter().filter(|u| u.state == UnitState::Failed).map(|u| u.id.as_ref()).collect(); failed.sort_unstable();
    field(&mut canonical, &failed.len().to_string()); for id in failed { field(&mut canonical, id); }
    field(&mut canonical, if run.verified_end() { "1" } else { "0" });
    field(&mut canonical, if run.report.is_some() { "1" } else { "0" });
    canonical.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
}
pub fn is_unread(run: &Run, prefs: &UxPrefs) -> bool { prefs.read.get(run.session.as_ref()).map_or(true, |m| m.key != attention_key(run)) }
pub fn is_archived(run: &Run, prefs: &UxPrefs) -> bool { prefs.read.get(run.session.as_ref()).is_some_and(|m| m.archived && m.key == attention_key(run)) }
pub fn visible(run: &Run, prefs: &UxPrefs, f: ReadFilter) -> bool {
    match f { ReadFilter::All => !is_archived(run, prefs), ReadFilter::Unread => is_unread(run, prefs), ReadFilter::Archived => is_archived(run, prefs) }
}
pub fn unread_count(runs: &[Run], prefs: &UxPrefs) -> usize { runs.iter().filter(|r| is_unread(r, prefs)).count() }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAction { MarkRead, MarkUnread, Archive, Unarchive }
pub fn read_actions(run: &Run, prefs: &UxPrefs) -> Vec<ReadAction> {
    vec![if is_unread(run, prefs) { ReadAction::MarkRead } else { ReadAction::MarkUnread }, if is_archived(run, prefs) { ReadAction::Unarchive } else { ReadAction::Archive }]
}
pub fn action_label(a: ReadAction, lang: Lang) -> &'static str {
    match (a, lang) { (ReadAction::MarkRead, Lang::En) => "Mark as read", (ReadAction::MarkRead, Lang::ZhTw) => "標為已讀", (ReadAction::MarkUnread, Lang::En) => "Mark as unread", (ReadAction::MarkUnread, Lang::ZhTw) => "標為未讀", (ReadAction::Archive, Lang::En) => "Archive", (ReadAction::Archive, Lang::ZhTw) => "封存", (ReadAction::Unarchive, Lang::En) => "Move back from archive", (ReadAction::Unarchive, Lang::ZhTw) => "移出封存" }
}
/// Apply an action against the current run snapshot; stale archives never hide updates.
pub(in crate::desktop) fn change_mark(run: &Run, prefs: &mut UxPrefs, action: ReadAction) {
    match action {
        ReadAction::MarkRead => { let archived = is_archived(run, prefs); prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(run), archived }); }
        ReadAction::MarkUnread => { prefs.read.remove(run.session.as_ref()); }
        ReadAction::Archive => { prefs.read.insert(run.session.to_string(), ReadMark { key: attention_key(run), archived: true }); }
        ReadAction::Unarchive => { if let Some(mark) = prefs.read.get_mut(run.session.as_ref()) { mark.archived = false; } }
    }
}
impl AppState {
    pub fn mark_read(&mut self, session: &str, cx: &mut Context<Self>) { self.apply_read_action(session, ReadAction::MarkRead, cx); }
    pub fn mark_unread(&mut self, session: &str, cx: &mut Context<Self>) { self.apply_read_action(session, ReadAction::MarkUnread, cx); }
    pub fn archive(&mut self, session: &str, cx: &mut Context<Self>) { self.apply_read_action(session, ReadAction::Archive, cx); }
    pub fn unarchive(&mut self, session: &str, cx: &mut Context<Self>) { self.apply_read_action(session, ReadAction::Unarchive, cx); }
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        Store::edit(cx, |s| for run in &self.runs { if !is_archived(run, &s.ux) { change_mark(run, &mut s.ux, ReadAction::MarkRead); } }); cx.notify();
    }
    pub fn set_read_filter(&mut self, f: ReadFilter, cx: &mut Context<Self>) { Store::edit(cx, |s| s.ux.read_filter = f); cx.notify(); }
    pub fn apply_read_action(&mut self, session: &str, action: ReadAction, cx: &mut Context<Self>) {
        if let Some(run) = self.runs.iter().find(|r| r.session.as_ref() == session) {
            Store::edit(cx, |s| change_mark(run, &mut s.ux, action)); cx.notify();
        }
    }
}
