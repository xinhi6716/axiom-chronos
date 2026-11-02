use std::{fs, path::Path};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const BASE: &str = "E:/codex/OPEN_DESIGN/axiom/experiments/refs-20261102/";
const LIBRARIES: [(&str, &str, &str, &str); 10] = [
    (
        "DeepSeek",
        "deepseek-ai-dsh-superagent-prototype",
        "6337fc823ce20251c8de8085948165c81aaa701e",
        "orch-gates",
    ),
    (
        "Orca",
        "stablyai-orca",
        "8cd6a7e5ee77c4ed04e78a3e924f5fc83a366bbf",
        "orch-gates",
    ),
    (
        "Codebase Memory",
        "DeusData-codebase-memory-mcp",
        "a5b248f47272413189746589b9a2c551c6b6c5ec",
        "engine-text",
    ),
    (
        "Agency Agents",
        "msitarzewski-agency-agents",
        "b37b5bb33b78f2c1d6567d98e25ca818e3259219",
        "engine-text",
    ),
    (
        "Trail-Code",
        "trailhq-Trail-Code",
        "a44da3dfb5894c8e316e320526041ef8e5ddbb8c",
        "engine-text",
    ),
    (
        "Stop That Shit",
        "lennney-stop-that-shit",
        "e4fe5296f6917849f7eec0860d6a1b68bc0292de",
        "orch-gates",
    ),
    (
        "ECC",
        "affaan-m-ECC",
        "b38be6a7f6350964d59328b52f0624e1d1d552d1",
        "install",
    ),
    (
        "Orkas",
        "Orkas-AI-Orkas",
        "dce78adb2a2ffdfbf9159cc89352e4335940bc64",
        "unit-run",
    ),
    (
        "HERO",
        "wanshuiyin-HERO-Anti-OverDefense",
        "92686ca5c7dc9f4852cd323e2852219b7e86a8ea",
        "wish-flow",
    ),
    (
        "HarnessMM",
        "harnessmm-2610.32813/paper.txt",
        "",
        "unit-run",
    ),
];
const LABELS: [&str; 8] = [
    "來源：",
    "閱讀：",
    "未讀：",
    "採納：",
    "落點：",
    "問題：",
    "測試：",
    "不採納：",
];
const MECHANISMS: [&str; 8] = [
    "持久化 claim",
    "進度通知",
    "快照綁定審查",
    "記憶去重",
    "前景 memory-keeper",
    "研究紀律",
    "行程樹診斷",
    "Gemini 審查",
];
const FACETS: [&str; 15] = [
    "Token 效率",
    "速度",
    "成功率",
    "返工",
    "子代理調度",
    "忙等卡死",
    "穩健性",
    "狀態機與權限",
    "研究計畫",
    "RSI",
    "記憶",
    "驗證品質",
    "版本一致性",
    "可觀察性",
    "維護成本",
];
const METRICS: [&str; 7] = [
    "口徑：is_error 以 tool_result 的 is_error 為真計數（含取消）；review_code 以該工具呼叫去重後計數，狀態取回覆的 status；Read 以名為 Read 的工具呼叫計數，memory-keeper Read 以其父 Agent 的 subagent_type 為 chronos:memory-keeper 者計數；時長為日誌第一與最後時間戳之差；費用取 result 事件的 total_cost_usd。",
    "時長｜第四輪：4.13 h｜第五輪：1.15 h（截至 2026-11-02T17:51:35Z，進行中）",
    "費用｜第四輪：$10.16｜第五輪：未結算（日誌尚無 result 事件）",
    "is_error｜第四輪：41（含 5 次取消）｜第五輪：4（截至同一時間）",
    "review_code｜第四輪：27（completed 14、too_large 5、timeout 3、cancelled 5）｜第五輪：0",
    "Read｜第四輪：84｜第五輪：30（截至同一時間）",
    "memory-keeper Read｜第四輪：59｜第五輪：0（memory-keeper 尚未執行）",
];

