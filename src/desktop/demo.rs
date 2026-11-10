//! Sample data for `chronos --demo`, so every screen can be seen without a live wish.
use super::model::*;
use gpui_kit::*;

fn inline(text: &str) -> Option<TextSource> {
    Some(TextSource::Inline(SharedString::from(text.to_owned())))
}

fn stage(name: &'static str, phase: Phase, note: Option<&'static str>) -> Stage {
    Stage {
        name: name.into(),
        phase,
        note: note.map(Into::into),
    }
}

fn line(time: &'static str, source: &'static str, text: &'static str, level: LogLevel) -> LogLine {
    LogLine {
        time: time.into(),
        source: source.into(),
        text: text.into(),
        level,
    }
}

const BRIEF: &str = "GOAL    Render every page of a paper to an image for the reader.\n\nSCOPE   tools/pdftext.py\n        tests/test_pdftext.py\n\nVERIFY  python -m pytest tests/test_pdftext.py -q\n\nCONTEXT Render at 300 dpi. Text stays on disk for citation checks and is never sent to the model.";
const REPORT: &str = "Rendered pages through the cache and added layout tests.\n\n18 tests passed. No files outside SCOPE changed.";
const VERIFY: &str = "$ python -m pytest tests/test_pdftext.py -q\n..................                                       [100%]\n18 passed in 2.41s";
const REVIEW: &str = "Verdict: pass\n\nminor  tests/test_pdftext.py:88  The 2x2 tile case asserts size but not page order.";
const CONTRACT: &str = "render_page(pdf_path, sha256, pdf_page, cache_root, dpi) -> Path\n\nReturns a cached PNG. The same inputs give the same file. A page that cannot be rendered returns None and logs an issue.";

