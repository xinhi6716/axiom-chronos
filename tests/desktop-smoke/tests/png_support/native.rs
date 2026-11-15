#[path = "../isolation_support/mod.rs"]
mod isolation;
use super::Png;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Gdi::*,
    Storage::Xps::PrintWindow,
    UI::{
        HiDpi::{
            GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
        WindowsAndMessaging::*,
    },
};

struct DpiContext(DPI_AWARENESS_CONTEXT);
impl DpiContext {
    fn enter() -> Result<Self, String> {
        let old =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if old.is_null() {
            Err("cannot set per-monitor DPI awareness".into())
        } else {
            Ok(Self(old))
        }
    }
}
impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
pub struct App {
    child: Mutex<isolation::Launched>,
    _temp: tempfile::TempDir,
    log: PathBuf,
    stderr: PathBuf,
}
pub fn launch(
    exe: &Path,
    home: &Path,
    target: &str,
    state: &str,
    lang: &str,
    theme: &str,
    size: (u32, u32),
) -> App {
    let temp = tempfile::tempdir().expect("temporary capture directory");
    let log = temp.path().join("perf.jsonl");
    let stderr = temp.path().join("stderr.log");
    let err = fs::File::create(&stderr).expect("stderr capture");
    let size_arg = format!("{}x{}", size.0, size.1);
    let log_arg = log.to_str().expect("capture log path is not UTF-8");
    let mut args = vec!["--demo", "--target", target, "--state", state];
    if lang == "en" {
        args.push("--english");
    }
    if theme == "light" {
        args.push("--light");
    }
    args.extend(["--size", &size_arg, "--perf-log", log_arg]);
    App {
        child: Mutex::new(isolation::launch(exe, home, &args, Stdio::from(err))),
        _temp: temp,
        log,
        stderr,
    }
}
fn check(app: &App) -> Result<(), String> {
    let stderr = fs::read_to_string(&app.stderr).unwrap_or_default();
    if stderr.contains("unregistered target") {
        return Err(stderr);
    }
    if let Some(status) = app
        .child
        .lock()
        .unwrap()
        .child
        .try_wait()
        .map_err(|e| e.to_string())?
    {
        return Err(format!("application exited {status}: {stderr}"));
    }
    Ok(())
}
struct Search {
    pid: u32,
    hwnd: HWND,
    area: i64,
}
unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
    let search = &mut *(param as *mut Search);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == search.pid && IsWindowVisible(hwnd) != 0 {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) != 0 {
            let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
            if area > search.area {
                search.area = area;
                search.hwnd = hwnd;
            }
        }
    }
    1
}
fn window(app: &App) -> Result<HWND, String> {
    let mut s = Search {
        pid: app.child.lock().unwrap().child.id(),
        hwnd: std::ptr::null_mut(),
        area: 0,
    };
    unsafe {
        EnumWindows(Some(visit), &mut s as *mut _ as isize);
    }
    if s.hwnd.is_null() {
        Err("no visible application window".into())
    } else {
        Ok(s.hwnd)
    }
}
pub fn dpi(app: &App) -> u32 {
    let Ok(_dpi_context) = DpiContext::enter() else {
        return 0;
    };
    window(app)
        .map(|w| unsafe { GetDpiForWindow(w) })
        .unwrap_or(0)
}
pub fn capture(app: &App) -> Result<Png, String> {
    let _dpi_context = DpiContext::enter()?;
    let start = Instant::now();
    loop {
        check(app)?;
        let ready = fs::read_to_string(&app.log)
            .unwrap_or_default()
            .lines()
            .any(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .is_some_and(|v| v["t"] == "first_frame")
            });
        if ready {
            break;
        }
        if start.elapsed() >= Duration::from_secs(30) {
            return Err("first_frame timed out".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(500));
    check(app)?;
    let hwnd = window(app)?;
    {
        let launched = app.child.lock().unwrap();
        isolation::shown(&launched, hwnd)?;
        isolation::not_foreground(&launched)?;
    }
    let result = unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err("GetWindowRect failed".into());
        }
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        if w <= 0 || h <= 0 {
            return Err("empty window".into());
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return Err("GetDC failed".into());
        }
        let mem = CreateCompatibleDC(dc);
        if mem.is_null() {
            ReleaseDC(hwnd, dc);
            return Err("CreateCompatibleDC failed".into());
        }
        let bitmap = CreateCompatibleBitmap(dc, w, h);
        if bitmap.is_null() {
            DeleteDC(mem);
            ReleaseDC(hwnd, dc);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let old = SelectObject(mem, bitmap);
        let printed = PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT);
        SelectObject(mem, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let lines = GetDIBits(
            mem,
            bitmap,
            0,
            h as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteObject(bitmap);
        DeleteDC(mem);
        ReleaseDC(hwnd, dc);
        if printed == 0 || lines != h {
            Err::<Png, String>("PrintWindow or GetDIBits failed".into())
        } else {
            for p in pixels.chunks_exact_mut(4) {
                p.swap(0, 2);
                p[3] = 255;
            }
            Ok(Png {
                width: w as u32,
                height: h as u32,
                rgba: pixels,
            })
        }
    }?;
    check(app)?;
    if !result.nonblank() {
        return Err("blank or near-uniform capture".into());
    }
    Ok(result)
}
