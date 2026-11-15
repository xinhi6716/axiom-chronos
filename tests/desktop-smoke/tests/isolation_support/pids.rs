use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::fs::OpenOptionsExt,
    path::Path,
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE},
    System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
};

// An exclusive open gives a cross-process lock that is released even on hard kill.
fn locked<T>(file: &Path, f: impl FnOnce() -> T) -> T {
    let parent = file.parent().expect("pid registry parent");
    fs::create_dir_all(parent).expect("create pid registry directory");
    let lock = file.with_extension("lock");
    let _handle = loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .share_mode(0)
            .open(&lock)
        {
            Ok(handle) => break handle,
            Err(e) if e.raw_os_error() == Some(32) => thread::sleep(Duration::from_millis(10)),
            Err(e) => panic!("open pid registry lock: {e}"),
        }
    };
    f()
}

pub(super) unsafe fn handle_creation_time(h: HANDLE) -> Option<u64> {
    let (mut start, mut exit, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) =
        std::mem::zeroed();
    let ok = GetProcessTimes(h, &mut start, &mut exit, &mut kernel, &mut user) != 0;
    ok.then_some(((start.dwHighDateTime as u64) << 32) | start.dwLowDateTime as u64)
}
pub fn creation_time(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let start = handle_creation_time(h);
        CloseHandle(h);
        start
    }
}
pub fn record(file: &Path, pid: u32, start: u64) {
    locked(file, || {
        let mut out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .expect("open pid registry");
        writeln!(out, "{}", serde_json::json!({"pid":pid,"start":start}))
            .expect("write pid registry");
        out.flush().expect("flush pid registry");
    });
}
pub fn sweep(file: &Path, creation: &dyn Fn(u32) -> Option<u64>, stop: &dyn Fn(u32)) -> Vec<u32> {
    locked(file, || {
        let content = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => panic!("read pid registry: {e}"),
        };
        let mut stopped = Vec::new();
        let mut invalid = Vec::new();
        for line in content.lines() {
            let entry = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| {
                    Some((
                        u32::try_from(v["pid"].as_u64()?).ok()?,
                        v["start"].as_u64()?,
                    ))
                });
            if let Some((pid, start)) = entry {
                if creation(pid) == Some(start) {
                    stop(pid);
                    stopped.push(pid);
                }
            } else {
                invalid.push(line);
            }
        }
        let mut rest = invalid.join("\n");
        if !rest.is_empty() {
            rest.push('\n');
        }
        fs::write(file, rest).expect("rewrite pid registry");
        stopped
    })
}
