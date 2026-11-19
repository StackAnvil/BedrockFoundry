//! Serial service worker for the desktop. It owns the backend so every account,
//! purchase, and server change runs one at a time on the same state the CLI uses.

use anyhow::Result;
use bedrock_foundry::{
    account::AccountInfo,
    backend::Backend,
    marketplace::{Addon, CatalogPage, CatalogQuery, PurchaseQuote, TransactionFailure},
    server::{Processes, ServerConfig},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct ServerView {
    pub config: ServerConfig,
    pub running: bool,
    pub ready: bool,
    pub logs: Vec<String>,
}

#[derive(Default, Clone)]
pub struct Snapshot {
    pub servers: Vec<ServerView>,
    /// Add-ons the connected account owns, after a library refresh.
    pub library: Vec<Addon>,
    /// Add-ons stored on this computer. They stay installable offline.
    pub downloaded: Vec<Addon>,
    pub account: Option<AccountInfo>,
    pub sample: Option<Addon>,
    pub catalog: Option<CatalogPage>,
    pub wallet: Option<u64>,
    pub quote: Option<PurchaseQuote>,
    pub root: PathBuf,
}

impl Snapshot {
    pub fn is_downloaded(&self, id: Uuid) -> bool {
        self.downloaded.iter().any(|addon| addon.id == id)
    }
    pub fn server(&self, id: Uuid) -> Option<&ServerView> {
        self.servers.iter().find(|server| server.config.id == id)
    }
}

pub enum Request {
    Restore,
    Login,
    Logout,
    Library,
    Browse(CatalogQuery),
    Balance,
    Quote(Uuid),
    Buy(Uuid),
    CancelPurchase,
    Sample,
    Claim(Uuid),
    Download(Uuid),
    Create(String, u16),
    Provision(Uuid),
    Import(Uuid, PathBuf),
    Install(Uuid, Uuid),
    Start(Uuid),
    Stop(Uuid),
    Console(Uuid, String),
    Save(ServerConfig),
    Backup(Uuid),
    Quit,
}

/// The kind of a submitted request, so the interface can place its result.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Restore,
    Login,
    Logout,
    Library,
    Browse,
    Balance,
    Quote,
    Buy,
    CancelPurchase,
    Sample,
    Claim,
    Download,
    Create,
    Provision,
    Import,
    Install,
    Start,
    Stop,
    Console,
    Save,
    Backup,
}

impl Request {
    pub fn op(&self) -> Op {
        match self {
            Request::Restore => Op::Restore,
            Request::Login => Op::Login,
            Request::Logout => Op::Logout,
            Request::Library => Op::Library,
            Request::Browse(_) => Op::Browse,
            Request::Balance => Op::Balance,
            Request::Quote(_) => Op::Quote,
            Request::Buy(_) => Op::Buy,
            Request::CancelPurchase => Op::CancelPurchase,
            Request::Sample => Op::Sample,
            Request::Claim(_) => Op::Claim,
            Request::Download(_) => Op::Download,
            Request::Create(..) => Op::Create,
            Request::Provision(_) => Op::Provision,
            Request::Import(..) => Op::Import,
            Request::Install(..) => Op::Install,
            Request::Start(_) => Op::Start,
            Request::Stop(_) => Op::Stop,
            Request::Console(..) => Op::Console,
            Request::Save(_) => Op::Save,
            Request::Backup(_) => Op::Backup,
            Request::Quit => unreachable!("Quit is never submitted as work"),
        }
    }
}

pub enum Outcome {
    /// A finished action with a confirmation for the status bar.
    Message(String),
    /// A finished action that needs no confirmation.
    Quiet,
    Connected(String),
    Created(Uuid),
    BackupSaved(PathBuf),
    /// The purchase was accepted. `true` means the library already lists it.
    Purchased(bool),
}

