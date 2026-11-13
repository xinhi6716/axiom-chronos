use crate::{
    perf_support::{results, workload},
    release_support, win,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Log {
    pub interactive: Option<u64>,
    pub frames: Vec<Value>,
    pub events: Vec<Value>,
    pub pixels: Vec<Value>,
}
pub fn parse(text: &str, require_interactive: bool) -> Result<Log, String> {
    let mut log = Log::default();
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break;
        }
        let v: Value =
            serde_json::from_str(line).map_err(|e| format!("invalid perf JSONL: {e}"))?;
        match v["t"].as_str() {
            Some("interactive") => {
                let n = v["since_start_us"].as_u64().ok_or("invalid interactive")?;
                if log.interactive.replace(n).is_some() {
                    return Err("duplicate interactive".into());
                }
            }
            Some("frame") => {
                for k in ["start_us", "cost_us", "interval_us"] {
                    if v[k].as_u64().is_none() {
                        return Err(format!("invalid frame {k}"));
                    }
                }
                log.frames.push(v);
            }
            Some("event") => {
                if v["seq"].as_u64().is_none() || v["at_us"].as_u64().is_none() {
                    return Err("invalid event".into());
                }
                log.events.push(v);
            }
            Some("pixel") => {
                if v["seq"].as_u64().is_none() || v["latency_us"].as_u64().is_none() {
                    return Err("invalid pixel".into());
                }
                log.pixels.push(v);
            }
            _ => {}
        }
    }
    if require_interactive && log.interactive.is_none() {
        return Err("missing interactive record (window could not receive foreground input or did not record it)".into());
    }
    Ok(log)
}
pub fn pairs(log: &Log) -> Result<Vec<Value>, String> {
    let mut pixels = BTreeMap::new();
    for p in &log.pixels {
        let seq = p["seq"].as_u64().unwrap();
        if pixels.insert(seq, p).is_some() {
            return Err(format!("duplicate pixel {seq}"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for e in &log.events {
        let seq = e["seq"].as_u64().unwrap();
        if !seen.insert(seq) {
            return Err(format!("duplicate event {seq}"));
        }
        let p = pixels
            .get(&seq)
            .ok_or_else(|| format!("unpaired event {seq}"))?;
        out.push(json!({"seq":seq,"at_us":e["at_us"],"latency_us":p["latency_us"]}));
    }
    Ok(out)
}
fn read(path: &Path, require: bool) -> Result<Log, String> {
    parse(&fs::read_to_string(path).unwrap_or_default(), require)
}
struct App {
    child: Child,
    log: PathBuf,
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl App {
    fn start(exe: &Path, home: &Path, target: &str, name: &str) -> Self {
        let log = home.join(format!("{name}.jsonl"));
        let _ = fs::remove_file(&log);
        let child = Command::new(exe)
            .args(["--target", target, "--state", "live", "--perf-log"])
            .arg(&log)
            .env("AXIOM_HOME", home)
            .spawn()
            .expect("launch release chronos");
        Self { child, log }
    }
    fn interactive(&mut self) -> u64 {
        let window = win::wait_window(&mut self.child);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut next_click = Instant::now();
        loop {
            if Instant::now() >= next_click {
                win::click(window);
                next_click = Instant::now() + Duration::from_millis(200);
            }
            if let Some(t) = read(&self.log, false).unwrap().interactive {
                return t;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "app exited before interactive"
            );
            assert!(Instant::now() < deadline, "missing interactive record (window could not receive foreground input or did not record it)");
            thread::sleep(Duration::from_millis(2));
        }
    }
}
const WISH: &str = "w-1234567890123-abcdef";
fn write(path: &Path, data: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}
pub(super) fn seed_run(home: &Path) -> PathBuf {
    let store = home.join("orchestrate/proj").join(WISH);
    fs::create_dir_all(&store).unwrap();
    write(
        &home.join("goals/s1.json"),
        &json!({"wish":"Performance run","wish_id":WISH,"project":"proj","status":"active"})
            .to_string(),
    );
    write(
        &home.join("logs/20261109-100000-wish.jsonl"),
        &format!(
            "{}\n",
            json!({"type":"system","subtype":"init","session_id":"s1"})
        ),
    );
    store
}
fn append(store: &Path, count: usize) {
    for _ in 0..count {
        axiom::orch::events::append_event(store, "heartbeat", None, None, json!({}))
            .expect("append live run event");
    }
}
fn wait_events(app: &mut App, count: usize, timeout: Duration) -> Vec<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let log = read(&app.log, false).unwrap();
        if log.events.len() >= count {
            if let Ok(p) = pairs(&log) {
                if p.len() >= count {
                    return p;
                }
            }
        }
        assert!(
            app.child.try_wait().unwrap().is_none(),
            "app exited waiting for events"
        );
        assert!(
            Instant::now() < deadline,
            "missing event/pixel pairs: events={}, pixels={}; {}",
            log.events.len(),
            log.pixels.len(),
            pairs(&log).err().unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(100));
    }
}
pub fn pilot() {
    let exe = release_support::release_exe("chronos").unwrap_or_else(|e| {
        panic!(
            "{e}; build with cargo build --locked --offline --release --features app --bin chronos"
        )
    });
    let home = release_support::isolated_home();
    let store = seed_run(home.path());
    let mut app = App::start(&exe, home.path(), "run", "pilot");
    app.interactive();
    append(&store, 5);
    let p = wait_events(&mut app, 5, Duration::from_secs(20));
    let log = read(&app.log, true).unwrap();
    assert!(!log.frames.is_empty(), "no frame records");
    assert!(p.len() >= 5);
}
fn seed_wishes(home: &Path) {
    for n in 0..2000 {
        let session = format!("s{n}");
        let wish = format!("w-{n:013}-abcdef");
        write(&home.join("goals").join(format!("{session}.json")), &json!({"wish":format!("Performance wish {n}"),"wish_id":wish,"project":"proj","status":"active"}).to_string());
        write(
            &home
                .join("logs")
                .join(format!("20261109-100000-wish-{n}.jsonl")),
            &format!(
                "{}\n",
                json!({"type":"system","subtype":"init","session_id":session})
            ),
        );
    }
    assert_eq!(
        axiom::app::snapshot(home).wishes.len(),
        2000,
        "seed did not produce 2000 wishes"
    );
}
fn record(
    out: &mut Vec<Value>,
    dir: &Path,
    phase: &str,
    budgets: &Value,
    gate: &str,
    view: &str,
    scenario: &str,
    raw: Vec<Value>,
    values: Vec<f64>,
    value: Option<f64>,
) {
    let path = results::write_raw(dir, phase, &format!("{scenario}.jsonl"), &raw);
    out.push(results::measurement(
        gate, view, scenario, &path, &values, value, budgets,
    ));
}
fn scalar(
    out: &mut Vec<Value>,
    dir: &Path,
    phase: &str,
    budgets: &Value,
    gate: &str,
    values: Vec<f64>,
) {
    let raw = values.iter().map(|v| json!({"value":v})).collect();
    let v = *values.last().unwrap();
    record(
        out,
        dir,
        phase,
        budgets,
        gate,
        "*",
        gate,
        raw,
        values,
        Some(v),
    );
}
fn frames(
    exe: &Path,
    phase: &str,
    dir: &Path,
    budgets: &Value,
    hz: u32,
    target: &str,
    scenario: &str,
    scrolling: bool,
    out: &mut Vec<Value>,
) {
    let home = release_support::isolated_home();
    if scrolling {
        seed_wishes(home.path());
    } else {
        let store = seed_run(home.path());
        write(
            &store.join("astra/council-events.jsonl"),
            &format!(
                "{}\n",
                json!({"type":"council","subtype":"turn_start","member":"astra","phase":"A","round":1})
            ),
        );
    }
    let mut app = App::start(exe, home.path(), target, scenario);
    app.interactive();
    thread::sleep(Duration::from_secs(5));
    let start = Instant::now();
    let start_us = read(&app.log, true)
        .unwrap()
        .frames
        .last()
        .map(|v| v["start_us"].as_u64().unwrap())
        .unwrap_or(0);
    let w = win::wait_window(&mut app.child);
    while start.elapsed() < Duration::from_secs(30) {
        assert!(
            app.child.try_wait().unwrap().is_none(),
            "app exited during {scenario}"
        );
        if scrolling {
            win::wheel(
                w,
                if start.elapsed().as_secs() % 8 < 4 {
                    -120
                } else {
                    120
                },
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
    let log = read(&app.log, true).unwrap();
    let raw: Vec<_> = log
        .frames
        .into_iter()
        .filter(|v| v["start_us"].as_u64().unwrap() > start_us)
        .collect();
    assert!(!raw.is_empty(), "no frame samples for {scenario}");
    let values = raw
        .iter()
        .map(|v| v["cost_us"].as_f64().unwrap() / 1000.0)
        .collect();
    record(
        out,
        dir,
        phase,
        budgets,
        "frame",
        target,
        scenario,
        raw.clone(),
        values,
        None,
    );
    if hz >= 120 {
        let values = raw
            .iter()
            .map(|v| v["interval_us"].as_f64().unwrap() / 1000.0)
            .collect();
        record(
            out,
            dir,
            phase,
            budgets,
            "frame",
            target,
            &format!("{scenario}.interval"),
            raw,
            values,
            None,
        );
    }
}
pub fn measure(phase: &str) {
    let exe = release_support::release_exe("chronos").expect("build release chronos first");
    let repo = release_support::repo();
    let source = release_support::source_id(&repo)
        .expect("source_id requires committed application sources");
    let matrix = release_support::load_matrix();
    let budgets = &matrix.budgets;
    let dir = repo.join("tests/desktop-smoke/perf");
    let (gpu, hz, dpi) = win::machine();
    let mut out = Vec::new();
    frames(
        &exe,
        phase,
        &dir,
        budgets,
        hz,
        "run",
        "animation",
        false,
        &mut out,
    );
    frames(
        &exe,
        phase,
        &dir,
        budgets,
        hz,
        "wishes",
        "scrolling",
        true,
        &mut out,
    );
    let home = release_support::isolated_home();
    let store = seed_run(home.path());
    let mut app = App::start(&exe, home.path(), "run", "events");
    app.interactive();
    append(&store, 200);
    let raw = wait_events(&mut app, 200, Duration::from_secs(30));
    assert_eq!(raw.len(), 200, "expected 200 paired events");
    let vals: Vec<f64> = raw
        .iter()
        .map(|v| v["latency_us"].as_f64().unwrap() / 1000.0)
        .collect();
    let max = vals.iter().copied().fold(0.0, f64::max);
    record(
        &mut out,
        &dir,
        phase,
        budgets,
        "event_to_pixel",
        "run",
        "event_to_pixel",
        raw,
        vals,
        Some(max),
    );
    drop(app);
    let idle_view = matrix
        .views
        .iter()
        .find(|v| !v.animated && v.id == "research")
        .expect("matrix needs non-animated research target");
    let home = release_support::isolated_home();
    let mut app = App::start(&exe, home.path(), &idle_view.id, "idle");
    app.interactive();
    thread::sleep(Duration::from_secs(10));
    let idle = win::private_mib(&app.child);
    scalar(&mut out, &dir, phase, budgets, "memory", vec![idle]);
    let t = Instant::now();
    let cpu = win::cpu_ticks(win::handle(&app.child));
    thread::sleep(Duration::from_secs(60));
    assert!(
        app.child.try_wait().unwrap().is_none(),
        "idle app exited during CPU sampling"
    );
    let pct = win::cpu_percent(cpu, win::cpu_ticks(win::handle(&app.child)), t.elapsed());
    scalar(&mut out, &dir, phase, budgets, "idle_cpu", vec![pct]);
    drop(app);
    let mut starts = Vec::new();
    let mut start_raw = Vec::new();
    for n in 0..20 {
        let home = release_support::isolated_home();
        let now = Instant::now();
        let mut app = App::start(&exe, home.path(), "run", &format!("cold{n}"));
        let since_start_us = app.interactive();
        // The writer and poll introduce a small delay after the recorded input.
        let elapsed = now.elapsed().as_secs_f64();
        starts.push(elapsed);
        start_raw.push(json!({"value":elapsed,"since_start_us":since_start_us}));
    }
    record(
        &mut out,
        &dir,
        phase,
        budgets,
        "cold_start",
        "*",
        "cold_start",
        start_raw,
        starts,
        None,
    );
    let home = release_support::isolated_home();
    let work = workload::workload(home.path(), workload::HELDOUT_ROWS, workload::HELDOUT_SEED);
    axiom::research_workspace::session::save(
        &home.path().join("research"),
        &axiom::research_workspace::session::SessionPrefs {
            last: Some(axiom::research_workspace::session::ResearchRef {
                root: work.root.clone(),
                store: None,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let mut app = App::start(&exe, home.path(), "research", "large");
    let start = Instant::now();
    // PrivateUsage has no peak field; retain every observed sample.
    let mut samples = Vec::new();
    let mut interactive = false;
    let mut next_click = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        assert!(
            app.child.try_wait().unwrap().is_none(),
            "research app exited during sampling"
        );
        samples.push((start.elapsed().as_secs_f64(), win::private_mib(&app.child)));
        if !interactive {
            if Instant::now() >= next_click {
                if let Some(w) = win::window(app.child.id()) {
                    win::click(w);
                }
                next_click = Instant::now() + Duration::from_millis(200);
            }
            interactive = read(&app.log, false).unwrap().interactive.is_some();
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(interactive, "research app never became interactive");
    let restored = axiom::research_workspace::session::load(&home.path().join("research"));
    assert!(
        restored.recent.iter().any(|r| r.root == work.root),
        "research workspace was not opened by the app"
    );
    let values: Vec<_> = samples.iter().map(|(_, mib)| mib - idle).collect();
    let extra = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let raw = samples
        .iter()
        .map(|(at_s, mib)| json!({"at_s":at_s,"private_mib":mib,"idle_mib":idle,"value":mib-idle}))
        .collect();
    record(
        &mut out,
        &dir,
        phase,
        budgets,
        "large_data",
        "*",
        "large_data",
        raw,
        values,
        Some(extra),
    );
    scalar(
        &mut out,
        &dir,
        phase,
        budgets,
        "exe_size",
        vec![fs::metadata(&exe).unwrap().len() as f64 / 1048576.0],
    );
    results::write_results(
        &dir,
        &json!({"source_id":source,"commit":release_support::commit(&repo),"machine":{"gpu":gpu,"refresh_hz":hz,"dpi":dpi},"phase":phase,"measurements":out,"workload":{"rows":work.rows,"seed":work.seed,"sha256":work.sha256}}),
    );
}
