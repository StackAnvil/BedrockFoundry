//! Bedrock Foundry desktop interface.

mod account;
mod library;
mod market;
mod servers;
mod style;
mod thumbs;
mod worker;

use bedrock_foundry::marketplace::{CatalogQuery, PurchaseQuote};
use gpui::{
    AnyElement, App, AppContext, Application, Bounds, ClipboardItem, Div, Entity, FontWeight,
    ObjectFit, PathPromptOptions, ScrollHandle, SharedString, Stateful, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, rgba,
    size,
};
use gpui_component::{
    ActiveTheme, Disableable, Root, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState},
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::Duration,
};
use style::{
    CONSOLE, CONSOLE_TEXT, DEEPSLATE, DEEPSLATE_MUTED, DEEPSLATE_RAISED, DEEPSLATE_TEXT, INK, LINE,
    MOSS, MOSS_SOFT, MUTED, RED, STONE, SURFACE, Tone, callout, dot, field, mark, minecoins, muted,
    panel, pill, section, title,
};
use thumbs::Thumbs;
use uuid::Uuid;
use worker::{Event, Op, Outcome, Request, ServerView, Snapshot, Worker};

const RAIL_WIDTH: f32 = 248.;
const DEFAULT_PORT: u16 = 19132;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Server,
    NewServer,
    Marketplace,
    Library,
    Account,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerTab {
    Console,
    Addons,
    Settings,
    Backups,
}

/// The purchase dialog. Each state names what has, or has not, been sent.
#[derive(Clone)]
enum Purchase {
    /// The listing and balance are being read. Nothing has been sent.
    Checking {
        id: Uuid,
        title: String,
    },
    Review(Box<PurchaseQuote>),
    /// The confirmed purchase was handed to the worker. It cannot be canceled.
    Submitting(Box<PurchaseQuote>),
    Finished {
        id: Uuid,
        title: String,
        ready: bool,
    },
    Failed {
        title: String,
        message: String,
        submitted: bool,
    },
}

