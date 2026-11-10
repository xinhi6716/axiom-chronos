//! The command palette (Ctrl+K), the shortcuts overlay (?) and the one table every keyboard
//! shortcut is read from. Matching is pure; a large index is searched off the main thread, so
//! typing never waits on it.
use super::control::offer;
use super::i18n::{fill, tr, translate, Lang, Table};
use super::model::*;
use super::motion::{self, Driver, Presence};
use super::ornament;
use super::theme::{Tokened as _, Tokens};
use super::wishes::{can_resume, one_line};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MovePageDown, MovePageUp, MoveUp};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, StyledExt as _};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::ops::Range;
use std::sync::Arc;

gpui_kit::actions!(chronos, [NewWish, OpenPalette, ShowShortcuts]);

pub(super) const T: Table = &[
    ("Search wishes, goals, modules, units and memory, or run a command", "搜尋願望、目標、模組、單元與記憶，或執行指令"),
    ("Command", "指令"),
    ("Wish", "願望"),
    ("Goal", "目標"),
    ("Module", "模組"),
    ("Unit", "單元"),
    ("Make a new wish", "許一個新願望"),
    ("Go to Run", "前往執行"),
    ("Go to Wishes", "前往願望"),
    ("Go to Memory", "前往記憶"),
    ("Go to Usage", "前往用量"),
    ("Go to Settings", "前往設定"),
    ("Switch to the light theme", "改用淺色主題"),
    ("Switch to the dark theme", "改用深色主題"),
    ("Show the interface in English", "介面改用英文（English）"),
    ("Show the interface in Traditional Chinese (繁體中文)", "介面改用繁體中文"),
    ("Stop the current wish", "停止目前的願望"),
    ("Resume the current wish", "接續目前的願望"),
    ("Resume the current wish now", "現在就接續目前的願望"),
    ("Keyboard shortcuts", "鍵盤快捷鍵"),
    ("Wishes · {}", "願望 · {}"),
    ("{} › Goals", "{} › 目標"),
    ("{} › Modules", "{} › 模組"),
    ("{} › {}", "{} › {}"),
    ("Memory · {}", "記憶 · {}"),
    ("Nothing matches “{}”.", "沒有符合「{}」的項目。"),
    ("Try fewer letters, or part of a word.", "試著少打幾個字，或只打詞的一部分。"),
    ("{} results", "{} 個結果"),
    // Areas of the shortcut table.
    ("Anywhere", "任何地方"),
    ("Command palette", "指令面板"),
    ("New wish dialog", "許願對話框"),
    ("Wishes and Memory lists", "願望與記憶清單"),
    ("Checklist", "檢查清單"),
    ("Event log", "事件紀錄"),
    ("Message to the run", "給這次執行的訊息"),
    // What each shortcut does.
    ("Search and run commands", "搜尋與執行指令"),
    ("Show keyboard shortcuts", "顯示鍵盤快捷鍵"),
    ("Move between controls", "在控制項之間移動"),
    ("Move through the results", "在結果之間移動"),
    ("Move a page at a time", "一次移動一頁"),
    ("Go to the selected result", "前往選取的結果"),
    ("Close", "關閉"),
    ("Start the wish", "開始這個願望"),
    ("New line", "換行"),
    ("Point at a file or folder in the project", "指向專案裡的檔案或資料夾"),
    ("Choose in the file list", "在檔案清單中選擇"),
    ("Insert the chosen path", "插入選取的路徑"),
    ("Close the file list", "關閉檔案清單"),
    ("Close the dialog (asks before losing text)", "關閉對話框（有內容時會先詢問）"),
    ("File list after @", "輸入 @ 後的檔案清單"),
    ("Search the list", "搜尋清單"),
    ("From the search box, move into the list", "從搜尋框移到清單"),
    ("First or last row", "第一列或最後一列"),
    ("Delete the selected memory (asks first)", "刪除選取的記憶（會先詢問）"),
    ("Move through the checklist", "在檢查清單中移動"),
    ("Fold or unfold", "收合或展開"),
    ("Select it everywhere", "在各處選取這一項"),
    ("First or last item", "第一項或最後一項"),
    ("Clear the selection", "清除選取"),
    ("Scroll a line", "捲動一行"),
    ("Scroll a page", "捲動一頁"),
    ("Go to the oldest line", "跳到最早的一行"),
    ("Follow new lines", "跟隨最新的紀錄"),
    ("Close the log", "關閉紀錄"),
    ("Send", "送出"),
];

// ---------------------------------------------------------------------------------------------
// The shortcut table
// ---------------------------------------------------------------------------------------------

/// Where a shortcut works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Area {
    Anywhere,
    Palette,
    NewWish,
    Lists,
    Checklist,
    Log,
    Message,
    Files,
}

impl Area {
    pub(super) const ALL: [Area; 8] =
        [Area::Anywhere, Area::Palette, Area::NewWish, Area::Files, Area::Message, Area::Lists, Area::Checklist, Area::Log];

    pub(super) fn title(self) -> &'static str {
        match self {
            Area::Anywhere => "Anywhere",
            Area::Palette => "Command palette",
            Area::NewWish => "New wish dialog",
            Area::Lists => "Wishes and Memory lists",
            Area::Checklist => "Checklist",
            Area::Log => "Event log",
            Area::Message => "Message to the run",
            Area::Files => "File list after @",
        }
    }
}

/// Shortcuts the shell binds itself, for the whole window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Global {
    Palette,
    NewWish,
    Shortcuts,
}

pub(super) struct Shortcut {
    pub area: Area,
    /// Alternatives, each in gpui's keystroke syntax.
    pub keys: &'static [&'static str],
    pub does: &'static str,
    /// Set when the shell binds these keys from this row, so the overlay and the binding agree.
    pub global: Option<Global>,
}

/// The platform's command modifier: Ctrl, or Cmd on macOS.
const fn primary(other: &'static str, mac: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

const fn row(area: Area, keys: &'static [&'static str], does: &'static str) -> Shortcut {
    Shortcut { area, keys, does, global: None }
}

/// Every keyboard shortcut the app has. The global ones are bound from here; the tests read the
/// sources of every screen and fail when a key handled there is missing from this table.
pub(super) const SHORTCUTS: &[Shortcut] = &[
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-k", "cmd-k")], does: "Search and run commands", global: Some(Global::Palette) },
    Shortcut { area: Area::Anywhere, keys: &[primary("ctrl-n", "cmd-n")], does: "Make a new wish", global: Some(Global::NewWish) },
    Shortcut { area: Area::Anywhere, keys: &["?"], does: "Show keyboard shortcuts", global: Some(Global::Shortcuts) },
    row(Area::Anywhere, &["tab", "shift-tab"], "Move between controls"),
    row(Area::Palette, &["up", "down"], "Move through the results"),
    row(Area::Palette, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Palette, &["enter"], "Go to the selected result"),
    row(Area::Palette, &["escape"], "Close"),
    row(Area::NewWish, &[primary("ctrl-enter", "cmd-enter")], "Start the wish"),
    row(Area::NewWish, &["enter"], "New line"),
    row(Area::NewWish, &["@"], "Point at a file or folder in the project"),
    row(Area::NewWish, &["escape"], "Close the dialog (asks before losing text)"),
    row(Area::Files, &["up", "down"], "Choose in the file list"),
    row(Area::Files, &["enter", "tab"], "Insert the chosen path"),
    row(Area::Files, &["escape"], "Close the file list"),
    row(Area::Message, &["enter"], "Send"),
    row(Area::Message, &["shift-enter"], "New line"),
    row(Area::Lists, &[primary("ctrl-f", "cmd-f")], "Search the list"),
    row(Area::Lists, &["down"], "From the search box, move into the list"),
    row(Area::Lists, &["home", "end"], "First or last row"),
    row(Area::Lists, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Lists, &["delete"], "Delete the selected memory (asks first)"),
    row(Area::Checklist, &["up", "down"], "Move through the checklist"),
    row(Area::Checklist, &["left", "right"], "Fold or unfold"),
    row(Area::Checklist, &["enter", "space"], "Select it everywhere"),
    row(Area::Checklist, &["home", "end"], "First or last item"),
    row(Area::Checklist, &["pageup", "pagedown"], "Move a page at a time"),
    row(Area::Checklist, &["escape"], "Clear the selection"),
    row(Area::Log, &["up", "down"], "Scroll a line"),
    row(Area::Log, &["pageup", "pagedown"], "Scroll a page"),
    row(Area::Log, &["home"], "Go to the oldest line"),
    row(Area::Log, &["end"], "Follow new lines"),
    row(Area::Log, &["escape"], "Close the log"),
];