pub enum Event {
    Snapshot(Box<Snapshot>),
    Done(Op, std::result::Result<Outcome, Failure>),
    DeviceCode(String, String),
    Processes(Processes),
    /// The server list could not be read. The app keeps running.
    Warning(String),
    /// The data folder could not be opened.
    Fatal(String),
}

pub struct Failure {
    pub message: String,
    pub unknown_purchase: bool,
}

pub struct Worker {
    pub tx: mpsc::Sender<Request>,
    pub rx: mpsc::Receiver<Event>,
    pub cancel: Arc<AtomicBool>,
    pub processes: Option<Processes>,
}

impl Worker {
    pub fn start(root: Option<PathBuf>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = cancel.clone();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "The receiver can disconnect during shutdown"
                    )]
                    let _ = events.send(Event::Fatal(error.to_string()));
                    return;
                }
            };
            let mut backend = match Backend::open(root) {
                Ok(backend) => backend,
                Err(error) => {
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "The receiver can disconnect during shutdown"
                    )]
                    let _ = events.send(Event::Fatal(format!("{error:#}")));
                    return;
                }
            };
            #[expect(
                clippy::let_underscore_must_use,
                reason = "The receiver can disconnect during shutdown"
            )]
            let _ = events.send(Event::Processes(backend.processes.clone()));
            let mut session = Session::default();
            let mut last_warning = None;
            loop {
                let request = match requests.recv_timeout(Duration::from_millis(500)) {
                    Ok(Request::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(request) => Some(request),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                };
                if let Some(request) = request {
                    let op = request.op();
                    let result = runtime.block_on(run(
                        &mut backend,
                        &mut session,
                        request,
                        &events,
                        &cancellation,
                    ));
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "The receiver can disconnect during shutdown"
                    )]
                    let _ = events.send(Event::Done(
                        op,
                        result.map_err(|error| Failure {
                            unknown_purchase: op == Op::Buy
                                && error.downcast_ref::<TransactionFailure>()
                                    == Some(&TransactionFailure::Unknown),
                            message: format!("{error:#}"),
                        }),
                    ));
                }
                match snapshot(&backend, &session) {
                    Ok(snapshot) => {
                        last_warning = None;
                        #[expect(
                            clippy::let_underscore_must_use,
                            reason = "The receiver can disconnect during shutdown"
                        )]
                        let _ = events.send(Event::Snapshot(Box::new(snapshot)));
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if last_warning.as_ref() != Some(&message) {
                            #[expect(
                                clippy::let_underscore_must_use,
                                reason = "The receiver can disconnect during shutdown"
                            )]
                            let _ = events.send(Event::Warning(message.clone()));
                            last_warning = Some(message);
                        }
                    }
                }
            }
            backend.processes.stop_all();
        });
        Self {
            tx,
            rx,
            cancel,
            processes: None,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        #[expect(
            clippy::let_underscore_must_use,
            reason = "The receiver can disconnect during shutdown"
        )]
        let _ = self.tx.send(Request::Quit);
        if let Some(processes) = &self.processes {
            processes.stop_all();
        }
    }
}

#[derive(Default)]
struct Session {
    account: Option<AccountInfo>,
    sample: Option<Addon>,
}