struct Notice {
    text: String,
    tone: Tone,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "These flags track independent UI states and requests"
)]
struct Foundry {
    worker: Worker,
    data: Snapshot,
    loaded: bool,
    fatal: Option<String>,
    page: Page,
    selected: Option<Uuid>,
    pending_select: Option<Uuid>,
    server_tab: ServerTab,
    pending: VecDeque<(Op, SharedString)>,
    notice: Option<Notice>,
    device: Option<(String, String)>,
    library_loaded: bool,
    catalog_requested: bool,
    /// The last failed catalog or library read, shown on its page.
    catalog_error: Option<String>,
    library_error: Option<String>,
    new_name: Entity<InputState>,
    new_port: Entity<InputState>,
    name: Entity<InputState>,
    port: Entity<InputState>,
    players: Entity<InputState>,
    console: Entity<InputState>,
    search: Entity<InputState>,
    mode: String,
    difficulty: String,
    resync_form: bool,
    addons_only: bool,
    detail: Option<Uuid>,
    install_open: Option<Uuid>,
    purchase: Option<Purchase>,
    last_backup: HashMap<Uuid, PathBuf>,
    thumbs: Thumbs,
    console_scroll: ScrollHandle,
    console_signature: (usize, Option<String>),
    clock: i64,
    _subscriptions: Vec<Subscription>,
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

impl Foundry {
    fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let worker = Worker::start(root);
        let new_name = cx.new(|cx| InputState::new(window, cx).default_value("My Bedrock server"));
        let new_port =
            cx.new(|cx| InputState::new(window, cx).default_value(DEFAULT_PORT.to_string()));
        let name = cx.new(|cx| InputState::new(window, cx));
        let port = cx.new(|cx| InputState::new(window, cx));
        let players = cx.new(|cx| InputState::new(window, cx));
        let console = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a command, such as list or say Hello")
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search the Marketplace by name or creator")
        });
        let subscriptions = vec![
            cx.subscribe_in(&console, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.send_command(window, cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.browse_from_start(cx);
                }
            }),
            cx.subscribe_in(&new_name, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.create_server(cx);
                }
            }),
        ];
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this.update_in(cx, Self::receive).is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut foundry = Self {
            worker,
            data: Snapshot::default(),
            loaded: false,
            fatal: None,
            page: Page::Server,
            selected: None,
            pending_select: None,
            server_tab: ServerTab::Console,
            pending: VecDeque::new(),
            notice: None,
            device: None,
            library_loaded: false,
            catalog_requested: false,
            catalog_error: None,
            library_error: None,
            new_name,
            new_port,
            name,
            port,
            players,
            console,
            search,
            mode: "survival".into(),
            difficulty: "normal".into(),
            resync_form: false,
            addons_only: true,
            detail: None,
            install_open: None,
            purchase: None,
            last_backup: HashMap::new(),
            thumbs: Thumbs::start(),
            console_scroll: ScrollHandle::new(),
            console_signature: (0, None),
            clock: now(),
            _subscriptions: subscriptions,
        };
        foundry.enqueue(Request::Restore, "Checking for a saved Microsoft account…");
        foundry
    }

    fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    fn working_on(&self, op: Op) -> bool {
        self.pending.iter().any(|(pending, _)| *pending == op)
    }

    fn connected(&self) -> bool {
        self.data.account.is_some()
    }

    /// Sends a user action. Actions wait for each other because the worker
    /// runs them in order, so a second click is ignored while one runs.
    fn submit(&mut self, request: Request, activity: &str, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.notice = None;
        self.enqueue(request, activity);
        cx.notify();
    }

    /// Queues work behind any running action without clearing the status.
    fn enqueue(&mut self, request: Request, activity: &str) {
        let Some(op) = request.op() else { return };
        if self.worker.tx.send(request).is_err() {
            self.notice = Some(Notice {
                text: "The background worker stopped. Restart Bedrock Foundry.".into(),
                tone: Tone::Bad,
            });
        } else {
            self.pending
                .push_back((op, SharedString::from(activity.to_owned())));
        }
    }

    fn fail(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(Notice {
            text: text.into(),
            tone: Tone::Bad,
        });
        cx.notify();
    }

    fn receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = self.thumbs.poll();
        while let Ok(event) = self.worker.rx.try_recv() {
            changed = true;
            match event {
                Event::Snapshot(snapshot) => self.apply_snapshot(*snapshot, window, cx),
                Event::Done(op, result) => self.finish(op, result),
                Event::DeviceCode(url, code) => self.device = Some((url, code)),
                Event::Processes(processes) => self.worker.processes = Some(processes),
                Event::Warning(text) => {
                    self.notice = Some(Notice {
                        text,
                        tone: Tone::Bad,
                    });
                }
                Event::Fatal(message) => {
                    self.fatal = Some(message);
                    self.pending.clear();
                }
            }
        }
        // The purchase review shows a countdown, so redraw once per second.
        if matches!(self.purchase, Some(Purchase::Review(_))) && self.clock != now() {
            self.clock = now();
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        self.loaded = true;
        self.data = snapshot;
        if let Some(id) = self.pending_select.take() {
            self.select(id, window, cx);
        } else if self
            .selected
            .is_none_or(|id| self.data.server(id).is_none())
        {
            match self.data.servers.first().map(|s| s.config.id) {
                Some(id) => self.select(id, window, cx),
                None => self.selected = None,
            }
        } else if self.resync_form
            && let Some(id) = self.selected
        {
            self.select(id, window, cx);
        }
        self.resync_form = false;
        if self.page == Page::Server && self.selected.is_none() {
            self.page = Page::NewServer;
        }
        // Follow new console output.
        if let Some(server) = self.selected.and_then(|id| self.data.server(id)) {
            let signature = (server.logs.len(), server.logs.last().cloned());
            if signature != self.console_signature {
                self.console_signature = signature;
                self.console_scroll.scroll_to_bottom();
            }
        }
        // The review opens once the worker has a quote for the listing.
        match &self.purchase {
            Some(Purchase::Checking { id, .. }) => {
                if let Some(quote) = self.data.quote.clone().filter(|q| q.item.id == *id) {
                    self.clock = now();
                    self.purchase = Some(Purchase::Review(Box::new(quote)));
                }
            }
            Some(Purchase::Review(quote))
                if self.data.quote.as_ref().map(|q| q.id) != Some(quote.id) =>
            {
                self.purchase = Some(Purchase::Failed {
                    title: quote.item.title.clone(),
                    message: "This review is no longer current. Nothing was charged. Check the price again to continue.".into(),
                    submitted: false,
                });
            }
            _ => {}
        }
    }

    fn finish(&mut self, op: Op, result: Result<Outcome, worker::Failure>) {
        if let Some(index) = self.pending.iter().position(|(pending, _)| *pending == op) {
            self.pending.remove(index);
        }
        let failure = match result {
            Err(failure) => failure,
            Ok(outcome) => {
                match op {
                    Op::Browse => self.catalog_error = None,
                    Op::Library | Op::Claim => self.library_error = None,
                    Op::Restore
                    | Op::Login
                    | Op::Logout
                    | Op::Balance
                    | Op::Quote
                    | Op::Buy
                    | Op::CancelPurchase
                    | Op::Sample
                    | Op::Download
                    | Op::Create
                    | Op::Provision
                    | Op::Import
                    | Op::Install
                    | Op::Start
                    | Op::Stop
                    | Op::Console
                    | Op::Save
                    | Op::Backup => {}
                }
                self.succeed(op, outcome);
                return;
            }
        };
        let message = failure.message;
        match op {
            Op::Browse => self.catalog_error = Some(message.clone()),
            Op::Library => self.library_error = Some(message.clone()),
            Op::Restore
            | Op::Login
            | Op::Logout
            | Op::Balance
            | Op::Quote
            | Op::Buy
            | Op::CancelPurchase
            | Op::Sample
            | Op::Claim
            | Op::Download
            | Op::Create
            | Op::Provision
            | Op::Import
            | Op::Install
            | Op::Start
            | Op::Stop
            | Op::Console
            | Op::Save
            | Op::Backup => {}
        }
        match (op, self.purchase.take()) {
            (Op::Quote, Some(Purchase::Checking { title, .. })) => {
                self.purchase = Some(Purchase::Failed {
                    title,
                    message,
                    submitted: false,
                });
            }
            (Op::Buy, Some(Purchase::Submitting(quote))) => {
                self.purchase = Some(Purchase::Failed {
                    title: quote.item.title.clone(),
                    message,
                    submitted: failure.unknown_purchase,
                });
            }
            (_, purchase) => {
                self.purchase = purchase;
                if op == Op::Login {
                    self.device = None;
                }
                self.notice = Some(Notice {
                    text: message,
                    tone: Tone::Bad,
                });
            }
        }
    }

    fn succeed(&mut self, op: Op, outcome: Outcome) {
        match outcome {
            Outcome::Quiet => {}
            Outcome::Message(text) => {
                self.notice = Some(Notice {
                    text,
                    tone: Tone::Good,
                });
            }
            Outcome::Connected(gamertag) => {
                self.device = None;
                self.library_loaded = false;
                self.catalog_requested = false;
                if op == Op::Login {
                    self.notice = Some(Notice {
                        text: format!("Connected as {gamertag}"),
                        tone: Tone::Good,
                    });
                }
                self.enqueue(Request::Library, "Reading your Marketplace library…");
                if self.page == Page::Marketplace {
                    self.load_catalog();
                }
            }
            Outcome::Created(id) => {
                self.pending_select = Some(id);
                self.page = Page::Server;
                self.server_tab = ServerTab::Console;
                self.notice = Some(Notice {
                    text: "Server created. Install Bedrock Dedicated Server next.".into(),
                    tone: Tone::Good,
                });
            }
            Outcome::BackupSaved(path) => {
                if let Some(id) = self.selected {
                    self.last_backup.insert(id, path);
                }
                self.notice = Some(Notice {
                    text: "Backup saved".into(),
                    tone: Tone::Good,
                });
            }
            Outcome::Purchased(ready) => {
                if let Some(Purchase::Submitting(quote)) = self.purchase.take() {
                    self.purchase = Some(Purchase::Finished {
                        id: quote.item.id,
                        title: quote.item.title.clone(),
                        ready,
                    });
                }
                self.library_loaded = ready;
            }
        }
        match op {
            Op::Library | Op::Claim => self.library_loaded = true,
            Op::Logout => {
                self.catalog_error = None;
                self.library_error = None;
                self.library_loaded = false;
                self.catalog_requested = false;
                self.detail = None;
                self.purchase = None;
            }
            Op::Restore
            | Op::Login
            | Op::Browse
            | Op::Balance
            | Op::Quote
            | Op::Buy
            | Op::CancelPurchase
            | Op::Sample
            | Op::Download
            | Op::Create
            | Op::Provision
            | Op::Import
            | Op::Install
            | Op::Start
            | Op::Stop
            | Op::Console
            | Op::Save
            | Op::Backup => {}
        }
    }

    fn select(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != Some(id) {
            self.server_tab = ServerTab::Console;
            self.console_signature = (0, None);
            self.console_scroll.scroll_to_bottom();
        }
        self.selected = Some(id);
        if let Some(server) = self.data.server(id).cloned() {
            self.name.update(cx, |input, cx| {
                input.set_value(server.config.name.clone(), window, cx);
            });
            self.port.update(cx, |input, cx| {
                input.set_value(server.config.port.to_string(), window, cx);
            });
            self.players.update(cx, |input, cx| {
                input.set_value(server.config.max_players.to_string(), window, cx);
            });
            self.mode.clone_from(&server.config.gamemode);
            self.difficulty.clone_from(&server.config.difficulty);
        }
        cx.notify();
    }

    fn open(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.detail = None;
        match page {
            Page::Marketplace => self.load_catalog(),
            Page::Library => {
                if self.connected() && !self.library_loaded && !self.working_on(Op::Library) {
                    self.enqueue(Request::Library, "Reading your Marketplace library…");
                }
            }
            Page::NewServer => {
                if !self.data.servers.is_empty() {
                    let name = format!("Bedrock server {}", self.data.servers.len() + 1);
                    self.new_name
                        .update(cx, |input, cx| input.set_value(name, window, cx));
                }
                let port = self.free_port();
                self.new_port.update(cx, |input, cx| {
                    input.set_value(port.to_string(), window, cx);
                });
            }
            Page::Server | Page::Account => {}
        }
        cx.notify();
    }

    fn load_catalog(&mut self) {
        if self.connected()
            && self.data.catalog.is_none()
            && !self.catalog_requested
            && !self.working_on(Op::Browse)
        {
            self.catalog_requested = true;
            self.enqueue(
                Request::Browse(CatalogQuery {
                    addons_only: self.addons_only,
                    ..CatalogQuery::default()
                }),
                "Loading Marketplace listings…",
            );
        }
    }

    /// The first port pair that no server uses, starting at Bedrock's default.
    fn free_port(&self) -> u16 {
        let mut port = DEFAULT_PORT;
        while port < 65533
            && self.data.servers.iter().any(|s| {
                let used = s.config.port;
                used == port || used == port + 1 || used + 1 == port
            })
        {
            port += 2;
        }
        port
    }

    fn selected_server(&self) -> Option<ServerView> {
        self.selected.and_then(|id| self.data.server(id)).cloned()
    }

    /// A row of mutually exclusive choices with a visible selected state.
    fn segmented(
        id: &'static str,
        options: &[(&'static str, &'static str)],
        current: &str,
        disabled: bool,
        cx: &Context<Self>,
        on_pick: fn(&mut Self, &'static str, &mut Context<Self>),
    ) -> Div {
        let mut row = div()
            .flex()
            .p(px(3.))
            .gap(px(3.))
            .rounded(px(7.))
            .bg(rgb(0xe1e4de))
            .border_1()
            .border_color(rgb(LINE));
        for (value, label) in options.iter().copied() {
            let selected = current == value;
            row = row.child(
                div()
                    .id(SharedString::from(format!("{id}-{value}")))
                    .px_3()
                    .py_1()
                    .rounded(px(5.))
                    .text_sm()
                    .when(selected, |this| {
                        this.bg(rgb(SURFACE))
                            .text_color(rgb(INK))
                            .font_weight(FontWeight::MEDIUM)
                            .shadow_xs()
                    })
                    .when(!selected, |this| this.text_color(rgb(MUTED)))
                    .when(disabled, |this| this.opacity(0.55))
                    .when(!disabled && !selected, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.text_color(rgb(INK)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                on_pick(this, value, cx);
                                cx.notify();
                            }))
                    })
                    .child(label),
            );
        }
        row
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep sidebar layout and its navigation handlers together"
    )]
    fn rail(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let _ = window;
        let nav_item = |id: &'static str, label: String, active: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .text_sm()
                .cursor_pointer()
                .when(active, |this| {
                    this.bg(rgb(DEEPSLATE_RAISED))
                        .text_color(rgb(0xffffff))
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!active, |this| {
                    this.text_color(rgb(DEEPSLATE_TEXT))
                        .hover(|style| style.bg(rgb(0x323a3d)))
                })
                .child(label)
        };
        let mut servers = div().flex().flex_col().gap_0p5();
        for server in &self.data.servers {
            let id = server.config.id;
            let active = self.page == Page::Server && self.selected == Some(id);
            let (state, tone) = servers::status(server);
            servers = servers.child(
                div()
                    .id(SharedString::from(format!("rail-server-{id}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(active, |this| this.bg(rgb(DEEPSLATE_RAISED)))
                    .when(!active, |this| this.hover(|style| style.bg(rgb(0x323a3d))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(id, window, cx);
                        this.open(Page::Server, window, cx);
                    }))
                    .child(dot(tone))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_sm()
                                    .text_color(rgb(if active { 0xffffff } else { DEEPSLATE_TEXT }))
                                    .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                                    .child(server.config.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(DEEPSLATE_MUTED))
                                    .child(format!("{state} · Port {}", server.config.port)),
                            ),
                    ),
            );
        }
        if self.loaded && self.data.servers.is_empty() {
            servers = servers.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(DEEPSLATE_MUTED))
                    .child("No servers yet"),
            );
        }
        let account = match &self.data.account {
            Some(account) => account.gamertag.clone(),
            None if self.working_on(Op::Restore) => "Checking account…".into(),
            None => "Connect Microsoft account".into(),
        };
        div()
            .w(px(RAIL_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(DEEPSLATE))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_5()
                    .pt_5()
                    .pb_4()
                    .child(mark(1.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child("Bedrock Foundry"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .w_full()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(rgb(DEEPSLATE_MUTED))
                                    .child("Set up Bedrock servers with your Marketplace add-ons."),
                            ),
                    ),
            )
            .child(
                div()
                    .id("rail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px_3()
                            .pt_3()
                            .pb_1()
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(DEEPSLATE_MUTED))
                                    .child("Servers"),
                            )
                            .child(
                                div()
                                    .id("rail-new-server")
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(4.))
                                    .text_xs()
                                    .cursor_pointer()
                                    .text_color(rgb(if self.page == Page::NewServer {
                                        0xffffff
                                    } else {
                                        DEEPSLATE_TEXT
                                    }))
                                    .bg(rgb(if self.page == Page::NewServer {
                                        DEEPSLATE_RAISED
                                    } else {
                                        DEEPSLATE
                                    }))
                                    .hover(|style| style.bg(rgb(0x323a3d)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open(Page::NewServer, window, cx);
                                    }))
                                    .child("+ New server"),
                            ),
                    )
                    .child(servers)
                    .child(div().h_px().mx_3().my_3().bg(rgb(0x41494c)))
                    .child(
                        nav_item(
                            "rail-marketplace",
                            "Marketplace".into(),
                            self.page == Page::Marketplace,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open(Page::Marketplace, window, cx);
                        })),
                    )
                    .child(
                        nav_item(
                            "rail-library",
                            match self.library_count() {
                                0 => "My add-ons".to_string(),
                                count => format!("My add-ons ({count})"),
                            },
                            self.page == Page::Library,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open(Page::Library, window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(0x3a4245))
                    .child(
                        div()
                            .id("rail-account")
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(self.page == Page::Account, |this| {
                                this.bg(rgb(DEEPSLATE_RAISED))
                            })
                            .hover(|style| style.bg(rgb(0x323a3d)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Page::Account, window, cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(DEEPSLATE_MUTED))
                                    .child("Microsoft account"),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(rgb(if self.connected() {
                                        0xffffff
                                    } else {
                                        DEEPSLATE_TEXT
                                    }))
                                    .child(account),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .pt_2()
                            .text_xs()
                            .text_color(rgb(0x6c7773))
                            .child(concat!("Version ", env!("CARGO_PKG_VERSION"))),
                    ),
            )
    }

    /// Owned add-ons plus downloaded ones that the library has not listed.
    fn library_count(&self) -> usize {
        let mut count = self.data.library.len();
        for addon in &self.data.downloaded {
            if !self.data.library.iter().any(|owned| owned.id == addon.id) {
                count += 1;
            }
        }
        count
    }

    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let bar = div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(40.))
            .px_5()
            .border_t_1()
            .border_color(rgb(LINE))
            .text_sm();
        if let Some((_, activity)) = self.pending.front() {
            let more = self.pending.len() - 1;
            return bar
                .bg(rgb(SURFACE))
                .child(dot(Tone::Busy))
                .child(div().flex_1().truncate().child(activity.clone()))
                .when(more > 0, |this| {
                    this.child(muted(format!("{more} more queued")))
                });
        }
        match &self.notice {
            Some(notice) => {
                let (fg, bg) = notice.tone.colors();
                bar.bg(rgb(bg))
                    .text_color(rgb(if notice.tone == Tone::Bad { fg } else { INK }))
                    .child(dot(notice.tone))
                    .child(div().flex_1().truncate().child(notice.text.clone()))
                    .child(
                        Button::new("dismiss-notice")
                            .ghost()
                            .xsmall()
                            .label("Dismiss")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            })),
                    )
            }
            None => bar.bg(rgb(SURFACE)).child(muted("Ready")),
        }
    }
}

