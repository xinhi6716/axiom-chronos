use super::{Failure, Res};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub home: PathBuf,
    pub name: String,
}

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn endpoint_for(home: &Path) -> Res<Endpoint> {
    let home = home.canonicalize().map_err(error)?;
    #[cfg(windows)]
    let name = {
        let digest = Sha256::digest(home.to_string_lossy().to_lowercase().as_bytes());
        format!(r"\\.\pipe\axiom-chronos-{}", hex16(&digest))
    };
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStrExt;
        let direct = home.join("service/axiom.sock");
        let path = if direct.as_os_str().as_bytes().len() <= 100 {
            direct
        } else {
            let digest = Sha256::digest(home.as_os_str().as_bytes());
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            let uid = unsafe { libc::geteuid() };
            let candidate = runtime
                .join(format!("axiom-{uid}"))
                .join(format!("{}.sock", hex16(&digest)));
            if candidate.as_os_str().as_bytes().len() <= 100 {
                candidate
            } else {
                PathBuf::from(format!("/tmp/axiom-{uid}/{}.sock", hex16(&digest)))
            }
        };
        path.to_str()
            .ok_or_else(|| Failure::new("invalid_argument", "socket path is not UTF-8"))?
            .to_owned()
    };
    Ok(Endpoint { home, name })
}
fn hex16(bytes: &[u8]) -> String {
    bytes[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn write_json(conn: &mut Conn, v: &Value) -> Res<()> {
    serde_json::to_writer(&mut *conn, v).map_err(error)?;
    conn.write_all(b"\n").map_err(error)
}
pub fn read_json(reader: &mut BufReader<Conn>, max_bytes: usize) -> Res<Option<Value>> {
    let mut line = Vec::new();
    let mut saw_data = false;
    loop {
        let available = reader.fill_buf().map_err(error)?;
        if available.is_empty() {
            break;
        }
        saw_data = true;
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        let end = available[count - 1] == b'\n';
        let content = count - usize::from(end);
        if line.len().saturating_add(content) > max_bytes {
            return Err(Failure::new("too_large", "JSON line exceeds limit"));
        }
        line.extend_from_slice(&available[..content]);
        reader.consume(count);
        if end {
            break;
        }
    }
    if !saw_data {
        return Ok(None);
    }
    serde_json::from_slice(&line).map(Some).map_err(error)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        fs,
        os::unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        sync::Mutex,
        time::Instant,
    };

    fn private_dir(path: &Path) -> Res<()> {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder
            .mode(0o700)
            .recursive(true)
            .create(path)
            .map_err(error)?;
        let m = fs::symlink_metadata(path).map_err(error)?;
        if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } {
            return Err(Failure::new(
                "io",
                "socket directory is not owned by current user",
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(error)
    }
    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
        dev: u64,
        ino: u64,
        accept: Mutex<()>,
    }
    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> Res<Self> {
            let path = PathBuf::from(&endpoint.name);
            private_dir(
                path.parent()
                    .ok_or_else(|| Failure::new("io", "missing socket parent"))?,
            )?;
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    let metadata = fs::symlink_metadata(&path).map_err(error)?;
                    if !metadata.file_type().is_socket() {
                        return Err(Failure::new("in_use", e.to_string()));
                    }
                    match UnixStream::connect(&path) {
                        Ok(_) => return Err(Failure::new("in_use", e.to_string())),
                        Err(ref e) if e.kind() == io::ErrorKind::ConnectionRefused => (),
                        Err(e) => return Err(error(e)),
                    }
                    let current = fs::symlink_metadata(&path).map_err(error)?;
                    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
                        return Err(Failure::new("in_use", "socket changed during bind"));
                    }
                    fs::remove_file(&path).map_err(error)?;
                    UnixListener::bind(&path).map_err(|e| {
                        if e.kind() == io::ErrorKind::AddrInUse {
                            Failure::new("in_use", e.to_string())
                        } else {
                            error(e)
                        }
                    })?
                }
                Err(e) => return Err(error(e)),
            };
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(error)?;
            let m = fs::symlink_metadata(&path).map_err(error)?;
            listener.set_nonblocking(true).map_err(error)?;
            Ok(Self {
                inner: listener,
                path,
                dev: m.dev(),
                ino: m.ino(),
                accept: Mutex::new(()),
            })
        }
        pub fn accept_timeout(&self, timeout: Duration) -> Res<Option<Conn>> {
            let _guard = self.accept.lock().map_err(error)?;
            let start = Instant::now();
            loop {
                match self.inner.accept() {
                    Ok((stream, _)) => {
                        if !same_user(&stream)? {
                            continue;
                        }
                        return Ok(Some(Conn(stream)));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if start.elapsed() >= timeout {
                            return Ok(None);
                        }
                        std::thread::sleep(
                            Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
                        );
                    }
                    Err(e) => return Err(error(e)),
                }
            }
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            if let Ok(m) = fs::symlink_metadata(&self.path) {
                if m.dev() == self.dev && m.ino() == self.ino {
                    let _ = fs::remove_file(&self.path);
                }
            }
        }
    }
    fn same_user(stream: &UnixStream) -> Res<bool> {
        use std::os::fd::AsRawFd;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        unsafe {
            let mut cred: libc::ucred = std::mem::zeroed();
            let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
            if libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut _,
                &mut len,
            ) != 0
            {
                return Err(error(io::Error::last_os_error()));
            }
            return Ok(cred.uid == libc::geteuid());
        }
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        unsafe {
            let (mut uid, mut gid) = (0, 0);
            if libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) != 0 {
                return Err(error(io::Error::last_os_error()));
            }
            return Ok(uid == libc::geteuid());
        }
        #[allow(unreachable_code)]
        {
            let _ = stream;
            Ok(true)
        }
    }
    pub struct Conn(UnixStream);
    impl Conn {
        pub fn connect(endpoint: &Endpoint, _timeout: Duration) -> Res<Self> {
            let stream = UnixStream::connect(&endpoint.name).map_err(|e| {
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) {
                    Failure::new("unavailable", e.to_string())
                } else {
                    error(e)
                }
            })?;
            if !same_user(&stream)? {
                return Err(Failure::new("unavailable", "peer has different user"));
            }
            Ok(Self(stream))
        }
        pub fn try_clone(&self) -> Res<Self> {
            self.0.try_clone().map(Self).map_err(error)
        }
        pub fn shutdown(&self) {
            let _ = self.0.shutdown(std::net::Shutdown::Both);
        }
    }
    impl Read for Conn {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Conn {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
        sync::{Arc, Condvar, Mutex, Weak},
        time::Instant,
    };
    use windows_sys::Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Pipes::*, Threading::*, IO::*},
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    struct Handle(HANDLE);
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn security() -> io::Result<*mut core::ffi::c_void> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = Handle(token);
            let mut size = 0;
            GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut size);
            let mut data =
                vec![0usize; (size as usize + size_of::<usize>() - 1) / size_of::<usize>()];
            if GetTokenInformation(
                token.0,
                TokenUser,
                data.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let user = &*(data.as_ptr() as *const TOKEN_USER);
            let mut sid = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut len = 0;
            while *sid.add(len) != 0 {
                len += 1;
            }
            let sid_string = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
            LocalFree(sid.cast());
            let sddl = wide(&format!("D:P(A;;FA;;;SY)(A;;FA;;;{sid_string})"));
            let mut descriptor = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(descriptor)
        }
    }
    fn instance(name: &str, first: bool) -> io::Result<Handle> {
        unsafe {
            let descriptor = security()?;
            let attrs = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let h = CreateNamedPipeW(
                wide(name).as_ptr(),
                PIPE_ACCESS_DUPLEX
                    | FILE_FLAG_OVERLAPPED
                    | if first {
                        FILE_FLAG_FIRST_PIPE_INSTANCE
                    } else {
                        0
                    },
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                &attrs,
            );
            let result = if h == INVALID_HANDLE_VALUE {
                Err(io::Error::last_os_error())
            } else {
                Ok(Handle(h))
            };
            LocalFree(descriptor.cast());
            result
        }
    }
    struct State {
        handle: Option<Handle>,
        stopped: bool,
        active: usize,
    }
    struct Shared {
        state: Mutex<State>,
        idle: Condvar,
    }
    impl Shared {
        fn stop(&self, close: bool) {
            let mut state = self.state.lock().unwrap();
            state.stopped = true;
            if let Some(handle) = &state.handle {
                unsafe {
                    CancelIoEx(handle.0, null());
                }
            }
            if close {
                while state.active != 0 {
                    state = self.idle.wait(state).unwrap();
                }
                state.handle.take();
            }
        }
    }
    pub struct Conn(Arc<Shared>);
    impl Conn {
        fn new(handle: Handle) -> Self {
            Self(Arc::new(Shared {
                state: Mutex::new(State {
                    handle: Some(handle),
                    stopped: false,
                    active: 0,
                }),
                idle: Condvar::new(),
            }))
        }
        pub fn connect(endpoint: &Endpoint, timeout: Duration) -> Res<Self> {
            let start = Instant::now();
            loop {
                unsafe {
                    let h = CreateFileW(
                        wide(&endpoint.name).as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OVERLAPPED,
                        null_mut(),
                    );
                    if h != INVALID_HANDLE_VALUE {
                        return Ok(Self::new(Handle(h)));
                    }
                    let e = io::Error::last_os_error();
                    if !matches!(e.raw_os_error(), Some(2 | 231 | 233)) {
                        return Err(error(e));
                    }
                    if start.elapsed() >= timeout || e.raw_os_error() == Some(2) {
                        return Err(Failure::new("unavailable", e.to_string()));
                    }
                    std::thread::sleep(
                        Duration::from_millis(5).min(timeout.saturating_sub(start.elapsed())),
                    );
                }
            }
        }
        pub fn try_clone(&self) -> Res<Self> {
            Ok(Self(self.0.clone()))
        }
        pub fn shutdown(&self) {
            self.0.stop(false);
        }
        fn io(&self, buf: *mut u8, len: usize, write: bool) -> io::Result<usize> {
            unsafe {
                let event = CreateEventW(null(), 1, 0, null());
                if event.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let event = Handle(event);
                let mut ov: OVERLAPPED = zeroed();
                ov.hEvent = event.0;
                let mut count = 0;
                let mut guard = self.0.state.lock().unwrap();
                if guard.stopped {
                    return Err(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "connection shut down",
                    ));
                }
                let handle = guard.handle.as_ref().unwrap().0;
                guard.active += 1;
                let ok = if write {
                    WriteFile(
                        handle,
                        buf,
                        len.min(u32::MAX as usize) as u32,
                        &mut count,
                        &mut ov,
                    )
                } else {
                    ReadFile(
                        handle,
                        buf,
                        len.min(u32::MAX as usize) as u32,
                        &mut count,
                        &mut ov,
                    )
                };
                let err = if ok == 0 {
                    Some(io::Error::last_os_error())
                } else {
                    None
                };
                drop(guard);
                let result = if let Some(e) = err {
                    if e.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                        if !write && matches!(e.raw_os_error(), Some(109 | 232 | 233)) {
                            Ok(0)
                        } else {
                            Err(e)
                        }
                    } else if GetOverlappedResult(handle, &ov, &mut count, 1) == 0 {
                        let e = io::Error::last_os_error();
                        if !write && matches!(e.raw_os_error(), Some(109 | 232 | 233)) {
                            Ok(0)
                        } else {
                            Err(e)
                        }
                    } else {
                        Ok(count as usize)
                    }
                } else {
                    Ok(count as usize)
                };
                let mut state = self.0.state.lock().unwrap();
                state.active -= 1;
                if state.active == 0 {
                    self.0.idle.notify_all();
                }
                result
            }
        }
    }
    impl Read for Conn {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            if b.is_empty() {
                return Ok(0);
            }
            self.io(b.as_mut_ptr(), b.len(), false)
        }
    }
    impl Write for Conn {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if b.is_empty() {
                return Ok(0);
            }
            self.io(b.as_ptr() as *mut u8, b.len(), true)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    pub struct Listener {
        name: String,
        pending: Mutex<Handle>,
        connections: Mutex<Vec<Weak<Shared>>>,
    }
    impl Listener {
        pub fn bind(endpoint: &Endpoint) -> Res<Self> {
            let pending = instance(&endpoint.name, true).map_err(|e| {
                if matches!(e.raw_os_error(), Some(5 | 231)) {
                    Failure::new("in_use", e.to_string())
                } else {
                    error(e)
                }
            })?;
            Ok(Self {
                name: endpoint.name.clone(),
                pending: Mutex::new(pending),
                connections: Mutex::new(Vec::new()),
            })
        }
        pub fn accept_timeout(&self, timeout: Duration) -> Res<Option<Conn>> {
            unsafe {
                let mut pending = self.pending.lock().map_err(error)?;
                let event = CreateEventW(null(), 1, 0, null());
                if event.is_null() {
                    return Err(error(io::Error::last_os_error()));
                }
                let event = Handle(event);
                let mut ov: OVERLAPPED = zeroed();
                ov.hEvent = event.0;
                let ok = ConnectNamedPipe(pending.0, &mut ov);
                if ok == 0 {
                    let e = io::Error::last_os_error();
                    match e.raw_os_error() {
                        Some(535) => (),
                        Some(997) => {
                            let wait = WaitForSingleObject(
                                event.0,
                                timeout.as_millis().min((u32::MAX - 1) as u128) as u32,
                            );
                            if wait != WAIT_OBJECT_0 {
                                CancelIoEx(pending.0, &ov);
                                let mut count = 0;
                                GetOverlappedResult(pending.0, &ov, &mut count, 1);
                                if wait == WAIT_TIMEOUT {
                                    return Ok(None);
                                }
                                return Err(error(io::Error::last_os_error()));
                            }
                            let mut count = 0;
                            if GetOverlappedResult(pending.0, &ov, &mut count, 0) == 0 {
                                return Err(error(io::Error::last_os_error()));
                            }
                        }
                        _ => return Err(error(e)),
                    }
                }
                let next = instance(&self.name, false).map_err(error)?;
                let connected = std::mem::replace(&mut *pending, next);
                let conn = Conn::new(connected);
                let mut connections = self.connections.lock().map_err(error)?;
                connections.retain(|c| c.strong_count() != 0);
                connections.push(Arc::downgrade(&conn.0));
                Ok(Some(conn))
            }
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            for connection in self
                .connections
                .get_mut()
                .unwrap()
                .iter()
                .filter_map(Weak::upgrade)
            {
                connection.stop(true);
            }
        }
    }
}

pub use platform::{Conn, Listener};