/// The key context in which `?` opens the overlay: anywhere but a text box, where it is typed.
const OUTSIDE_TEXT: &str = "!Input";

/// The window-wide bindings, made from the table's global rows.
pub(super) fn global_bindings() -> Vec<KeyBinding> {
    SHORTCUTS
        .iter()
        .filter_map(|shortcut| Some((shortcut.global?, shortcut.keys)))
        .flat_map(|(global, keys)| {
            keys.iter().map(move |&keys| match global {
                Global::Palette => KeyBinding::new(keys, OpenPalette, None),
                Global::NewWish => KeyBinding::new(keys, NewWish, None),
                Global::Shortcuts => KeyBinding::new(keys, ShowShortcuts, Some(OUTSIDE_TEXT)),
            })
        })
        .collect()
}

/// The first keys of a global shortcut, to show beside the command it runs.
pub(super) fn keys_of(global: Global) -> &'static str {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.global == Some(global))
        .and_then(|shortcut| shortcut.keys.first().copied())
        .unwrap_or_default()
}

pub(super) fn in_area(area: Area) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS.iter().filter(move |shortcut| shortcut.area == area)
}

/// A key cap. Keys that do not parse never reach here: the tests parse every row.
pub(super) fn key_cap(keys: &'static str) -> impl IntoElement {
    let stroke = Keystroke::parse(keys).unwrap_or_else(|_| Keystroke { key: keys.to_owned(), ..Default::default() });
    Kbd::new(stroke)
}

// ---------------------------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------------------------

const SCORE_MATCH: i32 = 16;
const BONUS_START: i32 = 10;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CAMEL: i32 = 6;
const BONUS_CONSECUTIVE: i32 = 10;
const BONUS_WHOLE: i32 = 24;
const GAP_START: i32 = 3;
const GAP_EXTEND: i32 = 2;
const GAP_CAP: usize = 24;
const LEAD_CAP: usize = 30;
/// A match scoring less than this per query character is noise: letters strewn across the text.
const MIN_PER_CHAR: i32 = 7;
/// How many places the first query character may start a scattered match from.
const MAX_STARTS: usize = 24;

/// A query matched against one text: higher scores are better matches, and `positions` holds the
/// byte offset of every matched character, for highlighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '_' | '-' | '/' | '\\' | '.' | ':' | ',' | '(' | ')' | '[' | ']' | '·' | '›' | '，' | '。' | '、' | '：' | '「' | '」')
}

/// How strongly a character at this place starts a word.
fn boundary(prev: Option<char>, cur: char) -> i32 {
    match prev {
        None => BONUS_START,
        Some(p) if is_separator(p) || !p.is_alphanumeric() => BONUS_BOUNDARY,
        Some(p) if p.is_lowercase() && cur.is_uppercase() => BONUS_CAMEL,
        // A change of script, as from Latin letters to Chinese, also starts a word.
        Some(p) if p.is_ascii() != cur.is_ascii() => BONUS_CAMEL,
        _ => 0,
    }
}

fn score_positions(chars: &[char], idx: &[usize]) -> i32 {
    let mut score = 0;
    for (k, &i) in idx.iter().enumerate() {
        let bonus = boundary(i.checked_sub(1).map(|p| chars[p]), chars[i]);
        score += SCORE_MATCH + if k == 0 { bonus * 2 } else { bonus };
        if k > 0 {
            let gap = i - idx[k - 1] - 1;
            score += if gap == 0 { BONUS_CONSECUTIVE } else { -GAP_START - gap.min(GAP_CAP) as i32 * GAP_EXTEND };
        }
    }
    score -= idx[0].min(LEAD_CAP) as i32 / 2;
    if idx.len() == chars.len() {
        score += BONUS_WHOLE;
    }
    score
}

/// Matches `query` against `text`, ignoring case: every query character must appear in order.
/// Contiguous runs, word starts and early matches score higher; scattered letters do not match.
pub(super) fn fuzzy(query: &str, text: &str) -> Option<Match> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).map(fold).collect();
    if q.is_empty() || text.is_empty() {
        return None;
    }
    // Rejects most texts before anything is allocated.
    let mut need = 0;
    for c in text.chars() {
        if need < q.len() && fold(c) == q[need] {
            need += 1;
        }
    }
    if need < q.len() {
        return None;
    }
    let indexed: Vec<(usize, char)> = text.char_indices().collect();
    let chars: Vec<char> = indexed.iter().map(|&(_, c)| c).collect();
    let lower: Vec<char> = chars.iter().map(|&c| fold(c)).collect();
    let (m, n) = (q.len(), lower.len());
    let mut best: Option<(i32, Vec<usize>)> = None;
    let mut consider = |idx: Vec<usize>| {
        let score = score_positions(&chars, &idx);
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, idx));
        }
    };
    let mut contiguous = false;
    if m <= n {
        for s in 0..=n - m {
            if lower[s..s + m] == q[..] {
                contiguous = true;
                consider((s..s + m).collect());
            }
        }
    }
    if !contiguous {
        let mut starts = 0;
        for s in 0..n {
            if lower[s] != q[0] {
                continue;
            }
            starts += 1;
            if starts > MAX_STARTS {
                break;
            }
            let mut k = 0;
            let mut end = None;
            for (i, &c) in lower.iter().enumerate().skip(s) {
                if c == q[k] {
                    k += 1;
                    if k == m {
                        end = Some(i);
                        break;
                    }
                }
            }
            // No full match from here means none from any later start either.
            let Some(end) = end else { break };
            // Walking back from the end finds the tightest window that still holds the query.
            let mut idx = Vec::with_capacity(m);
            let mut k = m;
            for i in (s..=end).rev() {
                if lower[i] == q[k - 1] {
                    idx.push(i);
                    k -= 1;
                    if k == 0 {
                        break;
                    }
                }
            }
            idx.reverse();
            consider(idx);
        }
    }
    let (score, idx) = best?;
    if score < m as i32 * MIN_PER_CHAR {
        return None;
    }
    Some(Match { score, positions: idx.into_iter().map(|i| indexed[i].0).collect() })
}

