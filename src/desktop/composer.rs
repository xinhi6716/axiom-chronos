//! The New wish dialog: choose a project, say what you want, start. Typing `@` in the wish offers
//! the project's files and folders, so a wish can name exactly what it is about.
use super::control::{ask, Ask};
use super::i18n::{fill, tr, Table};
use super::model::*;
use super::palette::{self, fuzzy, in_area, key_cap, marked, Area};
use super::superswitch::{self, Activation, Members};
use super::theme::Tokened as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, OutdentInline, Textarea, TextareaState,
};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const T: Table = &[
    ("Choose folder…", "選擇資料夾…"),
    ("Choose the project folder", "選擇專案資料夾"),
    ("Project folder", "專案資料夾"),
    ("No folder chosen yet", "尚未選擇資料夾"),
    ("Recent", "最近"),
    ("Checking this folder…", "正在檢查這個資料夾…"),
    ("branch", "分支"),
    ("detached at {head}", "分離於 {head}"),
    ("no commits yet", "還沒有提交"),
    ("No uncommitted changes", "沒有未提交的變更"),
    ("{n} uncommitted changes. Commit first so this can be undone.", "有 {n} 個未提交的變更。請先提交，之後才能復原。"),
    ("No commits yet. Commit first so this can be undone.", "還沒有任何提交。請先提交，之後才能復原。"),
    ("Not a git repository. You can still start, but changes cannot be undone with git.", "這不是 git 儲存庫。仍可開始，但無法用 git 復原變更。"),
    ("Could not read the git state: {error}", "讀不到 git 狀態：{error}"),
    ("This folder does not exist.", "這個資料夾不存在。"),
    ("Wish", "願望"),
    ("Say what you want done and how you will know it is done.", "說明你想完成什麼，以及怎樣算完成。"),
    ("Creative", "創意模式"),
    ("Let Chronos find one small, evidence-backed improvement.", "讓 Chronos 自己找一個有證據支持的小改進。"),
    ("Advanced", "進階"),
    ("Maximum turns", "最多回合數"),
    ("Maximum turns: {n}", "最多回合數：{n}"),
    ("No limit", "不限"),
    ("Leave blank for no limit.", "留空代表不限。"),
    ("Start", "開始"),
    ("Choose a project folder to start.", "請先選擇專案資料夾。"),
    ("Pick a folder that exists to start.", "請選擇存在的資料夾。"),
    ("Checking the folder first…", "正在檢查資料夾…"),
    ("Write your wish to start.", "請先寫下願望。"),
    ("Maximum turns must be a whole number above 0.", "最多回合數必須是大於 0 的整數。"),
    ("Not connected to a session, so this wish cannot start.", "目前沒有連上工作階段，無法開始這個願望。"),
    ("Advanced research", "高級研究"),
    (
        "Astra, Fable 5.2 and Argon, three models from different families, deliberate before the plan. Fable is capped at $5; Argon's quota is small.",
        "Astra、Fable 5.2、Argon 三個不同家族的模型先討論再定計畫。Fable 上限 $5，Argon 額度很小。",
    ),
    ("agy is not installed, so the council will run with fewer members: without Argon.", "沒有安裝 agy，議會會少一位成員：沒有 Argon。"),
    ("claude is not installed, so the council will run with fewer members: without Fable.", "沒有安裝 claude，議會會少一位成員：沒有 Fable。"),
    ("agy and claude are not installed, so the council will run with fewer members: Astra alone.", "沒有安裝 agy 和 claude，議會會少兩位成員：只剩 Astra。"),
    ("Discard this wish?", "要捨棄這個願望嗎？"),
    ("What you wrote will be lost.", "你寫的內容不會保留。"),
    ("Discard", "捨棄"),
    ("Keep writing", "繼續寫"),
    ("Type @ to point at a file or folder.", "輸入 @ 可以指向檔案或資料夾。"),
    ("Files in {}", "{} 裡的檔案"),
    ("Showing the first {} files.", "只列出前 {} 個檔案。"),
    ("Reading the project's files…", "正在讀取專案的檔案…"),
    ("No file or folder matches “{}”.", "沒有符合「{}」的檔案或資料夾。"),
    ("Choose a project folder first to point at its files.", "請先選擇專案資料夾，才能指向其中的檔案。"),
    ("The project's files could not be listed: {}", "無法列出專案的檔案：{}"),
];

const GIT_TIMEOUT: Duration = Duration::from_secs(6);
const RECENT_LIMIT: usize = 5;
/// The most entries the `@` list reads from a git repository, and from a folder walked by hand.
const FILE_CAP: usize = 200_000;
const WALK_CAP: usize = 50_000;
const WALK_DEPTH: usize = 16;
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// Folders a hand walk never enters: hidden ones, and these.
const SKIP_DIRS: [&str; 3] = ["node_modules", "__pycache__", "target"];
const MENTION_LIMIT: usize = 50;
/// Up to this many entries are ranked on the main thread; more are ranked in the background.
const MENTION_SYNC: usize = 4000;
const MENTION_ROW_H: f32 = 30.;
const MENTION_ROWS: f32 = 7.5;
const WISH_H: f32 = 132.;
/// The list's header, footer and padding, which sit outside its scrolling rows.
const MENTION_CHROME: f32 = 84.;
/// Kept between the list and the window's bottom edge.
const MENTION_EDGE: f32 = 12.;

/// How tall the list's rows may be when `room` is the window space below the wish box: the usual
/// height, cut down to what fits, but never less than one row so the list stays usable.
fn mention_rows_height(room: f32) -> f32 {
    (room - MENTION_CHROME - MENTION_EDGE).clamp(MENTION_ROW_H, MENTION_ROW_H * MENTION_ROWS)
}

/// What a git probe found about the chosen folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FolderCheck {
    Idle,
    Checking,
    Repo { head: Head, changes: usize },
    NotRepo,
    Missing,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Head {
    Branch(String),
    Detached(String),
    Unborn(String),
}

/// Why Start is disabled. The text is shown beside the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Blocked {
    NoFolder,
    FolderMissing,
    Checking,
    NoWish,
    BadTurns,
}

impl Blocked {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::NoFolder => "Choose a project folder to start.",
            Self::FolderMissing => "Pick a folder that exists to start.",
            Self::Checking => "Checking the folder first…",
            Self::NoWish => "Write your wish to start.",
            Self::BadTurns => "Maximum turns must be a whole number above 0.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Ready {
    pub wish: String,
    pub max_turns: Option<u32>,
}

/// Decides whether Start is available. A folder that merely warns does not block.
pub(super) fn validate(has_folder: bool, check: &FolderCheck, wish: &str, turns: &str) -> Result<Ready, Blocked> {
    if !has_folder {
        return Err(Blocked::NoFolder);
    }
    match check {
        FolderCheck::Missing => return Err(Blocked::FolderMissing),
        FolderCheck::Checking => return Err(Blocked::Checking),
        _ => {}
    }
    let wish = wish.trim();
    if wish.is_empty() {
        return Err(Blocked::NoWish);
    }
    let max_turns = parse_turns(turns).ok_or(Blocked::BadTurns)?;
    Ok(Ready { wish: wish.to_owned(), max_turns })
}

/// `Some(None)` is blank (no limit), `Some(Some(n))` a limit, `None` something that is not a turn count.
pub(super) fn parse_turns(text: &str) -> Option<Option<u32>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    match text.parse::<u32>() {
        Ok(n) if n > 0 => Some(Some(n)),
        _ => None,
    }
}

/// Text in the wish box is the only thing Cancel can lose.
pub(super) fn asks_before_discarding(wish: &str) -> bool {
    !wish.trim().is_empty()
}

/// The request Start sends. The council is on only when the switch was turned on in this dialog.
pub(super) fn request(cwd: PathBuf, ready: Ready, creative: bool, research: &Activation) -> WishRequest {
    WishRequest { cwd, wish: ready.wish, creative, max_turns: ready.max_turns, advanced_research: research.is_on() }
}