/// The page frame: a title block with optional actions, then the content.
fn page(heading: impl IntoElement, actions: Option<AnyElement>, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .child(
            div()
                .flex()
                .items_start()
                .gap_4()
                .child(div().flex_1().min_w_0().child(heading))
                .when_some(actions, ParentElement::child),
        )
        .child(content)
}

fn heading(text: impl Into<SharedString>, description: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(title(text))
        .child(muted(description).max_w(px(640.)))
}

impl Render for Foundry {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = if let Some(message) = self.fatal.clone() {
            page(
                heading(
                    "Bedrock Foundry can't open its data folder",
                    "Fix the problem below, then restart Bedrock Foundry. Your servers and downloads were not changed.",
                ),
                None,
                callout(Tone::Bad, message),
            )
            .into_any_element()
        } else if !self.loaded {
            div()
                .pt_10()
                .child(muted("Opening your servers and downloads…"))
                .into_any_element()
        } else {
            match self.page {
                Page::Server => self.server_page(window, cx),
                Page::NewServer => self.new_server_page(cx),
                Page::Marketplace => self.marketplace_page(window, cx),
                Page::Library => self.library_page(cx),
                Page::Account => self.account_page(cx),
            }
        };
        let content_width =
            (f32::from(window.viewport_size().width) - RAIL_WIDTH - 64.).clamp(560., 1080.);
        let detail = (self.page == Page::Marketplace)
            .then(|| self.listing_sheet(cx))
            .flatten();
        let dialog = self.purchase_dialog(cx);
        div()
            .size_full()
            .relative()
            .flex()
            .bg(rgb(STONE))
            .text_color(rgb(INK))
            .text_sm()
            .child(self.rail(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("main-content")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_8()
                            .pt_8()
                            .pb_10()
                            // A definite width lets wrapped text measure correctly.
                            .child(div().w(px(content_width)).child(content)),
                    )
                    .child(self.status_bar(cx)),
            )
            .when_some(detail, ParentElement::child)
            .when_some(dialog, ParentElement::child)
    }
}

