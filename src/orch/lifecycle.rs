use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub enum Begin {
    Run { brief: Brief, attempt: u32 },
    Reused,
}
pub struct Finished {
    pub state: String,
    pub verify_exit: Option<i32>,
    pub verify_tail: String,
    pub outside_scope: Vec<String>,
}
struct Claim {
    // Units of different projects never block or excuse each other.
    store: PathBuf,
    scope: Vec<String>,
    files: BTreeMap<String, String>,
    peers: Vec<Vec<String>>,
}
static RUNNING: OnceLock<Mutex<HashMap<String, Claim>>> = OnceLock::new();
fn key(store: &Path, unit: &str) -> String {
    format!("{}|{unit}", store.display())
}
fn claims() -> &'static Mutex<HashMap<String, Claim>> {
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(crate) fn contents(repo: &Path) -> Res<BTreeMap<String, String>> {
    input_contents(repo, None)
}

/// Directory records use an impossible filesystem-name prefix. It lets scope
/// auditing distinguish a parent directory from an ordinary sibling file.
pub(crate) const DIRECTORY_RECORD: &str = "\0dir:";

fn input_error(message: impl Into<String>) -> Failure {
    Failure::new("input_snapshot", message)
}

fn is_git_metadata(name: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        name.to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(".git"))
    }
    #[cfg(not(windows))]
    {
        name == std::ffi::OsStr::new(".git")
    }
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    let link = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Directory junctions and other reparse points need not be symlinks.
        link || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        link
    }
}

fn input_permissions(meta: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        u32::from(meta.permissions().readonly())
    }
}

#[cfg(unix)]
fn input_file_id(file: &std::fs::File) -> Res<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|e| input_error(e.to_string()))?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn input_file_id(file: &std::fs::File) -> Res<(u64, u64)> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) };
    if ok == 0 {
        return Err(input_error(std::io::Error::last_os_error().to_string()));
    }
    let info = unsafe { info.assume_init() };
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    if index == 0 {
        return Err(input_error("input file identity is unavailable"));
    }
    Ok((u64::from(info.dwVolumeSerialNumber), index))
}

fn open_input(path: &Path) -> Res<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: inspect a replaced leaf instead of
        // following it to an unverified target.
        options.custom_flags(0x0020_0000);
    }
    options.open(path).map_err(|e| input_error(e.to_string()))
}

fn open_directory(path: &Path) -> Res<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000 | 0x0200_0000);
    }
    options.open(path).map_err(|e| input_error(e.to_string()))
}

fn regular_hash(path: &Path) -> Res<String> {
    regular_hash_with(path, || Ok(()))
}

fn regular_hash_with(path: &Path, after_read: impl FnOnce() -> Res<()>) -> Res<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = open_input(path)?;
    let before = file.metadata().map_err(|e| input_error(e.to_string()))?;
    if is_link(&before) || !before.is_file() {
        return Err(input_error("input is not a regular file"));
    }
    let identity = input_file_id(&file)?;
    let before_time = before.modified().map_err(|e| input_error(e.to_string()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| input_error(e.to_string()))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    after_read()?;
    let after = file.metadata().map_err(|e| input_error(e.to_string()))?;
    let named_file = open_input(path)?;
    let named = named_file
        .metadata()
        .map_err(|e| input_error(e.to_string()))?;
    if is_link(&named)
        || input_file_id(&named_file)? != identity
        || before.len() != after.len()
        || before.len() != named.len()
        || input_permissions(&before) != input_permissions(&after)
        || input_permissions(&before) != input_permissions(&named)
        || before_time != after.modified().map_err(|e| input_error(e.to_string()))?
        || before_time != named.modified().map_err(|e| input_error(e.to_string()))?
    {
        return Err(input_error("input changed while being hashed"));
    }
    Ok(format!(
        "{}:{:x}",
        input_permissions(&before),
        hash.finalize()
    ))
}

