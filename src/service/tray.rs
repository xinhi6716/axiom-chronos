//! A real tray is available on Windows only. macOS and Linux use a no-op tray;
//! the service is controlled from the desktop app.
use crate::base::{Failure, Res};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::Sender, Arc, Mutex};
#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    Open,
    PauseAll,
    Quit,
}
#[derive(Debug, Clone)]
pub struct TrayStatus {
    pub running: usize,
    pub text: String,
}
pub trait Tray: Send {
    fn update(&mut self, s: &TrayStatus);
    fn close(&mut self);
}
pub trait Desktop: Send + Sync {
    fn open(&self) -> Res<()>;
    fn confirm_quit(&self, running: usize) -> Res<()>;
}
pub fn status_text(running: usize) -> String {
    match running {
        0 => "Chronos: idle".into(),
        1 => "Chronos: 1 wish running".into(),
        n => format!("Chronos: {n} wishes running"),
    }
}
#[derive(Clone, Default)]
pub struct Recording {
    pub calls: Arc<Mutex<Vec<String>>>,
}
impl Recording {
    pub fn new(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { calls }
    }
}
impl Tray for Recording {
    fn update(&mut self, s: &TrayStatus) {
        self.calls.lock().unwrap().push(s.text.clone());
    }
    fn close(&mut self) {
        self.calls.lock().unwrap().push("close".into());
    }
}
impl Desktop for Recording {
    fn open(&self) -> Res<()> {
        self.calls.lock().unwrap().push("open".into());
        Ok(())
    }
    fn confirm_quit(&self, n: usize) -> Res<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("confirm_quit({n})"));
        Ok(())
    }
}
struct Noop;
impl Tray for Noop {
    fn update(&mut self, _: &TrayStatus) {}
    fn close(&mut self) {}
}
pub trait Spawner: Send + Sync {
    fn spawn(&self, exe: &Path, args: &[String]) -> Res<()>;
}
pub struct ProcessSpawner;
impl Spawner for ProcessSpawner {
    fn spawn(&self, exe: &Path, args: &[String]) -> Res<()> {
        let mut command = crate::base::proc::cmd(exe);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000200 | 0x00000008 | 0x08000000);
        }
        command
            .spawn()
            .map(|_| ())
            .map_err(|e| Failure::new("desktop", e.to_string()))
    }
}
pub struct Launcher {
    app: Option<PathBuf>,
    spawner: Box<dyn Spawner>,
}
impl Launcher {
    pub fn new(app: Option<PathBuf>, spawner: Box<dyn Spawner>) -> Self {
        Self { app, spawner }
    }
    fn launch(&self, args: &[String]) -> Res<()> {
        let app = self
            .app
            .as_deref()
            .ok_or_else(|| Failure::new("no_desktop", "no desktop app found; set AXIOM_APP_BIN"))?;
        self.spawner.spawn(app, args)
    }
}
impl Desktop for Launcher {
    fn open(&self) -> Res<()> {
        self.launch(&[])
    }
    fn confirm_quit(&self, _: usize) -> Res<()> {
        self.launch(&["--confirm-quit".into()])
    }
}
pub fn locate_app(env_app_bin: Option<&str>, current_exe: &Path) -> Option<PathBuf> {
    if let Some(path) = env_app_bin
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let sibling = current_exe.with_file_name(if cfg!(windows) {
        "chronos.exe"
    } else {
        "chronos"
    });
    if sibling.is_file() {
        return Some(sibling);
    }
    if current_exe.file_stem().is_some_and(|s| s == "chronos") {
        return Some(current_exe.to_path_buf());
    }
    None
}
pub fn platform_tray(sender: Sender<TrayAction>) -> Box<dyn Tray> {
    #[cfg(windows)]
    if let Some(tray) = windows_tray::WindowsTray::start(sender) {
        return Box::new(tray);
    }
    #[cfg(not(windows))]
    let _ = sender;
    Box::new(Noop)
}
pub fn platform_desktop() -> Box<dyn Desktop> {
    let app = std::env::current_exe()
        .ok()
        .and_then(|exe| locate_app(std::env::var("AXIOM_APP_BIN").ok().as_deref(), &exe));
    Box::new(Launcher::new(app, Box::new(ProcessSpawner)))
}