/// Byte ranges to highlight, from the byte offsets of matched characters.
pub(super) fn mark_ranges(text: &str, positions: &[usize]) -> Vec<Range<usize>> {
    let mut sorted = positions.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<Range<usize>> = vec![];
    for at in sorted {
        let Some(c) = text.get(at..).and_then(|rest| rest.chars().next()) else { continue };
        let range = at..at + c.len_utf8();
        match ranges.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => ranges.push(range),
        }
    }
    ranges
}

/// `text` with its matched characters drawn stronger.
pub(super) fn marked(text: SharedString, positions: &[usize], strong: Hsla) -> StyledText {
    let style = HighlightStyle { color: Some(strong), font_weight: Some(FontWeight::BOLD), ..Default::default() };
    let ranges = mark_ranges(&text, positions);
    StyledText::new(text).with_highlights(ranges.into_iter().map(|range| (range, style)))
}

// ---------------------------------------------------------------------------------------------
// What the palette searches
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    NewWish,
    Open(Screen),
    ToggleTheme,
    ToggleLanguage,
    Stop,
    Resume,
    Shortcuts,
}

/// Where choosing a result goes. Runs, modules and units are named by id, so a refresh that
/// reorders them between the search and the choice still lands on the right one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Command(Command),
    Run(SharedString),
    Goal(SharedString, SharedString),
    Module(SharedString, SharedString),
    Unit(SharedString, SharedString),
    Memory(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Command,
    Wish,
    Goal,
    Module,
    Unit,
    Memory,
}

impl Kind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Kind::Command => "Command",
            Kind::Wish => "Wish",
            Kind::Goal => "Goal",
            Kind::Module => "Module",
            Kind::Unit => "Unit",
            Kind::Memory => "Memory",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Item {
    pub kind: Kind,
    pub title: SharedString,
    /// Words shown beside the title, searched almost as strongly: an id or a purpose.
    pub detail: SharedString,
    /// Where the item lives, shown at the right.
    pub place: SharedString,
    /// Searched only for whole words or parts of them, never for strewn letters.
    pub keywords: SharedString,
    pub target: Target,
    /// Belongs to the run on screen, which ranks it a little higher.
    pub current: bool,
    pub unit_state: Option<UnitState>,
    /// A run's status and whether it ended verified.
    pub status: Option<(RunStatus, bool)>,
    pub keys: Option<&'static str>,
    pub icon: Option<IconKind>,
}

/// The icons rows may carry. Kept as plain data so items can cross to the background search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IconKind {
    Plus,
    Run,
    Wishes,
    Memory,
    Usage,
    Settings,
    Sun,
    Moon,
    Globe,
    Stop,
    Play,
    Keys,
    Goal,
    Module,
}

impl IconKind {
    fn name(self) -> IconName {
        match self {
            IconKind::Plus => IconName::Plus,
            IconKind::Run => IconName::LayoutDashboard,
            IconKind::Wishes => IconName::Inbox,
            IconKind::Memory => IconName::BookOpen,
            IconKind::Usage => IconName::ChartPie,
            IconKind::Settings => IconName::Settings,
            IconKind::Sun => IconName::Sun,
            IconKind::Moon => IconName::Moon,
            IconKind::Globe => IconName::Globe,
            IconKind::Stop => IconName::Square,
            IconKind::Play => IconName::Play,
            IconKind::Keys => IconName::Asterisk,
            IconKind::Goal => IconName::Star,
            IconKind::Module => IconName::Network,
        }
    }
}

fn command(title: &'static str, target: Command, icon: IconKind, keys: Option<&'static str>, lang: Lang) -> Item {
    Item {
        kind: Kind::Command,
        title: translate(lang, T, title).into(),
        detail: SharedString::default(),
        place: SharedString::default(),
        // The English words stay searchable in Chinese, so either language finds a command.
        keywords: title.into(),
        target: Target::Command(target),
        current: false,
        unit_state: None,
        status: None,
        keys,
        icon: Some(icon),
    }
}

/// The module whose selection stands for a goal: the first that is not yet verified, else the
/// first. A goal has no selection of its own.
pub(super) fn goal_lead(run: &Run, goal: &GoalItem) -> Option<usize> {
    let open = |&&m: &&usize| run.modules.get(m).is_some_and(|module| run.module_done(module) < module.units.len());
    goal.modules.iter().find(open).or(goal.modules.first()).copied().filter(|&m| m < run.modules.len())
}

/// Everything the palette can find, in the order it is listed before anything is typed.
pub(super) fn index(state: &AppState, lang: Lang, dark: bool) -> Vec<Item> {
    let t = |en: &'static str| translate(lang, T, en);
    let mut items = vec![command("Make a new wish", Command::NewWish, IconKind::Plus, Some(keys_of(Global::NewWish)), lang)];
    if let Some(run) = state.run() {
        let can = offer(run.status, run.hosted, run.cwd.is_some(), run.report.is_some());
        let wish = one_line(&SharedString::from(clean_wish(&run.wish)));
        if can.stop {
            let mut stop = command("Stop the current wish", Command::Stop, IconKind::Stop, None, lang);
            stop.detail = wish.clone();
            items.push(stop);
        }
        if can_resume(run.status) {
            let title = if run.status == RunStatus::Waiting { "Resume the current wish now" } else { "Resume the current wish" };
            let mut resume = command(title, Command::Resume, IconKind::Play, None, lang);
            resume.detail = wish;
            items.push(resume);
        }
    }
    for (title, screen, icon) in [
        ("Go to Run", Screen::Run, IconKind::Run),
        ("Go to Wishes", Screen::Wishes, IconKind::Wishes),
        ("Go to Memory", Screen::Memory, IconKind::Memory),
        ("Go to Usage", Screen::Usage, IconKind::Usage),
        ("Go to Settings", Screen::Doctor, IconKind::Settings),
    ] {
        items.push(command(title, Command::Open(screen), icon, None, lang));
    }
    items.push(if dark {
        command("Switch to the light theme", Command::ToggleTheme, IconKind::Sun, None, lang)
    } else {
        command("Switch to the dark theme", Command::ToggleTheme, IconKind::Moon, None, lang)
    });
    items.push(match lang {
        Lang::ZhTw => command("Show the interface in English", Command::ToggleLanguage, IconKind::Globe, None, lang),
        Lang::En => command("Show the interface in Traditional Chinese (繁體中文)", Command::ToggleLanguage, IconKind::Globe, None, lang),
    });
    items.push(command("Keyboard shortcuts", Command::Shortcuts, IconKind::Keys, Some(keys_of(Global::Shortcuts)), lang));

    for (i, run) in state.runs.iter().enumerate() {
        let current = state.current == Some(i);
        let project = run.project.as_ref();
        let session = run.session.clone();
        items.push(Item {
            kind: Kind::Wish,
            title: one_line(&SharedString::from(clean_wish(&run.wish))),
            detail: SharedString::default(),
            place: fill(t("Wishes · {}"), &[project]).into(),
            keywords: format!("{project} {}", translate(lang, T, run.status_label())).into(),
            target: Target::Run(session.clone()),
            current,
            unit_state: None,
            status: Some((run.status, run.verified_end())),
            keys: None,
            icon: None,
        });
        for goal in &run.goals {
            items.push(Item {
                kind: Kind::Goal,
                title: goal.title.clone(),
                detail: goal.id.clone(),
                place: fill(t("{} › Goals"), &[project]).into(),
                keywords: SharedString::default(),
                target: Target::Goal(session.clone(), goal.id.clone()),
                current,
                unit_state: None,
                status: None,
                keys: None,
                icon: Some(IconKind::Goal),
            });
        }
        for module in &run.modules {
            items.push(Item {
                kind: Kind::Module,
                title: module.id.clone(),
                detail: module.purpose.clone(),
                place: fill(t("{} › Modules"), &[project]).into(),
                keywords: SharedString::default(),
                target: Target::Module(session.clone(), module.id.clone()),
                current,
                unit_state: None,
                status: None,
                keys: None,
                icon: Some(IconKind::Module),
            });
        }
        for unit in &run.units {
            let module = run.modules.get(unit.module).map_or("", |m| m.id.as_ref());
            items.push(Item {
                kind: Kind::Unit,
                title: unit.title.clone(),
                detail: if unit.title == unit.id { SharedString::default() } else { unit.id.clone() },
                place: fill(t("{} › {}"), &[project, module]).into(),
                keywords: SharedString::default(),
                target: Target::Unit(session.clone(), unit.id.clone()),
                current,
                unit_state: Some(unit.state),
                status: None,
                keys: None,
                icon: None,
            });
        }
    }
    for entry in &state.memory {
        items.push(Item {
            kind: Kind::Memory,
            title: entry.title.clone(),
            detail: SharedString::default(),
            place: fill(t("Memory · {}"), &[entry.project.as_ref()]).into(),
            keywords: entry.body.clone(),
            target: Target::Memory(entry.id),
            current: false,
            unit_state: None,
            status: None,
            keys: None,
            icon: Some(IconKind::Memory),
        });
    }
    items
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub item: usize,
    pub score: i32,
    pub title_marks: Vec<usize>,
    pub detail_marks: Vec<usize>,
}

