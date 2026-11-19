//! Marketplace browsing, listing details, and the purchase review.

use super::*;
use bedrock_foundry::marketplace::{CatalogItem, PAGE_SIZE};
use gpui_component::input::Input;

const SHEET_WIDTH: f32 = 440.;

/// What this app can do with a listing for the connected account.
enum Availability {
    /// A world, skin pack, or other product. Visible here, used in Minecraft.
    ViewOnly,
    Downloaded,
    Owned,
    Free,
    ForSale(u64),
    Unavailable,
}

fn availability(item: &CatalogItem, data: &Snapshot) -> Availability {
    if item.addon.is_none() {
        Availability::ViewOnly
    } else if data.is_downloaded(item.id) {
        Availability::Downloaded
    } else if item.owned {
        Availability::Owned
    } else if item.purchasable && item.price == Some(0) {
        Availability::Free
    } else if let Some(price) = item.purchase_price.filter(|_| item.purchasable) {
        Availability::ForSale(price)
    } else {
        Availability::Unavailable
    }
}

fn price_tag(item: &CatalogItem, data: &Snapshot) -> Div {
    match availability(item, data) {
        Availability::Downloaded | Availability::Owned => pill("Owned", Tone::Good),
        Availability::ViewOnly
        | Availability::Free
        | Availability::ForSale(_)
        | Availability::Unavailable => match item.price {
            Some(0) => pill("Free", Tone::Good),
            Some(amount) => pill(minecoins(amount), Tone::Coin),
            None => pill("No price", Tone::Neutral),
        },
    }
}

impl Foundry {
    pub(super) fn browse_from_start(&mut self, cx: &mut Context<Self>) {
        if !self.connected() {
            return;
        }
        self.detail = None;
        self.catalog_requested = true;
        let search = self.search.read(cx).value().trim().to_string();
        self.submit(
            Request::Browse(CatalogQuery {
                search,
                addons_only: self.addons_only,
                skip: 0,
            }),
            "Searching the Marketplace…",
            cx,
        );
    }

