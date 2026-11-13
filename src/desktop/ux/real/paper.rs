// p8: Desktop adapter for real paper sessions.
use crate::{
    base::{self, Failure, Res},
    bridge::{
        paper_tool::ProductionMembers,
        server::{Bridge, Config},
    },
    council,
    desktop::ux::seams::{PaperService, Preview},
    paper::{
        self,
        cache::FetchMode,
        http::{CurlTransport, HttpRequest, HttpResponse, Transport},
        models::{self, PassModels},
        service::{PaperSession, PaperStatus},
        state, Requirements, SpendEstimate, Step,
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
    fn models(&self, session: &str) -> Res<PassModels> {
        models::load(&self.session(session)?.home)
    }
    fn set_models(&self, session: &str, m: PassModels) -> Res<()> {
        m.validate()?;
        let session = self.session(session)?;
        let map = lock_jobs()?;
        busy(&map, &session.store)?;
        models::save(&session.home, &m)
    }
    fn status(&self, session: &str) -> Res<PaperStatus> {
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
        Ok(status)
    }
    fn request_shortlist(&self, session: &str, req: Requirements, jcr: Option<PathBuf>) -> Res<()> {
        let session = self.session(session)?;
        self.spawn(
            session,
            move |s, _| {
                s.shortlist(&req, jcr.as_deref(), FetchMode::Online)
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
    fn estimate(&self, session: &str) -> Res<SpendEstimate> {
        let session = self.session(session)?;
        let map = lock_jobs()?;
        busy(&map, &session.store)?;
        session.estimate()
    }
    fn start(&self, session: &str, digest: &str) -> Res<()> {
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
    fn preview(&self, session: &str, page: u32, width: u32) -> Res<Preview> {
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
