//! Background, opt-in plain-language translations. Rendering never waits for the runner or disk.
use super::{i18n::Lang, settings};
use crate::base;
use gpui_kit::*;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const BATCH: usize = 40;
const DEBOUNCE: Duration = Duration::from_millis(400);
const CLAUDE_ARGS: [&str; 10] = [
    "-p", "--model", "claude-haiku-5-5", "--tools", "", "--strict-mcp-config",
    "--setting-sources", "local", "--output-format", "text",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Plain {
    pub shown: SharedString,
    pub original: Option<SharedString>,
}

pub trait Runner: Send + Sync + 'static {
    fn run(&self, prompt: &str) -> Result<String, String>;
}

pub enum Source {
    Claude { cache: PathBuf },
    Fake(Arc<dyn Runner>),
}

struct Claude;
impl Runner for Claude {
    fn run(&self, prompt: &str) -> Result<String, String> {
        let mut command = base::proc::cmd("claude");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .args(CLAUDE_ARGS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = format!("Translate each string into plain Traditional Chinese for a non-programmer. Keep code identifiers, paths and numbers unchanged. Reply with only the JSON array.\n{prompt}");
        // Writing may block if the child does not read, so it must not prevent the deadline.
        let mut stdin = child.stdin.take().ok_or("missing stdin")?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
        let result = base::proc::capture(child, Duration::from_secs(90)).map_err(|e| e.message)?;
        if result.timed_out || result.exit_code != 0 {
            return Err("translation process failed or timed out".into());
        }
        String::from_utf8(result.stdout).map_err(|e| e.to_string())
    }
}

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn wants(text: &str) -> bool {
    let mut ascii = false;
    let mut count = 0;
    for c in text.chars() {
        count += 1;
        if count >= 2000
            || matches!(c as u32, 0x1100..=0x11ff | 0x2e80..=0x2fff | 0x3000..=0x30ff | 0x3130..=0x318f | 0x31f0..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xff65..=0xff9f | 0x20000..=0x323af)
        {
            return false;
        }
        ascii |= c.is_ascii_alphabetic();
    }
    ascii
}

#[derive(Default)]
struct Core {
    cache: HashMap<String, String>,
    queue: VecDeque<(String, String)>,
    pending: HashSet<String>,
    attempted: HashSet<String>,
}

impl Core {
    fn lookup(&mut self, text: &str, active: bool) -> Option<String> {
        if !active || !wants(text) {
            return None;
        }
        let key = hash(text);
        if let Some(value) = self.cache.get(&key) {
            return Some(value.clone());
        }
        if !self.attempted.contains(&key) && self.pending.insert(key.clone()) {
            self.queue.push_back((key, text.to_owned()));
        }
        None
    }
    fn clear_queue(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
    fn take_batch(&mut self) -> Vec<(String, String)> {
        let mut batch = Vec::new();
        while batch.len() < BATCH {
            let Some((key, text)) = self.queue.pop_front() else {
                break;
            };
            self.pending.remove(&key);
            if !self.cache.contains_key(&key) {
                self.attempted.insert(key.clone());
                batch.push((key, text));
            }
        }
        batch
    }
    fn accept(&mut self, batch: &[(String, String)], response: &str) -> bool {
        let Ok(values) = serde_json::from_str::<Vec<String>>(response) else {
            return false;
        };
        if values.len() != batch.len() {
            return false;
        }
        for ((key, _), value) in batch.iter().zip(values) {
            self.cache.insert(key.clone(), value);
        }
        true
    }
}

fn load(path: &Path) -> HashMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save(path: &Path, cache: &HashMap<String, String>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec(cache).map_err(std::io::Error::other)?;
    base::atomic_write(path, &json)
}

// Run a single batch on the background executor, including persistence on success.
fn process_batch(core: &mut Core, runner: &dyn Runner, path: Option<&Path>) -> bool {
    let batch = core.take_batch();
    if batch.is_empty() {
        return false;
    }
    let texts: Vec<_> = batch.iter().map(|(_, text)| text).collect();
    let prompt = serde_json::to_string(&texts).expect("strings serialize");
    let success = runner.run(&prompt).ok().is_some_and(|response| core.accept(&batch, &response));
    if success {
        if let Some(path) = path {
            let _ = save(path, &core.cache);
        }
    }
    success
}

struct State {
    on: bool,
    core: Core,
    runner: Arc<dyn Runner>,
    path: Option<PathBuf>,
    loaded: bool,
    running: bool,
    last_queued: Instant,
}
struct GlobalState(Rc<RefCell<State>>);
impl Global for GlobalState {}

pub fn install(cx: &mut App, on: bool, source: Source) {
    let (runner, path): (Arc<dyn Runner>, _) = match source {
        Source::Claude { cache } => (Arc::new(Claude), Some(cache)),
        Source::Fake(runner) => (runner, None),
    };
    cx.set_global(GlobalState(Rc::new(RefCell::new(State {
        on,
        core: Core::default(),
        runner,
        loaded: path.is_none(),
        path,
        running: false,
        last_queued: Instant::now(),
    }))));
}

// called by the screens
#[allow(dead_code)]
pub fn enabled(cx: &App) -> bool {
    cx.try_global::<GlobalState>()
        .is_some_and(|s| s.0.borrow().on)
}
fn active(on: bool, lang: Lang) -> bool {
    on && lang == Lang::ZhTw
}

// called by the screens
#[allow(dead_code)]
pub fn set_enabled(on: bool, cx: &mut App) {
    settings::Store::edit(cx, |s| s.plain_words = on);
    if let Some(state) = cx.try_global::<GlobalState>() {
        let mut state = state.0.borrow_mut();
        state.on = on;
        if !on {
            state.core.clear_queue();
        }
    }
    cx.refresh_windows();
}

// called by the screens
#[allow(dead_code)]
pub fn plain(cx: &App, text: &SharedString) -> Plain {
    let original = || Plain {
        shown: text.clone(),
        original: None,
    };
    let Some(global) = cx.try_global::<GlobalState>() else {
        return original();
    };
    let lang = *cx.global::<Lang>();
    let mut state = global.0.borrow_mut();
    let on = active(state.on, lang);
    let before = state.core.queue.len();
    let result = state.core.lookup(text.as_ref(), on);
    if state.core.queue.len() != before {
        state.last_queued = Instant::now();
    }
    let start = on && !state.running && !state.core.queue.is_empty();
    if start {
        state.running = true;
    }
    drop(state);
    if start {
        start_worker(cx, global.0.clone());
    }
    match result {
        Some(value) if value != text.as_ref() => Plain {
            shown: value.into(),
            original: Some(text.clone()),
        },
        _ => original(),
    }
}

fn start_worker(cx: &App, state: Rc<RefCell<State>>) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let path = {
            let s = state.borrow();
            if s.loaded {
                None
            } else {
                s.path.clone()
            }
        };
        if let Some(path) = path {
            let cache = cx.background_spawn(async move { load(&path) }).await;
            let mut s = state.borrow_mut();
            s.core.cache = cache;
            s.loaded = true;
            drop(s);
            let _ = cx.update(|cx| cx.refresh_windows());
        }
        loop {
            loop {
                let delay = DEBOUNCE.saturating_sub(state.borrow().last_queued.elapsed());
                if delay.is_zero() {
                    break;
                }
                cx.background_executor().timer(delay).await;
            }
            let active = cx.update(|cx| *cx.global::<Lang>() == Lang::ZhTw);
            let (mut work, runner, path) = {
                let mut s = state.borrow_mut();
                if !s.on || !active {
                    s.core.clear_queue();
                    s.running = false;
                    return;
                }
                let batch = s.core.take_batch();
                if batch.is_empty() {
                    s.running = false;
                    return;
                }
                let work = Core {
                    cache: s.core.cache.clone(),
                    queue: batch.into(),
                    ..Core::default()
                };
                (work, s.runner.clone(), s.path.clone())
            };
            let (success, cache) = cx
                .background_spawn(async move {
                    let success = process_batch(&mut work, runner.as_ref(), path.as_deref());
                    (success, work.cache)
                })
                .await;
            if success {
                state.borrow_mut().core.cache = cache;
                let _ = cx.update(|cx| cx.refresh_windows());
            }
            let mut s = state.borrow_mut();
            if s.core.queue.is_empty() {
                s.running = false;
                return;
            }
        }
    })
    .detach();
}

