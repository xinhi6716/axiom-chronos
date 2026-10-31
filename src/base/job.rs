//! Keep shell descendants cancellable even after intermediate parents exit.
use super::super::{Failure, Res};
use std::{ffi::c_void, os::windows::io::AsRawHandle, process::Child};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
        Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> HANDLE;
    fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
    fn TerminateJobObject(job: HANDLE, exit_code: u32) -> i32;
    fn QueryInformationJobObject(job: HANDLE, class: i32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct Accounting {
    total_user: i64, total_kernel: i64, period_user: i64, period_kernel: i64,
    page_faults: u32, total_processes: u32, active_processes: u32, terminated_processes: u32,
}

pub(super) struct Job(HANDLE);
// A job handle can be used across threads; access to its Child is mutex-protected.
unsafe impl Send for Job {}
impl Job {
    pub fn attach(child: &Child) -> Res<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(error());
            }
            let job = Self(handle);
            if AssignProcessToJobObject(job.0, child.as_raw_handle()) == 0 {
                return Err(error());
            }
            Ok(job)
        }
    }
    pub fn terminate(&self) -> bool {
        unsafe { TerminateJobObject(self.0, 1) != 0 }
    }
    /// A successful termination request alone does not prove the job is empty.
    pub fn empty(&self) -> bool {
        let mut info = Accounting::default();
        unsafe {
            QueryInformationJobObject(self.0, 1, &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<Accounting>() as u32, std::ptr::null_mut()) != 0
                && info.active_processes == 0
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.terminate();
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn error() -> Failure {
    Failure::new("process", std::io::Error::last_os_error().to_string())
}

/// The shell is created suspended, so no descendant can escape before job assignment.
pub(super) fn resume(child: &Child) -> Res<()> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut more = Thread32First(snapshot, &mut entry);
        let mut result = Err(Failure::new("process", "suspended shell thread not found"));
        while more != 0 {
            if entry.th32OwnerProcessID == child.id() {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    result = if ResumeThread(thread) == u32::MAX {
                        Err(error())
                    } else {
                        Ok(())
                    };
                    CloseHandle(thread);
                } else {
                    result = Err(error());
                }
                break;
            }
            more = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        result
    }
}
