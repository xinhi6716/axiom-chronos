#![allow(dead_code)]
mod pids;
pub use pids::*;
use std::{
    collections::HashMap,
    fs, mem,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, Once, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, HWND, LPARAM, RECT, STILL_ACTIVE,
    },
    Graphics::Gdi::*,
    System::{
        ProcessStatus::K32EnumProcesses,
        Threading::{
            GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        },
    },
    UI::{
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub rect: (i32, i32, i32, i32),
    pub primary: bool,
    pub hz: u32,
    pub dpi: u32,
    pub device: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Secondary,
    Offscreen,
}
pub fn place(monitors: &[Monitor]) -> Place {
    if monitors.iter().any(|m| !m.primary) {
        Place::Secondary
    } else {
        Place::Offscreen
    }
}
pub fn place_args(p: Place) -> [&'static str; 2] {
    [
        "--place",
        match p {
            Place::Secondary => "secondary",
            Place::Offscreen => "offscreen",
        },
    ]
}
pub fn monitor_of(monitors: &[Monitor], rect: (i32, i32, i32, i32)) -> Option<&Monitor> {
    monitors
        .iter()
        .filter_map(|m| {
            let w = (i64::from(rect.2.min(m.rect.2)) - i64::from(rect.0.max(m.rect.0))).max(0);
            let h = (i64::from(rect.3.min(m.rect.3)) - i64::from(rect.1.max(m.rect.1))).max(0);
            (w > 0 && h > 0).then_some((w * h, m))
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, m)| m)
}
unsafe fn monitor_info(handle: HMONITOR) -> Option<Monitor> {
    let mut info: MONITORINFOEXW = mem::zeroed();
    info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(handle, &mut info.monitorInfo) == 0 {
        return None;
    }
    let mut mode: DEVMODEW = mem::zeroed();
    mode.dmSize = mem::size_of::<DEVMODEW>() as u16;
    if EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
        return None;
    }
    let (mut dx, mut dy) = (0, 0);
    if GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 || dx == 0 {
        return None;
    }
    let r = info.monitorInfo.rcMonitor;
    let len = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    Some(Monitor {
        rect: (r.left, r.top, r.right, r.bottom),
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        hz: mode.dmDisplayFrequency,
        dpi: dx,
        device: String::from_utf16_lossy(&info.szDevice[..len]),
    })
}
unsafe extern "system" fn visit_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
    let list = &mut *(data as *mut Vec<Monitor>);
    match monitor_info(h) {
        Some(m) => {
            list.push(m);
            1
        }
        None => 0,
    }
}
pub fn monitors() -> Vec<Monitor> {
    let mut list = Vec::new();
    assert_ne!(
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(visit_monitor),
                &mut list as *mut _ as LPARAM,
            )
        },
        0,
        "enumerating monitor details failed"
    );
    list
}
fn registry() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/isolation-pids.jsonl")
}
static FIRST: Once = Once::new();
static HOMES: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
fn homes() -> &'static Mutex<HashMap<PathBuf, u64>> {
    HOMES.get_or_init(|| Mutex::new(HashMap::new()))
}
fn filetime(time: SystemTime) -> u64 {
    116_444_736_000_000_000u64.saturating_add(
        time.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .saturating_div(100)
            .min(u64::MAX as u128) as u64,
    )
}
fn stop_if_same(pid: u32, start: u64) {
    unsafe {
        let h = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if !h.is_null() {
            if pids::handle_creation_time(h) == Some(start) {
                TerminateProcess(h, 1);
            }
            CloseHandle(h);
        }
    }
}
fn alive(pid: u32, start: u64) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        CloseHandle(h);
        ok && creation_time(pid) == Some(start)
    }
}
fn service_alive(pid: u32, expected: Option<u64>) -> Result<Option<u64>, String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            let error = GetLastError();
            return if error == ERROR_INVALID_PARAMETER {
                Ok(None)
            } else {
                Err(format!(
                    "cannot query service pid {pid}: Windows error {error}"
                ))
            };
        }
        let start = pids::handle_creation_time(h);
        let mut code = 0;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        let start =
            start.ok_or_else(|| format!("cannot read creation time of service pid {pid}"))?;
        if !ok {
            return Err(format!("cannot read exit status of service pid {pid}"));
        }
        Ok((code == STILL_ACTIVE as u32 && expected.is_none_or(|s| s == start)).then_some(start))
    }
}
fn image(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(h);
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}
fn service_image(pid: u32) -> bool {
    image(pid)
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .is_some_and(|n| {
            n.eq_ignore_ascii_case("chronos.exe") || n.eq_ignore_ascii_case("axiom.exe")
        })
}
fn service_pid(home: &Path) -> Result<Option<u32>, String> {
    let data = match fs::read(home.join("service/service.json")) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read service record: {e}")),
    };
    let v: serde_json::Value =
        serde_json::from_slice(&data).map_err(|e| format!("invalid service record: {e}"))?;
    v["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())
        .filter(|&p| p != 0)
        .map(Some)
        .ok_or_else(|| "service record has no valid pid".into())
}
pub struct Launched {
    pub child: Child,
    pub place: Place,
    home: PathBuf,
    since: u64,
    recorded_service: Mutex<Option<(u32, u64)>>,
}
pub fn launch(exe: &Path, home: &Path, args: &[&str], stderr: Stdio) -> Launched {
    FIRST.call_once(|| {
        let observed = std::cell::Cell::new(None);
        sweep(
            &registry(),
            &|pid| {
                let start = creation_time(pid);
                observed.set(start.map(|s| (pid, s)));
                start
            },
            &|pid| {
                if let Some((_, start)) = observed.get().filter(|(seen, _)| *seen == pid) {
                    stop_if_same(pid, start);
                }
            },
        );
    });
    let chosen = place(&monitors());
    let since = filetime(SystemTime::now());
    let child = Command::new(exe)
        .args(args)
        .args(place_args(chosen))
        .env("AXIOM_HOME", home)
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("launch application");
    let launched = Launched {
        child,
        place: chosen,
        home: home.to_owned(),
        since,
        recorded_service: Mutex::new(None),
    };
    homes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(home.to_owned(), since);
    let pid = launched.child.id();
    record(
        &registry(),
        pid,
        creation_time(pid).expect("read launched process creation time"),
    );
    launched
}
impl Launched {
    fn record_service(&self) -> Result<(), String> {
        if let Some(pid) = service_pid(&self.home)? {
            if let Some(start) = creation_time(pid) {
                if start < self.since || !service_image(pid) {
                    return Err(format!(
                        "service pid {pid} is not a process started by this launch"
                    ));
                }
                let mut seen = self
                    .recorded_service
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if *seen != Some((pid, start)) {
                    record(&registry(), pid, start);
                    *seen = Some((pid, start));
                }
            }
        }
        Ok(())
    }
}
impl Drop for Launched {
    fn drop(&mut self) {
        let pid = self.child.id();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            stop_serve_since(&self.home, Duration::from_secs(10), Some(self.since))
        }));
        let error = match cleanup {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("service cleanup panicked".into()),
        };
        if let Some(error) = error {
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), "cleanup after pid {pid}: {error}");
        }
        homes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.home);
    }
}
pub fn stop_serve(home: &Path, timeout: Duration) -> Result<(), String> {
    let since = homes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(home)
        .copied();
    stop_serve_since(home, timeout, since)
}
fn stop_serve_since(home: &Path, timeout: Duration, since: Option<u64>) -> Result<(), String> {
    let Some(pid) = service_pid(home)? else {
        return Ok(());
    };
    let Some(start) = service_alive(pid, None)? else {
        return Ok(());
    };
    let request = axiom::service::Client::connect(home).and_then(|client| client.quit(true));
    let until = Instant::now() + timeout;
    while service_alive(pid, Some(start))?.is_some() && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    if service_alive(pid, Some(start))?.is_none() {
        return Ok(());
    }
    if since.is_some_and(|s| start >= s) && service_image(pid) {
        stop_if_same(pid, start);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while service_alive(pid, Some(start))?.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    if service_alive(pid, Some(start))?.is_some() {
        Err(format!("service pid {pid} still alive; quit: {request:?}"))
    } else {
        Ok(())
    }
}
pub fn strays(exe: &Path, since: SystemTime) -> Vec<u32> {
    let mut list = vec![0u32; 1024];
    loop {
        let mut bytes = 0;
        assert_ne!(
            unsafe { K32EnumProcesses(list.as_mut_ptr(), (list.len() * 4) as u32, &mut bytes) },
            0,
            "K32EnumProcesses failed"
        );
        if bytes as usize == list.len() * 4 {
            list.resize(list.len() * 2, 0);
            continue;
        }
        list.truncate(bytes as usize / 4);
        break;
    }
    let expected = fs::canonicalize(exe)
        .unwrap_or_else(|_| exe.to_owned())
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_lowercase();
    list.into_iter()
        .filter(|&pid| {
            let Some(start) = creation_time(pid) else {
                return false;
            };
            start >= filetime(since)
                && alive(pid, start)
                && image(pid).is_some_and(|p| {
                    p.to_string_lossy()
                        .trim_start_matches(r"\\?\")
                        .eq_ignore_ascii_case(&expected)
                })
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct Shown {
    pub hwnd: HWND,
    pub monitor: Option<Monitor>,
    pub hz: u32,
    pub dpi: u32,
}
pub fn shown(l: &Launched, hwnd: HWND) -> Result<Shown, String> {
    l.record_service()?;
    let mut r: RECT = unsafe { mem::zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
        return Err("GetWindowRect failed".into());
    }
    let monitors = monitors();
    let m = monitor_of(&monitors, (r.left, r.top, r.right, r.bottom)).cloned();
    if let Some(primary) = m.as_ref().filter(|m| m.primary) {
        return Err(format!("window is on primary display {}", primary.device));
    }
    match (l.place, &m) {
        (Place::Secondary, None) => return Err("window is not on a secondary display".into()),
        (Place::Offscreen, Some(m)) => {
            return Err(format!("offscreen window overlaps display {}", m.device))
        }
        _ => {}
    }
    let display = match &m {
        Some(m) => m.clone(),
        None => unsafe { monitor_info(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)) }
            .ok_or("nearest display unavailable")?,
    };
    Ok(Shown {
        hwnd,
        monitor: m,
        hz: display.hz,
        dpi: display.dpi,
    })
}
pub fn not_foreground(l: &Launched) -> Result<(), String> {
    l.record_service()?;
    let mut pid = 0;
    unsafe {
        let w = GetForegroundWindow();
        if !w.is_null() {
            GetWindowThreadProcessId(w, &mut pid);
        }
    }
    if pid == l.child.id() {
        Err(format!("application pid {pid} is foreground"))
    } else {
        Ok(())
    }
}
pub mod input {
    use super::*;
    fn packed(x: i32, y: i32) -> isize {
        (((y as u16 as u32) << 16) | x as u16 as u32) as isize
    }
    pub fn post_click(hwnd: HWND, client: (i32, i32)) -> Result<(), String> {
        let pos = packed(client.0, client.1);
        for (msg, flags) in [(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)] {
            if unsafe { PostMessageW(hwnd, msg, flags, pos) } == 0 {
                return Err(format!("PostMessageW click {msg:#x} failed"));
            }
        }
        Ok(())
    }
    pub fn post_wheel(hwnd: HWND, delta: i16) -> Result<(), String> {
        let mut r: RECT = unsafe { mem::zeroed() };
        if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
            return Err("GetWindowRect failed".into());
        }
        let pos = packed(
            r.left + (r.right - r.left) * 3 / 4,
            r.top + (r.bottom - r.top) / 2,
        );
        if unsafe { PostMessageW(hwnd, WM_MOUSEWHEEL, (delta as u16 as usize) << 16, pos) } == 0 {
            Err("PostMessageW wheel failed".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
pub fn spawn_serve(exe: &Path, home: &Path) -> Child {
    use std::os::windows::process::CommandExt;
    Command::new(exe)
        .arg("serve")
        .env("AXIOM_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x00000008 | 0x00000200)
        .spawn()
        .expect("spawn detached service")
}
