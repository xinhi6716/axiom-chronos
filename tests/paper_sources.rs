use axiom::{
    paper::{
        cache::{Cache, FetchMode},
        http::{self, CannedTransport, ClockPause, Transport},
        journals::{doaj, openalex::OpenAlex, sjr},
        model::{Access, Apc, Known, Quartile, QuartileScheme, SourceKind, SourceTag, Sourced},
        provenance,
    },
    seams::{Clock, FakeClock},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;

const SOURCE: &str = "https://api.openalex.org/sources/S1";
const WORK: &str = "https://api.openalex.org/works/W1";
const PAGE1: &str = "https://api.openalex.org/works?search=cancer&filter=from_publication_date:2020-01-01&per-page=200&cursor=*";
const PAGE2: &str = "https://api.openalex.org/works?search=cancer&filter=from_publication_date:2020-01-01&per-page=200&cursor=page2";
struct Rig {
    _temp: TempDir,
    root: PathBuf,
    home: PathBuf,
    clock: Arc<FakeClock>,
    cache: Cache,
    transport: CannedTransport,
}
impl Rig {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let home = temp.path().join("home");
        let clock = Arc::new(FakeClock::new(1_700_000_000_000));
        let cache = Cache::open(&root, clock.clone(), Arc::new(ClockPause(clock.clone()))).unwrap();
        let transport = CannedTransport::from_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paper/sources"),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            home,
            clock,
            cache,
            transport,
        }
    }
    fn api(&self) -> OpenAlex<'_> {
        OpenAlex::new(&self.cache, &self.transport, FetchMode::Online, &self.home).unwrap()
    }
    fn get(&self, url: &str, mode: FetchMode) -> axiom::base::Res<axiom::paper::cache::Fetched> {
        self.cache
            .get(SourceKind::Openalex, url, mode, &self.transport)
    }
}
fn assert_unknown<T>(v: &Known<T>) {
    match v {
        Known::Unknown { reason } => assert!(!reason.trim().is_empty()),
        Known::Known(_) => panic!("expected unknown"),
    }
}
fn assert_raw(root: &Path, tag: &SourceTag, expected: &Value) {
    assert!(tag.url.is_some());
    assert!(tag.raw.contains('#'));
    assert_eq!(
        &provenance::resolve(root, tag).unwrap(),
        expected,
        "{}",
        tag.raw
    );
}
fn assert_raw_one_of(root: &Path, tag: &SourceTag, expected: &[Value]) {
    assert!(tag.url.is_some() && tag.raw.contains('#'));
    let raw = provenance::resolve(root, tag).unwrap();
    assert!(
        expected.contains(&raw),
        "unexpected raw value {raw} at {}",
        tag.raw
    );
}
fn check_known<T>(root: &Path, known: &Known<T>, expected: &Value) {
    match known {
        Known::Known(v) => assert_raw(root, &v.source, expected),
        Known::Unknown { reason } => assert!(!reason.trim().is_empty()),
    }
}
fn check_sourced<T>(root: &Path, sourced: &Sourced<T>, expected: &Value) {
    assert_raw(root, &sourced.source, expected);
}