/// Reads `git status --porcelain=v2 --branch`.
pub(super) fn parse_status(out: &str) -> FolderCheck {
    let (mut oid, mut branch) = ("", "");
    let mut changes = 0;
    for line in out.lines().filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            oid = rest;
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            branch = rest;
        } else if !line.starts_with('#') {
            changes += 1;
        }
    }
    let head = if branch == "(detached)" {
        Head::Detached(oid.chars().take(7).collect())
    } else if oid == "(initial)" {
        Head::Unborn(branch.to_owned())
    } else {
        Head::Branch(branch.to_owned())
    };
    FolderCheck::Repo { head, changes }
}

fn classify_git(success: bool, stdout: &str, stderr: &str) -> FolderCheck {
    if success {
        return parse_status(stdout);
    }
    if stderr.to_ascii_lowercase().contains("not a git repository") {
        return FolderCheck::NotRepo;
    }
    FolderCheck::Unknown(stderr.lines().next().unwrap_or("git failed").trim().to_owned())
}

/// A git command in `folder` that takes no locks and opens no console window.
fn git(folder: &Path, args: &[&str]) -> Command {
    let mut git = crate::base::proc::cmd("git");
    git.args(args).current_dir(folder).env("GIT_OPTIONAL_LOCKS", "0").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        git.creation_flags(CREATE_NO_WINDOW);
    }
    git
}

/// Runs a command with a deadline, so a hung drive cannot hold the dialog. `Err` is the deadline.
fn output_within(mut command: Command, deadline: Duration) -> Result<std::io::Result<Output>, ()> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(command.output());
    });
    receive.recv_timeout(deadline).map_err(|_| ())
}

/// Runs on a background thread.
fn check_folder(folder: &Path) -> FolderCheck {
    if !folder.is_dir() {
        return FolderCheck::Missing;
    }
    match output_within(git(folder, &["status", "--porcelain=v2", "--branch"]), GIT_TIMEOUT) {
        Ok(Ok(output)) => classify_git(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => FolderCheck::Unknown("git is not installed".to_owned()),
        Ok(Err(error)) => FolderCheck::Unknown(error.to_string()),
        Err(_) => FolderCheck::Unknown("git did not answer in time".to_owned()),
    }
}

/// The `@word` being typed at the cursor: the byte range of the whole word, `@` included, and the
/// part typed so far after the `@`. An `@` right after a letter or digit, as in an email address,
/// does not start one.
pub(super) fn mention_at(text: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    if query.contains(char::is_whitespace) {
        return None;
    }
    if before[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '@') {
        return None;
    }
    let rest = &text[cursor..];
    let end = cursor + rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some((at..end, query))
}

/// What replaces the `@word`: the path, quoted when it holds spaces, and a space after it unless
/// one already follows.
pub(super) fn mention_insert(path: &str, space_follows: bool) -> String {
    let path = if path.contains(char::is_whitespace) { format!("\"{path}\"") } else { path.to_owned() };
    if space_follows {
        path
    } else {
        format!("{path} ")
    }
}

/// A project's files and the folders that hold them, as relative paths with `/`. Folders end
/// with `/`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Listing {
    pub entries: Vec<String>,
    /// More files existed than were read.
    pub truncated: bool,
}

/// The listing for some files: each file, and every folder on the way to one.
pub(super) fn listing_from_files<'a>(files: impl IntoIterator<Item = &'a str>, cap: usize) -> Listing {
    let mut entries = BTreeSet::new();
    let mut truncated = false;
    let mut count = 0;
    for file in files {
        let file = file.trim_start_matches("./");
        if file.is_empty() || file.ends_with('/') {
            continue;
        }
        if count == cap {
            truncated = true;
            break;
        }
        count += 1;
        for (at, _) in file.match_indices('/') {
            entries.insert(file[..=at].to_owned());
        }
        entries.insert(file.to_owned());
    }
    Listing { entries: entries.into_iter().collect(), truncated }
}

/// One line of a `.gitignore`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    pattern: String,
    dir_only: bool,
    anchored: bool,
    negate: bool,
}

/// The rules of a top-level `.gitignore`, for folders that are not git repositories. Inside a
/// repository git itself answers, with every `.gitignore` and exclude file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct IgnoreRules(Vec<Rule>);

impl IgnoreRules {
    pub(super) fn parse(text: &str) -> Self {
        let rules = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let (negate, line) = line.strip_prefix('!').map_or((false, line), |rest| (true, rest));
                let (dir_only, line) = line.strip_suffix('/').map_or((false, line), |rest| (true, rest));
                let line = line.strip_prefix("**/").unwrap_or(line);
                let anchored = line.contains('/');
                Rule { pattern: line.trim_start_matches('/').to_owned(), dir_only, anchored, negate }
            })
            .collect();
        Self(rules)
    }

    /// Whether `rel`, a path relative to the folder, is ignored. The last rule that matches wins.
    pub(super) fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let mut ignored = false;
        for rule in &self.0 {
            if rule.dir_only && !is_dir {
                continue;
            }
            if glob(&rule.pattern, if rule.anchored { rel } else { name }) {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

/// Shell-style matching: `*` within one path segment, `**` across segments, `?` one character.
pub(super) fn glob(pattern: &str, text: &str) -> bool {
    fn matches(p: &[char], t: &[char]) -> bool {
        match p {
            [] => t.is_empty(),
            ['*', '*', rest @ ..] => {
                let rest = rest.strip_prefix(&['/']).unwrap_or(rest);
                (0..=t.len()).any(|i| matches(rest, &t[i..]))
            }
            ['*', rest @ ..] => (0..=t.len()).take_while(|&i| i == 0 || t[i - 1] != '/').any(|i| matches(rest, &t[i..])),
            ['?', rest @ ..] => t.first().is_some_and(|&c| c != '/') && matches(rest, &t[1..]),
            [c, rest @ ..] => t.first() == Some(c) && matches(rest, &t[1..]),
        }
    }
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    matches(&p, &t)
}

/// Lists a folder that git does not track by walking it, skipping hidden folders, the usual
/// build and dependency folders and what its top-level `.gitignore` names.
pub(super) fn walk(root: &Path, cap: usize) -> Listing {
    let rules = IgnoreRules::parse(&std::fs::read_to_string(root.join(".gitignore")).unwrap_or_default());
    let mut entries = vec![];
    let mut truncated = false;
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    'walk: while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut children: Vec<(String, bool)> = read
            .filter_map(|entry| {
                let entry = entry.ok()?;
                // Symbolic links are listed but never followed, so a loop cannot trap the walk.
                let is_dir = entry.file_type().ok()?.is_dir();
                Some((entry.file_name().into_string().ok()?, is_dir))
            })
            .collect();
        children.sort();
        for (name, is_dir) in children {
            if name.starts_with('.') || (is_dir && SKIP_DIRS.contains(&name.as_str())) {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if rules.ignored(&rel, is_dir) {
                continue;
            }
            if entries.len() == cap {
                truncated = true;
                break 'walk;
            }
            if is_dir {
                entries.push(format!("{rel}/"));
                if depth + 1 < WALK_DEPTH {
                    stack.push((dir.join(&name), format!("{rel}/"), depth + 1));
                }
            } else {
                entries.push(rel);
            }
        }
    }
    entries.sort();
    Listing { entries, truncated }
}

/// The files `@` can offer. In a git repository git lists them, tracked and untracked but not
/// ignored, which stays fast on big repositories; any other folder is walked.
fn list_files(folder: &Path) -> Result<Listing, String> {
    if !folder.is_dir() {
        return Err("the folder does not exist".to_owned());
    }
    let command = git(folder, &["-c", "core.quotepath=off", "ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    match output_within(command, LIST_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(listing_from_files(stdout.split('\0'), FILE_CAP))
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.to_ascii_lowercase().contains("not a git repository") {
                Ok(walk(folder, WALK_CAP))
            } else {
                Err(stderr.lines().next().unwrap_or("git failed").trim().to_owned())
            }
        }
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(walk(folder, WALK_CAP)),
        Ok(Err(error)) => Err(error.to_string()),
        Err(()) => Err("git did not answer in time".to_owned()),
    }
}

/// One path offered for an `@word`, with the byte offsets of its matched characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PathHit {
    pub entry: usize,
    pub marks: Vec<usize>,
}

fn path_depth(path: &str) -> usize {
    path.trim_end_matches('/').matches('/').count()
}

/// The entries that best match `query`, best first. A match in the file or folder name counts
/// double; a query with `/` in it is matched against the whole path. Ties go to the shorter path.
/// An empty query lists the top of the project, folders first.
pub(super) fn rank_paths(entries: &[String], query: &str, limit: usize) -> Vec<PathHit> {
    let query = query.replace('\\', "/");
    if query.is_empty() {
        let mut top: Vec<usize> = (0..entries.len()).filter(|&i| path_depth(&entries[i]) == 0).collect();
        top.sort_by_key(|&i| (!entries[i].ends_with('/'), entries[i].to_lowercase()));
        return top.into_iter().take(limit).map(|entry| PathHit { entry, marks: vec![] }).collect();
    }
    let by_path_only = query.contains('/');
    let mut scored: Vec<(i32, usize, Vec<usize>)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, path)| {
            let trimmed = path.trim_end_matches('/');
            let name_at = trimmed.rfind('/').map_or(0, |slash| slash + 1);
            let by_name = (!by_path_only)
                .then(|| fuzzy(&query, &trimmed[name_at..]))
                .flatten()
                .map(|m| (m.score * 2, m.positions.into_iter().map(|p| p + name_at).collect::<Vec<_>>()));
            let by_path = fuzzy(&query, trimmed).map(|m| (m.score, m.positions));
            let (score, marks) = match (by_name, by_path) {
                (Some(name), Some(path)) => if name.0 >= path.0 { name } else { path },
                (name, path) => name.or(path)?,
            };
            Some((score, i, marks))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0).then(entries[a.1].len().cmp(&entries[b.1].len())).then(entries[a.1].cmp(&entries[b.1]))
    });
    scored.into_iter().take(limit).map(|(_, entry, marks)| PathHit { entry, marks }).collect()
}