pub fn sample_run() -> Run {
    let modules: [(&str, &str, &[usize], &[(&str, UnitState, u32)]); 13] = [
        ("core", "Paths, hashing and shared I/O", &[], &[("core", UnitState::Verified, 1), ("core_retry", UnitState::Verified, 2)]),
        ("llm", "Model calls with a cache", &[0], &[("llm", UnitState::Verified, 1)]),
        ("pdftext", "Page text and page images", &[0], &[("pdftext", UnitState::Verified, 1), ("pdftext_tiles", UnitState::Verified, 1)]),
        ("inventory", "Find and dedupe PDFs", &[0, 2], &[("inventory", UnitState::Verified, 1)]),
        ("schema", "Record schema", &[0], &[("schema", UnitState::Verified, 1)]),
        ("validate", "Check citations against pages", &[2, 4], &[("validate_tm", UnitState::Verified, 1)]),
        ("read", "Screen and read papers", &[1, 2, 4], &[("read_allpages", UnitState::Verified, 2), ("read_batch", UnitState::Running, 1)]),
        ("benchmark", "Compare reader models", &[6], &[("benchmark_img", UnitState::Verified, 1), ("bench_ref_fix", UnitState::Running, 2), ("d10_scorer", UnitState::Built, 1)]),
        ("dataset", "Build the dataset", &[5, 6], &[("d12_dataset", UnitState::Verified, 1), ("d12_splits", UnitState::Built, 1)]),
        ("features", "Process features", &[8], &[("features_tm", UnitState::Verified, 1), ("d03_fix", UnitState::Verified, 2)]),
        ("protocol", "Analysis protocol", &[8], &[("protocol_fix", UnitState::Built, 2)]),
        ("causal", "Causal models", &[9, 10], &[("causal", UnitState::Failed, 2), ("causal_fix", UnitState::Pending, 0)]),
        ("candidates", "Six candidate recipes", &[11], &[("candidates_tm", UnitState::Pending, 0)]),
    ];
    let mut run = Run {
        session: "2584a4de-1b34-4066-9ea5-837eb58ea565".into(),
        project: "ml_hfo2".into(),
        cwd: Some("E:\\codex\\ML_HfO2".into()),
        wish: "全面盤點 HfO₂ 電阻式記憶體 PDF，建立可追溯的資料集，以因果模型提出六組可實驗驗證的製程方案".into(),
        status: RunStatus::Active,
        hosted: true,
        started: "Nov 3, 05:15".into(),
        started_at: Some(1_793_654_100),
        stages: vec![
            stage("Astra", Phase::Done, Some("14 min")),
            stage("Critic", Phase::Done, Some("10 findings ruled")),
            stage("Architect", Phase::Done, Some("13 modules")),
            stage("Build", Phase::Running, None),
            stage("Full test", Phase::Waiting, None),
            stage("Memory", Phase::Waiting, None),
            stage("Report", Phase::Waiting, None),
        ],
        goals: vec![
            GoalItem { id: "G1".into(), title: "盤點並精讀 PDF，保留原文頁碼".into(), modules: vec![0, 1, 2, 3, 6] },
            GoalItem { id: "G2".into(), title: "建立標示缺值與品質的資料集".into(), modules: vec![4, 5, 8] },
            GoalItem { id: "G3".into(), title: "比較讀取模型，採用較佳者".into(), modules: vec![7] },
            GoalItem { id: "G4".into(), title: "以因果模型分析製程與性能".into(), modules: vec![9, 10, 11] },
            GoalItem { id: "G5".into(), title: "提出六組可實驗驗證的方案".into(), modules: vec![12] },
        ],
        modules: vec![],
        units: vec![],
        usage: Usage {
            total_usd: Some(116.12),
            models: vec![
                ModelCost { model: "claude-sonnet-5-5".into(), usd: Some(91.85), input: Some(0), cached: Some(226_026_522), output: Some(802_372) },
                ModelCost { model: "claude-opus-5-5".into(), usd: Some(23.82), input: Some(0), cached: Some(36_001_210), output: Some(128_398) },
                ModelCost { model: "claude-haiku-5-5".into(), usd: Some(0.45), input: Some(0), cached: Some(633_994), output: Some(26_655) },
                ModelCost { model: "gpt-6.1-terra".into(), usd: None, input: Some(11_643_991), cached: Some(10_462_720), output: Some(195_816) },
            ],
            roles: vec![
                RoleCost { role: "chronos:dispatcher".into(), usd: Some(78.41), input: 0, cached: 198_312_906, output: 612_044 },
                RoleCost { role: "main".into(), usd: Some(23.82), input: 0, cached: 36_001_210, output: 128_398 },
                RoleCost { role: "chronos:reviewer".into(), usd: Some(13.44), input: 0, cached: 27_713_616, output: 190_328 },
                RoleCost { role: "chronos:memory-keeper".into(), usd: Some(0.45), input: 0, cached: 633_994, output: 26_655 },
                RoleCost { role: "codex terra".into(), usd: None, input: 11_643_991, cached: 10_462_720, output: 195_816 },
            ],
            contexts: vec![
                AgentContext { agent: "chronos:dispatcher".into(), peak_tokens: 965_312 },
                AgentContext { agent: "main".into(), peak_tokens: 181_204 },
                AgentContext { agent: "chronos:reviewer".into(), peak_tokens: 44_120 },
            ],
        },
        needs: vec![],
        log: vec![
            line("09:41:45", "core", "Usage limit reset; resuming the dispatcher", LogLevel::Info),
            line("09:41:52", "read_batch", "Screening: 1,467 included, 878 excluded, 5,258 pending", LogLevel::Info),
            line("09:48:03", "bench_ref_fix", "VERIFY passed", LogLevel::Ok),
            line("09:49:17", "causal", "Review failed: 1 major finding", LogLevel::Error),
            line("09:51:37", "dispatcher", "Reader decision still waiting for the benchmark", LogLevel::Warn),
        ],
        decisions: vec![
            "LUNA 需比低檔 TERRA 高 5 個百分點才採用，比較前先凍結門檻".into(),
            "初篩一律使用低檔 TERRA，LUNA 只參加精讀比較".into(),
            "基準答案由實作者對照原文寫出，標記為尚未人工複核".into(),
        ],
        report: None,
        elapsed: Some("4 h 36 m".into()),
        warnings: vec![],
    };
    for (m, (id, purpose, depends, units)) in modules.iter().enumerate() {
        let mut indices = vec![];
        for (uid, state, attempts) in units.iter() {
            indices.push(run.units.len());
            run.units.push(Unit {
                id: (*uid).into(),
                title: format!("{}: {}", purpose, uid).into(),
                module: m,
                state: *state,
                raw_state: state.label().to_lowercase().into(),
                attempts: *attempts,
                files: UnitFiles {
                    brief: inline(BRIEF),
                    report: (*attempts > 0).then(|| inline(REPORT)).flatten(),
                    verify: (*attempts > 0).then(|| inline(VERIFY)).flatten(),
                    review: (*state == UnitState::Verified || *state == UnitState::Failed)
                        .then(|| inline(REVIEW))
                        .flatten(),
                },
            });
        }
        run.modules.push(Module {
            id: (*id).into(),
            purpose: (*purpose).into(),
            depends: depends.to_vec(),
            units: indices,
            contract: inline(CONTRACT),
        });
    }
    run
}

