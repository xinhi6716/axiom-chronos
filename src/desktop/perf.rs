use gpui_kit::*;
use std::{fs::OpenOptions, io::{BufWriter, Write}, path::PathBuf, sync::{mpsc::{sync_channel, SyncSender}, Mutex, OnceLock}, time::Instant};

struct Recorder { tx: SyncSender<String>, started: Instant, last: Option<Instant>, first: bool, interactive: bool, pending: Vec<(u64, Instant)> }
static RECORDER: OnceLock<Mutex<Option<Recorder>>> = OnceLock::new();
fn recorder() -> &'static Mutex<Option<Recorder>> { RECORDER.get_or_init(|| Mutex::new(None)) }
fn send(r: &Recorder, line: String) { let _ = r.tx.try_send(line); }
fn us(d: std::time::Duration) -> u128 { d.as_micros() }

pub fn install(path: PathBuf, started: Instant, _cx: &mut App) { install_recorder(path, started); }
fn install_recorder(path: PathBuf, started: Instant) {
 let (tx, rx) = sync_channel::<String>(1024);
 std::thread::spawn(move || {
  let Ok(file) = OpenOptions::new().create(true).append(true).open(path) else { return };
  let mut writer = BufWriter::new(file);
  while let Ok(line) = rx.recv() { if writeln!(writer, "{line}").is_err() { break; } let _ = writer.flush(); }
 });
 *recorder().lock().unwrap() = Some(Recorder { tx, started, last: None, first: false, interactive: false, pending: Vec::new() });
}
pub fn interactive(_cx: &App) { mark_interactive(); }
fn mark_interactive() {
 if let Some(r) = recorder().lock().unwrap().as_mut() { if !r.interactive { r.interactive = true; send(r, format!("{{\"t\":\"interactive\",\"since_start_us\":{}}}", us(r.started.elapsed()))); } }
}
pub fn event_received(seq: u64) {
 if let Some(r) = recorder().lock().unwrap().as_mut() { let now = Instant::now(); send(r, format!("{{\"t\":\"event\",\"seq\":{seq},\"at_us\":{}}}", us(now.saturating_duration_since(r.started)))); r.pending.push((seq, now)); }
}
fn frame_end(start: Instant) {
 if let Some(r) = recorder().lock().unwrap().as_mut() {
  let now = Instant::now();
  let interval = r.last.map(|last| us(start.saturating_duration_since(last))).unwrap_or(0); r.last = Some(start);
  send(r, format!("{{\"t\":\"frame\",\"start_us\":{},\"cost_us\":{},\"interval_us\":{interval}}}", us(start.saturating_duration_since(r.started)), us(now.saturating_duration_since(start))));
  if !r.first { r.first = true; send(r, format!("{{\"t\":\"first_frame\",\"since_start_us\":{}}}", us(now.saturating_duration_since(r.started)))); }
  for (seq, at) in std::mem::take(&mut r.pending) { send(r, format!("{{\"t\":\"pixel\",\"seq\":{seq},\"latency_us\":{}}}", us(now.saturating_duration_since(at)))); }
 }
}
pub struct Root { view: AnyView }
impl Root { pub fn new(view: AnyView) -> Self { Self { view } } }
impl Render for Root {
 fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
  let start = Instant::now();
  div().relative().size_full()
   .capture_key_down(|_, _, cx| interactive(cx))
   .capture_any_mouse_down(|_, _, cx| interactive(cx))
   .child(self.view.clone()).child(canvas(|_,_,_| (), move |_,_,_,_| frame_end(start)).absolute().size_full())
 }
}

#[cfg(test)]
mod tests {
 use super::*;
 use std::prelude::v1::test;
 #[test]
 fn window_roots_record_input_and_mini_start_is_instrumented() {
  // Test support is not enabled for the gpui dependency in this crate; these
  // assertions guard the actual wiring until a window smoke test can run.
  let render = include_str!("perf.rs").split("impl Render for Root").nth(1).unwrap().split("#[cfg(test)]").next().unwrap();
  assert!(render.contains(".capture_key_down(|_, _, cx| interactive(cx))"));
  assert!(render.contains(".capture_any_mouse_down(|_, _, cx| interactive(cx))"));
  let startup = include_str!("mod.rs").split("fn instrument_mini").nth(1).unwrap().split("#[cfg(test)]").next().unwrap();
  assert!(startup.contains("base::Root::read(window, cx).view().clone()"));
  assert!(startup.contains("perf::Root::new(view)"));
  assert!(startup.contains("instrument_mini(handle, cx)"));
  assert!(!startup.contains("perf::interactive(cx)"));
 }
 #[test]
 fn bounded_writer_records_frames_events_and_pixels() {
  *recorder().lock().unwrap() = None;
  event_received(1); frame_end(Instant::now()); mark_interactive();
  assert!(recorder().lock().unwrap().is_none());
  let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("perf.jsonl");
  install_recorder(path.clone(),Instant::now());
  event_received(42); frame_end(Instant::now()); frame_end(Instant::now()); mark_interactive(); mark_interactive();
  let deadline = Instant::now() + std::time::Duration::from_secs(5);
  let text = loop {
   let text = std::fs::read_to_string(&path).unwrap_or_default();
   if text.lines().count() >= 6 { break text; }
   assert!(Instant::now() < deadline, "background writer did not flush: {text}");
   std::thread::sleep(std::time::Duration::from_millis(10));
  };
  let records: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
  for kind in ["frame","first_frame","interactive","event","pixel"] { assert!(records.iter().any(|r| r["t"] == kind),"{kind}"); }
  assert_eq!(records.iter().filter(|r| r["t"] == "frame").count(),2);
  assert_eq!(records.iter().filter(|r| r["t"] == "interactive").count(),1);
  assert!(records.iter().any(|r| r["t"] == "pixel" && r["seq"] == 42));
  *recorder().lock().unwrap() = None;
 }
}