/// What the `@` list knows about the chosen folder's files.
#[derive(Clone, Debug)]
enum Files {
    None,
    Loading(PathBuf),
    Ready(PathBuf, Arc<Listing>),
    Failed(PathBuf, String),
}

/// The `@word` being completed and the paths offered for it.
struct Mention {
    query: String,
    hits: Vec<PathHit>,
    /// The listing `hits` index into; `None` until the first ranking arrives.
    listing: Option<Arc<Listing>>,
    selected: usize,
}

/// The folders of past runs, one entry each, in the order the runs are listed.
pub(super) fn recent_projects(runs: &[Run]) -> Vec<(SharedString, PathBuf)> {
    let mut seen: Vec<(SharedString, PathBuf)> = Vec::new();
    for run in runs {
        if let Some(cwd) = &run.cwd {
            if !seen.iter().any(|(_, known)| known == cwd) {
                seen.push((run.project.clone(), cwd.clone()));
            }
        }
    }
    seen.truncate(RECENT_LIMIT);
    seen
}

pub struct Composer {
    state: Entity<AppState>,
    wish: Entity<TextareaState>,
    turns: Entity<InputState>,
    choose_focus: FocusHandle,
    open: bool,
    folder: Option<PathBuf>,
    check: FolderCheck,
    probe: Option<Task<()>>,
    creative: bool,
    research: Activation,
    /// Which council members this machine can run; `None` until the background look finishes.
    members: Option<Members>,
    members_probe: Option<Task<()>>,
    advanced: bool,
    not_connected: bool,
    files: Files,
    files_task: Option<Task<()>>,
    mention: Option<Mention>,
    mention_task: Option<Task<()>>,
    mention_scroll: ScrollHandle,
    /// Window space below the wish box, measured when it was last drawn.
    mention_room: Rc<Cell<f32>>,
    _subscriptions: Vec<Subscription>,
}

