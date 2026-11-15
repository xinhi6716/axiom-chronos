// p8: Desktop adapter for real paper sessions.
use crate::{
    base::{self, Failure, Res},
    bridge::{
        paper_tool::ProductionMembers,
        server::{Bridge, Config},
    },
    council,
    desktop::ux::seams::{self as dto, EstimateRow, PaperService, Preview},
    paper::{
        self,
        cache::FetchMode,
        http::{CurlTransport, HttpRequest, HttpResponse, Transport},
        models::{self, PassModels as EngineModels},
        service::{PaperSession, PaperStatus},
        state, SpendEstimate, Step,
    },
    seams::{sha256_hex, SystemClock},
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

impl From<SpendEstimate> for EstimateRow {
    fn from(estimate: SpendEstimate) -> Self {
        Self {
            digest: estimate.digest,
            usd: estimate.priced_usd,
            tokens: None,
            stale: false,
            line: estimate.line,
        }
    }
}

macro_rules! map_enum {
    ($from:ty, $to:ty, $($variant:ident),+) => {
        impl From<$from> for $to { fn from(value: $from) -> Self { match value { $(<$from>::$variant => <$to>::$variant),+ } } }
        impl From<$to> for $from { fn from(value: $to) -> Self { match value { $(<$to>::$variant => <$from>::$variant),+ } } }
    };
}
map_enum!(paper::Quartile, dto::Quartile, Q1, Q2, Q3, Q4);
map_enum!(paper::QuartileScheme, dto::QuartileScheme, Sjr, Jcr);
map_enum!(
    paper::AccessWanted,
    dto::AccessWanted,
    OpenAccess,
    Subscription,
    Either
);
impl From<paper::Money> for dto::Money {
    fn from(m: paper::Money) -> Self {
        Self {
            amount_minor: m.amount_minor,
            currency: m.currency,
        }
    }
}
impl From<dto::Money> for paper::Money {
    fn from(m: dto::Money) -> Self {
        Self {
            amount_minor: m.amount_minor,
            currency: m.currency,
        }
    }
}
impl From<paper::Requirements> for dto::Requirements {
    fn from(r: paper::Requirements) -> Self {
        Self {
            abstract_text: r.abstract_text,
            keywords: r.keywords,
            max_quartile: r.max_quartile.map(Into::into),
            tier_scheme: r.tier_scheme.into(),
            access: r.access.into(),
            apc_limit: r.apc_limit.map(Into::into),
            require_no_fee: r.require_no_fee,
            categories: r.categories,
            free_text: r.free_text,
        }
    }
}
impl From<dto::Requirements> for paper::Requirements {
    fn from(r: dto::Requirements) -> Self {
        Self {
            abstract_text: r.abstract_text,
            keywords: r.keywords,
            max_quartile: r.max_quartile.map(Into::into),
            tier_scheme: r.tier_scheme.into(),
            access: r.access.into(),
            apc_limit: r.apc_limit.map(Into::into),
            require_no_fee: r.require_no_fee,
            categories: r.categories,
            free_text: r.free_text,
        }
    }
}
impl From<EngineModels> for dto::PassModels {
    fn from(m: EngineModels) -> Self {
        Self {
            astra_draft: m.astra_draft,
            fable_revise: m.fable_revise,
            gemini_revise: m.gemini_revise,
            fable_style: m.fable_style,
        }
    }
}
impl From<dto::PassModels> for EngineModels {
    fn from(m: dto::PassModels) -> Self {
        Self {
            astra_draft: m.astra_draft,
            fable_revise: m.fable_revise,
            gemini_revise: m.gemini_revise,
            fable_style: m.fable_style,
        }
    }
}
fn source_label(kind: paper::SourceKind) -> String {
    match kind {
        paper::SourceKind::Openalex => "OpenAlex",
        paper::SourceKind::Sjr => "SJR",
        paper::SourceKind::Doaj => "DOAJ",
        paper::SourceKind::Crossref => "Crossref",
        paper::SourceKind::JcrImport => "JCR",
        paper::SourceKind::OfficialPage => "Official page",
        paper::SourceKind::Program => "Program",
        paper::SourceKind::Dataset => "Dataset",
        paper::SourceKind::Person => "Person",
    }
    .into()
}
fn metric<T>(
    label: &str,
    value: paper::Known<T>,
    format: impl FnOnce(T) -> String,
) -> dto::MetricRow {
    match value {
        paper::Known::Known(s) => dto::MetricRow {
            label: label.into(),
            value: format(s.value),
            source: source_label(s.source.kind),
            date: s.source.fetched_at,
        },
        paper::Known::Unknown { reason } => dto::MetricRow {
            label: label.into(),
            value: format!("Unknown ({reason})"),
            source: "Unknown".into(),
            date: "Unknown".into(),
        },
    }
}
fn journal(j: paper::JournalRecord, hash: &str) -> dto::JournalRow {
    let mut metrics = vec![
        metric("Publisher", j.publisher, |v| v),
        metric("Homepage", j.homepage, |v| v),
        metric("SJR", j.sjr, |v| v.to_string()),
        metric("Access", j.access, |v| {
            match v {
                paper::Access::OpenAccess => "Open access",
                paper::Access::Hybrid => "Hybrid",
                paper::Access::Subscription => "Subscription",
            }
            .into()
        }),
        metric("APC", j.apc, |v| match v {
            paper::Apc::NoFee => "No fee".into(),
            paper::Apc::Fee(m) => format!("{} {}", m.amount_minor, m.currency),
        }),
        metric("Review weeks", j.review_weeks, |v| v.to_string()),
        metric("Works", j.works_count, |v| v.to_string()),
    ];
    if j.quartiles.is_empty() {
        metrics.push(dto::MetricRow {
            label: "Quartile".into(),
            value: "Unknown".into(),
            source: "Unknown".into(),
            date: "Unknown".into(),
        });
    }
    for q in j.quartiles {
        metrics.push(dto::MetricRow {
            label: "Quartile".into(),
            value: q.value.label(),
            source: source_label(q.source.kind),
            date: q.source.fetched_at,
        });
    }
    dto::JournalRow {
        id: j.id,
        title: j.title,
        hash: hash.into(),
        metrics,
        ..Default::default()
    }
}
impl From<paper::Shortlist> for dto::ShortlistView {
    fn from(list: paper::Shortlist) -> Self {
        let mut rows = Vec::new();
        for r in list.ranked {
            let mut row = journal(r.journal, &list.hash);
            row.bucket = match r.bucket {
                paper::Bucket::Passes => "Passes",
                paper::Bucket::NeedsCheck => "Needs check",
            }
            .into();
            row.filters = r
                .filters
                .into_iter()
                .map(|f| dto::FilterRow {
                    name: f.name,
                    status: format!("{:?}", f.status),
                    detail: f.detail,
                })
                .chain(r.free_text.into_iter().map(|f| dto::FilterRow {
                    name: f.text,
                    status: format!("{:?}", f.status),
                    detail: f.detail,
                }))
                .collect();
            rows.push(row);
        }
        for r in list.excluded {
            let mut row = journal(r.journal, &list.hash);
            row.excluded = true;
            row.filters = r
                .filters
                .into_iter()
                .map(|f| dto::FilterRow {
                    name: f.name,
                    status: format!("{:?}", f.status),
                    detail: f.detail,
                })
                .collect();
            rows.push(row);
        }
        Self { rows }
    }
}
impl From<PaperStatus> for dto::PaperView {
    fn from(s: PaperStatus) -> Self {
        let shortlist = s
            .shortlist
            .map(|v| dto::ShortlistView::from(v).rows)
            .unwrap_or_default();
        let journal = s.choice.and_then(|c| {
            shortlist
                .iter()
                .find(|j| !j.excluded && j.id == c.journal_id && j.hash == c.shortlist_hash)
                .cloned()
        });
        let mut changes = Vec::new();
        let passes = s
            .passes
            .into_iter()
            .map(|p| {
                let number = p.pass.number();
                for c in p.change_log {
                    changes.push(dto::ChangeRow {
                        pass: number,
                        member: p.member.clone(),
                        location: c.location,
                        kind: format!("{:?}", c.kind),
                        reason: c.reason,
                        validation: false,
                    });
                }
                for v in p.validation {
                    changes.push(dto::ChangeRow {
                        pass: number,
                        member: p.member.clone(),
                        location: String::new(),
                        kind: String::new(),
                        reason: v,
                        validation: true,
                    });
                }
                dto::PassRow {
                    number,
                    status: format!("{:?}", p.status),
                }
            })
            .collect();
        let steps = s
            .steps
            .into_iter()
            .map(|(step, ok, detail)| dto::GateRow {
                label: format!("{:?}", step),
                ok,
                detail,
            })
            .collect();
        let readiness = s
            .ready
            .map(|r| {
                r.conditions
                    .into_iter()
                    .map(|(condition, ok, detail)| dto::GateRow {
                        label: format!("{:?}", condition),
                        ok,
                        detail: Some(detail),
                    })
                    .collect()
            })
            .unwrap_or_default();
        dto::PaperView {
            phase: if s.running { "running" } else { "idle" }.into(),
            passes,
            journal,
            shortlist,
            estimate: s.estimate.map(Into::into),
            changelog: changes,
            readiness,
            requirements: s.requirements.map(Into::into),
            evidence: s.evidence,
            pdf: s.pdf,
            running: s.running,
            last_error: s.last_error,
            steps,
        }
    }
}

pub struct SessionPaper {
    home: PathBuf,
    data_home: Res<PathBuf>,
}
struct EnvironmentTransport;
impl Transport for EnvironmentTransport {
    fn get(&self, request: &HttpRequest) -> Res<HttpResponse> {
        CurlTransport::from_env()?.get(request)
    }
}

// Jobs are keyed by store so separate adapters cannot write the same session concurrently.
fn jobs() -> &'static Mutex<HashMap<PathBuf, Arc<AtomicBool>>> {
    static JOBS: OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicBool>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn lock_jobs() -> Res<std::sync::MutexGuard<'static, HashMap<PathBuf, Arc<AtomicBool>>>> {
    jobs()
        .lock()
        .map_err(|e| Failure::new("lock", e.to_string()))
}
fn worker_errors() -> &'static Mutex<HashMap<PathBuf, Failure>> {
    static ERRORS: OnceLock<Mutex<HashMap<PathBuf, Failure>>> = OnceLock::new();
    ERRORS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn busy(map: &HashMap<PathBuf, Arc<AtomicBool>>, store: &Path) -> Res<()> {
    if map.contains_key(store) {
        Err(Failure::new("busy", "paper session is busy"))
    } else {
        Ok(())
    }
}
fn save_status(store: &Path, running: bool, error: Option<&Failure>) -> Res<()> {
    let dir = paper::paper_dir(store);
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes = serde_json::to_vec(&serde_json::json!({"running": running, "last_error": error.map(|e| serde_json::json!({"kind": e.kind, "reason": e.message}))})).map_err(|e| Failure::new("json", e.to_string()))?;
    base::atomic_write(&dir.join("status.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}
impl SessionPaper {
    #[cfg(test)]
    pub(super) fn with_data_home(home: &Path, data_home: &Path) -> Self {
        Self {
            home: home.into(),
            data_home: Ok(data_home.into()),
        }
    }
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            data_home: base::data_home(),
        }
    }
    fn session(&self, session: &str) -> Res<PaperSession> {
        let store = super::session_store(&self.home, session)?;
        let goal = crate::goal::load(&self.home, session)?
            .ok_or_else(|| Failure::new("not_found", "goal not found"))?;
        let root = goal
            .cwd
            .map(PathBuf::from)
            .ok_or_else(|| Failure::new("no_project", "goal has no project directory"))?;
        PaperSession::open(
            &store,
            self.data_home.as_ref().map_err(Clone::clone)?,
            &root,
            Arc::new(EnvironmentTransport),
            Arc::new(SystemClock),
        )
    }
    fn validate_start(session: &PaperSession, digest: &str) -> Res<()> {
        let status = session.status()?;
        let state = state::load(&session.store)?;
        let choice = status
            .choice
            .ok_or_else(|| Failure::new("no_journal", "current journal choice is missing"))?;
        let journal = fs::read(paper::readiness::journal_path(&session.store))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        if !state.as_ref().is_some_and(|s| {
            state::current(s, Step::Journal)
                && s.steps
                    .get(&Step::Shortlist)
                    .is_some_and(|r| r.output == choice.shortlist_hash)
                && s.steps[&Step::Journal].output == sha256_hex(&journal)
        }) {
            return Err(Failure::new(
                "no_journal",
                "current journal choice is missing",
            ));
        }
        if !status.shortlist.as_ref().is_some_and(|l| {
            l.hash == choice.shortlist_hash
                && l.ranked.iter().any(|j| j.journal.id == choice.journal_id)
        }) {
            return Err(Failure::new(
                "no_journal",
                "chosen journal not in current shortlist",
            ));
        }
        if paper::budget::estimate(
            &session.home,
            &models::estimate_models(),
            &models::load(&session.home)?,
        )?
        .digest
            != digest
        {
            return Err(Failure::new("estimate_changed", "estimate has changed"));
        }
        Ok(())
    }
    fn spawn(
        &self,
        session: PaperSession,
        work: impl FnOnce(PaperSession, Arc<AtomicBool>) -> Res<()> + Send + 'static,
        shortlist: bool,
    ) -> Res<()> {
        let store = session.store.clone();
        let mut map = lock_jobs()?;
        busy(&map, &store)?;
        let stop = Arc::new(AtomicBool::new(false));
        map.insert(store.clone(), stop.clone());
        let worker_store = store.clone();
        let result = std::thread::Builder::new()
            .name("paper-session".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(session, stop)))
                        .unwrap_or_else(|_| Err(Failure::new("thread", "paper worker panicked")));
                let saved = if shortlist || result.is_err() {
                    save_status(&worker_store, false, result.as_ref().err())
                } else {
                    Ok(())
                };
                if let Ok(mut map) = lock_jobs() {
                    if let Ok(mut errors) = worker_errors().lock() {
                        if let Err(error) = saved {
                            errors.insert(worker_store.clone(), error);
                        } else {
                            errors.remove(&worker_store);
                        }
                    }
                    map.remove(&worker_store);
                }
            });
        if let Err(e) = result {
            map.remove(&store);
            return Err(Failure::new("thread", e.to_string()));
        }
        Ok(())
    }
    fn pdf(&self, session: &str) -> Res<PathBuf> {
        let store = super::session_store(&self.home, session)?;
        let path = paper::readiness::pdf_path(&store);
        if path.is_file() {
            Ok(path)
        } else {
            Err(Failure::new("no_pdf", "paper PDF is missing"))
        }
    }
}
impl PaperService for SessionPaper {
    fn evidence_question(&self, session: &str) -> Res<Option<String>> {
        let store = super::session_store(&self.home, session)?;
        Ok(paper::evidence::load(&store)?.map(|bundle| bundle.question))
    }
    fn models(&self, session: &str) -> Res<dto::PassModels> {
        models::load(&self.session(session)?.home).map(Into::into)
    }
    fn set_models(&self, session: &str, m: dto::PassModels) -> Res<()> {
        m.validate()?;
        let session = self.session(session)?;
        let map = lock_jobs()?;
        busy(&map, &session.store)?;
        models::save(&session.home, &m.into())
    }
    fn status(&self, session: &str) -> Res<dto::PaperView> {
        let session = self.session(session)?;
        let mut status = session.status()?;
        let map = lock_jobs()?;
        if let Some(error) = worker_errors()
            .lock()
            .map_err(|e| Failure::new("lock", e.to_string()))?
            .get(&session.store)
        {
            return Err(error.clone());
        }
        if map.contains_key(&session.store) {
            status.running = true;
        }
        Ok(status.into())
    }
    fn request_shortlist(&self, session: &str, input: dto::ShortlistInput) -> Res<()> {
        let session = self.session(session)?;
        self.spawn(
            session,
            move |s, _| {
                s.shortlist(
                    &input.requirements.into(),
                    input.jcr.as_deref(),
                    FetchMode::Online,
                )
                .map(|_| ())
            },
            true,
        )
    }
    fn choose_journal(&self, session: &str, journal_id: &str, hash: &str) -> Res<()> {
        let session = self.session(session)?;
        let map = lock_jobs()?;
        busy(&map, &session.store)?;
        session.choose_journal(journal_id, hash).map(|_| ())
    }
    fn estimate(&self, session: &str) -> Res<EstimateRow> {
        let session = self.session(session)?;
        let map = lock_jobs()?;
        busy(&map, &session.store)?;
        session.estimate().map(Into::into)
    }
    fn estimate_is_fresh(&self, session: &str, digest: &str) -> Res<bool> {
        let session = self.session(session)?;
        Ok(paper::budget::estimate(
            &session.home,
            &models::estimate_models(),
            &models::load(&session.home)?,
        )?
        .digest
            == digest)
    }
    fn start(&self, session: &str, input: dto::StartInput) -> Res<()> {
        let digest = &input.estimate_digest;
        let session = self.session(session)?;
        {
            let map = lock_jobs()?;
            busy(&map, &session.store)?;
            Self::validate_start(&session, digest)?;
        }
        let digest = digest.to_owned();
        self.spawn(
            session,
            move |s, stop| {
                let bridge = Bridge::new(Config::from_env());
                let setup = council::Setup::new(
                    base::resolve_command("AXIOM_CLAUDE_CMD", "claude"),
                    base::resolve_command("AXIOM_AGY_CMD", "agy"),
                );
                let members = ProductionMembers::new(&bridge, &setup, &s.store, &s.project_root);
                s.write(&members.members(), &digest, &stop, &|_| {})
                    .map(|_| ())
            },
            false,
        )
    }
    fn cancel(&self, session: &str) -> Res<()> {
        let store = super::session_store(&self.home, session)?;
        if let Some(stop) = lock_jobs()?.get(&store) {
            stop.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn preview(&self, session: &str, input: dto::PreviewInput) -> Res<Preview> {
        let page = input.page;
        let width = input.width_px;
        let pdf = self.pdf(session)?;
        if page == 0 {
            return Err(Failure::new("bad_page", "page numbers start at 1"));
        }
        #[cfg(windows)]
        {
            render_pdf(pdf, page, width)
        }
        #[cfg(not(windows))]
        {
            let _ = width;
            let report = paper::figures::pages::check_pdf(&pdf, Path::new(""), None)?;
            if report.pages > 0 && page > report.pages {
                return Err(Failure::new("bad_page", "page is out of range"));
            }
            Ok(Preview::Unavailable {
                reason: "PDF preview requires Windows".into(),
                pdf,
            })
        }
    }
    fn open_pdf(&self, session: &str) -> Res<()> {
        let pdf = self.pdf(session)?;
        let default = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let argv = base::resolve_command("AXIOM_OPEN_CMD", default)?;
        base::proc::cmd(&argv[0])
            .args(&argv[1..])
            .arg(pdf)
            .spawn()
            .map_err(|e| Failure::new("open_pdf", e.to_string()))?;
        Ok(())
    }
}

#[cfg(windows)]
fn render_pdf(pdf: PathBuf, page: u32, width: u32) -> Res<Preview> {
    use crate::desktop::ux::seams::PageImage;
    use windows::{
        Data::Pdf::{PdfDocument, PdfPageRenderOptions},
        Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
    };
    let render = || -> Res<PageImage> {
        let err = |e: windows::core::Error| Failure::new("preview", e.to_string());
        let bytes = fs::read(&pdf).map_err(|e| Failure::new("io", e.to_string()))?;
        let input = InMemoryRandomAccessStream::new().map_err(err)?;
        let writer =
            DataWriter::CreateDataWriter(&input.GetOutputStreamAt(0).map_err(err)?).map_err(err)?;
        writer.WriteBytes(&bytes).map_err(err)?;
        writer.StoreAsync().map_err(err)?.get().map_err(err)?;
        input.Seek(0).map_err(err)?;
        let document = PdfDocument::LoadFromStreamAsync(&input)
            .map_err(err)?
            .get()
            .map_err(err)?;
        let pages = document.PageCount().map_err(err)?;
        if page > pages {
            return Err(Failure::new("bad_page", "page is out of range"));
        }
        let pdf_page = document.GetPage(page - 1).map_err(err)?;
        let size = pdf_page.Size().map_err(err)?;
        let output = InMemoryRandomAccessStream::new().map_err(err)?;
        let options = PdfPageRenderOptions::new().map_err(err)?;
        options.SetDestinationWidth(width).map_err(err)?;
        pdf_page
            .RenderWithOptionsToStreamAsync(&output, &options)
            .map_err(err)?
            .get()
            .map_err(err)?;
        let length = u32::try_from(output.Size().map_err(err)?)
            .map_err(|e| Failure::new("preview", e.to_string()))?;
        let reader =
            DataReader::CreateDataReader(&output.GetInputStreamAt(0).map_err(err)?).map_err(err)?;
        reader.LoadAsync(length).map_err(err)?.get().map_err(err)?;
        let mut png = vec![0; length as usize];
        reader.ReadBytes(&mut png).map_err(err)?;
        Ok(PageImage {
            png,
            width_px: width,
            height_px: (size.Height * width as f32 / size.Width).round() as u32,
            page,
            pages,
        })
    };
    match render() {
        Ok(image) => Ok(Preview::Ready(image)),
        Err(error) if error.kind == "bad_page" => Err(error),
        Err(error) => Ok(Preview::Unavailable {
            reason: error.to_string(),
            pdf,
        }),
    }
}
// p8: end