#[cfg(windows)]
fn stream_names(path: &Path) -> Res<Vec<(String, u64)>> {
    use std::{mem::MaybeUninit, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Foundation::{ERROR_HANDLE_EOF, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
            WIN32_FIND_STREAM_DATA,
        },
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data = MaybeUninit::<WIN32_FIND_STREAM_DATA>::zeroed();
    let handle = unsafe {
        FindFirstStreamW(
            wide.as_ptr(),
            FindStreamInfoStandard,
            data.as_mut_ptr().cast(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
            Ok(Vec::new())
        } else {
            Err(input_error(format!(
                "cannot enumerate input streams: {error}"
            )))
        };
    }
    struct FindGuard(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for FindGuard {
        fn drop(&mut self) {
            unsafe {
                FindClose(self.0);
            }
        }
    }
    let _guard = FindGuard(handle);
    let mut streams = Vec::new();
    loop {
        let entry = unsafe { data.assume_init() };
        let end = entry
            .cStreamName
            .iter()
            .position(|c| *c == 0)
            .ok_or_else(|| input_error("unterminated input stream name"))?;
        let name = String::from_utf16(&entry.cStreamName[..end])
            .map_err(|_| input_error("invalid Unicode input stream name"))?;
        if !name.starts_with(':') || !name.ends_with(":$DATA") || entry.StreamSize < 0 {
            return Err(input_error("unsupported input stream"));
        }
        if name != "::$DATA" {
            streams.push((name, entry.StreamSize as u64));
        }
        let next = unsafe { FindNextStreamW(handle, data.as_mut_ptr().cast()) };
        if next == 0 {
            let error = std::io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(v) if v == ERROR_HANDLE_EOF as i32 || v == ERROR_NO_MORE_FILES as i32)
            {
                return Err(input_error(format!(
                    "cannot enumerate input streams: {error}"
                )));
            }
            break;
        }
    }
    streams.sort();
    if streams.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(input_error("duplicate input stream name"));
    }
    Ok(streams)
}

#[cfg(windows)]
fn stream_digest(path: &Path, directory: bool) -> Res<String> {
    use sha2::{Digest, Sha256};
    let open_base = || {
        if directory {
            open_directory(path)
        } else {
            open_input(path)
        }
    };
    let start = open_base()?;
    let start_meta = start.metadata().map_err(|e| input_error(e.to_string()))?;
    if is_link(&start_meta) || start_meta.is_dir() != directory {
        return Err(input_error("input stream base changed type"));
    }
    let identity = input_file_id(&start)?;
    let before = stream_names(path)?;
    let mut hash = Sha256::new();
    hash.update(b"axiom-ntfs-streams-v1\0");
    for (name, size) in &before {
        let mut stream = path.as_os_str().to_os_string();
        stream.push(name);
        if input_file_id(&open_input(Path::new(&stream))?)? != identity {
            return Err(input_error("input stream belongs to a different file"));
        }
        let content = regular_hash(Path::new(&stream))?;
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update(size.to_be_bytes());
        hash.update((content.len() as u64).to_be_bytes());
        hash.update(content.as_bytes());
    }
    let end = open_base()?;
    let end_meta = end.metadata().map_err(|e| input_error(e.to_string()))?;
    if stream_names(path)? != before
        || is_link(&end_meta)
        || end_meta.is_dir() != directory
        || input_file_id(&end)? != identity
    {
        return Err(input_error("input streams changed while being hashed"));
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(not(windows))]
fn stream_digest(_path: &Path, _directory: bool) -> Res<String> {
    Ok("not-supported-on-this-platform".into())
}

fn file_value(path: &Path) -> Res<String> {
    let original = open_input(path)?;
    let identity = input_file_id(&original)?;
    let bytes = regular_hash(path)?;
    let streams = stream_digest(path, false)?;
    if input_file_id(&open_input(path)?)? != identity {
        return Err(input_error("input replaced while being hashed"));
    }
    Ok(format!("file:{bytes}:{streams}"))
}

fn directory_value(path: &Path, meta: &std::fs::Metadata) -> Res<String> {
    Ok(format!(
        "directory:{}:{}",
        input_permissions(meta),
        stream_digest(path, true)?
    ))
}

/// Enumerate actual worktree entries, including ignored files and empty
/// directories. Git metadata is deliberately outside this input contract.
/// File bytes are hashed through a bounded buffer and are never written to
/// Git objects or reports. Access errors and unsafe links fail closed.
pub(crate) fn input_contents(
    repo: &Path,
    requested: Option<&[String]>,
) -> Res<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let inside = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|e| input_error(e.to_string()))?;
    if !inside.status.success() || inside.stdout != b"true\n" && inside.stdout != b"true\r\n" {
        return Err(input_error("cannot enumerate a Git worktree"));
    }
    let root = repo
        .canonicalize()
        .map_err(|e| input_error(e.to_string()))?;
    let mut effective = requested.unwrap_or(&[]).to_vec();
    #[cfg(windows)]
    for path in requested.unwrap_or(&[]) {
        if path == "." || path.contains(['*', '?', '[']) || path.starts_with(':') {
            continue;
        }
        let original = Path::new(path);
        // A scoped NTFS stream is identified by its containing file; all of
        // that file's streams must enter the manifest.
        let base = original.file_name().and_then(|n| n.to_str()).and_then(|n| {
            n.split_once(':')
                .map(|(file, _)| original.with_file_name(file))
        });
        let candidate = base.as_deref().unwrap_or(original);
        if let Some(base) = &base {
            effective.push(base.to_string_lossy().replace('\\', "/"));
        }
        let selected = repo.join(candidate);
        if selected.exists() {
            let resolved = selected
                .canonicalize()
                .map_err(|e| input_error(e.to_string()))?;
            let relative = resolved
                .strip_prefix(&root)
                .map_err(|_| input_error("scoped input escapes repository"))?;
            if relative
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(n) if is_git_metadata(n)))
            {
                return Err(input_error("scoped input reaches Git metadata"));
            }
            let actual = relative
                .to_str()
                .ok_or_else(|| input_error("non-UTF-8 scoped input"))?
                .replace('\\', "/");
            if !actual.is_empty() {
                effective.push(actual);
            }
        }
    }
    // Git permits a directory path without a trailing slash. Preserve that
    // snapshot behaviour while regular scope checks still require the slash.
    for path in requested.unwrap_or(&[]) {
        if path != "." && !path.ends_with('/') && repo.join(path).is_dir() {
            effective.push(format!("{}/", path.replace('\\', "/")));
        }
    }
    // For Git pathspec wildcards, conservatively cover the whole worktree:
    // a scope change may invalidate extra evidence, but cannot go unnoticed.
    let all = requested.is_none()
        || effective
            .iter()
            .any(|s| s == "." || s.contains(['*', '?', '[']) || s.starts_with(':'));
    #[cfg(windows)]
    let casefolded: Vec<_> = effective.iter().map(|s| s.to_ascii_lowercase()).collect();
    #[cfg(windows)]
    let non_ascii_scope = effective.iter().any(|s| !s.is_ascii());
    let selected = |path: &str| {
        #[cfg(windows)]
        {
            // Include uncertain Unicode matches conservatively; guessing NTFS
            // case semantics could otherwise omit a real scoped input.
            non_ascii_scope || !path.is_ascii() || covers(&casefolded, &path.to_ascii_lowercase())
        }
        #[cfg(not(windows))]
        {
            covers(&effective, path)
        }
    };
    if requested.is_some() && effective.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut result = BTreeMap::new();
    let root_meta = std::fs::symlink_metadata(&root).map_err(|e| input_error(e.to_string()))?;
    if is_link(&root_meta) || !root_meta.is_dir() {
        return Err(input_error("worktree root is not a regular directory"));
    }
    result.insert(format!("{DIRECTORY_RECORD}./"), String::new());
    let mut stack = vec![(root.clone(), PathBuf::new())];
    while let Some((dir, relative)) = stack.pop() {
        let before_handle = open_directory(&dir)?;
        let before_id = input_file_id(&before_handle)?;
        let before_meta = before_handle
            .metadata()
            .map_err(|e| input_error(e.to_string()))?;
        if is_link(&before_meta) || !before_meta.is_dir() {
            return Err(input_error("input directory changed into a link"));
        }
        let before = before_meta
            .modified()
            .map_err(|e| input_error(e.to_string()))?;
        let relative_name = if relative.as_os_str().is_empty() {
            "./".into()
        } else {
            format!(
                "{}/",
                relative
                    .to_str()
                    .ok_or_else(|| input_error("non-UTF-8 input path"))?
                    .replace('\\', "/")
            )
        };
        let marker = format!("{DIRECTORY_RECORD}{relative_name}");
        let before_value = directory_value(&dir, &before_meta)?;
        result.insert(marker, before_value.clone());
        let entries = std::fs::read_dir(&dir).map_err(|e| input_error(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| input_error(e.to_string()))?;
            if is_git_metadata(&entry.file_name()) {
                continue;
            }
            let rel = relative.join(entry.file_name());
            let name = rel
                .to_str()
                .ok_or_else(|| input_error("non-UTF-8 input path"))?;
            // A backslash is a valid Unix filename character but also the
            // Windows path separator used by scope normalization. Refuse the
            // ambiguous Unix name instead of merging two distinct files.
            #[cfg(unix)]
            if name.contains('\\') {
                return Err(input_error("ambiguous backslash in Unix input path"));
            }
            let name = name.replace('\\', "/");
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| input_error(e.to_string()))?;
            if is_link(&meta) {
                // A directory link may be an ancestor of an explicitly scoped
                // file. Inspect it before Git can traverse that path.
                let possible_parent = format!("{DIRECTORY_RECORD}{name}/");
                if !all && !selected(&name) && !selected(&possible_parent) {
                    continue;
                }
                let resolved = path
                    .canonicalize()
                    .map_err(|e| input_error(e.to_string()))?;
                let inside = resolved
                    .strip_prefix(&root)
                    .map_err(|_| input_error("input link escapes repository"))?;
                if inside
                    .components()
                    .any(|c| matches!(c, std::path::Component::Normal(n) if is_git_metadata(n)))
                {
                    return Err(input_error("input link reaches Git metadata"));
                }
                if !std::fs::metadata(&resolved)
                    .map_err(|e| input_error(e.to_string()))?
                    .is_file()
                {
                    return Err(input_error(
                        "directory or special-file input link is unsupported",
                    ));
                }
                let target = std::fs::read_link(&path).map_err(|e| input_error(e.to_string()))?;
                let target_text = target
                    .to_str()
                    .ok_or_else(|| input_error("non-UTF-8 input link target"))?;
                let value = format!(
                    "link:{:x}:{}",
                    Sha256::digest(target_text.as_bytes()),
                    file_value(&resolved)?
                );
                if std::fs::read_link(&path).map_err(|e| input_error(e.to_string()))? != target
                    || path
                        .canonicalize()
                        .map_err(|e| input_error(e.to_string()))?
                        != resolved
                {
                    return Err(input_error("input link changed while being hashed"));
                }
                result.insert(name, value);
            } else if meta.is_dir() {
                let marker = format!("{DIRECTORY_RECORD}{name}/");
                if all || selected(&marker) {
                    result.insert(marker, String::new());
                    stack.push((path, rel));
                }
            } else if meta.is_file() {
                if all || selected(&name) {
                    result.insert(name, file_value(&path)?);
                }
            } else if all || selected(&name) {
                return Err(input_error("special-file input is unsupported"));
            }
        }
        let after_handle = open_directory(&dir)?;
        let after_meta = after_handle
            .metadata()
            .map_err(|e| input_error(e.to_string()))?;
        let after = after_meta
            .modified()
            .map_err(|e| input_error(e.to_string()))?;
        if is_link(&after_meta)
            || !after_meta.is_dir()
            || input_file_id(&after_handle)? != before_id
            || before != after
            || input_permissions(&before_meta) != input_permissions(&after_meta)
            || before_value != directory_value(&dir, &after_meta)?
        {
            return Err(input_error(
                "input directory changed while being enumerated",
            ));
        }
    }
    Ok(result)
}
fn dependencies(store: &Path, u: &Unit, units: &[Unit]) -> Res<Vec<String>> {
    if !store.join("modules.md").exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(store.join("modules.md"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let map = parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))?;
    let deps = map
        .modules
        .iter()
        .find(|m| m.id == u.track)
        .map(|m| m.depends.as_slice())
        .unwrap_or(&[]);
    let mut missing = Vec::new();
    for other in units.iter().filter(|x| deps.contains(&x.track)) {
        let rows = evidence(store, &other.id)?;
        let valid = rows.iter().rposition(|r| r.kind == "run").is_some_and(|i| {
            let run = &rows[i];
            let scope = rows[i + 1..].iter().find(|r| r.kind == "scope");
            let verify = rows[i + 1..].iter().find(|r| r.kind == "verify");
            !run.commit.is_empty()
                && run.exit_code == 0
                && scope.is_some_and(|r| r.exit_code == 0 && r.commit == run.commit)
                && verify.is_some_and(|r| r.exit_code == 0 && r.commit == run.commit)
        });
        if !valid {
            missing.push(other.id.clone());
        }
    }
    Ok(missing)
}
pub fn begin_unit(
    store: &Path,
    unit: &str,
    _role: &str,
    repo: &Path,
    astra_required: bool,
) -> Res<Begin> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let u = units
        .iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let brief = unit_brief(store, unit)?;
    let rows = evidence(store, unit)?;
    let missing = dependencies(store, u, &units)?;
    if !missing.is_empty() {
        return Err(Failure::new("not_ready", missing.join(", ")));
    }
    if active.contains_key(&key(store, unit))
        || active
            .values()
            .any(|c| c.store == store && overlaps(&brief.scope, &c.scope))
    {
        return Err(Failure::new("busy", format!("scope busy: {unit}")));
    }
    if derive_state(&rows, needs_accept(store, unit)?) == "verified"
        && rows
            .iter()
            .rev()
            .find(|r| r.kind == "verify")
            .is_some_and(|r| !r.commit.is_empty() && snapshot(repo, &brief.scope) == r.commit)
    {
        return Ok(Begin::Reused);
    }
    let count = rows.iter().filter(|r| r.kind == "run").count();
    if count >= 2 {
        return Err(Failure::new(
            "fix_limit",
            format!("unit {unit} has reached its fix limit"),
        ));
    }
    if astra_required {
        astra_gate(store)?;
    }
    let files = contents(repo)?;
    let peers: Vec<_> = active
        .values()
        .filter(|c| c.store == store)
        .map(|c| c.scope.clone())
        .collect();
    for peer in active.values_mut().filter(|c| c.store == store) {
        peer.peers.push(brief.scope.clone());
    }
    active.insert(
        key(store, unit),
        Claim {
            store: store.to_path_buf(),
            scope: brief.scope.clone(),
            files,
            peers,
        },
    );
    Ok(Begin::Run {
        brief,
        attempt: count as u32 + 1,
    })
}
pub fn abort_unit(store: &Path, unit: &str) {
    claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key(store, unit));
}
pub fn running() -> Vec<String> {
    let mut v: Vec<_> = claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .map(|k| k.rsplit('|').next().unwrap_or(k).to_owned())
        .collect();
    v.sort();
    v
}
pub fn ready_units(store: &Path) -> Res<Vec<String>> {
    let active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let mut selected: Vec<Vec<String>> = active
        .values()
        .filter(|c| c.store == store)
        .map(|c| c.scope.clone())
        .collect();
    let mut result = vec![];
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let state = derive_state(&rows, needs_accept(store, &u.id)?);
        if !["pending", "failed"].contains(&state.as_str())
            || rows.iter().filter(|r| r.kind == "run").count() >= 2
            || !dependencies(store, u, &units)?.is_empty()
        {
            continue;
        }
        let brief = match unit_brief(store, &u.id) {
            Ok(b) => b,
            Err(e) if e.kind == "no_brief" => continue,
            Err(e) => return Err(e),
        };
        if selected.iter().any(|s| overlaps(s, &brief.scope)) {
            continue;
        }
        selected.push(brief.scope);
        result.push(u.id.clone());
    }
    Ok(result)
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Waiting {
    pub unit: String,
    pub state: String,
    pub next: String,
}

