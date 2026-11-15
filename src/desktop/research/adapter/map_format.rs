use crate::{
    desktop::i18n::{translate, Lang, Table},
    research_workspace::{
        repro::{ReproResult, ReproStatus},
        stages::{
            Engineering, GateNotRun, GateResult, ResearchMap, ResearchStatus, Retry, StageState,
            StaleCause, Staleness,
        },
    },
};
const T: Table = &[
    ("Not started", "尚未開始"),
    ("Running", "進行中"),
    ("Paused", "已暫停"),
    ("Blocked", "受阻"),
    ("rate limited", "達到速率限制"),
    ("retry time unknown", "重試時間未知"),
    ("retry at", "重試時間"),
    ("Stalled", "已停滯"),
    ("Done", "已完成"),
    ("Failed", "失敗"),
    ("In progress", "進行中"),
    ("Stale", "已過期"),
    ("Passed", "通過"),
    ("Gate not run", "關卡尚未執行"),
    ("gate artifact", "關卡產物"),
    ("missing or corrupt", "遺失或損毀"),
    ("missing setting", "缺少設定"),
    ("missing gate threshold", "缺少關卡門檻"),
    ("missing gate pointer", "缺少關卡指標"),
    ("gate requires numeric values", "關卡需要數值"),
    ("Gate not declared", "未宣告關卡"),
    ("Gate failed", "關卡失敗"),
    ("Gate passed", "關卡通過"),
    ("Input changed", "輸入已變更"),
    ("Code changed", "程式碼已變更"),
    ("Prompt changed", "提示詞已變更"),
    ("recorded", "記錄值"),
    ("current", "目前值"),
    ("Open outputs", "開啟輸出"),
    ("Repro guard: pass", "重現性檢查：通過"),
    (
        "Repro guard: fail; overlapping groups",
        "重現性檢查：失敗；重疊群組",
    ),
    ("Repro guard: not applicable", "重現性檢查：不適用"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Done,
    Running,
    Waiting,
    Blocked,
    Stale,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapLine {
    pub id: String,
    pub label: String,
    pub state_text: String,
    pub progress_text: Option<String>,
    pub gate_text: String,
    pub stale_text: Vec<String>,
    pub tone: Tone,
}
fn rate_text(retry: &Retry, lang: Lang) -> String {
    match retry {
        Retry::Unknown => format!(
            "{}; {}",
            tr(lang, "rate limited"),
            tr(lang, "retry time unknown")
        ),
        Retry::At(t) => format!(
            "{}; {}: {t}",
            tr(lang, "rate limited"),
            tr(lang, "retry at")
        ),
    }
}
fn state_text(state: &StageState, lang: Lang) -> String {
    match state {
        StageState::NotStarted => tr(lang, "Not started").into(),
        StageState::Running => tr(lang, "Running").into(),
        StageState::Paused { reason } => format!("{}: {reason}", tr(lang, "Paused")),
        StageState::Blocked { reason } => format!("{}: {reason}", tr(lang, "Blocked")),
        StageState::RateLimited { retry } => rate_text(retry, lang),
        StageState::Stalled { since } => format!("{}: {since}", tr(lang, "Stalled")),
        StageState::Done => tr(lang, "Done").into(),
        StageState::Failed { reason } => format!("{}: {reason}", tr(lang, "Failed")),
    }
}
pub(crate) fn status_text(status: &ResearchStatus, lang: Lang) -> String {
    match status {
        ResearchStatus::Passed => tr(lang, "Passed").into(),
        ResearchStatus::InProgress => tr(lang, "In progress").into(),
        ResearchStatus::Paused { reason } => format!("{}: {reason}", tr(lang, "Paused")),
        ResearchStatus::Blocked { reason } => format!("{}: {reason}", tr(lang, "Blocked")),
        ResearchStatus::RateLimited { retry } => rate_text(retry, lang),
        ResearchStatus::Stalled => tr(lang, "Stalled").into(),
        ResearchStatus::Stale => tr(lang, "Stale").into(),
        ResearchStatus::NotStarted => tr(lang, "Not started").into(),
    }
}
pub(crate) fn stale_text(stale: &Staleness, lang: Lang) -> String {
    let (cause, recorded, current) = match &stale.cause {
        StaleCause::InputChanged {
            input,
            recorded,
            current,
        } => (
            format!("{}: {input}", tr(lang, "Input changed")),
            recorded,
            current,
        ),
        StaleCause::CodeChanged { recorded, current } => {
            (tr(lang, "Code changed").to_owned(), recorded, current)
        }
        StaleCause::PromptChanged { recorded, current } => {
            (tr(lang, "Prompt changed").to_owned(), recorded, current)
        }
    };
    format!(
        "{}: {cause}; {}: {recorded}; {}: {current}",
        stale.artifact,
        tr(lang, "recorded"),
        tr(lang, "current")
    )
}
fn gate_not_run_text(reason: &GateNotRun, lang: Lang) -> String {
    match reason {
        GateNotRun::ArtifactUnreadable { artifact } => format!(
            "{} {artifact} {}",
            tr(lang, "gate artifact"),
            tr(lang, "missing or corrupt")
        ),
        GateNotRun::MissingSetting { pointer } => {
            format!("{} {pointer}", tr(lang, "missing setting"))
        }
        GateNotRun::MissingThreshold => tr(lang, "missing gate threshold").into(),
        GateNotRun::MissingPointer { pointer } => {
            format!("{} {pointer}", tr(lang, "missing gate pointer"))
        }
        GateNotRun::NonNumeric => tr(lang, "gate requires numeric values").into(),
    }
}
pub(crate) fn map_lines(map: &ResearchMap, lang: Lang) -> Vec<MapLine> {
    map.stages
        .iter()
        .map(|stage| {
            let tone = match &stage.state {
                StageState::Done => Tone::Done,
                StageState::Running => Tone::Running,
                StageState::NotStarted => Tone::Waiting,
                _ => Tone::Blocked,
            };
            let tone = if !stage.stale.is_empty() && matches!(tone, Tone::Done | Tone::Running) {
                Tone::Stale
            } else {
                tone
            };
            let gate_text = match &stage.gate {
                GateResult::Passed => tr(lang, "Gate passed").into(),
                GateResult::Failed { detail } => format!("{}: {detail}", tr(lang, "Gate failed")),
                GateResult::NotRun { reason } => format!(
                    "{}: {}",
                    tr(lang, "Gate not run"),
                    gate_not_run_text(reason, lang)
                ),
                GateResult::NotDeclared => tr(lang, "Gate not declared").into(),
            };
            MapLine {
                id: stage.id.clone(),
                label: stage.label.clone(),
                state_text: state_text(&stage.state, lang),
                progress_text: stage
                    .progress
                    .as_ref()
                    .map(|p| format!("{}/{}", p.done, p.total)),
                gate_text,
                stale_text: stage.stale.iter().map(|s| stale_text(s, lang)).collect(),
                tone,
            }
        })
        .collect()
}
pub(crate) fn engineering_text(e: &Engineering, lang: Lang) -> String {
    match lang {
        Lang::En => format!(
            "Build work: {} of {} units verified (this is not a research result)",
            e.verified, e.total
        ),
        Lang::ZhTw => format!(
            "建置工作：{} / {} 個單位已驗證（這不是研究結果）",
            e.verified, e.total
        ),
    }
}
pub(crate) fn repro_text(repro: &ReproResult, lang: Lang) -> String {
    match &repro.status {
        ReproStatus::Pass => tr(lang, "Repro guard: pass").into(),
        ReproStatus::Fail { overlap } => format!(
            "{}: {}",
            tr(lang, "Repro guard: fail; overlapping groups"),
            overlap.join(", ")
        ),
        ReproStatus::NotApplicable { reason } => {
            format!("{}: {reason}", tr(lang, "Repro guard: not applicable"))
        }
    }
}
