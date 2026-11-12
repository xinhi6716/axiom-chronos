use crate::{
    base::{Failure, Res},
    seams::FakeClock,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout: Duration,
}
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
pub trait Transport: Send + Sync {
    fn get(&self, req: &HttpRequest) -> Res<HttpResponse>;
}
pub struct CurlTransport {
    command: Vec<String>,
}
impl CurlTransport {
    pub fn from_env() -> Res<Self> {
        Ok(Self {
            command: crate::base::resolve_command("AXIOM_CURL_CMD", "curl")?,
        })
    }
}
impl Transport for CurlTransport {
    fn get(&self, req: &HttpRequest) -> Res<HttpResponse> {
        let headers_file = HeadersFile::new()?;
        let mut cmd = crate::base::proc::command(&self.command);
        cmd.args([
            "-sS",
            "-L",
            "--max-time",
            &req.timeout.as_secs_f64().to_string(),
        ])
        .arg("-D")
        .arg(&headers_file.0)
        .args(["-w", "\nAXIOM_CURL_STATUS:%{http_code}"]);
        for (name, value) in &req.headers {
            cmd.arg("-H").arg(format!("{name}: {value}"));
        }
        let out = cmd
            .arg("--")
            .arg(&req.url)
            .output()
            .map_err(|e| Failure::new("http", e.to_string()))?;
        if !out.status.success() {
            return Err(Failure::new(
                "http",
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }
        const MARKER: &[u8] = b"\nAXIOM_CURL_STATUS:";
        let split = out
            .stdout
            .len()
            .checked_sub(MARKER.len() + 3)
            .ok_or_else(|| Failure::new("http", "invalid HTTP status"))?;
        let suffix = &out.stdout[split..];
        let status = suffix
            .strip_prefix(MARKER)
            .filter(|code| code.iter().all(u8::is_ascii_digit))
            .and_then(|code| std::str::from_utf8(code).ok())
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| Failure::new("http", "invalid HTTP status"))?;
        let header_bytes =
            std::fs::read(&headers_file.0).map_err(|e| Failure::new("http", e.to_string()))?;
        let head = String::from_utf8_lossy(&header_bytes);
        let mut headers = Vec::new();
        for line in head.lines() {
            if line.starts_with("HTTP/") {
                headers.clear();
            } else if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }
        Ok(HttpResponse {
            status,
            headers,
            body: out.stdout[..split].to_vec(),
        })
    }
}

struct HeadersFile(PathBuf);
impl HeadersFile {
    fn new() -> Res<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "axiom-curl-{}-{}.headers",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Failure::new("http", e.to_string())),
            }
        }
    }
}
impl Drop for HeadersFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub struct Offline;
impl Transport for Offline {
    fn get(&self, _: &HttpRequest) -> Res<HttpResponse> {
        Err(Failure::new("offline", "network disabled"))
    }
}
#[derive(Deserialize)]
struct Canned {
    url: String,
    status: u16,
    file: String,
    #[serde(default)]
    headers: HashMap<String, String>,
}
pub struct CannedTransport {
    dir: PathBuf,
    entries: Vec<Canned>,
    state: Mutex<(Vec<String>, HashMap<String, usize>)>,
}
impl CannedTransport {
    pub fn from_dir(dir: &Path) -> Res<Self> {
        let data =
            std::fs::read(dir.join("index.json")).map_err(|e| Failure::new("io", e.to_string()))?;
        let entries =
            serde_json::from_slice(&data).map_err(|e| Failure::new("json", e.to_string()))?;
        Ok(Self {
            dir: dir.to_owned(),
            entries,
            state: Mutex::new((Vec::new(), HashMap::new())),
        })
    }
    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().0.clone()
    }
}
impl Transport for CannedTransport {
    fn get(&self, req: &HttpRequest) -> Res<HttpResponse> {
        let mut state = self.state.lock().unwrap();
        state.0.push(req.url.clone());
        let idx = state.1.entry(req.url.clone()).or_default();
        let entry = self
            .entries
            .iter()
            .filter(|e| e.url == req.url)
            .nth(*idx)
            .ok_or_else(|| Failure::new("http", format!("no canned response for {}", req.url)))?;
        *idx += 1;
        let body = std::fs::read(self.dir.join(&entry.file))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(HttpResponse {
            status: entry.status,
            headers: entry
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            body,
        })
    }
}
pub trait Pause: Send + Sync {
    fn pause(&self, d: Duration);
}
pub struct RealPause;
impl Pause for RealPause {
    fn pause(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
pub struct ClockPause(pub Arc<FakeClock>);
impl Pause for ClockPause {
    fn pause(&self, d: Duration) {
        self.0
            .advance(i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::{CurlTransport, HttpRequest, Transport};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    #[test]
    fn curl_follows_redirect_and_preserves_body_and_final_headers() {
        let Some(curl) = crate::base::on_path("curl") else {
            return;
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for step in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&buf[..n]);
                }
                assert!(String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("x-test: value"));
                if step == 0 {
                    write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://{address}/final\r\nRetry-After: 99\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    let body = b"\0body\r\n\r\n\nAXIOM_CURL_STATUS:123\xff";
                    write!(stream, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                    stream.write_all(body).unwrap();
                }
            }
        });
        let response = CurlTransport {
            command: vec![curl],
        }
        .get(&HttpRequest {
            url: format!("http://{address}/start"),
            headers: vec![("X-Test".into(), "value".into())],
            timeout: Duration::from_secs(5),
        })
        .unwrap();
        server.join().unwrap();
        assert_eq!(response.status, 429);
        assert_eq!(response.body, b"\0body\r\n\r\n\nAXIOM_CURL_STATUS:123\xff");
        assert!(response
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("retry-after") && v == "2"));
        assert!(!response.headers.iter().any(|(_, v)| v == "99"));
    }
}