    fn thumbnail(&mut self, item: &CatalogItem, height: f32) -> Div {
        let image = item
            .thumbnail
            .as_deref()
            .and_then(|url| self.thumbs.get(url));
        let frame = div()
            .w_full()
            .h(px(height))
            .flex_none()
            .overflow_hidden()
            .bg(rgb(0xd9ddd6));
        match image {
            Some(image) => frame.child(img(image).size_full().object_fit(ObjectFit::Cover)),
            None => frame.flex().items_end().p_3().child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(item.kind.clone()),
            ),
        }
    }

    pub(super) fn connect_prompt(&self, text: &str, cx: &Context<Self>) -> Div {
        panel()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .w(px(620.))
            .child(section("Connect your Microsoft account"))
            .child(muted(text.to_owned()))
            .child(
                div().flex().child(
                    Button::new("connect-from-page")
                        .primary()
                        .label("Sign in with Microsoft")
                        .disabled(self.busy())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open(Page::Account, window, cx);
                            this.submit(Request::Login, "Starting Microsoft sign-in…", cx);
                        })),
                ),
            )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the listing layout, filters, and pagination handlers together"
    )]
    pub(super) fn marketplace_page(&mut self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let intro = heading(
            "Marketplace",
            "Browse everything in the Minecraft Marketplace. Standalone add-ons can be bought, downloaded, and installed on your servers here.",
        );
        if !self.connected() {
            return page(
                intro,
                None,
                self.connect_prompt(
                    "The Marketplace catalog, your ownership, and your Minecoin balance are read with the Microsoft account you use in Minecraft.",
                    cx,
                ),
            )
            .into_any_element();
        }
        let busy = self.busy();
        let wallet = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child({
                        let (fg, bg) = if self.data.wallet.is_some() {
                            Tone::Coin.colors()
                        } else {
                            Tone::Neutral.colors()
                        };
                        div()
                            .px_3()
                            .py_1()
                            .rounded(px(6.))
                            .bg(rgb(bg))
                            .text_color(rgb(fg))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(match self.data.wallet {
                                Some(amount) => minecoins(amount),
                                None if self.working_on(Op::Balance)
                                    || self.working_on(Op::Browse) =>
                                {
                                    "Reading balance…".into()
                                }
                                None => "Balance not loaded".into(),
                            })
                    })
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child("Add Minecoins in Minecraft"),
                    ),
            )
            .child(
                Button::new("refresh-balance")
                    .ghost()
                    .small()
                    .label("Refresh")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.submit(Request::Balance, "Reading your Minecoin balance…", cx);
                    })),
            );
        let search_text = self.search.read(cx).value().trim().to_string();
        let controls = div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().flex_1().child(Input::new(&self.search)))
            .child(
                Button::new("catalog-search")
                    .primary()
                    .label("Search")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.browse_from_start(cx))),
            )
            .child(Self::segmented(
                "scope",
                &[("all", "Everything"), ("addons", "Add-ons only")],
                if self.addons_only { "addons" } else { "all" },
                busy,
                cx,
                |this, value, cx| {
                    this.addons_only = value == "addons";
                    this.browse_from_start(cx);
                },
            ));
        let mut body = div().flex().flex_col().gap_5().child(controls);
        let loading = self.working_on(Op::Browse);
        if let Some(error) = self.catalog_error.clone().filter(|_| !loading) {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(callout(
                        Tone::Bad,
                        format!("The Marketplace couldn't be read. {error}"),
                    ))
                    .child(
                        div().flex().child(
                            Button::new("retry-catalog")
                                .outline()
                                .label("Try again")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.browse_from_start(cx))),
                        ),
                    ),
            );
        }
        let Some(catalog) = self.data.catalog.clone() else {
            body = body.child(if loading {
                muted("Loading listings…").py_6()
            } else if self.catalog_error.is_some() {
                div()
            } else {
                div()
                    .py_6()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(muted("The catalog isn't loaded."))
                    .child(
                        Button::new("load-catalog")
                            .outline()
                            .small()
                            .label("Load listings")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.browse_from_start(cx))),
                    )
            });
            return page(intro, Some(wallet.into_any_element()), body).into_any_element();
        };
        let query = catalog.query.clone();
        let noun = match (query.addons_only, catalog.total == 1) {
            (true, true) => "add-on",
            (true, false) => "add-ons",
            (false, true) => "listing",
            (false, false) => "listings",
        };
        let described = if query.search.is_empty() {
            noun.to_string()
        } else {
            format!("{noun} for “{}”", query.search)
        };
        if catalog.items.is_empty() {
            body = body.child(
                panel()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(section(format!("No {described}")))
                    .child(muted(
                        "Try a shorter search, or include every product type.",
                    ))
                    .child(
                        div().flex().child(
                            Button::new("clear-search")
                                .outline()
                                .label("Show all listings")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    this.addons_only = false;
                                    this.browse_from_start(cx);
                                })),
                        ),
                    ),
            );
            return page(intro, Some(wallet.into_any_element()), body).into_any_element();
        }
        let first = query.skip + 1;
        let last = query.skip + catalog.items.len() as u64;
        body = body.child(
            div()
                .flex()
                .items_center()
                .child(muted(format!(
                    "{first}–{last} of {} {described}",
                    group(catalog.total)
                )))
                .when(
                    !search_text.is_empty() && search_text != query.search,
                    |this| this.child(div().pl_3().child(muted("Press Search to apply."))),
                )
                .child(div().flex_1())
                .child(muted("Newest first")),
        );
        let width = f32::from(window.viewport_size().width) - RAIL_WIDTH - 64.;
        let cards_per_row = match (width.min(1080.) + 16.) / 236. {
            value if value >= 5. => 5,
            value if value >= 4. => 4,
            value if value >= 3. => 3,
            _ => 2,
        };
        let mut cards: Vec<AnyElement> = Vec::new();
        for item in catalog.items.clone() {
            let id = item.id;
            let selected = self.detail == Some(id);
            let thumbnail = self.thumbnail(&item, 116.);
            cards.push(
                div()
                    .id(SharedString::from(format!("listing-{id}")))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(if selected { MOSS } else { LINE }))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.border_color(rgb(0xa9b2a8)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail = Some(id);
                        cx.notify();
                    }))
                    .child(thumbnail)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_3()
                            .child(
                                div()
                                    .h(px(40.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_height(px(20.))
                                    .line_clamp(2)
                                    .child(item.title.clone()),
                            )
                            .child(
                                div()
                                    .line_clamp(1)
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(item.creator.clone()),
                            )
                            .child(
                                div()
                                    .pt_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(MUTED))
                                            .child(item.kind.clone()),
                                    )
                                    .child(price_tag(&item, &self.data)),
                            ),
                    )
                    .into_any_element(),
            );
        }
        let mut previous = query.clone();
        previous.skip = previous.skip.saturating_sub(PAGE_SIZE);
        let next = catalog.next_skip.map(|skip| CatalogQuery {
            skip,
            ..query.clone()
        });
        let page_number = query.skip / PAGE_SIZE + 1;
        let total_pages = catalog.total.div_ceil(PAGE_SIZE).max(1);
        let pager = div()
            .flex()
            .items_center()
            .justify_center()
            .gap_3()
            .pt_2()
            .child(
                Button::new("previous-page")
                    .outline()
                    .label("Previous")
                    .disabled(busy || query.skip == 0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail = None;
                        this.submit(
                            Request::Browse(previous.clone()),
                            "Loading the previous page…",
                            cx,
                        );
                    })),
            )
            .child(muted(format!(
                "Page {} of {}",
                group(page_number),
                group(total_pages)
            )))
            .child(
                Button::new("next-page")
                    .outline()
                    .label("Next")
                    .disabled(busy || next.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(next) = next.clone() {
                            this.detail = None;
                            this.submit(Request::Browse(next), "Loading the next page…", cx);
                        }
                    })),
            );
        // Rows of equal-width cards. Padding cells keep the last row aligned.
        let mut grid = div()
            .flex()
            .flex_col()
            .gap_4()
            .when(loading, |this| this.opacity(0.5));
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            let mut row = div().flex().gap_4();
            for _ in 0..cards_per_row {
                row = row.child(div().flex_1().min_w_0().flex().children(cards.next()));
            }
            grid = grid.child(row);
        }
        body = body
            .child(grid)
            .when(total_pages > 1, |this| this.child(pager));
        page(intro, Some(wallet.into_any_element()), body).into_any_element()
    }

    /// The right-hand sheet with one listing's description and actions.
    #[expect(
        clippy::too_many_lines,
        reason = "Keep listing details and their actions together"
    )]
    pub(super) fn listing_sheet(&mut self, cx: &Context<Self>) -> Option<AnyElement> {
        let id = self.detail?;
        let item = self
            .data
            .catalog
            .as_ref()?
            .items
            .iter()
            .find(|item| item.id == id)?
            .clone();
        let busy = self.busy();
        let thumbnail = self.thumbnail(&item, 230.);
        let mut action = div().flex().flex_col().gap_3();
        match availability(&item, &self.data) {
            Availability::ViewOnly => {
                action = action.child(callout(
                    Tone::Neutral,
                    format!(
                        "This is a {}. Bedrock Foundry installs standalone add-ons on servers, so get this one in Minecraft.",
                        item.kind.to_lowercase()
                    ),
                ));
            }
            Availability::Downloaded => {
                action = action
                    .child(callout(
                        Tone::Good,
                        "You own this add-on and it is downloaded. Install it on a stopped server.",
                    ))
                    .child(self.install_targets(id, cx));
            }
            Availability::Owned => {
                action = action
                    .child(muted(
                        "You own this add-on. Download it to install it on your servers.",
                    ))
                    .child(
                        div().flex().child(
                            Button::new("sheet-download")
                                .primary()
                                .label("Download")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.submit(
                                        Request::Download(id),
                                        "Downloading and decrypting your add-on…",
                                        cx,
                                    );
                                })),
                        ),
                    );
            }
            Availability::Free => {
                action = action
                    .child(muted(
                        "This add-on is free. Adding it to your library doesn't spend Minecoins.",
                    ))
                    .child(
                        div().flex().child(
                            Button::new("sheet-claim")
                                .primary()
                                .label("Add to my library")
                                .disabled(busy)
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
            Availability::ForSale(price) => {
                let title = item.title.clone();
                action = action
                    .child(muted(format!(
                        "Costs {}. You'll review the price and your balance before anything is spent.",
                        minecoins(price)
                    )))
                    .child(
                        div().flex().child(
                            Button::new("sheet-buy")
                                .primary()
                                .label("Buy…")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.busy() {
                                        return;
                                    }
                                    this.purchase = Some(Purchase::Checking {
                                        id,
                                        title: title.clone(),
                                    });
                                    this.submit(
                                        Request::Quote(id),
                                        "Checking the current price and your balance…",
                                        cx,
                                    );
                                })),
                        ),
                    );
            }
            Availability::Unavailable => {
                action = action.child(callout(
                    Tone::Neutral,
                    "This add-on can't be bought here because it isn't available or its Minecoin price couldn't be verified. Check it in Minecraft.",
                ));
            }
        }
        let description = if item.description.trim().is_empty() {
            "The creator didn't provide a description.".to_string()
        } else {
            item.description.clone()
        };
        Some(
            scrim("sheet-scrim")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.detail = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("listing-sheet")
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_full()
                        .w(px(SHEET_WIDTH))
                        .flex()
                        .flex_col()
                        .bg(rgb(SURFACE))
                        .border_l_1()
                        .border_color(rgb(LINE))
                        .shadow_lg()
                        .occlude()
                        // Clicks inside the sheet must not close it.
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .px_5()
                                .py_3()
                                .border_b_1()
                                .border_color(rgb(LINE))
                                .child(div().flex_1().child(muted(item.kind.clone())))
                                .child(
                                    Button::new("close-sheet")
                                        .ghost()
                                        .small()
                                        .label("Close")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.detail = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .id("sheet-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .child(thumbnail)
                                .child(
                                    div()
                                        .p_5()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .text_xl()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child(item.title.clone()),
                                                )
                                                .child(muted(format!("By {}", item.creator))),
                                        )
                                        .child(div().flex().child(price_tag(&item, &self.data)))
                                        .child(action)
                                        .child(
                                            div()
                                                .pt_2()
                                                .text_sm()
                                                .line_height(px(21.))
                                                .child(description),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The purchase dialog. It covers the window so no other action can run.
    #[expect(
        clippy::too_many_lines,
        reason = "Keep the purchase confirmation states and their handlers together"
    )]
    pub(super) fn purchase_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.purchase.clone()?;
        let mut card = div()
            .id("purchase-dialog")
            .w(px(480.))
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(SURFACE))
            .rounded(px(10.))
            .shadow_lg()
            .occlude();
        let close = |label: &'static str, cx: &mut Context<Self>| {
            Button::new("purchase-close")
                .outline()
                .label(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.purchase = None;
                    cx.notify();
                }))
        };
        let buttons = div().flex().justify_end().gap_2().pt_2();
        match state {
            Purchase::Checking { title, .. } => {
                card = card
                    .child(section(format!("Checking {title}")))
                    .child(muted(
                        "Reading the current price and your Minecoin balance. Nothing is spent at this step.",
                    ))
                    .child(buttons.child(
                        Button::new("purchase-cancel")
                            .outline()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.purchase = None;
                                // Clears the quote once the check finishes.
                                this.enqueue(Request::CancelPurchase, "Canceling purchase review…");
                                cx.notify();
                            })),
                    ));
            }
            Purchase::Review(quote) => {
                let remaining = quote.expires_at - now();
                let expired = remaining <= 0;
                let short = quote.amount.saturating_sub(quote.balance);
                let enough = short == 0;
                let busy = self.busy();
                let id = quote.id;
                let item_id = quote.item.id;
                let title = quote.item.title.clone();
                let thumbnail = self.thumbnail(&quote.item, 72.).w(px(128.)).rounded(px(6.));
                let row = |label: &str, value: String, strong: bool| {
                    div()
                        .flex()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(
                            div()
                                .flex_1()
                                .text_color(rgb(MUTED))
                                .child(label.to_owned()),
                        )
                        .child(
                            div()
                                .when(strong, |this| this.font_weight(FontWeight::SEMIBOLD))
                                .child(value),
                        )
                };
                card = card
                    .child(section(if enough && !expired {
                        "Confirm purchase"
                    } else {
                        "Review purchase"
                    }))
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .child(thumbnail)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .line_clamp(2)
                                            .child(quote.item.title.clone()),
                                    )
                                    .child(muted(format!("By {}", quote.item.creator))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(row("Price", minecoins(quote.amount), true))
                            .child(row("Your balance", minecoins(quote.balance), false))
                            .child(if enough {
                                row(
                                    "Balance after purchase",
                                    minecoins(quote.balance - quote.amount),
                                    false,
                                )
                            } else {
                                row("You need", format!("{} more", minecoins(short)), true)
                            }),
                    )
                    .child(muted(
                        "Minecoins are spent from the Microsoft account connected to Bedrock Foundry, and the add-on joins that account's library. The price and balance are checked again right before buying. Purchases can't be undone here.",
                    ));
                if expired {
                    card = card
                        .child(callout(
                            Tone::Busy,
                            "This review expired. Check the price again to continue.",
                        ))
                        .child(buttons.child(cancel_review(cx)).child(requote(
                            item_id,
                            title,
                            busy,
                            "Check price again",
                            cx,
                        )));
                } else if !enough {
                    card = card
                        .child(callout(
                            Tone::Bad,
                            "You don't have enough Minecoins. Add Minecoins in Minecraft, then check your balance again.",
                        ))
                        .child(
                            buttons
                                .child(cancel_review(cx))
                                .child(requote(item_id, title, busy, "Check balance again", cx)),
                        );
                } else {
                    card = card
                        .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                            "This review expires in {}:{:02}.",
                            remaining / 60,
                            remaining % 60
                        )))
                        .child(
                            buttons.child(cancel_review(cx)).child(
                                Button::new("purchase-confirm")
                                    .primary()
                                    .label(format!("Buy for {}", minecoins(quote.amount)))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(Purchase::Review(quote)) = this.purchase.clone()
                                        else {
                                            return;
                                        };
                                        if this.busy()
                                            || quote.id != id
                                            || now() >= quote.expires_at
                                            || quote.balance < quote.amount
                                        {
                                            return;
                                        }
                                        this.purchase = Some(Purchase::Submitting(quote));
                                        this.submit(
                                            Request::Buy(id),
                                            "Buying the add-on. Keep Bedrock Foundry open…",
                                            cx,
                                        );
                                    })),
                            ),
                        );
                }
            }
            Purchase::Submitting(quote) => {
                card = card
                    .child(section(format!("Buying {}", quote.item.title)))
                    .child(muted(format!(
                        "Spending {}. Keep Bedrock Foundry open until this finishes. This step can't be canceled.",
                        minecoins(quote.amount)
                    )));
            }
            Purchase::Finished { id, title, ready } => {
                let downloaded = self.data.is_downloaded(id);
                card = card.child(section(if ready {
                    "Purchase complete"
                } else {
                    "Purchase accepted"
                }));
                if ready {
                    card = card
                        .child(muted(format!(
                            "{title} is in your library. Download it to install it on a server."
                        )))
                        .child(buttons.child(close("Close", cx)).when(!downloaded, |this| {
                            this.child(
                                Button::new("purchase-download")
                                    .primary()
                                    .label("Download now")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.purchase = None;
                                        this.submit(
                                            Request::Download(id),
                                            "Downloading and decrypting your add-on…",
                                            cx,
                                        );
                                    })),
                            )
                        }));
                } else {
                    card = card
                        .child(muted(format!(
                            "Minecraft accepted the purchase of {title}, but your library doesn't list it yet. Refresh My add-ons in a few minutes. Don't buy it again."
                        )))
                        .child(
                            buttons.child(close("Close", cx)).child(
                                Button::new("purchase-library")
                                    .primary()
                                    .label("Refresh My add-ons")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.purchase = None;
                                        this.page = Page::Library;
                                        this.submit(
                                            Request::Library,
                                            "Reading your Marketplace library…",
                                            cx,
                                        );
                                        let _ = window;
                                    })),
                            ),
                        );
                }
            }
            Purchase::Failed {
                title,
                message,
                submitted,
            } => {
                card = card
                    .child(section(if submitted {
                        format!("The purchase outcome for {title} is unknown")
                    } else {
                        format!("Can't buy {title} right now")
                    }))
                    .child(callout(Tone::Bad, message));
                if submitted {
                    card = card
                        .child(muted(
                            "Check My add-ons before trying again. If the outcome is unknown, Bedrock Foundry blocks another purchase of this add-on until your library confirms ownership.",
                        ))
                        .child(
                            buttons.child(close("Close", cx)).child(
                                Button::new("purchase-failed-library")
                                    .primary()
                                    .label("Refresh My add-ons")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.purchase = None;
                                        this.page = Page::Library;
                                        this.submit(
                                            Request::Library,
                                            "Reading your Marketplace library…",
                                            cx,
                                        );
                                    })),
                            ),
                        );
                } else {
                    card = card.child(buttons.child(close("Close", cx)));
                }
            }
        }
        Some(
            scrim("purchase-scrim")
                .flex()
                .items_center()
                .justify_center()
                .child(card)
                .into_any_element(),
        )
    }
}

fn cancel_review(cx: &Context<Foundry>) -> Button {
    Button::new("purchase-cancel")
        .outline()
        .label("Cancel")
        .on_click(cx.listener(|this, _, _, cx| {
            this.purchase = None;
            this.enqueue(Request::CancelPurchase, "Canceling purchase review…");
            cx.notify();
        }))
}

fn requote(
    id: Uuid,
    title: String,
    busy: bool,
    label: &'static str,
    cx: &Context<Foundry>,
) -> Button {
    Button::new("purchase-requote")
        .primary()
        .label(label)
        .disabled(busy)
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.busy() {
                return;
            }
            this.purchase = Some(Purchase::Checking {
                id,
                title: title.clone(),
            });
            this.submit(
                Request::Quote(id),
                "Checking the current price and your balance…",
                cx,
            );
        }))
}

fn group(value: u64) -> String {
    let text = minecoins(value);
    text.split(' ').next().unwrap_or_default().to_owned()
}