/// Shrinks the sample's figures so runs differ. Peaks scale apart from cost: a cheap run can still hold a big context.
fn scale_usage(usage: &mut Usage, cost: f64, context: f64) {
    let tokens = |n: &mut u64| *n = (*n as f64 * cost) as u64;
    usage.total_usd = usage.total_usd.map(|usd| usd * cost);
    for model in &mut usage.models {
        model.usd = model.usd.map(|usd| usd * cost);
        [&mut model.input, &mut model.cached, &mut model.output].into_iter().flatten().for_each(tokens);
    }
    for role in &mut usage.roles {
        role.usd = role.usd.map(|usd| usd * cost);
        [&mut role.input, &mut role.cached, &mut role.output].into_iter().for_each(tokens);
    }
    for agent in &mut usage.contexts {
        agent.peak_tokens = (agent.peak_tokens as f64 * context) as u64;
    }
}

fn finished_run() -> Run {
    let mut run = sample_run();
    run.session = "9f360fac-b268-431e-abd7-9e792c58929e".into();
    run.project = "chronos-r5".into();
    run.cwd = Some("E:\\codex\\OPEN_DESIGN\\axiom\\experiments\\chronos-r5".into());
    run.wish = "第五輪：重新評估參考庫並全面優化".into();
    run.status = RunStatus::Done;
    run.hosted = false;
    run.started = "Nov 3, 00:42".into();
    run.started_at = Some(1_793_637_720);
    run.modules.truncate(6);
    run.units.retain(|u| u.module < 6);
    let mut next = 0;
    for module in &mut run.modules {
        let count = module.units.len();
        module.units = (next..next + count).collect();
        next += count;
    }
    for unit in &mut run.units {
        unit.state = UnitState::Verified;
    }
    for stage in &mut run.stages {
        stage.phase = Phase::Done;
        stage.note = None;
    }
    run.usage.models.truncate(3);
    run.usage.roles.truncate(4);
    scale_usage(&mut run.usage, 6.89 / 116.12, 0.22);
    run.goals.truncate(2);
    for goal in &mut run.goals {
        goal.modules.retain(|m| *m < 6);
    }
    run.report = inline("第五輪完成：9 個單元都審過，完整測試通過。");
    run.elapsed = Some("1 h 21 m".into());
    run
}

