//! Server setup, process control, console, add-ons, settings, and backups.

use super::*;
use anyhow::Result;
use bedrock_foundry::server::ServerConfig;
use gpui_component::{input::Input, switch::Switch};

const MODES: [(&str, &str); 3] = [
    ("survival", "Survival"),
    ("creative", "Creative"),
    ("adventure", "Adventure"),
];
const DIFFICULTIES: [(&str, &str); 4] = [
    ("peaceful", "Peaceful"),
    ("easy", "Easy"),
    ("normal", "Normal"),
    ("hard", "Hard"),
];

/// The process state shown wherever a server is listed.
pub fn status(server: &ServerView) -> (&'static str, Tone) {
    if server.config.version.is_none() {
        ("Needs setup", Tone::Neutral)
    } else if server.ready {
        ("Running", Tone::Good)
    } else if server.running {
        ("Starting", Tone::Busy)
    } else {
        ("Stopped", Tone::Neutral)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Done,
    Current,
    Later,
}

/// One step of the setup sequence. The current step can hold its controls.
fn step(number: u8, heading: &str, text: &str, stage: Stage, body: Option<AnyElement>) -> Div {
    let (fg, bg) = match stage {
        Stage::Done => (0xffffff, MOSS),
        Stage::Current => (MOSS, MOSS_SOFT),
        Stage::Later => (MUTED, 0xe1e4de),
    };
    div()
        .flex()
        .gap_4()
        .child(
            div()
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .bg(rgb(bg))
                .text_color(rgb(fg))
                .child(if stage == Stage::Done {
                    "✓".to_string()
                } else {
                    number.to_string()
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .pt(px(3.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .when(stage != Stage::Current, |this| this.text_color(rgb(MUTED)))
                        .when(stage == Stage::Current, Styled::text_base)
                        .child(heading.to_owned()),
                )
                .child(muted(text.to_owned()))
                .when_some(body, |this, body| this.child(div().pt_3().child(body))),
        )
}

/// The three setup steps. `create` is the first step's stage.
fn setup_steps(create: Stage, body: Option<AnyElement>) -> Div {
    let (create_body, install_body) = match create {
        Stage::Current => (body, None),
        Stage::Done | Stage::Later => (None, body),
    };
    div()
        .flex()
        .flex_col()
        .gap_5()
        .child(step(
            1,
            "Create the server",
            if create == Stage::Done {
                "Name and port are set."
            } else {
                "Choose a name and port. Nothing is downloaded yet."
            },
            create,
            create_body,
        ))
        .child(step(
            2,
            "Install Bedrock Dedicated Server",
            "Download the official server software from Minecraft, or import a ZIP you already have.",
            if create == Stage::Done {
                Stage::Current
            } else {
                Stage::Later
            },
            install_body,
        ))
        .child(step(
            3,
            "Add your add-ons and start",
            "Connect your Microsoft account, download add-ons you own, and install them before you start the server.",
            Stage::Later,
            None,
        ))
}

impl Foundry {
    pub(super) fn create_server(&mut self, cx: &mut Context<Self>) {
        let name = self.new_name.read(cx).value().trim().to_string();
        match self.new_port.read(cx).value().trim().parse::<u16>() {
            Ok(port) => self.submit(Request::Create(name, port), "Creating server…", cx),
            Err(_) => self.fail("Enter a port number from 1024 to 65534.", cx),
        }
    }

    pub(super) fn send_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server) = self.selected_server() else {
            return;
        };
        let text = self.console.read(cx).value().trim().to_string();
        if text.is_empty() || !server.running || self.busy() {
            return;
        }
        self.console
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.submit(
            Request::Console(server.config.id, text),
            "Sending command…",
            cx,
        );
    }

    pub(super) fn new_server_page(&self, cx: &Context<Self>) -> AnyElement {
        let first = self.data.servers.is_empty();
        let port = self.new_port.read(cx).value().trim().parse::<u16>().ok();
        let form = panel()
            .p_5()
            .flex()
            .flex_col()
            .gap_5()
            .w_full()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        field(
                            "Server name",
                            Input::new(&self.new_name),
                            Some("Players see this name in their server list.".into()),
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            "Port",
                            Input::new(&self.new_port),
                            Some(match port {
                                Some(port) if port < 65535 => {
                                    format!("Also uses {}.", port + 1)
                                }
                                _ => "Use 1024 to 65534.".into(),
                            }),
                        )
                        .w(px(150.)),
                    ),
            )
            .child(
                div().flex().child(
                    Button::new("create-server")
                        .primary()
                        .label(if self.working_on(Op::Create) {
                            "Creating…"
                        } else {
                            "Create server"
                        })
                        .disabled(self.busy())
                        .on_click(cx.listener(|this, _, _, cx| this.create_server(cx))),
                ),
            );
        if !first {
            return page(
                heading(
                    "New server",
                    "Each server has its own world, settings, and add-ons. Give each one a different port.",
                ),
                None,
                form,
            )
            .into_any_element();
        }
        page(
            heading(
                "Set up your first Bedrock server",
                "Bedrock Foundry runs Bedrock Dedicated Server on this computer and installs the Marketplace add-ons you own.",
            ),
            None,
            setup_steps(Stage::Current, Some(form.into_any_element())),
        )
        .into_any_element()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep server navigation and its actions together"
    )]
    pub(super) fn server_page(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let Some(server) = self.selected_server() else {
            return self.new_server_page(cx);
        };
        let id = server.config.id;
        let installed = server.config.version.is_some();
        let (state, tone) = status(&server);
        let busy = self.busy();
        let mut actions = div().flex().items_center().gap_2();
        if installed {
            actions = actions.child(
                Button::new("open-folder")
                    .ghost()
                    .label("Open folder")
                    .on_click({
                        let folder = self.data.root.join("servers").join(id.to_string());
                        move |_, _, _| {
                            if let Err(error) = open::that(&folder) {
                                eprintln!("Could not open the server folder: {error}");
                            }
                        }
                    }),
            );
            actions = actions.child(if server.running {
                Button::new("stop-server")
                    .danger()
                    .label(if self.working_on(Op::Stop) {
                        "Stopping…"
                    } else {
                        "Stop server"
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit(
                            Request::Stop(id),
                            "Stopping server and saving the world…",
                            cx,
                        );
                    }))
            } else {
                Button::new("start-server")
                    .primary()
                    .label(if self.working_on(Op::Start) {
                        "Starting…"
                    } else {
                        "Start server"
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.server_tab = ServerTab::Console;
                        this.submit(Request::Start(id), "Starting Bedrock Dedicated Server…", cx);
                    }))
            });
        }
        let address = if server.config.public {
            "Other devices can join"
        } else {
            "This computer only"
        };
        let title_block = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(title(server.config.name.clone()))
                    .child(pill(state, tone)),
            )
            .child(muted(match &server.config.version {
                Some(version) => format!(
                    "Bedrock Dedicated Server {version} · Port {} · {address}",
                    server.config.port
                ),
                None => format!("Not installed yet · Port {}", server.config.port),
            }));
        if !installed {
            let card = self.install_card(id, cx).into_any_element();
            return page(title_block, None, setup_steps(Stage::Done, Some(card)))
                .into_any_element();
        }
        let addon_count = server.config.addons.len();
        let tabs = [
            (ServerTab::Console, "Console".to_string()),
            (
                ServerTab::Addons,
                if addon_count == 0 {
                    "Add-ons".to_string()
                } else {
                    format!("Add-ons ({addon_count})")
                },
            ),
            (ServerTab::Settings, "Settings".to_string()),
            (ServerTab::Backups, "Backups".to_string()),
        ];
        let mut strip = div().flex().gap_6().border_b_1().border_color(rgb(LINE));
        for (tab, label) in tabs {
            let active = self.server_tab == tab;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .pb_2()
                    .mb(px(-1.))
                    .border_b_2()
                    .border_color(rgb(if active { MOSS } else { STONE }))
                    .text_color(rgb(if active { INK } else { MUTED }))
                    .when(active, |this| this.font_weight(FontWeight::MEDIUM))
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(INK)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.server_tab = tab;
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        let body = match self.server_tab {
            ServerTab::Console => self.console_tab(&server, cx),
            ServerTab::Addons => self.addons_tab(&server, cx),
            ServerTab::Settings => self.settings_tab(&server, window, cx),
            ServerTab::Backups => self.backups_tab(&server, cx),
        };
        page(
            title_block,
            Some(actions.into_any_element()),
            div().flex().flex_col().gap_5().child(strip).child(body),
        )
        .into_any_element()
    }

    fn install_card(&self, id: Uuid, cx: &Context<Self>) -> Div {
        let busy = self.busy();
        let installing = self.working_on(Op::Provision) || self.working_on(Op::Import);
        panel()
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(muted(
                "Download and install gets the current release for this computer from Minecraft.",
            ))
            .when(installing, |this| {
                this.child(callout(
                    Tone::Busy,
                    "Installing. Large downloads can take a few minutes.",
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("provision")
                            .primary()
                            .label("Download and install")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(
                                    Request::Provision(id),
                                    "Downloading Bedrock Dedicated Server from Minecraft…",
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("import-bds")
                            .outline()
                            .label("Import a ZIP…")
                            .disabled(busy)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let picker = cx.prompt_for_paths(PathPromptOptions {
                                    files: true,
                                    directories: false,
                                    multiple: false,
                                    prompt: Some("Choose bedrock-server-VERSION.zip".into()),
                                });
                                cx.spawn(async move |this, cx| {
                                    let selected = picker.await;
                                    drop(this.update(cx, |this, cx| match selected {
                                        Ok(Ok(Some(paths))) => {
                                            if let Some(path) = paths.into_iter().next() {
                                                this.submit(
                                                    Request::Import(id, path),
                                                    "Importing Bedrock Dedicated Server…",
                                                    cx,
                                                );
                                            }
                                        }
                                        Ok(Ok(None)) => {}
                                        Ok(Err(error)) => this.fail(format!("{error:#}"), cx),
                                        Err(error) => this.fail(error.to_string(), cx),
                                    }));
                                })
                                .detach();
                            })),
                    ),
            )
            .child(div().text_xs().text_color(rgb(MUTED)).child(
                "Import accepts the official bedrock-server-VERSION.zip for this operating system.",
            ))
    }

    fn console_tab(&self, server: &ServerView, cx: &Context<Self>) -> AnyElement {
        let id = server.config.id;
        let mono = cx.theme().mono_font_family.clone();
        let mut log = div()
            .id(SharedString::from(format!("console-{id}")))
            .track_scroll(&self.console_scroll)
            .h(px(380.))
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .rounded_t(px(8.))
            .bg(rgb(CONSOLE))
            .text_color(rgb(CONSOLE_TEXT))
            .font_family(mono)
            .text_xs()
            .line_height(px(18.));
        if server.logs.is_empty() {
            log = log.child(div().text_color(rgb(0x8f9a95)).child(if server.running {
                "Waiting for the server to write output…"
            } else {
                "The server is stopped. Start it to see its output here."
            }));
        } else {
            for line in server.logs.iter().rev().take(400).rev() {
                log = log.child(div().child(line.clone()));
            }
        }
        let can_send = server.running && !self.busy();
        div()
            .flex()
            .flex_col()
            .child(log)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded_b(px(8.))
                    .bg(rgb(0x2d3437))
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.console).disabled(!server.running)),
                    )
                    .child(
                        Button::new("send-command")
                            .primary()
                            .label("Send")
                            .disabled(!can_send)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_command(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(if server.running {
                        "Press Enter to send. Commands run as the server console."
                    } else {
                        "Commands are available while the server runs."
                    }),
            )
            .into_any_element()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the add-on list and installation controls together"
    )]
    fn addons_tab(&self, server: &ServerView, cx: &Context<Self>) -> AnyElement {
        let id = server.config.id;
        let busy = self.busy();
        let mut body = div().flex().flex_col().gap_6();
        let mut installed = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section("On this server"));
        if server.config.addons.is_empty() {
            installed = installed.child(muted("No add-ons installed yet."));
        }
        for addon in &server.config.addons {
            installed = installed.child(
                panel()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(addon.name.clone()),
                            )
                            .child(muted(pack_summary(addon.packs.len()))),
                    )
                    .child(pill("Installed", Tone::Good)),
            );
        }
        body = body.child(installed);
        let ready: Vec<_> = self
            .data
            .downloaded
            .iter()
            .filter(|addon| !server.config.addons.iter().any(|a| a.id == addon.id))
            .cloned()
            .collect();
        let mut available = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section("Ready to install"))
            .when(server.running, |this| {
                this.child(callout(Tone::Busy, "Stop the server to install add-ons."))
            });
        if ready.is_empty() {
            available = available.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(muted(if self.data.downloaded.is_empty() {
                        "Download an add-on you own to install it here."
                    } else {
                        "Every downloaded add-on is already on this server."
                    }))
                    .child(
                        Button::new("goto-library")
                            .outline()
                            .small()
                            .label("Open My add-ons")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Page::Library, window, cx);
                            })),
                    ),
            );
        }
        for addon in ready {
            let addon_id = addon.id;
            available = available.child(
                panel()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(addon.name.clone()),
                            )
                            .child(muted(format!(
                                "{} · {}",
                                addon.creator,
                                pack_summary(addon.packs.len())
                            ))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("server-install-{addon_id}")))
                            .primary()
                            .small()
                            .label("Install")
                            .disabled(busy || server.running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(
                                    Request::Install(id, addon_id),
                                    "Installing add-on packs…",
                                    cx,
                                );
                            })),
                    ),
            );
        }
        body.child(available).into_any_element()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep settings fields and their event handlers together"
    )]
    fn settings_tab(&self, server: &ServerView, window: &Window, cx: &Context<Self>) -> AnyElement {
        let _ = window;
        let id = server.config.id;
        let locked = server.running || self.busy();
        let form = self.form_config(&server.config, cx);
        let dirty = match &form {
            Ok(config) => !same_settings(config, &server.config),
            Err(_) => true,
        };
        let port_help = match self.port.read(cx).value().trim().parse::<u16>() {
            Ok(port) if port < 65535 => format!("Also uses {}.", port + 1),
            _ => "Use 1024 to 65534.".into(),
        };
        let general = panel()
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .child(section("Game"))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        field(
                            "Server name",
                            Input::new(&self.name).disabled(server.running),
                            None,
                        )
                        .flex_1(),
                    )
                    .child(
                        field(
                            "Port",
                            Input::new(&self.port).disabled(server.running),
                            Some(port_help),
                        )
                        .w(px(140.)),
                    )
                    .child(
                        field(
                            "Player limit",
                            Input::new(&self.players).disabled(server.running),
                            Some("1 to 100".into()),
                        )
                        .w(px(120.)),
                    ),
            )
            .child(field(
                "Game mode",
                div().flex().child(Self::segmented(
                    "mode",
                    &MODES,
                    &self.mode,
                    locked,
                    cx,
                    |this, value, _| this.mode = value.into(),
                )),
                None,
            ))
            .child(field(
                "Difficulty",
                div().flex().child(Self::segmented(
                    "difficulty",
                    &DIFFICULTIES,
                    &self.difficulty,
                    locked,
                    cx,
                    |this, value, _| this.difficulty = value.into(),
                )),
                None,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("save-settings")
                            .primary()
                            .label("Save changes")
                            .disabled(locked || !dirty)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let Some(server) = this.data.server(id).cloned() else {
                                    return;
                                };
                                match this.form_config(&server.config, cx) {
                                    Ok(config) => {
                                        this.resync_form = true;
                                        this.submit(Request::Save(config), "Saving settings…", cx);
                                    }
                                    Err(error) => this.fail(format!("{error:#}"), cx),
                                }
                            })),
                    )
                    .child(
                        Button::new("discard-settings")
                            .ghost()
                            .label("Discard changes")
                            .disabled(server.running || !dirty)
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.select(id, window, cx)),
                            ),
                    )
                    .when(dirty && !server.running, |this| {
                        this.child(muted("Unsaved changes"))
                    }),
            );
        let config = server.config.clone();
        let public = config.public;
        let network = panel()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(section("Network access"))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .flex_1().min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().font_weight(FontWeight::MEDIUM).child(
                                "Let other devices join",
                            ))
                            .child(muted(if public {
                                format!(
                                    "Other devices can connect. NetherNet uses TCP {} and UDP {}. Allow both in your firewall, and forward them on your router for players outside your network.",
                                    config.port,
                                    config.port + 1
                                )
                            } else {
                                "Only Minecraft on this computer can connect.".to_string()
                            })),
                    )
                    .child(
                        Switch::new("network-access")
                            .checked(public)
                            .disabled(locked)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut config = config.clone();
                                config.public = *checked;
                                this.submit(
                                    Request::Save(config),
                                    "Updating network access…",
                                    cx,
                                );
                            })),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .gap_4()
            .when(server.running, |this| {
                this.child(callout(
                    Tone::Busy,
                    "Stop the server to change its settings.",
                ))
            })
            .child(general)
            .child(network)
            .into_any_element()
    }

    /// Reads the settings form into a validated configuration.
    fn form_config(&self, base: &ServerConfig, cx: &App) -> Result<ServerConfig> {
        use anyhow::Context as _;
        let mut config = base.clone();
        config.name = self.name.read(cx).value().trim().to_string();
        config.port = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .context("Enter a port number from 1024 to 65534")?;
        config.max_players = self
            .players
            .read(cx)
            .value()
            .trim()
            .parse()
            .context("Enter a player limit from 1 to 100")?;
        config.gamemode.clone_from(&self.mode);
        config.difficulty.clone_from(&self.difficulty);
        config.validate()?;
        Ok(config)
    }

    fn backups_tab(&self, server: &ServerView, cx: &Context<Self>) -> AnyElement {
        let id = server.config.id;
        let backups = self.data.root.join("backups");
        let last = self.last_backup.get(&id).cloned();
        panel()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(section("Back up this server"))
                    .child(muted(
                        "A backup is a ZIP of the world, settings, allowlist, and installed packs. It leaves out the server program and its console log.",
                    )),
            )
            .when(server.running, |this| {
                this.child(callout(Tone::Busy, "Stop the server to create a backup."))
            })
            .when_some(last, |this, path| {
                let file = path
                    .file_name()
                    .map_or_default(|name| name.to_string_lossy().into_owned());
                this.child(callout(
                    Tone::Good,
                    format!("Saved {file} in the backups folder."),
                ))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("backup")
                            .primary()
                            .label("Back up now")
                            .disabled(self.busy() || server.running)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(Request::Backup(id), "Creating backup…", cx);
                            })),
                    )
                    .child(
                        Button::new("open-backups")
                            .outline()
                            .label("Open backups folder")
                            .disabled(!backups.is_dir())
                            .on_click(move |_, _, _| {
                                if let Err(error) = open::that(&backups) {
 eprintln!("Could not open the backup folder: {error}");
 }
                            }),
                    ),
            )
            .into_any_element()
    }
}

fn pack_summary(count: usize) -> String {
    match count {
        1 => "1 pack".into(),
        count => format!("{count} packs"),
    }
}

fn same_settings(a: &ServerConfig, b: &ServerConfig) -> bool {
    a.name == b.name
        && a.port == b.port
        && a.max_players == b.max_players
        && a.gamemode == b.gamemode
        && a.difficulty == b.difficulty
}
