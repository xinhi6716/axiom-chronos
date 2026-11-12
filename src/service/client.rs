use super::protocol::{Snapshot, StartRequest, WireEvent};
use crate::base::{
    ipc::{self, Conn, Endpoint},
    Failure, Res,
};
use serde_json::{json, Value};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Client {
    home: PathBuf,
    endpoint: Endpoint,
}
impl Client {
    pub fn connect(home: &Path) -> Res<Self> {
        let endpoint =
            ipc::endpoint_for(home).map_err(|e| Failure::new("service_unavailable", e.message))?;
        let client = Self {
            home: home.to_path_buf(),
            endpoint,
        };
        let response = client
            .request_with_timeout(
                json!({"op":"hello","version":1}),
                Duration::from_millis(300),
            )
            .map_err(|e| Failure::new("service_unavailable", e.message))?;
        if response["version"] != 1 {
            return Err(Failure::new(
                "service_unavailable",
                "invalid hello response",
            ));
        }
        Ok(client)
    }
    pub fn connect_or_spawn(home: &Path, exe: &Path, timeout: Duration) -> Res<Self> {
        if let Ok(c) = Self::connect(home) {
            return Ok(c);
        }
        std::fs::create_dir_all(home).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut cmd = crate::base::proc::cmd(exe);
        cmd.arg("serve")
            .env("AXIOM_HOME", home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x00000008 | 0x00000200);
        }
        cmd.spawn()
            .map_err(|e| Failure::new("service_unavailable", e.to_string()))?;
        let until = Instant::now() + timeout;
        loop {
            if let Ok(c) = Self::connect(home) {
                return Ok(c);
            }
            if Instant::now() >= until {
                return Err(Failure::new("service_unavailable", "service did not start"));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn request(&self, v: Value) -> Res<Value> {
        self.request_with_timeout(v, Duration::from_secs(2))
    }
    fn request_with_timeout(&self, v: Value, timeout: Duration) -> Res<Value> {
        let mut c = Conn::connect(&self.endpoint, timeout)
            .map_err(|e| Failure::new("service_unavailable", e.message))?;
        ipc::write_json(&mut c, &v)?;
        let response = ipc::read_json(&mut BufReader::new(c), 4 * 1024 * 1024)?
            .ok_or_else(|| Failure::new("service_unavailable", "service disconnected"))?;
        if response["ok"] == true {
            Ok(response)
        } else {
            Err(Failure::new(
                response["error"]["kind"].as_str().unwrap_or("protocol"),
                response["error"]["message"]
                    .as_str()
                    .unwrap_or("invalid response"),
            ))
        }
    }
    pub fn status(&self) -> Res<Snapshot> {
        self.snapshot_op("status")
    }
    pub fn snapshot(&self) -> Res<Snapshot> {
        self.snapshot_op("snapshot")
    }
    fn snapshot_op(&self, op: &str) -> Res<Snapshot> {
        let v = self.request(json!({"op":op}))?;
        serde_json::from_value(v.get("snapshot").cloned().unwrap_or(v))
            .map_err(|e| Failure::new("protocol", e.to_string()))
    }
    pub fn start(&self, req: StartRequest) -> Res<String> {
        let mut v =
            serde_json::to_value(req).map_err(|e| Failure::new("protocol", e.to_string()))?;
        v["op"] = json!("start");
        let r = self.request(v)?;
        Ok(r["handle"].as_str().unwrap_or_default().into())
    }
    pub fn resume(&self, key: &str, note: Option<&str>) -> Res<String> {
        let r = self.request(json!({"op":"resume","key":key,"note":note}))?;
        Ok(r["handle"].as_str().unwrap_or_default().into())
    }
    pub fn interject(&self, key: &str, text: &str) -> Res<()> {
        self.request(json!({"op":"interject","key":key,"text":text}))
            .map(|_| ())
    }
    pub fn stop(&self, key: &str) -> Res<()> {
        self.request(json!({"op":"stop","key":key})).map(|_| ())
    }
    pub fn pause_all(&self) -> Res<usize> {
        Ok(self.request(json!({"op":"pause_all"}))?["stopped"]
            .as_u64()
            .unwrap_or(0) as usize)
    }
    pub fn quit(&self, stop_running: bool) -> Res<()> {
        self.request(json!({"op":"quit","stop_running":stop_running}))
            .map(|_| ())
    }
    pub fn set_login_start(&self, enabled: bool) -> Res<()> {
        self.request(json!({"op":"set_login_start","enabled":enabled}))
            .map(|_| ())
    }
    pub fn subscribe(&self, after: u64) -> Res<Subscription> {
        let mut conn = Conn::connect(&self.endpoint, Duration::from_secs(2))?;
        ipc::write_json(&mut conn, &json!({"op":"subscribe","after":after}))?;
        let shutdown = conn.try_clone()?;
        let mut reader = BufReader::new(conn);
        let first = ipc::read_json(&mut reader, 4 * 1024 * 1024)?
            .ok_or_else(|| Failure::new("service_unavailable", "service disconnected"))?;
        if first["gap"] == true {
            return Err(Failure::new(
                "gap",
                "event buffer no longer contains requested cursor",
            ));
        }
        if first["ok"] != true {
            return Err(Failure::new("protocol", "subscription rejected"));
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || loop {
            match ipc::read_json(&mut reader, 4 * 1024 * 1024) {
                Ok(Some(v)) => {
                    let item = serde_json::from_value(v)
                        .map_err(|e| Failure::new("protocol", e.to_string()));
                    if tx.send(item).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        });
        Ok(Subscription { rx, shutdown })
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
}
pub struct Subscription {
    rx: mpsc::Receiver<Res<WireEvent>>,
    shutdown: Conn,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.shutdown.shutdown();
    }
}
impl Subscription {
    pub fn recv_timeout(&self, timeout: Duration) -> Res<Option<WireEvent>> {
        match self.rx.recv_timeout(timeout) {
            Ok(v) => v.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(Failure::new("service_unavailable", "subscription closed"))
            }
        }
    }
}
