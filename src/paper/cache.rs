use crate::{
    base::{Failure, Res},
    paper::{
        http::{HttpRequest, Pause, Transport},
        model::SourceKind,
    },
    seams::{sha256_hex, Clock},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchMode {
    Online,
    OfflineOnly,
}
#[derive(Debug)]
pub struct Fetched {
    pub body: Vec<u8>,
    pub locator: String,
    pub fetched_at: String,
    pub from_cache: bool,
    pub stale: bool,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    url: String,
    fetched_ms: i64,
    fetched_at: String,
    body: String,
    encoding: String,
}
pub struct Cache {
    root: PathBuf,
    clock: Arc<dyn Clock>,
    pause: Arc<dyn Pause>,
    hosts: Mutex<HashMap<String, i64>>,
}
fn kind_info(kind: &SourceKind) -> (&'static str, i64, i64) {
    match kind {
        SourceKind::Openalex => ("openalex", 7, 100),
        SourceKind::Doaj => ("doaj", 7, 500),
        SourceKind::Crossref => ("crossref", 30, 50),
        SourceKind::Sjr => ("sjr", 180, 1000),
        SourceKind::JcrImport => ("jcr_import", 30, 100),
        SourceKind::OfficialPage => ("official_page", 7, 100),
        SourceKind::Program => ("program", 7, 100),
        SourceKind::Dataset => ("dataset", 7, 100),
        SourceKind::Person => ("person", 7, 100),
    }
}
// Civil date conversion from Unix days; supports dates before the epoch too.
fn timestamp(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86400);
    let sec = seconds.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sec / 3600,
        (sec % 3600) / 60,
        sec % 60
    )
}
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn encode(bytes: &[u8]) -> String {
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | (c.get(2).copied().unwrap_or(0) as u32);
        s.push(B64[((n >> 18) & 63) as usize] as char);
        s.push(B64[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}
fn decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::new();
    for (i, c) in b.chunks(4).enumerate() {
        let val = |x| B64.iter().position(|v| *v == x).map(|v| v as u32);
        let a = val(c[0])?;
        let d = val(c[1])?;
        let e = if c[2] == b'=' { 0 } else { val(c[2])? };
        let f = if c[3] == b'=' { 0 } else { val(c[3])? };
        if (c[2] == b'=' && c[3] != b'=')
            || ((c[2] == b'=' || c[3] == b'=') && i + 1 != b.len() / 4)
        {
            return None;
        }
        let n = (a << 18) | (d << 12) | (e << 6) | f;
        out.push((n >> 16) as u8);
        if c[2] != b'=' {
            out.push((n >> 8) as u8)
        }
        if c[3] != b'=' {
            out.push(n as u8)
        }
    }
    Some(out)
}
impl Entry {
    fn body(&self) -> Option<Vec<u8>> {
        match self.encoding.as_str() {
            "utf8" => Some(self.body.as_bytes().to_vec()),
            "base64" => decode(&self.body),
            _ => None,
        }
    }
}
pub(crate) fn read_body(path: &Path) -> Res<Vec<u8>> {
    let bytes = std::fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
    let entry: Entry =
        serde_json::from_slice(&bytes).map_err(|e| Failure::new("cache", e.to_string()))?;
    entry
        .body()
        .ok_or_else(|| Failure::new("cache", "invalid body encoding"))
}
impl Cache {
    pub fn open(root: &Path, clock: Arc<dyn Clock>, pause: Arc<dyn Pause>) -> Res<Self> {
        std::fs::create_dir_all(root).map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(Self {
            root: root.to_owned(),
            clock,
            pause,
            hosts: Mutex::new(HashMap::new()),
        })
    }
    pub fn get(
        &self,
        kind: SourceKind,
        url: &str,
        mode: FetchMode,
        t: &dyn Transport,
    ) -> Res<Fetched> {
        let (name, days, interval) = kind_info(&kind);
        let locator = format!("{name}/{}.json", sha256_hex(url.as_bytes()));
        let path = self.root.join(&locator);
        if path.exists() {
            let entry = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Entry>(&b).ok())
                .filter(|e| e.url == url)
                .and_then(|e| e.body().map(|b| (e, b)));
            if let Some((e, body)) = entry {
                let stale = self.clock.now_ms().saturating_sub(e.fetched_ms) >= days * 86_400_000;
                if mode == FetchMode::OfflineOnly || !stale {
                    return Ok(Fetched {
                        body,
                        locator,
                        fetched_at: e.fetched_at,
                        from_cache: true,
                        stale,
                    });
                }
            } else if mode == FetchMode::Online {
                let mut n = 0;
                loop {
                    let target = path.with_extension(format!("corrupt-{n}"));
                    if !target.exists() {
                        std::fs::rename(&path, target)
                            .map_err(|e| Failure::new("io", e.to_string()))?;
                        break;
                    }
                    n += 1;
                }
            }
        }
        if mode == FetchMode::OfflineOnly {
            return Err(Failure::new("offline_miss", url));
        }
        let host = url
            .split_once("://")
            .map(|(_, s)| s.split('/').next().unwrap_or(s))
            .unwrap_or(url)
            .to_ascii_lowercase();
        let req = HttpRequest {
            url: url.into(),
            headers: vec![],
            timeout: Duration::from_secs(30),
        };
        for attempt in 0..4 {
            {
                let mut hosts = self.hosts.lock().unwrap();
                let now = self.clock.now_ms();
                if let Some(last) = hosts.get(&host) {
                    let wait = interval - (now - *last);
                    if wait > 0 {
                        self.pause.pause(Duration::from_millis(wait as u64));
                    }
                }
                hosts.insert(host.clone(), self.clock.now_ms());
            }
            let response = t.get(&req)?;
            if response.status == 429 || response.status == 503 {
                if attempt == 3 {
                    return Err(Failure::new(
                        if response.status == 429 {
                            "rate_limited"
                        } else {
                            "http"
                        },
                        format!("HTTP {}", response.status),
                    ));
                }
                let delay = response
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                    .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                    .unwrap_or(2);
                self.pause.pause(Duration::from_secs(delay));
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(Failure::new(
                    if response.status == 404 {
                        "not_found"
                    } else {
                        "http"
                    },
                    format!("HTTP {}", response.status),
                ));
            }
            let now = self.clock.now_ms();
            let at = timestamp(now);
            let (body, encoding) = match String::from_utf8(response.body.clone()) {
                Ok(s) => (s, "utf8"),
                Err(_) => (encode(&response.body), "base64"),
            };
            let entry = Entry {
                url: url.into(),
                fetched_ms: now,
                fetched_at: at.clone(),
                body,
                encoding: encoding.into(),
            };
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| Failure::new("io", e.to_string()))?;
            let bytes =
                serde_json::to_vec(&entry).map_err(|e| Failure::new("json", e.to_string()))?;
            crate::base::atomic_write(&path, &bytes)
                .map_err(|e| Failure::new("io", e.to_string()))?;
            return Ok(Fetched {
                body: response.body,
                locator,
                fetched_at: at,
                from_cache: false,
                stale: false,
            });
        }
        unreachable!()
    }
}