pub fn waiting_units(store: &Path) -> Res<Vec<Waiting>> {
    let ready = ready_units(store)?;
    let active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let mut waiting = Vec::new();
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let needs = needs_accept(store, &u.id)?;
        let state = derive_state(&rows, needs);
        if state == "verified" || ready.contains(&u.id) {
            continue;
        }
        let next = if active.contains_key(&key(store, &u.id)) {
            "running".to_string()
        } else if state == "built" {
            let verify = rows.iter().rposition(|r| r.kind == "verify");
            if needs && verify.is_some_and(|v| !rows[v + 1..].iter().any(|r| r.kind == "accept")) {
                "accept".into()
            } else {
                "review".into()
            }
        } else if state == "accept-failed" || state == "review-failed" {
            "fix".into()
        } else {
            let missing = dependencies(store, u, &units)?;
            if !missing.is_empty() {
                format!("wait {}", missing.join(","))
            } else if rows.iter().filter(|r| r.kind == "run").count() >= 2
                || unit_brief(store, &u.id).is_err()
            {
                "blocked".into()
            } else {
                "busy".into()
            }
        };
        waiting.push(Waiting {
            unit: u.id.clone(),
            state,
            next,
        });
    }
    Ok(waiting)
}
pub fn finish_unit(
    store: &Path,
    unit: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
) -> Res<Finished> {
    struct Release<'a>(&'a Path, &'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            abort_unit(self.0, self.1);
        }
    }
    let _release = Release(store, unit);
    let brief = unit_brief(store, unit)?;
    let (before, others) = {
        let active = claims().lock().unwrap_or_else(|e| e.into_inner());
        let c = active
            .get(&key(store, unit))
            .ok_or_else(|| Failure::new("not_found", "unit is not running"))?;
        (
            c.files.clone(),
            c.peers.iter().flatten().cloned().collect::<Vec<_>>(),
        )
    };
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let mk = |kind: &str, command: &str, code: i32, commit: String| Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: code,
        commit,
        ts: base::now_iso(),
    };
    let result_snapshot = snapshot(repo, &brief.scope);
    append_evidence(
        store,
        unit,
        &mk(
            "run",
            model,
            if status == "completed" { 0 } else { 1 },
            result_snapshot.clone(),
        ),
    )?;
    let dir = store.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let md = dir.join(format!("{unit}.md"));
    base::atomic_write(
        &md,
        report
            .get("final_message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes(),
    )
    .map_err(|e| Failure::new("io", e.to_string()))?;
    let json = serde_json::to_vec_pretty(report)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    base::atomic_write(&dir.join(format!("{unit}.json")), &json)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    append_inbox(store, role, unit, status, Some(&md.to_string_lossy()))?;
    let after = contents(repo)?;
    let mut paths: BTreeSet<String> = before
        .keys()
        .chain(after.keys())
        .filter(|p| {
            before.get(*p) != after.get(*p)
                && !covers_change(&others, p, before.contains_key(*p), after.contains_key(*p))
        })
        .cloned()
        .collect();
    if let Some(changes) = report.get("file_changes").and_then(Value::as_array) {
        for c in changes {
            if let Some(p) = c.get("path").and_then(Value::as_str) {
                let p = Path::new(p);
                let p = if p.is_absolute() {
                    p.strip_prefix(repo).unwrap_or(p)
                } else {
                    p
                };
                paths.insert(p.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let outside_scope: Vec<_> = paths
        .into_iter()
        .filter(|p| {
            !covers_change(
                &brief.scope,
                p,
                before.contains_key(p),
                after.contains_key(p),
            )
        })
        .map(|p| {
            p.strip_prefix(DIRECTORY_RECORD)
                .map(str::to_owned)
                .unwrap_or(p)
        })
        .collect();
    append_evidence(
        store,
        unit,
        &mk(
            "scope",
            &outside_scope.join(" "),
            if outside_scope.is_empty() { 0 } else { 1 },
            result_snapshot.clone(),
        ),
    )?;
    let mut verify_exit = None;
    let mut verify_tail = String::new();
    if status == "completed" && outside_scope.is_empty() {
        let result = base::proc::run_shell(&brief.verify, repo, Duration::from_secs(1800))?;
        verify_exit = Some(result.exit_code);
        verify_tail = result
            .output
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        append_evidence(
            store,
            unit,
            &mk(
                "verify",
                &brief.verify,
                result.exit_code,
                snapshot(repo, &brief.scope),
            ),
        )?;
        let current = snapshot(repo, &brief.scope);
        if result.exit_code == 0 && !current.is_empty() && current == result_snapshot {
            let root =
                std::fs::canonicalize(repo).map_err(|e| Failure::new("io", e.to_string()))?;
            let provenance = serde_json::json!({"repo":root, "unit":unit, "snapshot":current});
            base::atomic_write(
                &dir.join(format!("{unit}.repo.json")),
                &serde_json::to_vec(&provenance).map_err(|e| Failure::new("io", e.to_string()))?,
            )
            .map_err(|e| Failure::new("io", e.to_string()))?;
        }
    }
    let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
    set_unit_state(store, unit, &state)?;
    Ok(Finished {
        state,
        verify_exit,
        verify_tail,
        outside_scope,
    })
}

#[cfg(test)]
mod input_snapshot_tests {
    use super::*;

    #[test]
    fn replacement_with_same_length_and_timestamp_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("input");
        let replacement = tmp.path().join("replacement");
        std::fs::write(&path, b"before").unwrap();
        std::fs::write(&replacement, b"after!").unwrap();
        let timestamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
        assert_eq!(
            std::fs::metadata(&replacement).unwrap().modified().unwrap(),
            timestamp
        );
        let result = regular_hash_with(&path, || {
            std::fs::rename(&replacement, &path).map_err(|e| input_error(e.to_string()))
        });
        assert!(
            result.is_err(),
            "same-length replacement must not be accepted"
        );
    }
}
