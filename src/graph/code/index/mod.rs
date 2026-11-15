pub mod files;
pub mod resolve;
pub mod sync;

pub use files::{classify, discover, sha256_hex, Candidate};
pub use resolve::{resolve, FileInput, RefResolution, Resolution};
pub use sync::{health, refs_at, sync, Health, RefRow, SyncReport};

pub const PARSER_VERSION: &str = "rust-syn-2.0.119+script-lex-1";
pub const RESOLVER_VERSION: &str = "resolve-1";

#[derive(Clone, Debug)]
pub struct Limits {
    pub file_bytes: u64,
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 2 * 1024 * 1024,
            files: 20_000,
            nodes: 250_000,
            edges: 1_000_000,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct SyncOptions {
    pub limits: Limits,
    pub deadline: Option<std::time::Instant>,
}