#[cfg(windows)]
mod windows_tray {
    use super::*;
    use std::{
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
        sync::{mpsc, OnceLock},
        thread::{self, JoinHandle},
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
                NOTIFYICONDATAW,
            },
            WindowsAndMessaging::*,
        },
    };
    const CALLBACK: u32 = WM_APP + 1;
    const UPDATE: u32 = WM_APP + 2;
    const CLASS: &[u16] = &[
        65, 120, 105, 111, 109, 67, 104, 114, 111, 110, 111, 115, 84, 114, 97, 121, 0,
    ];
    static REGISTERED: OnceLock<bool> = OnceLock::new();

    pub struct WindowsTray {
        hwnd: isize,
        thread: Option<JoinHandle<()>>,
    }
    impl WindowsTray {
        pub fn start(sender: Sender<TrayAction>) -> Option<Self> {
            let (tx, rx) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || unsafe { run(sender, tx) });
            match rx.recv() {
                Ok(Some(hwnd)) => Some(Self {
                    hwnd,
                    thread: Some(thread),
                }),
                _ => {
                    let _ = thread.join();
                    None
                }
            }
        }
    }
    impl Tray for WindowsTray {
        fn update(&mut self, s: &TrayStatus) {
            if self.thread.is_some() {
                unsafe {
                    PostMessageW(self.hwnd as HWND, UPDATE, s.running, 0);
                }
            }
        }
        fn close(&mut self) {
            if let Some(thread) = self.thread.take() {
                unsafe {
                    PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
                }
                let _ = thread.join();
            }
        }
    }
    impl Drop for WindowsTray {
        fn drop(&mut self) {
            self.close();
        }
    }

    unsafe fn icon_data(hwnd: HWND, running: usize) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_TIP;
        for (i, c) in status_text(running).encode_utf16().take(127).enumerate() {
            data.szTip[i] = c;
        }
        data
    }
    unsafe fn run(sender: Sender<TrayAction>, ready: mpsc::SyncSender<Option<isize>>) {
        let instance = GetModuleHandleW(null());
        let registered = *REGISTERED.get_or_init(|| {
            let mut class: WNDCLASSW = zeroed();
            class.lpfnWndProc = Some(window_proc);
            class.hInstance = instance;
            class.lpszClassName = CLASS.as_ptr();
            RegisterClassW(&class) != 0
        });
        if !registered {
            let _ = ready.send(None);
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            CLASS.as_ptr(),
            CLASS.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            let _ = ready.send(None);
            return;
        }
        // The sender lives until after the window has been destroyed.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &sender as *const _ as isize);
        let mut data = icon_data(hwnd, 0);
        data.uFlags |= NIF_ICON | NIF_MESSAGE;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = LoadIconW(instance, 1usize as *const u16);
        if data.hIcon.is_null() {
            data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        }
        if data.hIcon.is_null() || Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            DestroyWindow(hwnd);
            let _ = ready.send(None);
            return;
        }
        if ready.send(Some(hwnd as isize)).is_err() {
            DestroyWindow(hwnd);
        }
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // Also clean up if the message loop ends unexpectedly.
        Shell_NotifyIconW(NIM_DELETE, &data);
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
    }
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            UPDATE => {
                let data = icon_data(hwnd, wparam);
                Shell_NotifyIconW(NIM_MODIFY, &data);
                0
            }
            CALLBACK => {
                let sender = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Sender<TrayAction>;
                if !sender.is_null() {
                    match lparam as u32 {
                        WM_LBUTTONDBLCLK => {
                            let _ = (*sender).send(TrayAction::Open);
                        }
                        WM_RBUTTONUP | WM_CONTEXTMENU => {
                            let menu = CreatePopupMenu();
                            if !menu.is_null() {
                                let open: Vec<u16> = "Open\0".encode_utf16().collect();
                                let pause: Vec<u16> = "Pause all\0".encode_utf16().collect();
                                let quit: Vec<u16> = "Quit\0".encode_utf16().collect();
                                AppendMenuW(menu, MF_STRING, 1, open.as_ptr());
                                AppendMenuW(menu, MF_STRING, 2, pause.as_ptr());
                                AppendMenuW(menu, MF_STRING, 3, quit.as_ptr());
                                let mut point: POINT = zeroed();
                                GetCursorPos(&mut point);
                                SetForegroundWindow(hwnd);
                                let selected = TrackPopupMenu(
                                    menu,
                                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                    point.x,
                                    point.y,
                                    0,
                                    hwnd,
                                    null(),
                                );
                                DestroyMenu(menu);
                                let action = match selected {
                                    1 => Some(TrayAction::Open),
                                    2 => Some(TrayAction::PauseAll),
                                    3 => Some(TrayAction::Quit),
                                    _ => None,
                                };
                                if let Some(action) = action {
                                    let _ = (*sender).send(action);
                                }
                            }
                        }
                        _ => (),
                    }
                }
                0
            }
            WM_CLOSE => {
                let data = icon_data(hwnd, 0);
                Shell_NotifyIconW(NIM_DELETE, &data);
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wording() {
        assert_eq!(status_text(0), "Chronos: idle");
        assert_eq!(status_text(1), "Chronos: 1 wish running");
        assert_eq!(status_text(2), "Chronos: 2 wishes running");
    }
}