/// A full-window scrim that blocks the interface behind a dialog or sheet.
fn scrim(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(rgba(0x1f252799))
        .occlude()
}

pub fn launch(root: Option<PathBuf>) {
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        {
            // The default Linux mono family is often missing, which would
            // misalign BDS console output. Use the first installed one.
            if cfg!(target_os = "linux") {
                let installed = cx.text_system().all_font_names();
                if let Some(family) = [
                    "DejaVu Sans Mono",
                    "Noto Sans Mono",
                    "Liberation Mono",
                    "Ubuntu Mono",
                    "Adwaita Mono",
                ]
                .into_iter()
                .find(|family| installed.iter().any(|name| name == family))
                {
                    cx.global_mut::<Theme>().mono_font_family = family.into();
                }
            }
            let theme = cx.global_mut::<Theme>();
            theme.radius = px(6.);
            theme.radius_lg = px(8.);
            let colors = &mut theme.colors;
            colors.primary = rgb(MOSS).into();
            colors.primary_hover = rgb(0x285c40).into();
            colors.primary_active = rgb(0x224f37).into();
            colors.primary_foreground = rgb(0xffffff).into();
            colors.ring = rgb(MOSS).into();
            colors.danger = rgb(RED).into();
            colors.danger_hover = rgb(0x963326).into();
            colors.danger_active = rgb(0x852d22).into();
            colors.background = rgb(SURFACE).into();
            colors.foreground = rgb(INK).into();
            colors.border = rgb(LINE).into();
            colors.input = rgb(0xc6ccc4).into();
            colors.muted_foreground = rgb(MUTED).into();
            colors.secondary = rgb(0xe4e7e1).into();
            colors.secondary_hover = rgb(0xd9ddd5).into();
            colors.secondary_active = rgb(0xcfd4cb).into();
            colors.secondary_foreground = rgb(INK).into();
            colors.switch = rgb(0xbfc5bd).into();
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let viewport = cx
            .primary_display()
            .map_or(size(px(1280.), px(900.)), |display| display.bounds().size);
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(
                        px(1240.).min(viewport.width - px(40.)),
                        px(860.).min(viewport.height - px(40.)),
                    ),
                    cx,
                ))),
                window_min_size: Some(size(px(940.), px(640.))),
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
}