#[test]
fn openalex_paging_follows_cursor() {
    let r = Rig::new();
    let works = r.api().search_works("cancer", 2020, 10).unwrap();
    assert_eq!(
        works.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        vec!["https://openalex.org/W1", "https://openalex.org/W2"]
    );
    assert_eq!(works[0].abstract_words, ["cancer", "study"]);
    assert_eq!(r.transport.requests(), vec![PAGE1, PAGE2]);
    let r = Rig::new();
    assert_eq!(r.api().search_works("cancer", 2020, 1).unwrap().len(), 1);
    assert_eq!(r.transport.requests(), vec![PAGE1]);
    let work = r.api().work("W1").unwrap().unwrap();
    assert_eq!(work.title.as_deref(), Some("Work 1"));
    assert_eq!(work.authors, ["A Author"]);
    assert_eq!(work.volume.as_deref(), Some("1"));
}
#[test]
fn rate_limit_429_retries_with_fake_clock() {
    let r = Rig::new();
    let start = r.clock.now_ms();
    let fetched = r
        .get("https://api.openalex.org/retry", FetchMode::Online)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&fetched.body).unwrap(),
        json!({"ok":true})
    );
    assert!(r.clock.now_ms() - start >= 3000);
    assert_eq!(r.transport.requests().len(), 2);
    let r = Rig::new();
    assert_eq!(
        r.get("https://api.openalex.org/limited", FetchMode::Online)
            .unwrap_err()
            .kind,
        "rate_limited"
    );
    assert_eq!(r.transport.requests().len(), 4);
    assert!(r.clock.now_ms() - 1_700_000_000_000 >= 9000);
}
#[test]
fn corrupt_cache_refetched_online_and_miss_offline() {
    let r = Rig::new();
    let fetched = r.get(SOURCE, FetchMode::Online).unwrap();
    let path = r.root.join(&fetched.locator);
    fs::write(&path, b"not json").unwrap();
    let before = r.transport.requests();
    assert_eq!(
        r.get(SOURCE, FetchMode::OfflineOnly).unwrap_err().kind,
        "offline_miss"
    );
    assert_eq!(r.transport.requests(), before);
    let fresh = r.get(SOURCE, FetchMode::Online).unwrap();
    assert!(!fresh.from_cache);
    assert_eq!(fresh.body, fetched.body);
    assert_eq!(r.transport.requests().len(), before.len() + 1);
    assert!(
        fs::read_dir(path.parent().unwrap()).unwrap().count() >= 2,
        "corrupt file must be moved aside"
    );
}
#[test]
fn offline_hit_and_miss_make_no_request() {
    let r = Rig::new();
    let initial = r.get(SOURCE, FetchMode::Online).unwrap();
    let before = r.transport.requests();
    let hit = r.get(SOURCE, FetchMode::OfflineOnly).unwrap();
    assert!(hit.from_cache);
    assert!(!hit.stale);
    assert_eq!(hit.body, initial.body);
    assert_eq!(
        r.get(WORK, FetchMode::OfflineOnly).unwrap_err().kind,
        "offline_miss"
    );
    assert_eq!(r.transport.requests(), before);
}
#[test]
fn expired_entry_refetched() {
    let r = Rig::new();
    r.get(SOURCE, FetchMode::Online).unwrap();
    r.clock.advance(8 * 24 * 60 * 60 * 1000);
    let before = r.transport.requests();
    let stale = r.get(SOURCE, FetchMode::OfflineOnly).unwrap();
    assert!(stale.stale && stale.from_cache);
    assert_eq!(r.transport.requests(), before);
    let fresh = r.get(SOURCE, FetchMode::Online).unwrap();
    assert!(!fresh.stale && !fresh.from_cache);
    assert_eq!(r.transport.requests().len(), before.len() + 1);
    assert_ne!(fresh.fetched_at, stale.fetched_at);
}
#[test]
fn every_known_field_resolves_to_raw() {
    let r = Rig::new();
    let source = r.api().source("S1").unwrap();
    assert_eq!(
        source.publisher.value().map(String::as_str),
        Some("Example Publisher")
    );
    assert_eq!(
        source.homepage.value().map(String::as_str),
        Some("https://example.org")
    );
    assert_eq!(source.works_count.value(), Some(&123));
    check_known(&r.root, &source.publisher, &json!("Example Publisher"));
    check_known(&r.root, &source.homepage, &json!("https://example.org"));
    check_known(&r.root, &source.works_count, &json!(123));
    assert_unknown(&source.sjr);
    assert_unknown(&source.apc);
    assert_unknown(&source.review_weeks);
    // OpenAlex is_oa is a raw boolean even though the model stores an Access enum.
    check_known(&r.root, &source.access, &json!(true));
    assert!(source.quartiles.is_empty());
    let row = sjr::load_year(&r.cache, &r.transport, FetchMode::Online, 2023)
        .unwrap()
        .lookup(&["1234-5678".into()])
        .unwrap();
    check_sourced(&r.root, &row.sjr, &json!("1,234"));
    for q in &row.quartiles {
        check_sourced(&r.root, q, &json!("Oncology (Q1); Hematology (Q2)"));
    }
    check_sourced(
        &r.root,
        row.publisher.as_ref().unwrap(),
        &json!("Example Publisher"),
    );
    check_sourced(&r.root, row.open_access.as_ref().unwrap(), &json!("Yes"));
    let d = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "1234-5678")
        .unwrap()
        .unwrap();
    assert_eq!(d.access.value, Access::OpenAccess);
    let apc = match &d.apc {
        Known::Known(v) => v,
        _ => panic!("APC missing"),
    };
    assert_raw_one_of(
        &r.root,
        &apc.source,
        &[
            json!(1200),
            json!({"price":1200,"currency":"USD"}),
            json!({"has_apc":true,"max":[{"price":1200,"currency":"USD"}]}),
        ],
    );
    check_known(&r.root, &d.review_weeks, &json!(12));
    check_known(&r.root, &d.homepage, &json!("https://example.org"));
    check_known(&r.root, &d.ai_policy, &json!("https://example.org/ai"));
    let bibjson = json!({"title":"Example Journal","apc":{"has_apc":true,"max":[{"price":1200,"currency":"USD"}]},"publication_time_weeks":12,"ref":{"journal":"https://example.org"},"ai_policy":{"url":"https://example.org/ai"}});
    assert_raw_one_of(
        &r.root,
        &d.access.source,
        &[
            json!("Example Journal"),
            bibjson.clone(),
            json!({"bibjson":bibjson}),
        ],
    );
}
#[test]
fn unknown_apc_is_not_zero() {
    let r = Rig::new();
    let missing = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "1111-1111")
        .unwrap()
        .unwrap();
    assert_unknown(&missing.apc);
    let fee = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "1234-5678")
        .unwrap()
        .unwrap();
    assert!(
        matches!(fee.apc.value(), Some(Apc::Fee(m)) if m.amount_minor == 120000 && m.currency == "USD")
    );
    let no_fee = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "3333-3333")
        .unwrap()
        .unwrap();
    assert_eq!(no_fee.apc.value(), Some(&Apc::NoFee));
}
#[test]
fn unknown_speed_has_no_estimate() {
    let r = Rig::new();
    let missing = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "1111-1111")
        .unwrap()
        .unwrap();
    assert_unknown(&missing.review_weeks);
    let present = doaj::journal(&r.cache, &r.transport, FetchMode::Online, "2222-2222")
        .unwrap()
        .unwrap();
    assert_eq!(present.review_weeks.value(), Some(&8));
}
#[test]
fn sjr_csv_semicolon_quartiles_labelled_sjr() {
    let r = Rig::new();
    for (year, score) in [(2023, 1.234), (2024, 2.345)] {
        let table = sjr::load_year(&r.cache, &r.transport, FetchMode::Online, year).unwrap();
        let row = table.lookup(&["8765-4321".into()]).unwrap();
        assert!((row.sjr.value - score).abs() < 1e-10);
        assert_eq!(row.quartiles.len(), 2);
        assert_eq!(row.quartiles[0].source.data_year, Some(year));
        assert_eq!(row.quartiles[0].value.scheme, QuartileScheme::Sjr);
        assert_eq!(row.quartiles[0].value.quartile, Quartile::Q1);
        assert_eq!(
            row.quartiles[0].value.label(),
            format!("SJR Q1 (Oncology, {year})")
        );
        assert_eq!(
            row.quartiles[1].value.label(),
            format!("SJR Q2 (Hematology, {year})")
        );
        assert!(table.lookup(&["0000-0000".into()]).is_none());
    }
}
#[test]
fn per_host_interval_advances_fake_clock() {
    let r = Rig::new();
    r.get(SOURCE, FetchMode::Online).unwrap();
    let then = r.clock.now_ms();
    r.get(WORK, FetchMode::Online).unwrap();
    assert!(r.clock.now_ms() - then >= 100);
}
#[test]
fn mailto_only_from_settings() {
    let r = Rig::new();
    r.api().search_works("cancer", 2020, 1).unwrap();
    assert_eq!(r.transport.requests(), vec![PAGE1]);
    let r = Rig::new();
    fs::create_dir_all(r.home.join("paper")).unwrap();
    fs::write(
        r.home.join("paper/settings.json"),
        r#"{"contact_email":"test@example.org"}"#,
    )
    .unwrap();
    r.api().search_works("cancer", 2020, 1).unwrap();
    assert_eq!(
        r.transport.requests(),
        vec![format!("{PAGE1}&mailto=test%40example.org")]
    );
}
#[test]
fn offline_transport_returns_offline_kind() {
    let request = http::HttpRequest {
        url: SOURCE.into(),
        headers: vec![],
        timeout: Duration::from_secs(30),
    };
    assert_eq!(http::Offline.get(&request).err().unwrap().kind, "offline");
}
#[test]
fn changed_cache_byte_breaks_provenance_trace() {
    let r = Rig::new();
    let record = r.api().source("S1").unwrap();
    let tag = &match &record.publisher {
        Known::Known(v) => v,
        _ => panic!("publisher missing"),
    }
    .source;
    let expected = json!("Example Publisher");
    assert_raw(&r.root, tag, &expected);
    let (locator, _) = tag.raw.split_once('#').unwrap();
    let path = r.root.join(locator);
    let original = fs::read(&path).unwrap();
    // Try single-byte substitutions, allowing either UTF-8 or base64 cache bodies.
    let mut changed = false;
    'outer: for i in 0..original.len() {
        for replacement in [b'A', b'B', b'C', b'X', b'Y', b'Z', b'0', b'1'] {
            if original[i] == replacement {
                continue;
            }
            let mut bytes = original.clone();
            bytes[i] = replacement;
            if serde_json::from_slice::<Value>(&bytes).is_err() {
                continue;
            }
            fs::write(&path, &bytes).unwrap();
            if let Ok(value) = provenance::resolve(&r.root, tag) {
                if value != expected {
                    assert_eq!(
                        original.iter().zip(&bytes).filter(|(a, b)| a != b).count(),
                        1
                    );
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    assert!(
        changed,
        "one cache-file byte must change the resolved raw value"
    );
}

#[test]
fn cache_ttls_and_host_intervals() {
    for (kind, host, days, interval) in [
        (SourceKind::Openalex, "api.openalex.org", 7, 100),
        (SourceKind::Doaj, "doaj.org", 7, 500),
        (SourceKind::Crossref, "api.crossref.org", 30, 50),
        (SourceKind::Sjr, "www.scimagojr.com", 180, 1000),
    ] {
        let r = Rig::new();
        let url = format!("https://{host}/contract-ttl");
        let first = r
            .cache
            .get(kind.clone(), &url, FetchMode::Online, &r.transport)
            .unwrap();
        assert_eq!(first.fetched_at, "2023-11-14T22:13:20Z");
        let at = r.clock.now_ms();
        r.cache
            .get(
                kind.clone(),
                &format!("https://{host}/contract-interval"),
                FetchMode::Online,
                &r.transport,
            )
            .unwrap();
        assert!(r.clock.now_ms() - at >= interval);
        r.clock.set(1_700_000_000_000 + days * 86_400_000 - 1);
        assert!(
            r.cache
                .get(kind.clone(), &url, FetchMode::Online, &r.transport)
                .unwrap()
                .from_cache
        );
        r.clock.advance(2);
        let before = r.transport.requests();
        assert!(
            r.cache
                .get(kind.clone(), &url, FetchMode::OfflineOnly, &r.transport)
                .unwrap()
                .stale
        );
        assert_eq!(r.transport.requests(), before);
        assert!(
            !r.cache
                .get(kind, &url, FetchMode::Online, &r.transport)
                .unwrap()
                .from_cache
        );
    }
}

#[test]
fn request_timeout_and_503_retry() {
    use std::sync::Mutex;
    struct Inspect<'a> {
        inner: &'a CannedTransport,
        timeouts: Mutex<Vec<Duration>>,
    }
    impl Transport for Inspect<'_> {
        fn get(&self, req: &http::HttpRequest) -> axiom::base::Res<http::HttpResponse> {
            self.timeouts.lock().unwrap().push(req.timeout);
            self.inner.get(req)
        }
    }
    let r = Rig::new();
    let t = Inspect {
        inner: &r.transport,
        timeouts: Mutex::new(Vec::new()),
    };
    let at = r.clock.now_ms();
    let result = r
        .cache
        .get(
            SourceKind::Openalex,
            "https://api.openalex.org/unavailable",
            FetchMode::Online,
            &t,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&result.body).unwrap(),
        json!({"ok":true})
    );
    assert!(r.clock.now_ms() - at >= 3000);
    assert_eq!(
        *t.timeouts.lock().unwrap(),
        vec![Duration::from_secs(30); 2]
    );
}

#[test]
fn provenance_tag_records_request_and_cache_pointer() {
    let r = Rig::new();
    let fetched = r.get(SOURCE, FetchMode::Online).unwrap();
    let expected_locator = format!(
        "openalex/{}.json",
        axiom::seams::sha256_hex(SOURCE.as_bytes())
    );
    assert_eq!(fetched.locator.replace('\\', "/"), expected_locator);
    let tag = provenance::tag(
        SourceKind::Openalex,
        &fetched,
        SOURCE,
        "/display_name",
        Some(2023),
    );
    assert_eq!(tag.kind, SourceKind::Openalex);
    assert_eq!(tag.url.as_deref(), Some(SOURCE));
    assert_eq!(tag.fetched_at, fetched.fetched_at);
    assert_eq!(tag.data_year, Some(2023));
    assert_eq!(tag.raw, format!("{}#/display_name", fetched.locator));
    assert_raw(&r.root, &tag, &json!("Example Journal"));
}
