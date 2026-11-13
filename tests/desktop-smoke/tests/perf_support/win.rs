use std::{
    mem,
    os::windows::io::AsRawHandle,
    process::Child,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, HANDLE, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        ClientToScreen, EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW,
        ENUM_CURRENT_SETTINGS,
    },
    System::{
        ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::GetProcessTimes,
    },
    UI::{
        HiDpi::GetDpiForSystem,
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEINPUT,
        },
        WindowsAndMessaging::{
            EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
            IsWindowVisible, PostMessageW, SetCursorPos, SetForegroundWindow, WM_MOUSEWHEEL,
        },
    },
};

pub fn machine() -> (String, u32, u32) {
    unsafe {
        let mut mode: DEVMODEW = mem::zeroed();
        mode.dmSize = mem::size_of::<DEVMODEW>() as u16;
        assert_ne!(
            EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode),
            0,
            "EnumDisplaySettingsW failed"
        );
        let mut display: DISPLAY_DEVICEW = mem::zeroed();
        display.cb = mem::size_of::<DISPLAY_DEVICEW>() as u32;
        let gpu = if EnumDisplayDevicesW(std::ptr::null(), 0, &mut display, 0) != 0 {
            let len = display
                .DeviceString
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(display.DeviceString.len());
            String::from_utf16_lossy(&display.DeviceString[..len])
        } else {
            "unknown".into()
        };
        (
            if gpu.trim().is_empty() {
                "unknown".into()
            } else {
                gpu
            },
            mode.dmDisplayFrequency,
            GetDpiForSystem(),
        )
    }
}
unsafe extern "system" fn find(hwnd: HWND, arg: LPARAM) -> i32 {
    let data = &mut *(arg as *mut (u32, HWND));
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == data.0 && IsWindowVisible(hwnd) != 0 {
        data.1 = hwnd;
        return 0;
    }
    1
}
pub fn window(pid: u32) -> Option<HWND> {
    let mut data: (u32, HWND) = (pid, std::ptr::null_mut());
    unsafe {
        EnumWindows(Some(find), &mut data as *mut _ as LPARAM);
    }
    (!data.1.is_null()).then_some(data.1)
}
pub fn wait_window(child: &mut Child) -> HWND {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(w) = window(child.id()) {
            return w;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "app exited before showing a window"
        );
        assert!(Instant::now() < deadline, "app window did not appear");
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn click(w: HWND) -> bool {
    unsafe {
        SetForegroundWindow(w);
        if GetForegroundWindow() != w {
            return false;
        }
        let mut point = POINT { x: 100, y: 100 };
        assert_ne!(ClientToScreen(w, &mut point), 0, "ClientToScreen failed");
        assert_ne!(SetCursorPos(point.x, point.y), 0, "SetCursorPos failed");
        let mouse = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let input = [mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)];
        assert_eq!(
            SendInput(
                input.len() as u32,
                input.as_ptr(),
                mem::size_of::<INPUT>() as i32
            ),
            input.len() as u32,
            "SendInput failed"
        );
        true
    }
}
pub fn wheel(w: HWND, delta: i16) {
    unsafe {
        let mut rect: RECT = mem::zeroed();
        assert_ne!(GetWindowRect(w, &mut rect), 0, "GetWindowRect failed");
        let x = rect.left + (rect.right - rect.left) * 3 / 4;
        let y = rect.top + (rect.bottom - rect.top) / 2;
        let location = (((y as u16 as u32) << 16) | x as u16 as u32) as isize;
        assert_ne!(
            PostMessageW(w, WM_MOUSEWHEEL, (delta as u16 as usize) << 16, location),
            0
        );
    }
}
pub fn cpu_ticks(h: HANDLE) -> u64 {
    unsafe {
        let (mut a, mut b, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) = mem::zeroed();
        assert_ne!(
            GetProcessTimes(h, &mut a, &mut b, &mut k, &mut u),
            0,
            "GetProcessTimes failed"
        );
        let value = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        value(k) + value(u)
    }
}
pub fn cpu_percent(start: u64, end: u64, wall: Duration) -> f64 {
    (end - start) as f64 / 10_000_000.0 / wall.as_secs_f64() * 100.0
}
pub fn handle(child: &Child) -> HANDLE {
    child.as_raw_handle() as HANDLE
}
pub fn private_mib(child: &Child) -> f64 {
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = mem::zeroed();
        c.cb = mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        assert_ne!(
            K32GetProcessMemoryInfo(
                handle(child),
                &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                c.cb
            ),
            0,
            "GetProcessMemoryInfo failed"
        );
        c.PrivateUsage as f64 / 1048576.0
    }
}