/// How many results are kept for one query.
pub(super) const LIMIT: usize = 60;
/// Wishes listed before anything is typed.
const RECENT_WISHES: usize = 8;
const CURRENT_BONUS: i32 = 6;
const KEYWORD_PER_CHAR: i32 = 6;
/// Up to this many items are searched on the main thread: well under a millisecond.
pub(super) const SYNC_LIMIT: usize = 1500;

fn literal(term: &str, text: &str) -> Option<i32> {
    let found = if text.is_ascii() && term.is_ascii() {
        text.to_ascii_lowercase().contains(&term.to_ascii_lowercase())
    } else {
        text.to_lowercase().contains(&term.to_lowercase())
    };
    found.then(|| term.chars().count() as i32 * KEYWORD_PER_CHAR)
}

fn score_item(item: &Item, terms: &[&str]) -> Option<Hit> {
    let mut hit = Hit { item: 0, score: 0, title_marks: vec![], detail_marks: vec![] };
    for term in terms {
        let title = fuzzy(term, &item.title);
        let detail = fuzzy(term, &item.detail).map(|m| Match { score: m.score * 3 / 4, ..m });
        match (title, detail) {
            (Some(t), Some(d)) if d.score > t.score => {
                hit.score += d.score;
                hit.detail_marks.extend(d.positions);
            }
            (Some(t), _) => {
                hit.score += t.score;
                hit.title_marks.extend(t.positions);
            }
            (None, Some(d)) => {
                hit.score += d.score;
                hit.detail_marks.extend(d.positions);
            }
            (None, None) => hit.score += literal(term, &item.place).or_else(|| literal(term, &item.keywords))?,
        }
    }
    if item.current {
        hit.score += CURRENT_BONUS;
    }
    Some(hit)
}

/// The results for `query`, best first. Every word of the query must be found. Equal scores keep
/// the index order, so results never shuffle between keystrokes. An empty query lists the
/// commands and the most recent wishes.
pub(super) fn search(items: &[Item], query: &str, limit: usize) -> Vec<Hit> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.is_empty() {
        let mut wishes = 0;
        return items
            .iter()
            .enumerate()
            .filter(|(_, item)| match item.kind {
                Kind::Command => true,
                Kind::Wish => {
                    wishes += 1;
                    wishes <= RECENT_WISHES
                }
                _ => false,
            })
            .take(limit)
            .map(|(item, _)| Hit { item, score: 0, title_marks: vec![], detail_marks: vec![] })
            .collect();
    }
    let mut hits: Vec<Hit> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| score_item(item, &terms).map(|hit| Hit { item: i, ..hit }))
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.item.cmp(&b.item)));
    hits.truncate(limit);
    hits
}

// ---------------------------------------------------------------------------------------------
// The overlay
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Overlay {
    Closed,
    Search,
    Shortcuts,
}

pub(super) enum PaletteEvent {
    Chose(Target),
}

const ROW_H: f32 = 40.;
/// Room for the kind word: two Chinese characters, or "Command".
const KIND_W: f32 = 58.;
const SHORTCUTS_TOP: f32 = 48.;
/// The overlay's frame, padding and title above and below its list.
const SHORTCUTS_CHROME: f32 = 150.;
const PAGE: isize = 8;

