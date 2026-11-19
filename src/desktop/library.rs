//! Owned and downloaded add-ons, and installing them on servers.

use super::*;
use bedrock_foundry::marketplace::Addon;

impl Foundry {
    /// Every server with its install state for one downloaded add-on.
    pub(super) fn install_targets(&mut self, addon: Uuid, cx: &mut Context<Self>) -> Div {
        let busy = self.busy();
        let mut list = div().flex().flex_col().gap_1();
        if self.data.servers.is_empty() {
            return list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(muted("Create a server to install this add-on."))
                    .child(
                        Button::new(SharedString::from(format!("targets-new-{addon}")))
                            .outline()
                            .small()
                            .label("New server")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Page::NewServer, window, cx);
                            })),
                    ),
            );
        }
        for server in self.data.servers.clone() {
            let server_id = server.config.id;
            let installed = server.config.addons.iter().any(|a| a.id == addon);
            let reason = if installed {
                None
            } else if server.config.version.is_none() {
                Some("Install Bedrock Dedicated Server first")
            } else if server.running {
                Some("Stop the server first")
            } else {
                None
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgb(0xf0f2ed))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(server.config.name))
                            .when_some(reason, |this, reason| {
                                this.child(div().text_xs().text_color(rgb(MUTED)).child(reason))
                            }),
                    )
                    .child(if installed {
                        pill("Installed", Tone::Good).into_any_element()
                    } else {
                        Button::new(SharedString::from(format!("install-{addon}-{server_id}")))
                            .primary()
                            .small()
                            .label("Install")
                            .disabled(busy || reason.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(
                                    Request::Install(server_id, addon),
                                    "Installing add-on packs…",
                                    cx,
                                );
                            }))
                            .into_any_element()
                    }),
            );
        }
        list
    }

    fn addon_row(&mut self, addon: &Addon, cx: &mut Context<Self>) -> Div {
        let id = addon.id;
        let downloaded = self.data.is_downloaded(id);
        let open = self.install_open == Some(id);
        let busy = self.busy();
        let servers: Vec<String> = self
            .data
            .servers
            .iter()
            .filter(|server| server.config.addons.iter().any(|a| a.id == id))
            .map(|server| server.config.name.clone())
            .collect();
        let summary = match servers.len() {
            0 if downloaded => "Downloaded · Not on a server yet".to_string(),
            0 => "Not downloaded".to_string(),
            _ => format!("Downloaded · On {}", servers.join(", ")),
        };
        let action = if downloaded {
            Button::new(SharedString::from(format!("library-install-{id}")))
                .outline()
                .small()
                .label(if open { "Done" } else { "Install on a server" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.install_open = if open { None } else { Some(id) };
                    cx.notify();
                }))
        } else {
            Button::new(SharedString::from(format!("library-download-{id}")))
                .primary()
                .small()
                .label("Download")
                .disabled(busy || !self.connected())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.submit(
                        Request::Download(id),
                        "Downloading and decrypting your add-on…",
                        cx,
                    );
                }))
        };
        panel()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(addon.name.clone()),
                                    )
                                    .when(!servers.is_empty(), |this| {
                                        this.child(pill("Installed", Tone::Good))
                                    }),
                            )
                            .child(muted(format!("{} · {summary}", addon.creator))),
                    )
                    .child(action),
            )
            .when(open, |this| {
                this.child(
                    div()
                        .px_4()
                        .pb_4()
                        .pt_1()
                        .border_t_1()
                        .border_color(rgb(LINE))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().pt_2().text_xs().text_color(rgb(MUTED)).child(
                            "Install adds the packs to the server's world. Start the server afterward to use them.",
                        ))
                        .child(self.install_targets(id, cx)),
                )
            })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the library view and its empty and loading states together"
    )]
    pub(super) fn library_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy();
        let connected = self.connected();
        let intro = heading(
            "My add-ons",
            "Add-ons you own on your Microsoft account. Download one, then install it on a stopped server.",
        );
        let actions = connected.then(|| {
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("free-sample")
                        .ghost()
                        .label("Try a free add-on")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.submit(Request::Sample, "Loading the free sample add-on…", cx);
                        })),
                )
                .child(
                    Button::new("refresh-library")
                        .outline()
                        .label("Refresh")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.submit(Request::Library, "Reading your Marketplace library…", cx);
                        })),
                )
                .into_any_element()
        });
        let mut body = div().flex().flex_col().gap_3();
        if let Some(sample) = self.data.sample.clone() {
            let id = sample.id;
            body = body.child(
                panel()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .border_color(rgb(0xb9cfbf))
                    .bg(rgb(0xf1f6f1))
                    .child(
                        div()
                            .flex_1()
                            .child(div().font_weight(FontWeight::MEDIUM).child(sample.name))
                            .child(muted(format!(
                                "Free add-on by {}. Adding it doesn't spend Minecoins.",
                                sample.creator
                            ))),
                    )
                    .child(
                        Button::new("claim-sample")
                            .primary()
                            .small()
                            .label(if sample.owned {
                                "Already in your library"
                            } else {
                                "Add to my library"
                            })
                            .disabled(sample.owned || busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit(
                                    Request::Claim(id),
                                    "Adding the free add-on to your library…",
                                    cx,
                                );
                            })),
                    ),
            );
        }
        // Owned add-ons first, then downloads the library didn't list, such as
        // after signing out or while offline.
        let mut addons = self.data.library.clone();
        for addon in &self.data.downloaded {
            if !addons.iter().any(|owned| owned.id == addon.id) {
                addons.push(addon.clone());
            }
        }
        if !connected {
            let text = if self.data.downloaded.is_empty() {
                "Sign in to see the add-ons you own and download them."
            } else {
                "Sign in to see the add-ons you own and download them. Add-ons already on this computer stay installable below."
            };
            body = body.child(self.connect_prompt(text, cx));
        }
        let reading = self.working_on(Op::Library);
        if let Some(error) = self.library_error.clone().filter(|_| connected && !reading) {
            body = body.child(callout(
                Tone::Bad,
                format!("Your library couldn't be read. {error}"),
            ));
        }
        if addons.is_empty() {
            if connected && self.library_error.is_some() && !reading {
                // The error above explains the empty list.
            } else if connected && (reading || !self.library_loaded) {
                body = body.child(muted("Reading your Marketplace library…").py_4());
            } else if connected {
                body = body.child(
                    panel()
                        .p_6()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(section("No add-ons yet"))
                        .child(muted(
                            "This account doesn't own any standalone add-ons. Find one in the Marketplace, or try the free sample.",
                        ))
                        .child(
                            div().flex().child(
                                Button::new("library-browse")
                                    .primary()
                                    .label("Browse the Marketplace")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.addons_only = true;
                                        this.open(Page::Marketplace, window, cx);
                                    })),
                            ),
                        ),
                );
            }
        } else {
            for addon in &addons {
                body = body.child(self.addon_row(addon, cx));
            }
        }
        page(intro, actions, body).into_any_element()
    }
}