#[expect(
    clippy::too_many_lines,
    reason = "Keep worker requests and their outcomes in one exhaustive match"
)]
async fn run(
    backend: &mut Backend,
    session: &mut Session,
    request: Request,
    events: &mpsc::Sender<Event>,
    cancellation: &AtomicBool,
) -> Result<Outcome> {
    Ok(match request {
        Request::Restore => {
            session.account = backend.restore().await?;
            match &session.account {
                Some(info) => Outcome::Connected(info.gamertag.clone()),
                None => Outcome::Quiet,
            }
        }
        Request::Login => {
            cancellation.store(false, Ordering::Relaxed);
            let info = backend
                .accounts
                .sign_in(
                    |url, code| {
                        #[expect(
                            clippy::let_underscore_must_use,
                            reason = "The receiver can disconnect during shutdown"
                        )]
                        let _ = events.send(Event::DeviceCode(url, code));
                    },
                    cancellation,
                )
                .await?;
            let gamertag = info.gamertag.clone();
            session.account = Some(info);
            session.sample = None;
            backend.clear_marketplace();
            Outcome::Connected(gamertag)
        }
        Request::Logout => {
            backend.accounts.sign_out()?;
            session.account = None;
            session.sample = None;
            backend.clear_marketplace();
            Outcome::Message("Microsoft account disconnected".into())
        }
        Request::Library => {
            backend.refresh_library().await?;
            Outcome::Quiet
        }
        Request::Browse(query) => {
            backend.search_catalog(query).await?;
            // A missing balance is shown as unknown; the listings still load.
            if backend.refresh_balance().await.is_err() {
                eprintln!("Could not refresh the Marketplace balance");
            }
            Outcome::Quiet
        }
        Request::Balance => {
            backend.refresh_balance().await?;
            Outcome::Quiet
        }
        Request::Quote(id) => {
            backend.quote_purchase(id).await?;
            Outcome::Quiet
        }
        Request::Buy(id) => Outcome::Purchased(backend.buy(id).await?),
        Request::CancelPurchase => {
            backend.purchase_quote = None;
            Outcome::Quiet
        }
        Request::Sample => {
            session.sample = Some(backend.sample().await?);
            Outcome::Quiet
        }
        Request::Claim(id) => {
            backend.claim(id).await?;
            backend.refresh_library().await?;
            session.sample = None;
            Outcome::Message("Free add-on added to your library".into())
        }
        Request::Download(id) => {
            backend.download(id).await?;
            Outcome::Message("Add-on downloaded. Install it on a server next.".into())
        }
        Request::Create(name, port) => Outcome::Created(backend.create(name, port)?.id),
        Request::Import(id, path) => {
            backend.import_server(id, path)?;
            Outcome::Message("Bedrock Dedicated Server imported".into())
        }
        Request::Provision(id) => {
            backend.provision(id).await?;
            Outcome::Message("Bedrock Dedicated Server installed".into())
        }
        Request::Install(server, addon) => {
            backend.install(server, addon)?;
            Outcome::Message("Add-on installed. Start the server to use it.".into())
        }
        Request::Start(id) => {
            backend.start(id)?;
            Outcome::Quiet
        }
        Request::Stop(id) => {
            backend.processes.stop(id)?;
            Outcome::Message("Server stopped".into())
        }
        Request::Console(id, command) => {
            backend.processes.command(id, &command)?;
            Outcome::Quiet
        }
        Request::Save(config) => {
            backend.update(&config)?;
            Outcome::Message("Server settings saved".into())
        }
        Request::Backup(id) => Outcome::BackupSaved(backend.backup(id)?),
        Request::Quit => unreachable!("Quit ends the worker loop"),
    })
}

fn snapshot(backend: &Backend, session: &Session) -> Result<Snapshot> {
    let servers = backend
        .servers()?
        .into_iter()
        .map(|config| {
            let running = backend.processes.running(config.id)?;
            let logs = backend.processes.logs(config.id);
            let ready = running && logs.iter().any(|line| line.contains("Server started."));
            Ok(ServerView {
                config,
                running,
                ready,
                logs,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut downloaded: Vec<Addon> = backend
        .available
        .values()
        .map(|cached| cached.addon.clone())
        .collect();
    downloaded.sort_by_key(|addon| (addon.name.to_lowercase(), addon.id));
    Ok(Snapshot {
        servers,
        library: backend.library.clone(),
        downloaded,
        account: session.account.clone(),
        sample: session.sample.clone(),
        catalog: backend.catalog.clone(),
        wallet: backend.wallet,
        quote: backend.purchase_quote.clone(),
        root: backend.store.root.clone(),
    })
}