impl Composer {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let wish = cx.new(|cx| {
            let mut wish = TextareaState::new(window, cx);
            wish.set_rows(6, cx);
            wish
        });
        let turns = cx.new(|cx| InputState::new(window, cx).validate(|text, _| text.chars().all(|c| c.is_ascii_digit())));
        let changed = |this: &mut Self, event: &InputEvent, cx: &mut Context<Self>| {
            if matches!(event, InputEvent::Change) {
                this.not_connected = false;
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if this.open && window.is_window_active() {
                    this.recheck(cx);
                }
            }),
            cx.subscribe_in(&wish, window, move |this, _, event, _, cx| {
                changed(this, event, cx);
                match event {
                    InputEvent::Change => this.update_mention(cx),
                    InputEvent::Blur => this.close_mention(cx),
                    _ => {}
                }
            }),
            cx.subscribe_in(&turns, window, move |this, _, event, _, cx| changed(this, event, cx)),
        ];
        cx.defer_in(window, |this, window, cx| this.apply_scene(window, cx));
        Self {
            choose_focus: cx.focus_handle(),
            state,
            wish,
            turns,
            open: false,
            folder: None,
            check: FolderCheck::Idle,
            probe: None,
            creative: false,
            research: Activation::default(),
            members: None,
            members_probe: None,
            advanced: false,
            not_connected: false,
            files: Files::None,
            files_task: None,
            mention: None,
            mention_task: None,
            mention_scroll: ScrollHandle::new(),
            mention_room: Rc::new(Cell::new(f32::MAX)),
            _subscriptions: subscriptions,
        }
    }

    /// `AppState::composing` is the only truth about whether the dialog is up.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.state.read(cx).composing;
        if composing == self.open {
            return;
        }
        self.open = composing;
        if composing {
            self.reset(window, cx);
            self.show(window, cx);
        } else {
            window.close_all_dialogs(cx);
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creative = false;
        // Every new wish starts with the council off; it is never carried over or remembered.
        self.research = Activation::default();
        self.look_for_members(cx);
        self.advanced = false;
        self.not_connected = false;
        self.close_mention(cx);
        // Read the files again each time the dialog opens: they change between wishes.
        self.files = Files::None;
        let (wish_hint, turns_hint) = (
            tr(cx, T, "Say what you want done and how you will know it is done."),
            tr(cx, T, "No limit"),
        );
        self.wish.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(wish_hint, window, cx);
        });
        self.turns.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.set_placeholder(turns_hint, window, cx);
        });
        let folder = self.state.read(cx).run().and_then(|r| r.cwd.clone());
        self.set_folder(folder, cx);
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            view.update(cx, |this, cx| {
                if first.replace(false) {
                    this.focus_first(window, cx);
                }
                // Frames are asked for only while the activation plays; once it settles, none.
                if this.research.wants_frame(Instant::now(), this.state.read(cx).reduce_motion) {
                    window.request_animation_frame();
                }
                this.fill(dialog, cx)
            })
        });
    }

    /// Looks on a background thread, so a slow PATH never holds the dialog.
    fn look_for_members(&mut self, cx: &mut Context<Self>) {
        self.members = None;
        let probe = cx.background_spawn(async { Members::detect() });
        self.members_probe = Some(cx.spawn(async move |this, cx| {
            let found = probe.await;
            let _ = this.update(cx, |this, cx| {
                this.members = Some(found);
                cx.notify();
            });
        }));
    }

    fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.folder.is_some() {
            self.wish.update(cx, |s, cx| s.focus(window, cx));
        } else {
            self.choose_focus.focus(window, cx);
        }
    }

    fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.folder = folder;
        self.recheck(cx);
        self.load_files(cx);
    }

    /// Reads the chosen folder's files in the background, once per folder.
    fn load_files(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            self.files = Files::None;
            self.files_task = None;
            return;
        };
        if matches!(&self.files, Files::Loading(f) | Files::Ready(f, _) | Files::Failed(f, _) if *f == folder) {
            return;
        }
        self.files = Files::Loading(folder.clone());
        let probe = cx.background_spawn({
            let folder = folder.clone();
            async move { list_files(&folder) }
        });
        self.files_task = Some(cx.spawn(async move |this, cx| {
            let found = probe.await;
            let _ = this.update(cx, |this, cx| {
                if !matches!(&this.files, Files::Loading(f) if *f == folder) {
                    return;
                }
                this.files = match found {
                    Ok(listing) => Files::Ready(folder, Arc::new(listing)),
                    Err(error) => Files::Failed(folder, error),
                };
                this.rank_mention(cx);
                cx.notify();
            });
        }));
    }

    /// Follows the text and cursor of the wish box: opens, narrows or closes the `@` list.
    fn update_mention(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let wish = self.wish.read(cx);
            (wish.value().to_string(), wish.cursor())
        };
        let Some((_, query)) = mention_at(&text, cursor) else {
            self.close_mention(cx);
            return;
        };
        if self.mention.as_ref().is_some_and(|m| m.query == query) {
            return;
        }
        let keep = self.mention.as_ref().and_then(|m| m.listing.clone());
        self.mention = Some(Mention { query: query.to_owned(), hits: vec![], listing: keep, selected: 0 });
        self.load_files(cx);
        self.rank_mention(cx);
        cx.notify();
    }

    fn close_mention(&mut self, cx: &mut Context<Self>) {
        self.mention_task = None;
        if self.mention.take().is_some() {
            cx.notify();
        }
    }

    /// Ranks the files for the current `@word`; a big project is ranked off the main thread and
    /// only the newest word's answer is kept.
    fn rank_mention(&mut self, cx: &mut Context<Self>) {
        let (Some(mention), Files::Ready(_, listing)) = (&self.mention, &self.files) else { return };
        let (listing, query) = (listing.clone(), mention.query.clone());
        if listing.entries.len() <= MENTION_SYNC {
            let hits = rank_paths(&listing.entries, &query, MENTION_LIMIT);
            self.take_hits(&query, listing, hits, cx);
            return;
        }
        self.mention_task = Some(cx.spawn(async move |this, cx| {
            let (ranked, word) = (listing.clone(), query.clone());
            let hits = cx.background_spawn(async move { rank_paths(&ranked.entries, &word, MENTION_LIMIT) }).await;
            let _ = this.update(cx, |this, cx| this.take_hits(&query, listing, hits, cx));
        }));
    }

    fn take_hits(&mut self, query: &str, listing: Arc<Listing>, hits: Vec<PathHit>, cx: &mut Context<Self>) {
        let Some(mention) = self.mention.as_mut().filter(|m| m.query == query) else { return };
        mention.hits = hits;
        mention.listing = Some(listing);
        mention.selected = 0;
        self.mention_scroll.scroll_to_item(0);
        cx.notify();
    }

    fn mention_open(&self) -> bool {
        self.mention.as_ref().is_some_and(|m| !m.hits.is_empty())
    }

    fn step_mention(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(mention) = self.mention.as_mut() else { return };
        let count = mention.hits.len() as isize;
        if count == 0 {
            return;
        }
        mention.selected = (mention.selected as isize + by).rem_euclid(count) as usize;
        self.mention_scroll.scroll_to_item(mention.selected);
        cx.notify();
    }

    /// Puts the chosen path where the `@word` was. Returns false when there was nothing to insert,
    /// so the key can do what it normally does.
    fn accept_mention(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(mention) = &self.mention else { return false };
        let Some(listing) = &mention.listing else { return false };
        let Some(hit) = mention.hits.get(index.unwrap_or(mention.selected)) else { return false };
        let path = listing.entries[hit.entry].clone();
        let (text, cursor) = {
            let wish = self.wish.read(cx);
            (wish.value().to_string(), wish.cursor())
        };
        let Some((span, _)) = mention_at(&text, cursor) else {
            self.close_mention(cx);
            return false;
        };
        let insert = mention_insert(&path, text[span.end..].starts_with(char::is_whitespace));
        self.close_mention(cx);
        self.wish.update(cx, |wish, cx| {
            wish.set_selected_range(span, cx);
            wish.replace(insert, window, cx);
            wish.focus(window, cx);
        });
        cx.notify();
        true
    }

    fn recheck(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.folder.clone() else {
            self.check = FolderCheck::Idle;
            self.probe = None;
            cx.notify();
            return;
        };
        self.check = FolderCheck::Checking;
        let probe = cx.background_spawn(async move { check_folder(&folder) });
        self.probe = Some(cx.spawn(async move |this, cx| {
            let found = probe.await;
            let _ = this.update(cx, |this, cx| {
                this.check = found;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pick_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr(cx, T, "Choose the project folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(folder) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                this.set_folder(Some(folder), cx);
                this.focus_first(window, cx);
            });
        })
        .detach();
    }

    fn verdict(&self, cx: &App) -> Result<Ready, Blocked> {
        validate(self.folder.is_some(), &self.check, &self.wish.read(cx).value(), &self.turns.read(cx).value())
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Ok(ready) = self.verdict(cx) else { return };
        let Some(cwd) = self.folder.clone() else { return };
        if self.state.read(cx).backend.is_none() {
            self.not_connected = true;
            cx.notify();
            return;
        }
        let request = request(cwd, ready, self.creative, &self.research);
        self.state.update(cx, |s, cx| {
            s.set_view(Screen::Run, cx);
            s.start_wish(request, cx);
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if asks_before_discarding(&self.wish.read(cx).value()) {
            let state = self.state.clone();
            ask(
                Ask {
                    title: tr(cx, T, "Discard this wish?"),
                    detail: tr(cx, T, "What you wrote will be lost."),
                    confirm: tr(cx, T, "Discard"),
                    back: tr(cx, T, "Keep writing"),
                    danger: true,
                    focus_confirm: false,
                },
                window,
                cx,
                move |_, cx| state.update(cx, |s, cx| s.set_composing(false, cx)),
            );
        } else {
            self.state.update(cx, |s, cx| s.set_composing(false, cx));
        }
    }

    fn fill(&mut self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let view = cx.entity().downgrade();
        dialog
            .w(px(640.))
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.cancel(window, cx));
                false
            })
            .title(tr(cx, T, "New wish"))
            .child(self.form(cx))
            .footer(self.footer(cx))
    }

    fn label(&self, text: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, T, text))
    }

    fn form(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                if action.secondary {
                    cx.stop_propagation();
                    this.start(cx);
                } else if !action.shift && this.mention_open() && this.accept_mention(None, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                cx.stop_propagation();
                if !(this.mention_open() && this.accept_mention(None, window, cx)) {
                    window.focus_next(cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.mention_open() {
                    cx.stop_propagation();
                    this.step_mention(-1, cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.mention_open() {
                    cx.stop_propagation();
                    this.step_mention(1, cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                if this.mention.is_some() {
                    cx.stop_propagation();
                    this.close_mention(cx);
                }
            }))
            .capture_action(|_: &OutdentInline, window, cx| {
                cx.stop_propagation();
                window.focus_prev(cx);
            })
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" && event.keystroke.modifiers.control {
                    cx.stop_propagation();
                    this.start(cx);
                }
            }))
            .child(self.folder_section(cx))
            .child(self.wish_section(cx))
            .child(self.creative_row(cx))
            .child(self.research_card(cx))
            .child(self.advanced_section(cx))
    }

    fn folder_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let tip = self.folder.as_ref().map(|f| f.display().to_string());
        let current = self.folder.clone();
        let recents: Vec<_> = recent_projects(&self.state.read(cx).runs);
        v_flex()
            .gap_2()
            .child(self.label("Project folder", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .id("composer-path")
                            .flex_1()
                            .min_w_0()
                            .h_8()
                            .px_2()
                            .gap_1p5()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.canvas)
                            .text_sm()
                            .when_some(tip.clone(), |this, tip| {
                                let tip: SharedString = tip.into();
                                this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            })
                            .child(Icon::new(IconName::Folder).size(px(14.)).text_color(t.muted))
                            .child(match tip {
                                Some(path) => div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis_start()
                                    .child(path)
                                    .into_any_element(),
                                None => div().text_color(t.muted).child(tr(cx, T, "No folder chosen yet")).into_any_element(),
                            }),
                    )
                    .child(
                        Button::new("composer-choose")
                            .outline()
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(tr(cx, T, "Choose folder…"))
                            .track_focus(&self.choose_focus)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_folder(window, cx))),
                    ),
            )
            .when(!recents.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap_1p5()
                        .flex_wrap()
                        .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Recent")))
                        .children(recents.into_iter().enumerate().map(|(i, (name, path))| {
                            let tip = path.display().to_string();
                            let selected = current.as_ref() == Some(&path);
                            Button::new(("composer-recent", i))
                                .outline()
                                .xsmall()
                                .label(name)
                                .selected(selected)
                                .tooltip(tip)
                                .on_click(cx.listener(move |this, _, _, cx| this.set_folder(Some(path.clone()), cx)))
                        })),
                )
            })
            .child(self.git_line(cx))
    }

    fn git_line(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let mono = cx.theme().mono_font_family.clone();
        let warn = |text: String, cx: &mut Context<Self>| -> AnyElement {
            let _ = &cx;
            h_flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap_1p5()
                .px_2()
                .py_1()
                .rounded(px(6.))
                .bg(t.review.opacity(0.12))
                .text_xs()
                .text_color(t.review)
                .child(div().pt(px(1.)).child(Icon::new(IconName::TriangleAlert).size(px(13.))))
                .child(div().min_w_0().child(text))
                .into_any_element()
        };
        let branch = |head: &Head, cx: &mut Context<Self>| -> AnyElement {
            let name = match head {
                Head::Branch(name) => name.clone(),
                Head::Unborn(name) => format!("{name} ({})", tr(cx, T, "no commits yet")),
                Head::Detached(sha) => tr(cx, T, "detached at {head}").replace("{head}", sha),
            };
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(tr(cx, T, "branch"))
                .child(
                    div()
                        .px_1p5()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.canvas)
                        .text_color(t.text)
                        .font_family(mono.clone())
                        .child(name),
                )
                .into_any_element()
        };
        let line: AnyElement = match &self.check {
            FolderCheck::Idle => div().into_any_element(),
            FolderCheck::Checking => div().text_xs().text_color(t.muted).child(tr(cx, T, "Checking this folder…")).into_any_element(),
            FolderCheck::Repo { head, changes } => {
                let warning = match (head, changes) {
                    (Head::Unborn(_), _) => Some(tr(cx, T, "No commits yet. Commit first so this can be undone.").to_owned()),
                    (_, 0) => None,
                    (_, n) => Some(tr(cx, T, "{n} uncommitted changes. Commit first so this can be undone.").replace("{n}", &n.to_string())),
                };
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().pt_1().child(branch(head, cx)))
                    .child(match warning {
                        Some(text) => warn(text, cx),
                        None => h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(t.muted)
                            .child(Icon::new(IconName::Check).size(px(12.)).text_color(t.ok))
                            .child(tr(cx, T, "No uncommitted changes"))
                            .into_any_element(),
                    })
                    .into_any_element()
            }
            FolderCheck::NotRepo => warn(tr(cx, T, "Not a git repository. You can still start, but changes cannot be undone with git.").to_owned(), cx),
            FolderCheck::Unknown(error) => warn(tr(cx, T, "Could not read the git state: {error}").replace("{error}", error), cx),
            FolderCheck::Missing => h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.fail)
                .child(Icon::new(IconName::CircleX).size(px(13.)))
                .child(tr(cx, T, "This folder does not exist."))
                .into_any_element(),
        };
        div().min_h_6().child(line)
    }

    fn wish_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let popup = self.mention_popup(cx);
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(self.label("Wish", cx))
                    .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Type @ to point at a file or folder."))),
            )
            .child(
                div()
                    .relative()
                    .child(Textarea::new(&self.wish).h(px(WISH_H)))
                    .child({
                        // Measures how far down the window the wish box ends, so the list can stay inside it.
                        let room = self.mention_room.clone();
                        canvas(
                            move |bounds, window, _| {
                                room.set(f32::from(window.viewport_size().height - bounds.bottom()));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                    })
                    .children(popup),
            )
    }

    /// The `@` list, drawn over the controls below the wish box.
    fn mention_popup(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mention = self.mention.as_ref()?;
        let t = cx.tokens();
        let note = |text: String| div().px_3().py_2().text_xs().text_color(t.muted).child(text);
        let mut rows: Vec<AnyElement> = vec![];
        let header: Option<String>;
        let mut footer = false;
        match (&self.files, &mention.listing) {
            (Files::None, _) => header = Some(tr(cx, T, "Choose a project folder first to point at its files.").to_owned()),
            (Files::Failed(_, error), _) => header = Some(fill(tr(cx, T, "The project's files could not be listed: {}"), &[error])),
            (Files::Loading(_), None) | (Files::Ready(..), None) => header = Some(tr(cx, T, "Reading the project's files…").to_owned()),
            (files, Some(listing)) => {
                let folder = match files {
                    Files::Ready(folder, _) | Files::Loading(folder) => folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    _ => String::new(),
                };
                if mention.hits.is_empty() {
                    header = Some(fill(tr(cx, T, "No file or folder matches “{}”."), &[&mention.query]));
                } else {
                    let mut text = fill(tr(cx, T, "Files in {}"), &[&folder]);
                    if listing.truncated {
                        text = format!("{text} {}", fill(tr(cx, T, "Showing the first {} files."), &[&listing.entries.len().to_string()]));
                    }
                    header = Some(text);
                    footer = true;
                    for (ix, hit) in mention.hits.iter().enumerate() {
                        rows.push(self.mention_row(ix, ix == mention.selected, &listing.entries[hit.entry], &hit.marks, cx).into_any_element());
                    }
                }
            }
        }
        let panel = v_flex()
            .id("composer-mention")
            .absolute()
            .top(px(WISH_H + 4.))
            .left_0()
            .right_0()
            .occlude()
            .rounded(px(4.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_md()
            .py_1()
            .children(header.map(note))
            .when(!rows.is_empty(), |this| {
                this.child(
                    div()
                        .id("composer-mention-rows")
                        .max_h(px(mention_rows_height(self.mention_room.get())))
                        .overflow_y_scroll()
                        .track_scroll(&self.mention_scroll)
                        .children(rows),
                )
            })
            .when(footer, |this| {
                this.child(
                    h_flex()
                        .mt_1()
                        .px_3()
                        .pt_1()
                        .gap_4()
                        .border_t_1()
                        .border_color(t.border)
                        // The same rows the shortcuts overlay shows, so the two never disagree.
                        .children(in_area(Area::Files).map(|shortcut| {
                            h_flex()
                                .gap_1()
                                .children(shortcut.keys.iter().map(|&keys| key_cap(keys)))
                                .child(div().pl_1().text_xs().text_color(t.muted).child(tr(cx, palette::T, shortcut.does)))
                        })),
                )
            });
        // Above the dialog, which is itself drawn late, at priority 10 and up.
        // `test_support` lets the headless keyboard tests see the list; it is nothing in the app.
        Some(deferred(panel.test_support()).with_priority(gpui_kit::base::POPUP_PRIORITY).into_any_element())
    }

    fn mention_row(&self, ix: usize, selected: bool, path: &str, marks: &[usize], cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let folder = path.ends_with('/');
        let trimmed = path.trim_end_matches('/');
        let name_at = trimmed.rfind('/').map_or(0, |slash| slash + 1);
        let (dir, name) = (&path[..name_at], &path[name_at..]);
        let dir_marks: Vec<usize> = marks.iter().copied().filter(|&m| m < name_at).collect();
        let name_marks: Vec<usize> = marks.iter().filter(|&&m| m >= name_at).map(|&m| m - name_at).collect();
        let strong = if selected { t.gold } else { t.text };
        h_flex()
            .id(("composer-mention-row", ix))
            .relative()
            .h(px(MENTION_ROW_H))
            .px_3()
            .gap_2()
            .cursor_pointer()
            .when(selected, |this| this.bg(t.raised))
            .hover(|this| this.bg(t.raised))
            .when(selected, |this| this.child(div().absolute().left_0().top(px(7.)).w(px(2.)).h(px(16.)).bg(t.gold)))
            // On mouse down, before the wish box hears that it lost the keyboard and closes the list.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.accept_mention(Some(ix), window, cx);
                }),
            )
            .child(Icon::new(if folder { IconName::Folder } else { IconName::File }).size(px(13.)).text_color(t.muted))
            .child(
                h_flex()
                    .min_w_0()
                    .text_sm()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(div().flex_shrink(1.).min_w_0().truncate().text_color(t.muted).child(marked(dir.to_owned().into(), &dir_marks, t.text)))
                    .child(div().flex_none().text_color(strong).child(marked(name.to_owned().into(), &name_marks, strong))),
            )
    }

    fn creative_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        h_flex()
            .gap_2()
            .child(
                Switch::new("composer-creative")
                    .checked(self.creative)
                    .accessibility_label(tr(cx, T, "Creative"))
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.creative = *checked;
                        cx.notify();
                    })),
            )
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, T, "Creative")))
            .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Let Chronos find one small, evidence-backed improvement.")))
    }

    fn research_card(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let look = self.research.look(Instant::now(), self.state.read(cx).reduce_motion);
        let shortfall = self.members.and_then(Members::shortfall).map(|text| tr(cx, T, text));
        div()
            .id("composer-research")
            .relative()
            .w_full()
            .rounded(px(8.))
            .border_1()
            .border_color(superswitch::border(look, &t))
            .bg(t.canvas)
            .overflow_hidden()
            .child(superswitch::backdrop(look, t))
            .child(
                h_flex()
                    .relative()
                    .min_h(px(84.))
                    .pl(px(superswitch::EMBLEM_GUTTER))
                    .pr_4()
                    .py_3()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr(cx, T, "Advanced research")))
                            .child(div().text_xs().text_color(t.muted).child(tr(
                                cx,
                                T,
                                "Astra, Fable 5.2 and Argon, three models from different families, deliberate before the plan. Fable is capped at $5; Argon's quota is small.",
                            )))
                            .children(shortfall.map(|text| {
                                h_flex()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(t.review)
                                    .child(Icon::new(IconName::TriangleAlert).size(px(12.)))
                                    .child(text)
                            })),
                    )
                    .child(
                        Switch::new("composer-research-switch")
                            .checked(self.research.is_on())
                            .accessibility_label(tr(cx, T, "Advanced research"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.research.set(*checked, Instant::now());
                                cx.notify();
                            })),
                    ),
            )
    }

    fn advanced_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let turns = self.turns.read(cx).value();
        let summary = (!self.advanced && !turns.trim().is_empty()).then(|| tr(cx, T, "Maximum turns: {n}").replace("{n}", turns.trim()));
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("composer-advanced")
                            .ghost()
                            .xsmall()
                            .icon(if self.advanced { IconName::ChevronDown } else { IconName::ChevronRight })
                            .label(tr(cx, T, "Advanced"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.advanced = !this.advanced;
                                cx.notify();
                            })),
                    )
                    .children(summary.map(|text| div().text_xs().text_color(t.muted).child(text))),
            )
            .when(self.advanced, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .pl_5()
                        .child(div().text_sm().child(tr(cx, T, "Maximum turns")))
                        .child(div().w(px(96.)).child(Input::new(&self.turns).small()))
                        .child(div().text_xs().text_color(t.muted).child(tr(cx, T, "Leave blank for no limit."))),
                )
            })
    }

    fn footer(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let verdict = self.verdict(cx);
        let blocked = verdict.as_ref().err().copied();
        let reason: AnyElement = if self.not_connected {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.fail)
                .child(Icon::new(IconName::CircleAlert).size(px(13.)))
                .child(tr(cx, T, "Not connected to a session, so this wish cannot start."))
                .into_any_element()
        } else if let Some(blocked) = blocked {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(Icon::new(IconName::Info).size(px(13.)))
                .child(tr(cx, T, blocked.reason()))
                .into_any_element()
        } else {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(t.muted)
                .child(Kbd::new(Keystroke::parse("ctrl-enter").expect("valid keystroke")))
                .child(tr(cx, T, "Start"))
                .into_any_element()
        };
        DialogFooter::new()
            .justify_between()
            .gap_3()
            .child(div().flex_1().min_w_0().child(reason))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("composer-cancel")
                            .label(tr(cx, T, "Cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        Button::new("composer-start")
                            .primary()
                            .label(tr(cx, T, "Start"))
                            .disabled(blocked.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    ),
            )
    }

    /// Demo only: opens the dialog in a named state so it can be captured.
    fn apply_scene(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(scene) = self.state.read(cx).scene.clone() else { return };
        let Some(scene) = scene.strip_prefix("compose") else { return };
        let scene = scene.strip_suffix("-min").map_or(scene, |base| {
            window.resize(size(px(1100.), px(700.)));
            base
        });
        self.state.update(cx, |s, _| s.composing = true);
        self.sync(window, cx);
        let wish = "把 RRAM 讀取流程改成背景批次，保留每一頁的引用。";
        let fill = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.wish.update(cx, |s, cx| s.set_value(wish, window, cx));
        };
        match scene {
            "-no-folder" => self.set_folder(None, cx),
            "-ready" => fill(self, window, cx),
            "-dirty" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("r6/ui-control".into()), changes: 3 };
            }
            "-clean" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 };
            }
            "-not-git" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::NotRepo;
            }
            "-missing" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Missing;
            }
            "-advanced" => {
                fill(self, window, cx);
                self.advanced = true;
                self.turns.update(cx, |s, cx| s.set_value("40", window, cx));
            }
            "-bad-turns" => {
                fill(self, window, cx);
                self.advanced = true;
                self.turns.update(cx, |s, cx| s.set_value("0", window, cx));
            }
            "-discard" => {
                fill(self, window, cx);
                self.cancel(window, cx);
            }
            "-research" => {
                // Shows the dialog off for a moment, then turns the switch on as a click would.
                fill(self, window, cx);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update(cx, |this, cx| {
                        this.research.set(true, Instant::now());
                        cx.notify();
                    });
                })
                .detach();
            }
            "-research-short" => {
                fill(self, window, cx);
                self.members_probe = None;
                self.members = Some(Members { fable: true, argon: false });
                self.research.set(true, Instant::now());
            }
            "-research-reduced" => {
                fill(self, window, cx);
                self.state.update(cx, |s, _| s.reduce_motion = true);
                self.research.set(true, Instant::now());
            }
            held if held.starts_with("-research-at-") => {
                fill(self, window, cx);
                if let Ok(ms) = held["-research-at-".len()..].parse::<u32>() {
                    self.research.hold(ms as f32 / 1000.0);
                }
            }
            "-at" => {
                // Points the dialog at this source tree and types the start of a file name.
                let text = "讓指令面板先列出名稱完全相同的結果，參考 @desk";
                self.set_folder(Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))), cx);
                self.wish.update(cx, |s, cx| {
                    s.set_value(text, window, cx);
                    s.set_selected_range(text.len()..text.len(), cx);
                });
                self.update_mention(cx);
            }
            "-not-connected" => {
                fill(self, window, cx);
                self.probe = None;
                self.check = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 };
                self.start(cx);
            }
            "" => {}
            _ => {}
        }
        cx.notify();
    }
}