fn paths(module: &str) -> &'static [&'static str] {
    match module {
        "wish-flow" => &["engine/skills/wish/SKILL.md", "tests/wish_flow.rs"],
        "install" => &["tests/installed_assets.rs"],
        "engine-text" => &[
            "engine/DISPATCH.md",
            "engine/agents/dispatcher.md",
            "engine/agents/reviewer.md",
            "engine/agents/critic.md",
            "engine/agents/memory-keeper.md",
            "engine/skills/research/SKILL.md",
            "tests/engine/text_cases.rs",
            "tests/research_text.rs",
            "tests/engine_text.rs",
        ],
        "orch-gates" => &[
            "src/orch/commands.rs",
            "src/orch/lifecycle.rs",
            "tests/orch/lifecycle_cases.rs",
            "tests/orch_gates.rs",
        ],
        "unit-run" => &[
            "src/bridge/codex_tool.rs",
            "src/goal/mod.rs",
            "tests/bridge/codex_cases.rs",
            "tests/unit_run.rs",
        ],
        _ => &[],
    }
}

fn sections(text: &str) -> Vec<(&str, Vec<&str>)> {
    let mut result: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            result.push((name, Vec::new()));
        } else if let Some((_, lines)) = result.last_mut() {
            lines.push(line);
        }
    }
    result
}

fn check(round5: &str, round4: &str) -> Result<(), String> {
    let r5 = round5.replace("\r\n", "\n");
    let r4 = round4.replace("\r\n", "\n");
    if r5.lines().next() != Some("# 第五輪參考資料採納") {
        return Err("title".into());
    }
    let parts = sections(&r5);
    let expected: Vec<_> = LIBRARIES
        .iter()
        .map(|x| x.0)
        .chain(["第四輪機制重判", "全面優化", "量測"])
        .collect();
    if parts.iter().map(|x| x.0).collect::<Vec<_>>() != expected {
        return Err("sections".into());
    }
    for ((name, lines), (_, dir, head, module)) in parts.iter().take(10).zip(LIBRARIES) {
        let mut values = Vec::new();
        for label in LABELS {
            let found: Vec<_> = lines
                .iter()
                .filter_map(|line| line.strip_prefix(label))
                .collect();
            if found.len() != 1 || found[0].trim().is_empty() {
                return Err(format!("{name}: {label}"));
            }
            values.push(found[0].trim());
        }
        let source = if head.is_empty() {
            dir.to_owned()
        } else {
            format!("{BASE}{dir} HEAD {head}")
        };
        if values[0] != source || !values[1].starts_with("本輪實讀 ") || values[1] == "本輪實讀 "
        {
            return Err(format!("{name}: source or reading"));
        }
        let mut landing = values[4].split_whitespace();
        if landing.next() != Some(module) {
            return Err(format!("{name}: module"));
        }
        let landing: Vec<_> = landing.collect();
        if landing.is_empty()
            || landing
                .iter()
                .any(|p| !paths(module).contains(p) || !Path::new(ROOT).join(p).is_file())
        {
            return Err(format!("{name}: landing"));
        }
        let tests: Vec<_> = values[6].split_whitespace().collect();
        if tests.is_empty() {
            return Err(format!("{name}: tests"));
        }
        for test in tests {
            let (file, function) = test
                .split_once("::")
                .ok_or_else(|| format!("{name}: test format"))?;
            if !file.starts_with("tests/")
                || !file.ends_with(".rs")
                || file.contains('\\')
                || file.split('/').any(|p| p == ".." || p == ".")
                || function.is_empty()
                || !function
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!("{name}: test path"));
            }
            let content = fs::read_to_string(Path::new(ROOT).join(file))
                .map_err(|_| format!("{name}: test file"))?;
            if !content.contains(&format!("fn {function}(")) {
                return Err(format!("{name}: function"));
            }
        }
    }
    let bullets: Vec<_> = parts[10]
        .1
        .iter()
        .filter_map(|s| s.strip_prefix("- "))
        .collect();
    if bullets.len() != MECHANISMS.len() {
        return Err("mechanisms".into());
    }
    for (line, name) in bullets.iter().zip(MECHANISMS) {
        let value = line
            .strip_prefix(&format!("{name}："))
            .ok_or("mechanism name")?;
        if !(value.contains("保留") || value.contains("移除"))
            || !value.contains('，')
            || (name == "Gemini 審查" && !value.contains("移除"))
        {
            return Err("mechanism decision".into());
        }
    }
    let bullets: Vec<_> = parts[11]
        .1
        .iter()
        .filter_map(|s| s.strip_prefix("- "))
        .collect();
    if bullets.len() != FACETS.len() {
        return Err("facets".into());
    }
    for (line, name) in bullets.iter().zip(FACETS) {
        let value = line
            .strip_prefix(&format!("{name}："))
            .ok_or("facet name")?;
        let unchanged = value.split_once("本輪不動").is_some_and(|(_, reason)| {
            !reason
                .trim_matches(|c: char| c.is_whitespace() || c == '，' || c == '。')
                .is_empty()
        });
        let id = (1..=11)
            .filter(|n| *n != 8)
            .any(|n| value.contains(&format!("R5-{n:02}")));
        if !unchanged && !(id && value.contains("前：") && value.contains("後：")) {
            return Err("facet decision".into());
        }
    }
    let measurement = &parts[12].1;
    for phrase in METRICS {
        if measurement.iter().filter(|line| **line == phrase).count() != 1 {
            return Err("metric".into());
        }
    }
    for phrase in [
        "開發證據",
        "人工分類的開發估計",
        "14/167",
        "獨立驗收集",
        "30%",
        "96k→12k",
        "20261102-193323-wish.jsonl",
        "20261103-004231-wish.jsonl",
        "2026-11-02T17:51:35Z",
    ] {
        if !measurement.iter().any(|line| line.contains(phrase)) {
            return Err(format!("measurement {phrase}"));
        }
    }
    let old = sections(&r4);
    for name in [
        "DeepSeek",
        "Orca",
        "Stop That Shit",
        "Codebase Memory",
        "Trail-Code",
        "Agency Agents",
        "ECC",
    ] {
        let lines = &old
            .iter()
            .find(|(section, _)| *section == name)
            .ok_or("round4 section")?
            .1;
        let pointers: Vec<_> = lines
            .iter()
            .filter_map(|line| line.strip_prefix("第五輪重評："))
            .collect();
        if pointers.len() != 1
            || !pointers[0].contains("round5-adoption.zh-TW.md")
            || !pointers[0].contains("來源")
            || !pointers[0].contains("實讀")
        {
            return Err(format!("round4 {name}"));
        }
    }
    Ok(())
}