/// Deterministic translations for the demo, with no process or persistent cache.
pub struct DemoRunner;
impl Runner for DemoRunner {
    fn run(&self, prompt: &str) -> Result<String, String> {
        let input: Vec<String> = serde_json::from_str(prompt).map_err(|e| e.to_string())?;
        let output: Vec<String> = input
            .iter()
            .map(|text| {
                match text.as_str() {
                    "Screening: 1,467 included, 878 excluded, 5,258 pending" => {
                        return "篩選：納入 1,467，排除 878，待處理 5,258".into()
                    }
                    "Review failed: 1 major finding" => return "審查未通過：1 項重大問題".into(),
                    _ => {}
                }
                let (prefix, suffix) = text.split_once(": ").unwrap_or((text, ""));
                let translated = match prefix {
                    "Paths, hashing and shared I/O" => "路徑、雜湊與共用輸入輸出",
                    "Model calls with a cache" => "使用快取呼叫模型",
                    "Page text and page images" => "頁面文字與圖片",
                    "Find and dedupe PDFs" => "尋找並去除重複的 PDF",
                    "Record schema" => "紀錄格式",
                    "Check citations against pages" => "對照頁面檢查引用",
                    "Screen and read papers" => "篩選並閱讀論文",
                    "Compare reader models" => "比較閱讀模型",
                    "Build the dataset" => "建立資料集",
                    "Process features" => "處理特徵",
                    "Analysis protocol" => "分析流程",
                    "Causal models" => "因果模型",
                    "Six candidate recipes" => "六組候選方案",
                    "Fold the late findings into the summary" => "將後續發現納入摘要",
                    "Usage limit reset; resuming the dispatcher" => {
                        "用量限制已重設，繼續執行 dispatcher"
                    }
                    "VERIFY passed" => "VERIFY 通過",
                    "Reader decision still waiting for the benchmark" => {
                        "閱讀模型的決定仍在等待基準測試"
                    }
                    _ => return text.clone(),
                };
                if suffix.is_empty() {
                    translated.to_owned()
                } else {
                    format!("{translated}: {suffix}")
                }
            })
            .collect();
        serde_json::to_string(&output).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{prelude::v1::test, sync::Mutex};

    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        response: String,
    }
    impl Fake {
        fn new(response: &str) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                response: response.into(),
            }
        }
        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }
    impl Runner for Fake {
        fn run(&self, prompt: &str) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::from_str(prompt).unwrap());
            Ok(self.response.clone())
        }
    }
    fn process(core: &mut Core, runner: &dyn Runner) -> bool {
        process_batch(core, runner, None)
    }

    #[test]
    fn claude_arguments_are_pinned() {
        assert_eq!(CLAUDE_ARGS, [
            "-p", "--model", "claude-haiku-5-5", "--tools", "", "--strict-mcp-config",
            "--setting-sources", "local", "--output-format", "text",
        ]);
    }

    #[test]
    fn demo_runner_covers_demo_unit_titles_and_log_lines() {
        let state = super::super::demo::state();
        let texts: Vec<&str> = state.runs.iter().flat_map(|run| {
            run.units.iter().map(|unit| unit.title.as_ref())
                .chain(run.log.iter().map(|line| line.text.as_ref()))
        }).collect();
        assert!(!texts.is_empty());
        let response = DemoRunner.run(&serde_json::to_string(&texts).unwrap()).unwrap();
        let translated: Vec<String> = serde_json::from_str(&response).unwrap();
        assert_eq!(translated.len(), texts.len());
        for (text, result) in texts.iter().zip(translated.iter()) {
            if wants(text) {
                assert_ne!(*text, result, "missing demo translation: {text}");
            }
        }
    }

    #[test]
    fn batch_maps_responses_in_order_and_deduplicates() {
        let mut core = Core::default();
        let fake = Fake::new(r#"["第一", "第二"]"#);
        assert_eq!(core.lookup("First", true), None);
        core.lookup("First", true);
        core.lookup("Second", true);
        assert_eq!(core.queue.len(), 2);
        assert!(process(&mut core, &fake));
        assert_eq!(*fake.calls.lock().unwrap(), vec![vec!["First", "Second"]]);
        assert_eq!(core.lookup("First", true).as_deref(), Some("第一"));
        assert_eq!(core.lookup("Second", true).as_deref(), Some("第二"));
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn cache_survives_restart_and_changed_text_misses() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.axiom-tmp/plain-text-1"));
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(&root).unwrap();
        let path = dir.path().join("cache.json");
        let fake = Fake::new(r#"["原文翻譯"]"#);
        let mut first = Core::default();
        first.lookup("Original text", true);
        assert!(process_batch(&mut first, &fake, Some(&path)));
        let mut restarted = Core {
            cache: load(&path),
            ..Core::default()
        };
        assert_eq!(
            restarted.lookup("Original text", true).as_deref(),
            Some("原文翻譯")
        );
        assert_eq!(restarted.lookup("Original text changed", true), None);
        assert_eq!(restarted.queue.len(), 1);
        let before = std::fs::read(&path).unwrap();
        let invalid_path = dir.path().join("invalid.json");
        assert!(!process_batch(&mut restarted, &Fake::new("[]"), Some(&invalid_path)));
        assert!(!invalid_path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"{bad").unwrap();
        assert!(load(&path).is_empty());
        dir.close().unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn bad_responses_are_not_cached_or_retried() {
        for response in ["not json", "[]", "[1]"] {
            let mut core = Core::default();
            let fake = Fake::new(response);
            core.lookup("Failure", true);
            assert!(!process(&mut core, &fake));
            assert!(core.cache.is_empty());
            core.lookup("Failure", true);
            assert!(core.queue.is_empty());
            assert!(!process(&mut core, &fake));
            assert_eq!(fake.calls(), 1);
        }
    }

    #[test]
    fn disabled_or_english_never_enqueue() {
        let mut core = Core::default();
        for (on, lang) in [(false, Lang::ZhTw), (true, Lang::En)] {
            assert_eq!(core.lookup("Some English", active(on, lang)), None);
            assert!(core.queue.is_empty());
            assert!(core.take_batch().is_empty());
        }
        assert_eq!(core.lookup("Some English", active(true, Lang::ZhTw)), None);
        assert!(!core.queue.is_empty());
    }

    #[test]
    fn filters_wanted_text() {
        assert!(wants("Error at file.rs:12"));
        assert!(!wants("繁體中文"));
        assert!(!wants("混合 English"));
        assert!(!wants("English 日本語"));
        assert!(!wants("English 한국어"));
        assert!(!wants("12345"));
        assert!(wants(&"a".repeat(1999)));
        assert!(!wants(&"a".repeat(2000)));
    }

    #[test]
    fn batches_are_limited_to_forty() {
        let mut core = Core::default();
        for i in 0..41 {
            core.lookup(&format!("Item {i}"), true);
        }
        assert_eq!(core.take_batch().len(), 40);
        assert_eq!(core.take_batch().len(), 1);
    }
}