impl Render for Composer {
    /// The dialog lives in the window's dialog layer; the shell only mounts this view while composing.
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named, because the glob above also brings in gpui's own `test` macro.
    use std::prelude::v1::test;
    use crate::desktop::demo;

    fn repo() -> FolderCheck {
        FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 }
    }

    #[test]
    fn the_file_list_shrinks_to_the_room_below_the_wish_box() {
        let full = MENTION_ROW_H * MENTION_ROWS;
        assert_eq!(mention_rows_height(f32::MAX), full);
        assert_eq!(mention_rows_height(1000.0), full);
        let cut = mention_rows_height(300.0);
        assert!(cut < full && cut + MENTION_CHROME + MENTION_EDGE <= 300.0, "{cut}");
        assert_eq!(mention_rows_height(0.0), MENTION_ROW_H, "never less than one row");
    }

    #[test]
    fn start_needs_a_folder_then_a_wish() {
        assert_eq!(validate(false, &FolderCheck::Idle, "wish", ""), Err(Blocked::NoFolder));
        assert_eq!(validate(true, &repo(), "", ""), Err(Blocked::NoWish));
        assert_eq!(validate(true, &repo(), "   \n ", ""), Err(Blocked::NoWish));
    }

    #[test]
    fn no_folder_is_reported_before_no_wish() {
        assert_eq!(validate(false, &FolderCheck::Idle, "", ""), Err(Blocked::NoFolder));
    }

    #[test]
    fn a_folder_that_is_gone_blocks_start() {
        assert_eq!(validate(true, &FolderCheck::Missing, "wish", ""), Err(Blocked::FolderMissing));
    }

    #[test]
    fn start_waits_while_the_folder_is_being_checked() {
        assert_eq!(validate(true, &FolderCheck::Checking, "wish", ""), Err(Blocked::Checking));
    }

    #[test]
    fn warnings_do_not_block_start() {
        let dirty = FolderCheck::Repo { head: Head::Branch("main".into()), changes: 4 };
        for check in [dirty, FolderCheck::NotRepo, FolderCheck::Unknown("x".into())] {
            assert!(validate(true, &check, "wish", "").is_ok(), "{check:?}");
        }
    }

    #[test]
    fn a_ready_wish_is_trimmed_and_carries_its_limit() {
        assert_eq!(validate(true, &repo(), "  wish\n", "40"), Ok(Ready { wish: "wish".into(), max_turns: Some(40) }));
        assert_eq!(validate(true, &repo(), "wish", ""), Ok(Ready { wish: "wish".into(), max_turns: None }));
    }

    #[test]
    fn maximum_turns_must_be_a_positive_whole_number() {
        assert_eq!(parse_turns(""), Some(None));
        assert_eq!(parse_turns("  "), Some(None));
        assert_eq!(parse_turns("12"), Some(Some(12)));
        assert_eq!(parse_turns("0"), None);
        assert_eq!(parse_turns("-3"), None);
        assert_eq!(parse_turns("1.5"), None);
        assert_eq!(parse_turns("99999999999"), None);
        assert_eq!(validate(true, &repo(), "wish", "0"), Err(Blocked::BadTurns));
    }

    #[test]
    fn the_request_carries_the_research_switch_only_when_it_was_turned_on() {
        let ready = || Ready { wish: "wish".into(), max_turns: Some(9) };
        let off = request(PathBuf::from("C:\\p"), ready(), true, &Activation::default());
        assert!(!off.advanced_research);
        assert_eq!((off.wish.as_str(), off.creative, off.max_turns), ("wish", true, Some(9)));
        let mut switch = Activation::default();
        switch.set(true, Instant::now());
        assert!(request(PathBuf::from("C:\\p"), ready(), false, &switch).advanced_research);
        switch.set(false, Instant::now());
        assert!(!request(PathBuf::from("C:\\p"), ready(), false, &switch).advanced_research);
    }

    #[test]
    fn cancelling_asks_only_when_there_is_text_to_lose() {
        assert!(!asks_before_discarding(""));
        assert!(!asks_before_discarding(" \n\t"));
        assert!(asks_before_discarding("x"));
    }

    #[test]
    fn git_status_gives_the_branch_and_the_number_of_changes() {
        let out = "# branch.oid 1a2b3c4d5e\n# branch.head main\n# branch.upstream origin/main\n1 .M N... 100644 100644 100644 a b src/a.rs\n? new.txt\n";
        assert_eq!(parse_status(out), FolderCheck::Repo { head: Head::Branch("main".into()), changes: 2 });
    }

    #[test]
    fn a_clean_tree_has_no_changes() {
        let out = "# branch.oid 1a2b3c4d5e\n# branch.head feature/x\n";
        assert_eq!(parse_status(out), FolderCheck::Repo { head: Head::Branch("feature/x".into()), changes: 0 });
    }

    #[test]
    fn a_detached_head_and_an_empty_repository_are_named() {
        let detached = "# branch.oid 1a2b3c4d5e6f\n# branch.head (detached)\n";
        assert_eq!(parse_status(detached), FolderCheck::Repo { head: Head::Detached("1a2b3c4".into()), changes: 0 });
        let unborn = "# branch.oid (initial)\n# branch.head main\n";
        assert_eq!(parse_status(unborn), FolderCheck::Repo { head: Head::Unborn("main".into()), changes: 0 });
    }

    #[test]
    fn git_failures_are_told_apart() {
        assert_eq!(classify_git(false, "", "fatal: not a git repository (or any of the parent directories): .git"), FolderCheck::NotRepo);
        assert_eq!(classify_git(false, "", "fatal: detected dubious ownership\nmore"), FolderCheck::Unknown("fatal: detected dubious ownership".into()));
        assert_eq!(classify_git(true, "# branch.oid 1\n# branch.head main\n", ""), FolderCheck::Repo { head: Head::Branch("main".into()), changes: 0 });
    }

    #[test]
    fn a_folder_that_cannot_be_found_is_missing() {
        assert_eq!(check_folder(Path::new("Z:\\no\\such\\folder\\anywhere")), FolderCheck::Missing);
    }

    #[test]
    fn an_at_sign_after_a_space_or_chinese_text_starts_a_mention() {
        let text = "see @src/pa";
        assert_eq!(mention_at(text, text.len()), Some((4..11, "src/pa")));
        assert_eq!(mention_at("@", 1), Some((0..1, "")));
        let zh = "參考@desk";
        assert_eq!(mention_at(zh, zh.len()), Some(("參考".len()..zh.len(), "desk")));
        // The cursor in the middle of a word: the query is what was typed before it, the span the whole word.
        assert_eq!(mention_at("x @src/lib.rs y", 6), Some((2..13, "src")));
    }

    #[test]
    fn an_email_address_or_finished_word_is_not_a_mention() {
        assert_eq!(mention_at("mail a@b.com", 12), None);
        assert_eq!(mention_at("@src/lib.rs done", 16), None);
        assert_eq!(mention_at("no at sign", 5), None);
        assert_eq!(mention_at("@@x", 3), None);
        assert_eq!(mention_at("abc", 99), None, "a cursor past the end is ignored");
    }

    #[test]
    fn the_inserted_path_is_quoted_when_it_has_spaces_and_spaced_from_what_follows() {
        assert_eq!(mention_insert("src/lib.rs", false), "src/lib.rs ");
        assert_eq!(mention_insert("src/lib.rs", true), "src/lib.rs");
        assert_eq!(mention_insert("docs/My Notes.md", false), "\"docs/My Notes.md\" ");
        let text = "fix @src/li please";
        let (span, _) = mention_at(text, 11).unwrap();
        let replaced = format!("{}{}{}", &text[..span.start], mention_insert("src/lib.rs", text[span.end..].starts_with(' ')), &text[span.end..]);
        assert_eq!(replaced, "fix src/lib.rs please");
    }

    #[test]
    fn a_listing_holds_every_file_and_the_folders_on_the_way() {
        let listing = listing_from_files(["src/desktop/palette.rs", "src/lib.rs", "README.md", "", "src/lib.rs"], 100);
        assert_eq!(listing.entries, vec!["README.md", "src/", "src/desktop/", "src/desktop/palette.rs", "src/lib.rs"]);
        assert!(!listing.truncated);
        let capped = listing_from_files(["a", "b", "c"], 2);
        assert_eq!(capped.entries, vec!["a", "b"]);
        assert!(capped.truncated);
    }

    #[test]
    fn gitignore_rules_follow_git() {
        let rules = IgnoreRules::parse("# build\ntarget/\n*.log\n/notes.txt\ndocs/**/draft*\n!keep.log\n");
        assert!(rules.ignored("target", true));
        assert!(!rules.ignored("target", false), "target/ names folders only");
        assert!(rules.ignored("a/b/run.log", false));
        assert!(!rules.ignored("keep.log", false), "a later ! rule brings a file back");
        assert!(rules.ignored("notes.txt", false));
        assert!(!rules.ignored("sub/notes.txt", false), "a leading / anchors to the top");
        assert!(rules.ignored("docs/a/b/draft-1.md", false));
        assert!(!rules.ignored("src/draft.md", false));
    }

    #[test]
    fn globs_stop_single_stars_at_slashes() {
        assert!(glob("*.rs", "lib.rs"));
        assert!(!glob("*.rs", "src/lib.rs"));
        assert!(glob("src/**/*.rs", "src/a/b/lib.rs"));
        assert!(glob("src/**/lib.rs", "src/lib.rs"));
        assert!(glob("?at", "cat"));
        assert!(!glob("?at", "/at"));
    }

    #[test]
    fn a_folder_outside_git_is_walked_without_what_it_ignores() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        for dir in ["src", "build", "node_modules/pkg", ".hidden"] {
            std::fs::create_dir_all(p.join(dir)).unwrap();
        }
        for file in [".gitignore", "src/main.rs", "src/debug.log", "build/out.bin", "node_modules/pkg/index.js", ".hidden/x", "README.md"] {
            std::fs::write(p.join(file), if file == ".gitignore" { "build/\n*.log\n" } else { "x" }).unwrap();
        }
        let listing = walk(p, 100);
        assert_eq!(listing.entries, vec!["README.md", "src/", "src/main.rs"]);
        assert!(!walk(p, 2).entries.contains(&"src/main.rs".to_owned()));
        assert!(walk(p, 2).truncated);
    }

    #[test]
    fn a_git_repository_lists_tracked_and_new_files_but_not_ignored_ones() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        let run = |args: &[&str]| git(p, args).output().map(|o| o.status.success()).unwrap_or(false);
        assert!(run(&["init", "-q"]), "git must be installed to list repositories");
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::create_dir_all(p.join("target/debug")).unwrap();
        std::fs::write(p.join(".gitignore"), "target/\n*.tmp\n").unwrap();
        for file in ["src/lib.rs", "target/debug/app.exe", "scratch.tmp", "notes with space.md"] {
            std::fs::write(p.join(file), "x").unwrap();
        }
        assert!(run(&["add", "src/lib.rs"]));
        let listing = list_files(p).expect("listed");
        assert_eq!(listing.entries, vec![".gitignore", "notes with space.md", "src/", "src/lib.rs"]);
        assert!(list_files(&p.join("nope")).is_err());
    }

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn ranked(entries: &[String], query: &str) -> Vec<String> {
        rank_paths(entries, query, 10).into_iter().map(|hit| entries[hit.entry].clone()).collect()
    }

    #[test]
    fn a_name_match_beats_a_match_spread_along_the_path() {
        let entries = paths(&["src/desktop/", "src/desktop/palette.rs", "src/desktop/superswitch.rs", "src/pal/notes.md", "docs/app-design.md"]);
        assert_eq!(ranked(&entries, "pal")[0], "src/desktop/palette.rs");
        assert_eq!(ranked(&entries, "desk")[0], "src/desktop/", "folders are offered too");
        assert!(!ranked(&entries, "pal").contains(&"docs/app-design.md".to_owned()));
    }

    #[test]
    fn equal_names_go_to_the_shorter_path_and_slashes_match_the_whole_path() {
        let entries = paths(&["src/desktop/model.rs", "src/desktop/mod.rs", "tests/mod.rs"]);
        assert_eq!(ranked(&entries, "mod"), vec!["tests/mod.rs", "src/desktop/mod.rs", "src/desktop/model.rs"]);
        assert_eq!(ranked(&entries, "desk/mod")[..2], ["src/desktop/mod.rs", "src/desktop/model.rs"]);
        assert_eq!(ranked(&entries, "desk\\mod")[0], "src/desktop/mod.rs", "Windows separators work");
    }

    #[test]
    fn marks_point_at_the_matched_characters_of_the_path() {
        let entries = paths(&["src/desktop/palette.rs"]);
        let hit = &rank_paths(&entries, "pal", 10)[0];
        assert_eq!(hit.marks, vec![12, 13, 14]);
    }

    #[test]
    fn an_empty_query_lists_the_top_of_the_project_folders_first() {
        let entries = paths(&["README.md", "src/", "src/lib.rs", "Cargo.toml", "docs/"]);
        assert_eq!(ranked(&entries, ""), vec!["docs/", "src/", "Cargo.toml", "README.md"]);
    }

    #[test]
    fn a_big_repository_is_ranked_quickly() {
        let entries: Vec<String> = (0..60_000).map(|i| format!("crates/c{}/src/module_{}/file_{i}.rs", i % 97, i % 31)).collect();
        let started = Instant::now();
        let hits = rank_paths(&entries, "file_4242", MENTION_LIMIT);
        let took = started.elapsed();
        assert_eq!(entries[hits[0].entry], "crates/c71/src/module_26/file_4242.rs");
        // It runs off the main thread; this only guards against it becoming quadratic.
        assert!(took < Duration::from_secs(3), "{took:?} for {} paths", entries.len());
    }

    #[test]
    fn every_translation_key_used_here_exists() {
        let source = include_str!("composer.rs");
        let used = regex::Regex::new(r#"tr\(cx, T, "((?:[^"\\]|\\.)*)"\)"#).unwrap();
        let mut found = 0;
        for capture in used.captures_iter(source) {
            found += 1;
            let known = T.iter().chain(super::super::i18n::COMMON.iter()).any(|(en, _)| *en == &capture[1]);
            assert!(known, "missing translation: {}", &capture[1]);
        }
        assert!(found > 20);
    }

    #[test]
    fn recent_projects_are_distinct_folders_in_run_order() {
        let mut runs = demo::state().runs;
        runs[3].cwd = None;
        let recent = recent_projects(&runs);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].0.as_ref(), "ml_hfo2");
        let mut many = demo::state().runs;
        for (i, run) in many.iter_mut().enumerate() {
            run.cwd = Some(PathBuf::from(format!("C:\\p{i}")));
        }
        assert_eq!(recent_projects(&many).len(), 4);
    }
}