pub struct Palette {
    state: Entity<AppState>,
    query: Entity<InputState>,
    overlay: Overlay,
    items: Arc<Vec<Item>>,
    hits: Vec<Hit>,
    /// The query the shown results answer.
    answered: String,
    selected: usize,
    scroll: ScrollHandle,
    searching: Option<Task<()>>,
    focus: FocusHandle,
    /// Where the keyboard was before the overlay opened, to give it back on close.
    restore: Option<FocusHandle>,
    /// The selected result while the index is rebuilt, so a refresh does not move the selection.
    keep: Option<Target>,
    /// Opens with the soft spring and leaves quickly; while leaving, `ghost` is what is drawn.
    presence: Presence,
    ghost: Overlay,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            // The input reports Change only for committed text, so an IME word being composed never searches.
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh(cx);
                }
            }),
            cx.observe_in(&state, window, |this, _, _, cx| {
                if this.overlay == Overlay::Search {
                    this.reindex(cx);
                }
            }),
        ];
        Self {
            state,
            query,
            overlay: Overlay::Closed,
            items: Arc::new(vec![]),
            hits: vec![],
            answered: String::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            searching: None,
            focus: cx.focus_handle(),
            restore: None,
            keep: None,
            presence: Presence::default(),
            ghost: Overlay::Closed,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn overlay(&self) -> Overlay {
        self.overlay
    }

    fn remember_focus(&mut self, window: &Window, cx: &App) {
        if self.overlay == Overlay::Closed {
            self.restore = window.focused(cx);
        }
    }

    pub(super) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_focus(window, cx);
        self.overlay = Overlay::Search;
        self.ghost = Overlay::Search;
        self.presence.show(motion::now(cx));
        let placeholder = tr(cx, T, "Search wishes, goals, modules, units and memory, or run a command");
        self.query.update(cx, |query, cx| {
            query.set_value("", window, cx);
            query.set_placeholder(placeholder, window, cx);
            query.focus(window, cx);
        });
        self.answered.clear();
        self.selected = 0;
        self.reindex(cx);
        cx.notify();
    }

    pub(super) fn show_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_focus(window, cx);
        self.overlay = Overlay::Shortcuts;
        self.ghost = Overlay::Shortcuts;
        self.presence.show(motion::now(cx));
        self.searching = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::Closed {
            return;
        }
        self.overlay = Overlay::Closed;
        self.presence.hide(motion::now(cx));
        self.searching = None;
        if let Some(handle) = self.restore.take() {
            handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Demo only: types a query as a person would.
    pub(super) fn type_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |query, cx| query.set_value(text.to_owned(), window, cx));
        self.refresh(cx);
    }

    /// Demo only: presses Enter on the selected result.
    pub(super) fn choose_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose(self.selected, window, cx);
    }

    fn reindex(&mut self, cx: &mut Context<Self>) {
        let lang = *cx.global::<Lang>();
        let dark = cx.theme().is_dark();
        self.keep = self.selected_target();
        self.items = Arc::new(index(self.state.read(cx), lang, dark));
        self.run_search(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.run_search(cx);
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).value().to_string();
        let items = self.items.clone();
        if items.len() <= SYNC_LIMIT {
            let hits = search(&items, &text, LIMIT);
            self.show(hits, text, cx);
            return;
        }
        // Dropping the previous task cancels it, so only the newest query is answered.
        self.searching = Some(cx.spawn(async move |this, cx| {
            let query = text.clone();
            let hits = cx.background_spawn(async move { search(&items, &query, LIMIT) }).await;
            let _ = this.update(cx, |this, cx| this.show(hits, text, cx));
        }));
    }

    /// Takes new results. A refresh of the same query keeps the selected result where it is.
    fn show(&mut self, hits: Vec<Hit>, query: String, cx: &mut Context<Self>) {
        let kept = self.keep.take().filter(|_| query == self.answered);
        self.hits = hits;
        self.selected = kept
            .and_then(|target| self.hits.iter().position(|hit| self.items.get(hit.item).is_some_and(|item| item.target == target)))
            .unwrap_or(0);
        self.answered = query;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn items_target(&self, item: usize) -> Option<Target> {
        self.items.get(item).map(|item| item.target.clone())
    }

    fn selected_target(&self) -> Option<Target> {
        self.hits.get(self.selected).and_then(|hit| self.items_target(hit.item))
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.hits.len() as isize;
        if count == 0 {
            return;
        }
        let next = self.selected as isize + by;
        self.selected = if by.abs() == 1 { next.rem_euclid(count) } else { next.clamp(0, count - 1) } as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn choose(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.hits.get(ix).and_then(|hit| self.items_target(hit.item)) else { return };
        self.close(window, cx);
        cx.emit(PaletteEvent::Chose(target));
    }

    /// The dimmed window behind an overlay. While the overlay leaves it is only drawn: it takes no
    /// clicks and has its own id, so nothing mistakes it for an open overlay.
    fn scrim(&self, id: &'static str, level: f32, live: bool, t: &Tokens, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(if live { id } else { "overlay-leaving" })
            .absolute()
            .inset_0()
            .bg(t.canvas.opacity(0.72 * level.clamp(0.0, 1.0)))
            .flex()
            .flex_col()
            .items_center()
            .when(live, |this| {
                this.occlude().on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close(window, cx)))
            })
    }

    /// The overlay's panel as it opens or leaves: it fades and settles down into place.
    fn panel_motion(panel: Div, level: f32) -> Div {
        panel.relative().top(px(-14.0 * (1.0 - level))).opacity(level.clamp(0.0, 1.0))
    }

    fn finish(element: Stateful<Div>, live: bool) -> AnyElement {
        // Lets the headless keyboard tests see the overlay; nothing without `test-support`.
        if live {
            element.test_support().into_any_element()
        } else {
            element.into_any_element()
        }
    }

    fn result_row(&self, ix: usize, hit: &Hit, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let item = &self.items[hit.item];
        let selected = ix == self.selected;
        let mark = if selected { t.gold } else { t.text };
        let title_color = if selected { t.gold } else { t.text };
        let lead: AnyElement = match (item.unit_state, item.status, item.icon) {
            (Some(state), _, _) => ornament::glyph(state, 14., t.unit(state), 0.0).into_any_element(),
            (_, Some((status, verified)), _) => div().size(px(7.)).rounded_full().bg(t.outcome(status, verified)).into_any_element(),
            (_, _, Some(icon)) => Icon::new(icon.name()).size(px(14.)).text_color(t.muted).into_any_element(),
            _ => div().into_any_element(),
        };
        h_flex()
            .id(("palette-row", ix))
            .relative()
            .flex_none()
            .h(px(ROW_H))
            .px_3()
            .gap_3()
            .rounded(px(3.))
            .cursor_pointer()
            .when(selected, |this| this.bg(t.raised))
            .hover(|this| this.bg(t.raised))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(ix, window, cx)))
            .when(selected, |this| {
                this.child(div().absolute().left_0().top(px(9.)).w(px(2.)).h(px(22.)).bg(t.gold))
            })
            .child(div().w(px(16.)).flex_none().flex().justify_center().child(lead))
            .child(
                div()
                    .w(px(KIND_W))
                    .flex_none()
                    .text_xs()
                    .text_color(t.muted)
                    .child(tr(cx, T, item.kind.label())),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink(0.1)
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(title_color)
                            .child(marked(item.title.clone(), &hit.title_marks, mark)),
                    )
                    .when(!item.detail.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w(px(40.))
                                .truncate()
                                .text_xs()
                                .text_color(t.muted)
                                .child(marked(item.detail.clone(), &hit.detail_marks, t.text)),
                        )
                    }),
            )
            .when(!item.place.is_empty(), |this| {
                this.child(
                    div()
                        .max_w(px(210.))
                        .flex_none()
                        .truncate()
                        .text_xs()
                        .text_color(t.muted)
                        .child(item.place.clone()),
                )
            })
            .when_some(item.keys, |this, keys| this.child(div().flex_none().child(key_cap(keys))))
    }

    /// The palette's own keys, from the table. Paging is left to the overlay, to keep one line.
    fn hints(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let shown = in_area(Area::Palette).filter(|shortcut| !shortcut.keys.contains(&"pageup"));
        h_flex().gap_3().flex_wrap().children(shown.map(|shortcut| {
            h_flex()
                .gap_1()
                .children(shortcut.keys.iter().map(|&keys| key_cap(keys)))
                .child(div().pl_1().text_xs().text_color(t.muted).child(tr(cx, T, shortcut.does)))
        }))
    }

    fn render_search(&mut self, level: f32, live: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        let rows: Vec<AnyElement> = self
            .hits
            .clone()
            .iter()
            .enumerate()
            .map(|(ix, hit)| self.result_row(ix, hit, &t, cx).into_any_element())
            .collect();
        let query = self.answered.trim().to_owned();
        let count = fill(tr(cx, T, "{} results"), &[&self.hits.len().to_string()]);
        let body: AnyElement = if rows.is_empty() {
            v_flex()
                .py_6()
                .gap_1()
                .items_center()
                .child(div().text_sm().text_color(t.text).child(fill(tr(cx, T, "Nothing matches “{}”."), &[&query])))
                .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Try fewer letters, or part of a word.")))
                .into_any_element()
        } else {
            div()
                .id("palette-results")
                .max_h(px(ROW_H * 10.5))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .py_1()
                .children(rows)
                .into_any_element()
        };
        let shortcuts_keys = keys_of(Global::Shortcuts);
        let scrim = self.scrim("palette-scrim", level, live, &t, cx)
            .child(
                Self::panel_motion(div(), level)
                    .mt(px(72.))
                    .w(px(780.))
                    .max_w(relative(0.92))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        ornament::frame(&t, t.border, t.surface)
                            .p(px(10.))
                            .child(
                                v_flex()
                                    .p(px(4.))
                                    .gap_1()
                                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                                        cx.stop_propagation();
                                        this.step(-1, cx);
                                    }))
                                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                                        cx.stop_propagation();
                                        this.step(1, cx);
                                    }))
                                    .capture_action(cx.listener(|this, _: &MovePageUp, _, cx| {
                                        cx.stop_propagation();
                                        this.step(-PAGE, cx);
                                    }))
                                    .capture_action(cx.listener(|this, _: &MovePageDown, _, cx| {
                                        cx.stop_propagation();
                                        this.step(PAGE, cx);
                                    }))
                                    .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                                        cx.stop_propagation();
                                        this.choose(this.selected, window, cx);
                                    }))
                                    .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                                        cx.stop_propagation();
                                        this.close(window, cx);
                                    }))
                                    .child(
                                        Input::new(&self.query)
                                            .appearance(false)
                                            .prefix(Icon::new(IconName::Search).size(px(16.)).text_color(t.muted))
                                            .h(px(40.)),
                                    )
                                    .child(div().h(px(1.)).bg(t.border))
                                    .child(body)
                                    .child(div().h(px(1.)).bg(t.border))
                                    .child(
                                        h_flex()
                                            .pt_1()
                                            .px_1()
                                            .gap_4()
                                            .justify_between()
                                            .child(self.hints(cx))
                                            .child(
                                                h_flex()
                                                    .gap_3()
                                                    .flex_none()
                                                    .child(div().text_xs().text_color(t.muted).child(count))
                                                    .child(
                                                        h_flex()
                                                            .gap_1()
                                                            .child(key_cap(shortcuts_keys))
                                                            .child(div().pl_1().text_xs().text_color(t.muted).child(tr(cx, T, "Keyboard shortcuts"))),
                                                    ),
                                            ),
                                    ),
                            ),
                    ),
            );
        Self::finish(scrim, live)
    }

    fn area_section(&self, area: Area, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        v_flex()
            .gap_0p5()
            .child(div().pb_1().text_xs().font_semibold().text_color(t.muted).child(tr(cx, T, area.title())))
            .children(in_area(area).map(|shortcut| {
                h_flex()
                    .min_h(px(26.))
                    .gap_3()
                    .justify_between()
                    .child(div().text_sm().text_color(t.text).child(tr(cx, T, shortcut.does)))
                    .child(h_flex().flex_none().gap_1().children(shortcut.keys.iter().map(|&keys| key_cap(keys))))
            }))
    }

    fn render_shortcuts(&mut self, level: f32, live: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.tokens();
        // The list scrolls only when the window is too short to show it whole.
        let room = (f32::from(window.viewport_size().height) - SHORTCUTS_TOP - SHORTCUTS_CHROME).max(160.);
        let column = |areas: &[Area], this: &Self, cx: &mut Context<Self>| {
            v_flex().flex_1().min_w_0().gap_5().children(areas.iter().map(|&area| this.area_section(area, cx)).collect::<Vec<_>>())
        };
        let left = column(&Area::ALL[..5], self, cx);
        let right = column(&Area::ALL[5..], self, cx);
        let scrim = self.scrim("shortcuts-scrim", level, live, &t, cx)
            .child(
                Self::panel_motion(div(), level)
                    .mt(px(SHORTCUTS_TOP))
                    .w(px(860.))
                    .max_w(relative(0.94))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        ornament::frame(&t, t.border, t.surface)
                            .p(px(10.))
                            .child(
                                v_flex()
                                    .id("shortcuts")
                                    .track_focus(&self.focus)
                                    .key_context("ChronosShortcuts")
                                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "escape" {
                                            cx.stop_propagation();
                                            this.close(window, cx);
                                        }
                                    }))
                                    .px_5()
                                    .py_4()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .child(div().text_base().font_semibold().text_color(t.text).child(tr(cx, T, "Keyboard shortcuts")))
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .child(key_cap("escape"))
                                                    .child(div().pl_1().text_xs().text_color(t.muted).child(tr(cx, T, "Close"))),
                                            ),
                                    )
                                    .child(ornament::divider(&t, 0.0))
                                    .child(
                                        div().id("shortcuts-body").max_h(px(room)).overflow_y_scroll().child(
                                            h_flex().items_start().gap_10().child(left).child(right),
                                        ),
                                    ),
                            ),
                    ),
            );
        Self::finish(scrim, live)
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reduce = self.state.read(cx).reduce_motion;
        let mut motion = Driver::new(reduce, cx);
        let level = motion.level(&self.presence);
        // A closed overlay is still drawn, without taking input, until it has faded out.
        let (overlay, live) = match self.overlay {
            Overlay::Closed if self.presence.visible(motion.now(), reduce) => (self.ghost, false),
            overlay => (overlay, true),
        };
        let element = match overlay {
            Overlay::Closed => div().into_any_element(),
            Overlay::Search => self.render_search(level, live, cx),
            Overlay::Shortcuts => self.render_shortcuts(level, live, window, cx),
        };
        motion.finish(window);
        element
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use crate::desktop::demo;

    fn score(query: &str, text: &str) -> i32 {
        fuzzy(query, text).map_or(i32::MIN, |m| m.score)
    }

    #[test]
    fn every_query_character_must_appear_in_order() {
        assert!(fuzzy("pdf", "pdftext").is_some());
        assert!(fuzzy("PDF", "pdftext").is_some(), "case is ignored");
        assert!(fuzzy("fdp", "pdftext").is_none(), "order matters");
        assert!(fuzzy("pdfz", "pdftext").is_none());
        assert!(fuzzy("", "pdftext").is_none());
        assert!(fuzzy("a", "").is_none());
    }

    #[test]
    fn a_contiguous_prefix_beats_letters_spread_through_a_word() {
        assert!(score("pdf", "pdftext") > score("pdf", "paper_draft_format"));
        assert!(score("read", "read_batch") > score("read", "reviewer_audit"));
    }

    #[test]
    fn word_starts_beat_the_middle_of_words() {
        // `b` starts a word in read_batch but sits inside "rubber".
        assert!(score("rb", "read_batch") > score("rb", "rubber"));
        assert!(score("cr", "core_retry") > score("cr", "scrap"));
        assert!(score("dt", "DataTable") > score("dt", "update"));
    }

    #[test]
    fn earlier_and_whole_matches_rank_higher() {
        assert!(score("core", "core") > score("core", "core_retry"));
        assert!(score("core", "core_retry") > score("core", "the core retry"));
    }

    #[test]
    fn letters_strewn_across_a_long_text_are_not_a_match() {
        // a, b and c all sit inside words, far apart.
        assert!(fuzzy("abc", "waxed hobby ducks").is_none());
        // The same letters starting words read as initials and do match.
        assert!(fuzzy("abc", "a big castle").is_some());
        assert!(fuzzy("ace", "ace").is_some());
    }

    #[test]
    fn chinese_text_is_matched_by_character() {
        let wish = "全面盤點 HfO₂ 電阻式記憶體 PDF，建立可追溯的資料集";
        let m = fuzzy("電阻", wish).expect("found");
        let ranges = mark_ranges(wish, &m.positions);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&wish[ranges[0].clone()], "電阻");
        assert!(fuzzy("記憶 資料", wish).is_some(), "spaces inside one term are ignored");
    }

    #[test]
    fn marks_cover_whole_characters_and_merge_neighbours() {
        let text = "a電b";
        let m = fuzzy("電b", text).unwrap();
        assert_eq!(mark_ranges(text, &m.positions), vec![1..5]);
        assert_eq!(mark_ranges("abc", &[2, 0, 0]), vec![0..1, 2..3]);
    }

    #[test]
    fn the_empty_query_lists_commands_then_recent_wishes() {
        let state = demo::state();
        let items = index(&state, Lang::En, true);
        let hits = search(&items, "  ", LIMIT);
        let kinds: Vec<Kind> = hits.iter().map(|hit| items[hit.item].kind).collect();
        let first_wish = kinds.iter().position(|k| *k == Kind::Wish).expect("wishes listed");
        assert!(kinds[..first_wish].iter().all(|k| *k == Kind::Command));
        assert!(kinds[first_wish..].iter().all(|k| *k == Kind::Wish));
        assert_eq!(kinds.iter().filter(|k| **k == Kind::Wish).count(), state.runs.len().min(RECENT_WISHES));
    }

    #[test]
    fn one_box_finds_every_kind_of_thing() {
        let state = demo::state();
        let items = index(&state, Lang::En, true);
        let first_kind = |query: &str| search(&items, query, LIMIT).first().map(|hit| items[hit.item].kind);
        assert_eq!(first_kind("new wish"), Some(Kind::Command));
        assert_eq!(first_kind("pdftext"), Some(Kind::Module));
        assert_eq!(first_kind("read_batch"), Some(Kind::Unit));
        assert_eq!(first_kind("tiled sheets"), Some(Kind::Memory));
        assert_eq!(first_kind("電阻式"), Some(Kind::Wish));
        let goal = &state.runs[0].goals[0];
        assert_eq!(first_kind(&goal.title), Some(Kind::Goal));
    }

    #[test]
    fn every_word_must_be_found_somewhere() {
        let state = demo::state();
        let items = index(&state, Lang::En, true);
        let hits = search(&items, "read ml_hfo2", LIMIT);
        assert!(!hits.is_empty());
        for hit in &hits {
            let item = &items[hit.item];
            let all = format!("{} {} {} {}", item.title, item.detail, item.place, item.keywords).to_lowercase();
            assert!(all.contains("ml_hfo2"), "{} lacks the project", item.title);
        }
        assert!(search(&items, "read zzzzqq", LIMIT).is_empty());
    }

    #[test]
    fn memory_bodies_are_searched_for_words_not_scattered_letters() {
        let state = demo::state();
        let items = index(&state, Lang::En, true);
        let memory_titles = |query: &str| -> Vec<String> {
            search(&items, query, LIMIT).iter().filter(|h| items[h.item].kind == Kind::Memory).map(|h| items[h.item].title.to_string()).collect()
        };
        assert_eq!(memory_titles("downscaled"), vec!["Page images beat tiled sheets".to_string()]);
        // "dwn" is strewn through the same body but is not a word in it.
        assert!(memory_titles("dwnscld").is_empty());
    }

    #[test]
    fn the_open_run_ranks_ahead_of_an_equal_match_elsewhere() {
        let mut state = demo::state();
        let twin = state.runs[0].clone();
        state.runs.push(Run { session: "twin".into(), ..twin });
        state.current = Some(0);
        let items = index(&state, Lang::En, true);
        let hits = search(&items, "pdftext", LIMIT);
        let modules: Vec<&Item> = hits.iter().map(|h| &items[h.item]).filter(|i| i.kind == Kind::Module).collect();
        assert!(modules.len() >= 2, "the demo runs share their modules");
        assert!(modules[0].current && modules[1..].iter().all(|m| !m.current));
        assert_eq!(modules[0].target, Target::Module(state.runs[0].session.clone(), "pdftext".into()));
        state.current = Some(4);
        let items = index(&state, Lang::En, true);
        let first = search(&items, "pdftext", LIMIT).into_iter().map(|h| &items[h.item]).find(|i| i.kind == Kind::Module).unwrap();
        assert_eq!(first.target, Target::Module("twin".into(), "pdftext".into()));
    }

    #[test]
    fn equal_scores_keep_the_index_order() {
        let state = demo::state();
        let items = index(&state, Lang::En, true);
        let hits = search(&items, "verified", LIMIT);
        for pair in hits.windows(2) {
            assert!(pair[0].score > pair[1].score || pair[0].item < pair[1].item);
        }
    }

    #[test]
    fn results_are_capped() {
        let mut state = demo::state();
        for i in 0..40 {
            let mut run = state.runs[0].clone();
            run.session = format!("copy-{i}").into();
            state.runs.push(run);
        }
        let items = index(&state, Lang::En, true);
        assert!(items.len() > 1000);
        assert_eq!(search(&items, "e", 25).len(), 25);
    }

    #[test]
    fn stop_and_resume_are_offered_only_where_the_control_bar_allows_them() {
        let commands = |status: RunStatus, hosted: bool| -> Vec<Command> {
            let mut state = demo::state();
            state.runs[0].status = status;
            state.runs[0].hosted = hosted;
            state.current = Some(0);
            index(&state, Lang::En, true)
                .into_iter()
                .filter_map(|item| match item.target {
                    Target::Command(c @ (Command::Stop | Command::Resume)) => Some(c),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(commands(RunStatus::Active, true), vec![Command::Stop]);
        assert_eq!(commands(RunStatus::Active, false), vec![], "a run started in a terminal is view only");
        assert_eq!(commands(RunStatus::LimitWait, true), vec![Command::Stop]);
        assert_eq!(commands(RunStatus::Blocked, true), vec![Command::Resume]);
        assert_eq!(commands(RunStatus::Interrupted, false), vec![Command::Resume]);
        assert_eq!(commands(RunStatus::Waiting, true), vec![Command::Resume]);
        assert_eq!(commands(RunStatus::Done, true), vec![]);
        let mut none = demo::state();
        none.current = None;
        assert!(!index(&none, Lang::En, true).iter().any(|i| matches!(i.target, Target::Command(Command::Stop | Command::Resume))));
    }

    #[test]
    fn the_theme_command_offers_the_other_theme() {
        let state = demo::state();
        let theme = |dark: bool| index(&state, Lang::En, dark).into_iter().find(|i| i.target == Target::Command(Command::ToggleTheme)).unwrap().title;
        assert_eq!(theme(true).as_ref(), "Switch to the light theme");
        assert_eq!(theme(false).as_ref(), "Switch to the dark theme");
    }

    #[test]
    fn commands_are_found_by_their_english_words_in_chinese_too() {
        let state = demo::state();
        let items = index(&state, Lang::ZhTw, true);
        let first = |q: &str| search(&items, q, LIMIT).first().map(|h| items[h.item].target.clone());
        assert!(search(&items, "go to memory", LIMIT).iter().any(|h| items[h.item].target == Target::Command(Command::Open(Screen::Memory))));
        assert_eq!(first("前往記憶"), Some(Target::Command(Command::Open(Screen::Memory))));
        assert_eq!(items[0].title.as_ref(), "許一個新願望");
    }

    #[test]
    fn a_goal_stands_for_its_first_open_module() {
        let run = demo::sample_run();
        for goal in &run.goals {
            let Some(lead) = goal_lead(&run, goal) else { continue };
            assert!(goal.modules.contains(&lead));
            let module = &run.modules[lead];
            let all_done = goal.modules.iter().all(|&m| run.module_done(&run.modules[m]) == run.modules[m].units.len());
            assert!(all_done || run.module_done(module) < module.units.len(), "{} chose a finished module", goal.id);
        }
        let empty = GoalItem { id: "G9".into(), title: "none".into(), modules: vec![] };
        assert_eq!(goal_lead(&run, &empty), None);
    }

    #[test]
    fn hundreds_of_items_are_searched_in_a_blink() {
        let mut state = demo::state();
        for i in 0..60 {
            let mut run = state.runs[0].clone();
            run.session = format!("copy-{i}").into();
            state.runs.push(run);
        }
        let items = index(&state, Lang::En, true);
        let started = std::time::Instant::now();
        for query in ["r", "re", "rea", "read", "read b", "zz"] {
            search(&items, query, LIMIT);
        }
        let per_query = started.elapsed() / 6;
        // A debug build on a slow machine stays far below one frame per query for ~2000 items.
        assert!(per_query < std::time::Duration::from_millis(60), "{per_query:?} per query over {} items", items.len());
    }

    // --- The shortcut table -----------------------------------------------------------------

    #[test]
    fn every_key_in_the_table_parses() {
        for shortcut in SHORTCUTS {
            assert!(!shortcut.keys.is_empty(), "{}", shortcut.does);
            for keys in shortcut.keys {
                assert!(Keystroke::parse(keys).is_ok(), "{keys} does not parse");
            }
        }
    }

    #[test]
    fn the_window_wide_bindings_are_exactly_the_global_rows() {
        let bindings = global_bindings();
        let globals: Vec<&Shortcut> = SHORTCUTS.iter().filter(|s| s.global.is_some()).collect();
        assert_eq!(bindings.len(), globals.iter().map(|s| s.keys.len()).sum::<usize>());
        for global in [Global::Palette, Global::NewWish, Global::Shortcuts] {
            assert_eq!(globals.iter().filter(|s| s.global == Some(global)).count(), 1, "{global:?} bound once");
        }
        let pairs = globals.iter().flat_map(|s| s.keys.iter().map(move |keys| (s.global.unwrap(), *keys)));
        for (binding, (global, keys)) in bindings.iter().zip(pairs) {
            let stroke = Keystroke::parse(keys).unwrap();
            let bound = binding.keystrokes();
            assert_eq!(bound.len(), 1);
            assert_eq!((&bound[0].inner().key, bound[0].inner().modifiers), (&stroke.key, stroke.modifiers), "{keys}");
            let action = match global {
                Global::Palette => "OpenPalette",
                Global::NewWish => "NewWish",
                Global::Shortcuts => "ShowShortcuts",
            };
            assert!(binding.action().name().ends_with(action), "{keys} runs {}", binding.action().name());
        }
        assert_eq!(keys_of(Global::Shortcuts), "?");
        assert!(keys_of(Global::Palette).ends_with("-k"));
    }

    #[test]
    fn no_shortcut_is_listed_twice_in_one_place() {
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in &SHORTCUTS[i + 1..] {
                if a.area == b.area {
                    for keys in a.keys {
                        assert!(!b.keys.contains(keys) || a.does == b.does, "{keys} means two things in {:?}", a.area);
                    }
                }
            }
        }
        for area in Area::ALL {
            assert!(in_area(area).next().is_some(), "{area:?} has no shortcuts");
        }
    }

    /// Every key a screen handles, read from the screens' own sources. A key handled anywhere in
    /// the app but missing from the table fails here, so the overlay cannot fall behind.
    #[test]
    fn every_key_the_screens_handle_is_in_the_table() {
        let sources = [
            ("composer.rs", include_str!("composer.rs")),
            ("control.rs", include_str!("control.rs")),
            ("detail.rs", include_str!("detail.rs")),
            ("doctor.rs", include_str!("doctor.rs")),
            ("memory.rs", include_str!("memory.rs")),
            ("mesh.rs", include_str!("mesh.rs")),
            ("run_view.rs", include_str!("run_view.rs")),
            ("settings.rs", include_str!("settings.rs")),
            ("shell.rs", include_str!("shell.rs")),
            ("superswitch.rs", include_str!("superswitch.rs")),
            ("todo.rs", include_str!("todo.rs")),
            ("usage.rs", include_str!("usage.rs")),
            ("wishes.rs", include_str!("wishes.rs")),
        ];
        let known: Vec<&str> = SHORTCUTS.iter().flat_map(|s| s.keys.iter().copied()).collect();
        let names = ["up", "down", "left", "right", "enter", "escape", "space", "tab", "home", "end", "pageup", "pagedown", "delete", "backspace"];
        let direct = regex::Regex::new(r#"(?:KeyBinding::new\(\s*|keystroke\.key\s*==\s*|Keystroke::parse\()"([^"]+)""#).unwrap();
        let arm = regex::Regex::new(r#"(?m)^\s*((?:"[a-z-]+"\s*\|\s*)*"[a-z-]+")\s*=>"#).unwrap();
        let literal = regex::Regex::new(r#""([a-z-]+)""#).unwrap();
        let mut found = 0;
        for (file, source) in sources {
            let mut keys: Vec<String> = direct.captures_iter(source).map(|c| c[1].to_owned()).collect();
            for capture in arm.captures_iter(source) {
                let arm_keys: Vec<String> = literal.captures_iter(&capture[1]).map(|c| c[1].to_owned()).collect();
                if arm_keys.iter().all(|k| names.contains(&k.as_str())) {
                    keys.extend(arm_keys);
                }
            }
            for key in keys {
                found += 1;
                let key = if cfg!(target_os = "macos") { key.replace("ctrl-", "cmd-") } else { key };
                assert!(known.contains(&key.as_str()), "{file} handles {key}, which the shortcut table does not list");
            }
        }
        assert!(found >= 20, "the scan found only {found} keys; the patterns no longer match the sources");
    }

    #[test]
    fn the_scan_catches_a_key_the_table_lacks() {
        let direct = regex::Regex::new(r#"(?:KeyBinding::new\(\s*|keystroke\.key\s*==\s*|Keystroke::parse\()"([^"]+)""#).unwrap();
        let source = r#"cx.bind_keys([KeyBinding::new("ctrl-shift-p", Thing, None)]);"#;
        let key = direct.captures(source).unwrap()[1].to_owned();
        assert!(!SHORTCUTS.iter().any(|s| s.keys.contains(&key.as_str())));
    }

    #[test]
    fn every_string_shown_has_a_chinese_translation() {
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|s| s.does).collect();
        keys.extend(Area::ALL.iter().map(|a| a.title()));
        keys.extend([Kind::Command, Kind::Wish, Kind::Goal, Kind::Module, Kind::Unit, Kind::Memory].map(Kind::label));
        let source = include_str!("palette.rs");
        let used = regex::Regex::new(r#"tr\(cx, T, "((?:[^"\\]|\\.)*)"\)"#).unwrap();
        let mut owned: Vec<String> = used.captures_iter(source).map(|c| c[1].to_owned()).collect();
        let command = regex::Regex::new(r#"command\(\s*"((?:[^"\\]|\\.)*)""#).unwrap();
        owned.extend(command.captures_iter(source).map(|c| c[1].to_owned()));
        let translate_keys = regex::Regex::new(r#"\bt\("((?:[^"\\]|\\.)*)"\)"#).unwrap();
        owned.extend(translate_keys.captures_iter(source).map(|c| c[1].to_owned()));
        keys.extend(owned.iter().map(String::as_str));
        assert!(keys.len() > 60);
        for key in keys {
            let found = T.iter().chain(crate::desktop::i18n::COMMON.iter()).any(|(en, _)| *en == key);
            assert!(found, "missing translation: {key}");
        }
    }
}
