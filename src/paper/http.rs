use crate::{base::Res, seams::FakeClock};
use std::{path::Path, sync::Arc, time::Duration};

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
pub struct CurlTransport;
impl CurlTransport {
    pub fn from_env() -> Res<Self> {
        unimplemented!()
    }
}
impl Transport for CurlTransport {
    fn get(&self, _req: &HttpRequest) -> Res<HttpResponse> {
        unimplemented!()
    }
}
pub struct Offline;
impl Transport for Offline {
    fn get(&self, _req: &HttpRequest) -> Res<HttpResponse> {
        unimplemented!()
    }
}
pub struct CannedTransport;
impl CannedTransport {
    pub fn from_dir(_dir: &Path) -> Res<Self> {
        unimplemented!()
    }
    pub fn requests(&self) -> Vec<String> {
        unimplemented!()
    }
}
impl Transport for CannedTransport {
    fn get(&self, _req: &HttpRequest) -> Res<HttpResponse> {
        unimplemented!()
    }
}
pub trait Pause: Send + Sync {
    fn pause(&self, d: Duration);
}
pub struct RealPause;
impl Pause for RealPause {
    fn pause(&self, _d: Duration) {
        unimplemented!()
    }
}
pub struct ClockPause(pub Arc<FakeClock>);
impl Pause for ClockPause {
    fn pause(&self, _d: Duration) {
        unimplemented!()
    }
}
