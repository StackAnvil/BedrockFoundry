use anyhow::{Context as _, Result};
use bedrock_foundry::{
    account::AccountInfo,
    backend::Backend,
    marketplace::Addon,
    server::{Processes, ServerConfig},
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Root, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use uuid::Uuid;

const INK: u32 = 0x172f46;
const MUTED: u32 = 0x697c8d;
const CANVAS: u32 = 0xf3f6f8;
const LINE: u32 = 0xdce4eb;
const TEAL: u32 = 0x176b73;

#[derive(Clone)]
struct ServerView {
    config: ServerConfig,
    running: bool,
    ready: bool,
    logs: Vec<String>,
}
#[derive(Default, Clone)]
struct Snapshot {
    servers: Vec<ServerView>,
    library: Vec<Addon>,
    downloaded: HashSet<Uuid>,
    account: Option<AccountInfo>,
    sample: Option<Addon>,
    root: PathBuf,
}
enum Request {
    Restore,
    Login,
    Logout,
    Library,
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
enum Event {
    Snapshot(Box<Snapshot>),
    Status(String, bool),
    DeviceCode(String, String),
    Select(Uuid),
    Processes(Processes),
}
struct Worker {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
    processes: Option<Processes>,
}
impl Worker {
    fn start(root: Option<PathBuf>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = cancel.clone();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(e) => {
                    let _ = events.send(Event::Status(e.to_string(), true));
                    return;
                }
            };
            let mut backend = match Backend::open(root) {
                Ok(b) => b,
                Err(e) => {
                    let _ = events.send(Event::Status(format!("{e:#}"), true));
                    return;
                }
            };
            let _ = events.send(Event::Processes(backend.processes.clone()));
            let mut account = None;
            let mut sample = None;
            loop {
                let request = match requests.recv_timeout(Duration::from_millis(500)) {
                    Ok(Request::Quit) => break,
                    Ok(request) => Some(request),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if let Some(request) = request {
                    let result: Result<String> = runtime.block_on(async {
                        match request {
                            Request::Restore => {
                                account = backend.restore().await?;
                                Ok("Ready".to_string())
                            }
                            Request::Login => {
                                cancellation.store(false, Ordering::Relaxed);
                                let info = backend
                                    .accounts
                                    .sign_in(
                                        |url, code| {
                                            let _ = events.send(Event::DeviceCode(url, code));
                                        },
                                        &cancellation,
                                    )
                                    .await?;
                                let message = format!("Connected as {}", info.gamertag);
                                account = Some(info);
                                Ok(message)
                            }
                            Request::Logout => {
                                backend.accounts.sign_out()?;
                                account = None;
                                backend.library.clear();
                                sample = None;
                                Ok("Account disconnected".into())
                            }
                            Request::Library => {
                                backend.refresh_library().await?;
                                Ok("Marketplace library refreshed".into())
                            }
                            Request::Sample => {
                                sample = Some(backend.sample().await?);
                                Ok("Free sample loaded".into())
                            }
                            Request::Claim(id) => {
                                backend.claim(id).await?;
                                backend.refresh_library().await?;
                                sample = None;
                                Ok("Free add-on claimed".into())
                            }
                            Request::Download(id) => {
                                backend.download(id).await?;
                                Ok("Add-on downloaded".into())
                            }
                            Request::Create(name, port) => {
                                let config = backend.create(name, port)?;
                                let _ = events.send(Event::Select(config.id));
                                Ok("Server created. Install BDS next.".into())
                            }
                            Request::Import(id, path) => {
                                backend.import_server(id, path)?;
                                Ok("BDS imported".into())
                            }
                            Request::Provision(id) => {
                                backend.provision(id).await?;
                                Ok("BDS installed".into())
                            }
                            Request::Install(server, addon) => {
                                backend.install(server, addon)?;
                                Ok("Add-on installed in the selected server".into())
                            }
                            Request::Start(id) => {
                                backend.start(id)?;
                                Ok("BDS process started".into())
                            }
                            Request::Stop(id) => {
                                backend.processes.stop(id)?;
                                Ok("Server stopped".into())
                            }
                            Request::Console(id, command) => {
                                backend.processes.command(id, &command)?;
                                Ok("Command sent".into())
                            }
                            Request::Save(config) => {
                                backend.update(&config)?;
                                Ok("Server settings saved".into())
                            }
                            Request::Backup(id) => {
                                let path = backend.backup(id)?;
                                Ok(format!("Backup saved: {}", path.display()))
                            }
                            Request::Quit => unreachable!(),
                        }
                    });
                    match result {
                        Ok(message) => {
                            let _ = events.send(Event::Status(message, false));
                        }
                        Err(error) => {
                            let _ = events.send(Event::Status(format!("{error:#}"), true));
                        }
                    }
                }
                match backend.servers() {
                    Ok(servers) => {
                        let servers = servers
                            .into_iter()
                            .map(|config| {
                                let running = backend.processes.running(config.id);
                                let logs = backend.processes.logs(config.id);
                                let ready = running
                                    && logs.iter().any(|line| line.contains("Server started."));
                                ServerView {
                                    config,
                                    running,
                                    ready,
                                    logs,
                                }
                            })
                            .collect();
                        let _ = events.send(Event::Snapshot(Box::new(Snapshot {
                            servers,
                            library: backend.library.clone(),
                            downloaded: backend.available.keys().copied().collect(),
                            account: account.clone(),
                            sample: sample.clone(),
                            root: backend.store.root.clone(),
                        })));
                    }
                    Err(error) => {
                        let _ = events.send(Event::Status(format!("{error:#}"), true));
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
        let _ = self.tx.send(Request::Quit);
        if let Some(processes) = &self.processes {
            processes.stop_all();
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Servers,
    Library,
    Account,
}
struct Foundry {
    worker: Worker,
    data: Snapshot,
    page: Page,
    selected: Option<Uuid>,
    pending_select: Option<Uuid>,
    busy: bool,
    message: String,
    error: bool,
    device: Option<(String, String)>,
    new_name: Entity<InputState>,
    new_port: Entity<InputState>,
    name: Entity<InputState>,
    port: Entity<InputState>,
    players: Entity<InputState>,
    console: Entity<InputState>,
    mode: String,
    difficulty: String,
}
impl Foundry {
    fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let worker = Worker::start(root);
        let _ = worker.tx.send(Request::Restore);
        let new_name = cx.new(|cx| InputState::new(window, cx).default_value("My Bedrock server"));
        let new_port = cx.new(|cx| InputState::new(window, cx).default_value("19132"));
        let name = cx.new(|cx| InputState::new(window, cx));
        let port = cx.new(|cx| InputState::new(window, cx));
        let players = cx.new(|cx| InputState::new(window, cx));
        let console =
            cx.new(|cx| InputState::new(window, cx).placeholder("Enter a server command"));
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if let Err(error) = this.update_in(cx, |this, window, cx| this.receive(window, cx))
                {
                    eprintln!("UI event loop: {error}");
                    break;
                }
            }
        })
        .detach();
        Self {
            worker,
            data: Snapshot::default(),
            page: Page::Servers,
            selected: None,
            pending_select: None,
            busy: true,
            message: "Opening Bedrock Foundry…".into(),
            error: false,
            device: None,
            new_name,
            new_port,
            name,
            port,
            players,
            console,
            mode: "survival".into(),
            difficulty: "normal".into(),
        }
    }
    fn receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(event) = self.worker.rx.try_recv() {
            changed = true;
            match event {
                Event::Snapshot(snapshot) => {
                    self.data = *snapshot;
                    let target = self
                        .pending_select
                        .take()
                        .or(self.selected)
                        .or_else(|| self.data.servers.first().map(|s| s.config.id));
                    if target != self.selected
                        && let Some(id) = target
                    {
                        self.select(id, window, cx);
                    }
                }
                Event::Status(message, error) => {
                    self.busy = false;
                    self.message = message;
                    self.error = error;
                    self.device = None;
                }
                Event::DeviceCode(url, code) => {
                    self.device = Some((url, code));
                    self.message = "Finish sign-in in your browser".into();
                }
                Event::Select(id) => self.pending_select = Some(id),
                Event::Processes(processes) => self.worker.processes = Some(processes),
            }
        }
        if changed {
            cx.notify();
        }
    }
    fn select(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id);
        if let Some(server) = self.data.servers.iter().find(|s| s.config.id == id) {
            self.name.update(cx, |input, cx| {
                input.set_value(server.config.name.clone(), window, cx)
            });
            self.port.update(cx, |input, cx| {
                input.set_value(server.config.port.to_string(), window, cx)
            });
            self.players.update(cx, |input, cx| {
                input.set_value(server.config.max_players.to_string(), window, cx)
            });
            self.mode = server.config.gamemode.clone();
            self.difficulty = server.config.difficulty.clone();
        }
        cx.notify();
    }
    fn submit(&mut self, request: Request, message: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = false;
        self.message = message.into();
        if self.worker.tx.send(request).is_err() {
            self.busy = false;
            self.error = true;
            self.message = "The service worker stopped. Restart Bedrock Foundry.".into();
        }
        cx.notify();
    }
    fn failure(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.error = true;
        self.message = format!("{error:#}");
        cx.notify();
    }
    fn server_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().flex().flex_col().gap_6().child(heading(
            "Servers",
            "Create a server, add your packs, and run it here.",
        ));
        let create = div()
            .flex()
            .items_end()
            .gap_3()
            .p_4()
            .bg(rgb(0xffffff))
            .rounded_lg()
            .border_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .flex_1()
                    .child(label("New server name"))
                    .child(Input::new(&self.new_name)),
            )
            .child(
                div()
                    .w(px(100.))
                    .child(label("Port"))
                    .child(Input::new(&self.new_port)),
            )
            .child(
                Button::new("create")
                    .primary()
                    .label("Create server")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let name = this.new_name.read(cx).value().to_string();
                        match this.new_port.read(cx).value().parse::<u16>() {
                            Ok(port) => {
                                this.submit(Request::Create(name, port), "Creating server…", cx)
                            }
                            Err(_) => {
                                this.failure(anyhow::anyhow!("Enter a valid port number"), cx)
                            }
                        }
                    })),
            );
        body = body.child(create);
        if self.data.servers.is_empty() {
            return body
                .child(
                    div()
                        .py_8()
                        .text_color(rgb(MUTED))
                        .child("Your servers will appear here. Create one above to get started."),
                )
                .into_any_element();
        }
        let mut tabs = div().flex().flex_wrap().gap_2();
        for server in self.data.servers.clone() {
            let id = server.config.id;
            let selected = self.selected == Some(id);
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!("server-{id}")))
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(if selected { TEAL } else { LINE }))
                    .bg(rgb(if selected { 0xe5f0f1 } else { 0xffffff }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| this.select(id, window, cx)))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(server.config.name),
                    )
                    .child(div().mt_1().text_xs().text_color(rgb(MUTED)).child(format!(
                        "{}  ·  Port {}",
                        if server.ready {
                            "Running"
                        } else if server.running {
                            "Starting"
                        } else {
                            "Stopped"
                        },
                        server.config.port
                    ))),
            );
        }
        body = body.child(tabs);
        let Some(server) = self
            .data
            .servers
            .iter()
            .find(|s| Some(s.config.id) == self.selected)
            .cloned()
        else {
            return body.into_any_element();
        };
        let id = server.config.id;
        let mut actions = div().flex().items_center().gap_2().child(
            div()
                .flex_1()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(server.config.name.clone()),
        );
        if server.config.version.is_none() {
            actions = actions.child(
                Button::new("provision")
                    .primary()
                    .label("Download and install BDS")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit(
                            Request::Provision(id),
                            "Downloading BDS from Minecraft…",
                            cx,
                        )
                    })),
            );
            actions = actions.child(
                Button::new("import-bds")
                    .label("Import BDS ZIP")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let picker = cx.prompt_for_paths(PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: false,
                            prompt: Some("Choose an official BDS ZIP".into()),
                        });
                        cx.spawn(async move |this, cx| {
                            let selected = picker.await;
                            let _ = this.update(cx, |this, cx| match selected {
                                Ok(Ok(Some(paths))) => {
                                    if let Some(path) = paths.into_iter().next() {
                                        this.submit(
                                            Request::Import(id, path),
                                            "Importing BDS…",
                                            cx,
                                        );
                                    }
                                }
                                Ok(Ok(None)) => {}
                                Ok(Err(error)) => this.failure(error, cx),
                                Err(error) => this.failure(error.into(), cx),
                            });
                        })
                        .detach();
                    })),
            );
        } else if server.running {
            actions = actions.child(
                Button::new("stop")
                    .danger()
                    .label("Stop server")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit(Request::Stop(id), "Stopping server…", cx)
                    })),
            );
        } else {
            actions = actions.child(
                Button::new("start")
                    .primary()
                    .label("Start server")
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit(Request::Start(id), "Starting BDS…", cx)
                    })),
            );
        }
        actions = actions.child(
            Button::new("backup")
                .label("Back up")
                .disabled(self.busy || server.running || server.config.version.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.submit(Request::Backup(id), "Creating backup…", cx)
                })),
        );
        let folder = self.data.root.join("servers").join(id.to_string());
        actions = actions.child(Button::new("folder").label("Open folder").on_click(
            move |_, _, _| {
                let _ = open::that(&folder);
            },
        ));
        body = body.child(actions);
        let installed = server
            .config
            .addons
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        body = body.child(div().text_sm().text_color(rgb(MUTED)).child(format!(
            "BDS {}   |   {}",
            server.config.version.as_deref().unwrap_or("not installed"),
            if installed.is_empty() {
                "No add-ons installed".to_string()
            } else {
                installed
            }
        )));
        let mut settings = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(rgb(0xffffff))
            .rounded_lg()
            .border_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Server settings"),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .child(label("Name"))
                            .child(Input::new(&self.name).disabled(server.running)),
                    )
                    .child(
                        div()
                            .w_24()
                            .child(label("Port"))
                            .child(Input::new(&self.port).disabled(server.running)),
                    )
                    .child(
                        div()
                            .w_24()
                            .child(label("Players"))
                            .child(Input::new(&self.players).disabled(server.running)),
                    ),
            );
        settings = settings.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    Button::new("mode")
                        .label(format!("Mode: {}", self.mode))
                        .disabled(server.running || self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mode = match this.mode.as_str() {
                                "survival" => "creative",
                                "creative" => "adventure",
                                _ => "survival",
                            }
                            .into();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("difficulty")
                        .label(format!("Difficulty: {}", self.difficulty))
                        .disabled(server.running || self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.difficulty = match this.difficulty.as_str() {
                                "peaceful" => "easy",
                                "easy" => "normal",
                                "normal" => "hard",
                                _ => "peaceful",
                            }
                            .into();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("save")
                        .label("Save settings")
                        .disabled(server.running || self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let result = (|| -> Result<ServerConfig> {
                                let mut config = this
                                    .data
                                    .servers
                                    .iter()
                                    .find(|s| s.config.id == id)
                                    .context("Select a server")?
                                    .config
                                    .clone();
                                config.name = this.name.read(cx).value().to_string();
                                config.port = this
                                    .port
                                    .read(cx)
                                    .value()
                                    .parse()
                                    .context("Enter a valid port number")?;
                                config.max_players = this
                                    .players
                                    .read(cx)
                                    .value()
                                    .parse()
                                    .context("Enter a valid player limit")?;
                                config.gamemode = this.mode.clone();
                                config.difficulty = this.difficulty.clone();
                                config.validate()?;
                                Ok(config)
                            })();
                            match result {
                                Ok(config) => {
                                    this.submit(Request::Save(config), "Saving settings…", cx)
                                }
                                Err(e) => this.failure(e, cx),
                            }
                        })),
                ),
        );
        let config = server.config.clone();
        let public = config.public;
        settings=settings.child(div().mt_2().flex().items_center().gap_3().child(div().flex_1().text_sm().child(if public{format!("Network connections enabled. NetherNet uses TCP {} and UDP {}. Configure your firewall and router.",config.port,config.port+1)}else{"This server accepts connections from this computer only.".into()})).child(Button::new("network").label(if public{"Use this computer only"}else{"Allow network connections"}).disabled(server.running||self.busy).on_click(cx.listener(move|this,_,_,cx|{let mut config=config.clone();config.public = !public;this.submit(Request::Save(config),"Updating network access…",cx);}))))
            .child(div().text_xs().text_color(rgb(MUTED)).child("Stop the server before changing settings or creating a backup."));
        body = body.child(settings);
        let mut logs = div()
            .id("console-log")
            .h(px(220.))
            .overflow_y_scroll()
            .p_3()
            .bg(rgb(0x172f46))
            .text_color(rgb(0xdce9f3))
            .font_family("monospace")
            .text_xs();
        if server.logs.is_empty() {
            logs = logs.child(div().child("Start BDS to see its console output."));
        } else {
            for line in server.logs.iter().rev().take(100).rev() {
                logs = logs.child(div().child(line.clone()));
            }
        }
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("Console"))
                .child(logs)
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.console).disabled(!server.running)),
                        )
                        .child(
                            Button::new("send-command")
                                .label("Send command")
                                .disabled(!server.running || self.busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let text = this.console.read(cx).value().to_string();
                                    this.console
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    this.submit(Request::Console(id, text), "Sending command…", cx);
                                })),
                        ),
                ),
        );
        body.into_any_element()
    }
    fn library_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(heading(
                "Marketplace add-ons",
                "Download add-ons you own, then install them in a stopped server.",
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("refresh-library")
                            .primary()
                            .label("Refresh library")
                            .disabled(self.busy || self.data.account.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit(
                                    Request::Library,
                                    "Reading your Marketplace entitlements…",
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("free-sample")
                            .label("Try a free add-on")
                            .disabled(self.busy || self.data.account.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit(Request::Sample, "Loading the free add-on…", cx)
                            })),
                    ),
            );
        if self.data.account.is_none() {
            body = body.child(
                div()
                    .text_color(rgb(MUTED))
                    .child("Connect your Microsoft account to load your owned add-ons."),
            );
        }
        if self.data.library.is_empty() {
            body = body.child(
                div()
                    .py_5()
                    .text_color(rgb(MUTED))
                    .child("Your add-ons will appear after you refresh the library."),
            );
        }
        if let Some(sample) = self.data.sample.clone() {
            let id = sample.id;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .p_4()
                    .bg(rgb(0xe5f0f1))
                    .rounded_lg()
                    .child(
                        div()
                            .flex_1()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(sample.name))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("Free add-on from Jigarbov Productions"),
                            ),
                    )
                    .child(
                        Button::new("claim-sample")
                            .label(if sample.owned {
                                "Already owned"
                            } else {
                                "Add to my library for free"
                            })
                            .disabled(sample.owned || self.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(Request::Claim(id), "Claiming the free add-on…", cx)
                            })),
                    ),
            );
        }
        for addon in self.data.library.clone() {
            let id = addon.id;
            let downloaded = self.data.downloaded.contains(&id);
            let server = self
                .selected
                .and_then(|id| self.data.servers.iter().find(|s| s.config.id == id))
                .cloned();
            let mut row = div()
                .flex()
                .items_center()
                .gap_4()
                .p_4()
                .bg(rgb(0xffffff))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .child(
                    div()
                        .flex_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(addon.name))
                        .child(div().mt_1().text_sm().text_color(rgb(MUTED)).child(format!(
                            "{}  |  {} packs  |  {}",
                            addon.creator,
                            addon.packs.len(),
                            if downloaded { "Downloaded" } else { "Owned" }
                        ))),
                );
            if !downloaded {
                row = row.child(
                    Button::new(SharedString::from(format!("download-{id}")))
                        .label("Download")
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.submit(
                                Request::Download(id),
                                "Downloading and decrypting your add-on…",
                                cx,
                            )
                        })),
                );
            } else if let Some(server) = server {
                let server_id = server.config.id;
                let installed = server.config.addons.iter().any(|a| a.id == id);
                row = row.child(
                    Button::new(SharedString::from(format!("install-{id}")))
                        .label(if installed {
                            "Installed"
                        } else {
                            "Install in selected server"
                        })
                        .disabled(
                            self.busy
                                || server.running
                                || installed
                                || server.config.version.is_none(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.submit(
                                Request::Install(server_id, id),
                                "Installing add-on packs…",
                                cx,
                            )
                        })),
                );
            }
            body = body.child(row);
        }
        body.child(div().text_sm().text_color(rgb(MUTED)).child("Paid purchases are not available here. Buy add-ons in Minecraft, then refresh this library.")).into_any_element()
    }
    fn account_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div().flex().flex_col().gap_5().child(heading(
            "Microsoft account",
            "Connect the account that owns your Marketplace add-ons.",
        ));
        if let Some(account) = &self.data.account {
            body = body
                .child(
                    div()
                        .text_lg()
                        .child(format!("Connected as {}", account.gamertag)),
                )
                .child(
                    Button::new("logout")
                        .label("Disconnect account")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.submit(Request::Logout, "Disconnecting account…", cx)
                        })),
                );
        } else {
            body = body.child(
                Button::new("login")
                    .primary()
                    .label("Connect Microsoft account")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.submit(Request::Login, "Starting Microsoft sign-in…", cx)
                    })),
            );
        }
        if let Some((url, code)) = self.device.clone() {
            let link = url.clone();
            let copy = code.clone();
            body = body.child(
                div()
                    .p_5()
                    .bg(rgb(0xffffff))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(LINE))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("Open {url} and enter this code:")),
                    )
                    .child(
                        div()
                            .my_4()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(code),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("open-login")
                                    .primary()
                                    .label("Open Microsoft sign-in")
                                    .on_click(move |_, _, _| {
                                        let _ = open::that(&link);
                                    }),
                            )
                            .child(Button::new("copy-code").label("Copy code").on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                },
                            ))
                            .child(Button::new("cancel-login").label("Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.worker.cancel.store(true, Ordering::Relaxed);
                                    this.message = "Canceling sign-in…".into();
                                    cx.notify();
                                }),
                            )),
                    ),
            );
        }
        body.child(div().text_sm().text_color(rgb(MUTED)).child("Account credentials stay in your OS keychain. Marketplace downloads stay on this computer.")).into_any_element()
    }
}
fn label(text: &str) -> Div {
    div()
        .mb_2()
        .text_sm()
        .text_color(rgb(MUTED))
        .child(text.to_string())
}
fn heading(title: &str, description: &str) -> Div {
    div()
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .child(
            div()
                .mt_2()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(description.to_string()),
        )
}
impl Render for Foundry {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut navigation = div()
            .w(px(192.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(0xffffff))
            .border_r_1()
            .border_color(rgb(LINE))
            .p_4()
            .flex()
            .flex_col()
            .gap_2();
        for (page, name) in [
            (Page::Servers, "Servers"),
            (Page::Library, "Marketplace add-ons"),
            (Page::Account, "Microsoft account"),
        ] {
            navigation = navigation.child(
                Button::new(name)
                    .ghost()
                    .when(self.page == page, |button| button.primary())
                    .label(name)
                    .w_full()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    })),
            );
        }
        navigation = navigation
            .child(div().flex_1())
            .child(
                div().text_xs().text_color(rgb(MUTED)).child(
                    self.data
                        .account
                        .as_ref()
                        .map(|a| a.gamertag.clone())
                        .unwrap_or("No account connected".into()),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Bedrock Foundry 0.1.0"),
            );
        let content = match self.page {
            Page::Servers => self.server_view(cx),
            Page::Library => self.library_view(cx),
            Page::Account => self.account_view(cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(CANVAS))
            .text_color(rgb(INK))
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(84.))
                    .px_6()
                    .gap_4()
                    .bg(rgb(INK))
                    .text_color(rgb(0xffffff))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().w_8().h_2().bg(rgb(0x6ac0c0)))
                            .child(div().w_5().h_2().bg(rgb(0xb5d8ec)))
                            .child(div().w_8().h_2().bg(rgb(0x6ac0c0))),
                    )
                    .child(
                        div()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Bedrock Foundry"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .text_color(rgb(0xc1d0dd))
                                    .child("Set up Bedrock servers with your Marketplace add-ons."),
                            ),
                    ),
            )
            .child(
                div().flex().flex_1().min_h_0().child(navigation).child(
                    div()
                        .id("main-content")
                        .flex_1()
                        .min_w_0()
                        .overflow_y_scroll()
                        .p_6()
                        .child(content),
                ),
            )
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(if self.error { 0xffeded } else { 0xffffff }))
                    .text_color(rgb(if self.error { 0x9b2630 } else { MUTED }))
                    .child(format!(
                        "{}{}",
                        if self.busy { "Working: " } else { "" },
                        self.message
                    )),
            )
    }
}
pub fn launch(root: Option<PathBuf>) -> Result<()> {
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        cx.global_mut::<Theme>().colors.primary = rgb(0x315fd7).into();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let viewport = cx
            .primary_display()
            .map(|display| display.bounds().size)
            .unwrap_or(size(px(1280.), px(900.)));
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(
                        px(1180.).min(viewport.width - px(40.)),
                        px(850.).min(viewport.height - px(40.)),
                    ),
                    cx,
                ))),
                window_min_size: Some(size(px(900.), px(650.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Bedrock Foundry".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let foundry = cx.new(|cx| Foundry::new(root.clone(), window, cx));
                cx.new(|cx| Root::new(foundry, window, cx))
            },
        ) {
            eprintln!("Cannot open the desktop window: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}