fn waiting_run() -> Run {
    let mut run = sample_run();
    run.session = "3d5d3623-0540-44a4-b7c3-58f280e1e433".into();
    run.project = "rram-batch".into();
    run.wish = "把剩下的精讀改成背景批次".into();
    run.status = RunStatus::Waiting;
    run.hosted = false;
    run.started = "Nov 4, 12:50".into();
    run.started_at = Some(1_793_767_800);
    scale_usage(&mut run.usage, 0.33, 0.4);
    run.warnings = vec!["快取在長時間等待後過期，重寫了 6 次（約 2,100 萬 token）。等待長批次時請讓它在背景執行，這次對話先結束。".into()];
    for unit in &mut run.units {
        if unit.state == UnitState::Running {
            unit.state = UnitState::Built;
        }
    }
    run.needs = vec![Need {
        kind: NeedKind::Waiting,
        title: "Background batch running".into(),
        note: "Progress in reports/progress.json. The batch writes reports/done.flag when it ends. Resume with axiom wish --resume.".into(),
    }];
    run
}

fn blocked_run() -> Run {
    let mut run = sample_run();
    run.session = "525c10d8-3031-4f4b-8dd8-6ee3dd2bda5a".into();
    run.project = "billing".into();
    run.wish = "Migrate billing exports to the new schema".into();
    run.status = RunStatus::Blocked;
    run.hosted = false;
    run.started = "Nov 2, 19:33".into();
    run.started_at = Some(1_793_619_180);
    run.usage.total_usd = None;
    run.usage.models.iter_mut().for_each(|model| model.usd = None);
    run.usage.roles.iter_mut().for_each(|role| role.usd = None);
    run.usage.contexts.truncate(2);
    run.needs = vec![Need {
        kind: NeedKind::Blocked,
        title: "A production credential is needed".into(),
        note: "The migration test reads the staging database. Provide STAGING_DSN, then resume.".into(),
    }];
    run
}

pub fn state() -> AppState {
    AppState {
        view: Screen::Run,
        runs: vec![sample_run(), waiting_run(), blocked_run(), finished_run()],
        current: Some(0),
        selection: Selection::None,
        memory: vec![
            MemoryEntry { id: 48, project: "chronos-r5".into(), title: "Stop hook blocks idle turns".into(), body: "While a goal is active the Stop hook refuses to end the turn, so long waits turn into polling. A waiting status lets the session end.".into(), stale: false, wish: "把剩下的精讀改成背景批次".into(), confirmed: "Nov 8, 21:40".into(), corrected: false },
            MemoryEntry { id: 41, project: "ml_hfo2".into(), title: "Page images beat tiled sheets".into(), body: "Single pages at 300 dpi, eight per call, cost less than 2x2 tiles because tiles are downscaled and need more tokens per page.".into(), stale: false, wish: "Read every PDF page and keep the original page numbers".into(), confirmed: "Nov 5, 09:12".into(), corrected: true },
            MemoryEntry { id: 33, project: "chronos-r4".into(), title: "CRLF breaks doc tests on Linux".into(), body: "Read documents tolerating CRLF and write them with LF.".into(), stale: true, wish: "Fix the failing Linux tests".into(), confirmed: "Oct 29, 14:03".into(), corrected: false },
        ],
        checks: vec![
            Check { name: "claude".into(), status: CheckStatus::Ok, detail: "2.1.317 (Claude Code)".into() },
            Check { name: "codex".into(), status: CheckStatus::Ok, detail: "codex-cli 0.159.3".into() },
            Check { name: "sqlite fts5".into(), status: CheckStatus::Ok, detail: "SQLite 3.46.0 with trigram".into() },
            Check { name: "state".into(), status: CheckStatus::Ok, detail: "C:\\Users\\gppup\\.axiom".into() },
            Check { name: "codegraph".into(), status: CheckStatus::Warn, detail: "Not installed (optional)".into() },
        ],
        reduce_motion: false,
        composing: false,
        backend: None,
        scene: None,
        errors: vec![],
        next_error: 0,
        renames: Default::default(),
        address: None,
    }
}