#[test]
fn adoption_document_and_negative_controls() {
    let r5 = include_str!("../docs/round5-adoption.zh-TW.md").replace("\r\n", "\n");
    let r4 = include_str!("../docs/round4-adoption.zh-TW.md").replace("\r\n", "\n");
    assert!(check(&r5, &r4).is_ok(), "{:?}", check(&r5, &r4));
    assert!(check(&r5.replace('\n', "\r\n"), &r4.replace('\n', "\r\n")).is_ok());
    let swapped = r5
        .replacen(LIBRARIES[0].2, "TEMP_HEAD", 1)
        .replacen(LIBRARIES[1].2, LIBRARIES[0].2, 1)
        .replacen("TEMP_HEAD", LIBRARIES[1].2, 1);
    assert!(check(&swapped, &r4).is_err());
    let reading = r5.lines().find(|l| l.starts_with("閱讀：")).unwrap();
    assert!(check(&r5.replacen(reading, "", 1), &r4).is_err());
    assert!(check(
        &r5.replacen(
            "落點：orch-gates src/orch/lifecycle.rs",
            "落點：orch-gates Cargo.toml",
            1
        ),
        &r4
    )
    .is_err());
    assert!(check(&r5.replacen("落點：orch-gates", "落點：unit-run", 1), &r4).is_err());
    assert!(check(&r5.replacen("## HERO", "## renamed", 1), &r4).is_err());
    assert!(check(&r5.replacen("## HERO\n", "", 1), &r4).is_err());
    assert!(check(
        &r5.replacen(
            "::dependencies_require_verified_unit",
            "::missing_function",
            1
        ),
        &r4
    )
    .is_err());
    assert!(check(&r5.replacen("第四輪：4.13 h", "第四輪：4.14 h", 1), &r4).is_err());
    let pointer = r4.lines().find(|l| l.starts_with("第五輪重評：")).unwrap();
    assert!(check(&r5, &r4.replacen(pointer, "", 1)).is_err());
    for phrase in METRICS {
        assert!(r5.contains(phrase));
        assert!(check(&r5.replacen(phrase, "", 1), &r4).is_err());
    }
    for phrase in [
        "開發證據",
        "人工分類的開發估計",
        "14/167",
        "獨立驗收集",
        "30%",
        "96k→12k",
        "20261102-193323-wish.jsonl",
        "20261103-004231-wish.jsonl",
        "2026-11-02T17:51:35Z",
    ] {
        assert!(sections(&r5)[12].1.iter().any(|line| line.contains(phrase)));
        assert!(check(&r5.replace(phrase, ""), &r4).is_err());
    }
}
