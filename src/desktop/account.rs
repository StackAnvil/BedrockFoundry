//! Microsoft account connection.

use super::*;
use std::sync::atomic::Ordering;

impl Foundry {
    #[expect(
        clippy::too_many_lines,
        reason = "Keep account page layout and its event handlers together"
    )]
    pub(super) fn account_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy();
        let intro = heading(
            "Microsoft account",
            "Connect the account you use in Minecraft. Bedrock Foundry uses it to read the add-ons you own, download them, and buy add-ons with Minecoins you already have.",
        );
        let body = if let Some((url, code)) = self.device.clone() {
            let link = url.clone();
            let copy = code.clone();
            panel()
                .p_6()
                .flex()
                .flex_col()
                .gap_5()
                .w(px(620.))
                .child(section("Finish signing in"))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(sign_in_step(1, format!("Open {url} in your browser.")))
                        .child(sign_in_step(2, "Enter this code:".into()))
                        .child(
                            div()
                                .ml(px(44.))
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .px_4()
                                        .py_2()
                                        .rounded(px(6.))
                                        .bg(rgb(STONE))
                                        .border_1()
                                        .border_color(rgb(LINE))
                                        .text_size(px(28.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(code),
                                )
                                .child(
                                    Button::new("copy-code")
                                        .outline()
                                        .label("Copy code")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ));
                                        }),
                                ),
                        )
                        .child(sign_in_step(
                            3,
                            "Choose the account you use in Minecraft and approve access. This page updates by itself.".into(),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("open-login")
                                .primary()
                                .label("Open sign-in page")
                                .on_click(move |_, _, _| {
                                    if open::that(&link).is_err() {
 eprintln!("Could not open the sign-in link");
 }
                                }),
                        )
                        .child(
                            Button::new("cancel-login")
                                .ghost()
                                .label("Cancel sign-in")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.worker.cancel.store(true, Ordering::Relaxed);
                                    cx.notify();
                                })),
                        ),
                )
        } else if let Some(account) = self.data.account.clone() {
            panel()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(620.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(muted("Connected as"))
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(account.gamertag),
                        ),
                )
                .when_some(self.data.wallet, |this, amount| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(muted("Minecoin balance"))
                            .child(pill(minecoins(amount), Tone::Coin)),
                    )
                })
                .child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(LINE))
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(muted(
                            "Disconnecting removes the saved sign-in from this computer. Your servers and downloaded add-ons stay.",
                        ).flex_1().min_w_0())
                        .child(
                            Button::new("logout")
                                .danger()
                                .outline()
                                .label("Disconnect")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.submit(
                                        Request::Logout,
                                        "Disconnecting Microsoft account…",
                                        cx,
                                    );
                                })),
                        ),
                )
        } else {
            let signing_in = self.working_on(Op::Login);
            panel()
                .p_6()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(620.))
                .child(section("No account connected"))
                .child(muted(
                    "You sign in on Microsoft's website with a short code. Bedrock Foundry never sees your password.",
                ))
                .child(
                    div().flex().child(
                        Button::new("login")
                            .primary()
                            .label(if signing_in {
                                "Starting sign-in…"
                            } else {
                                "Sign in with Microsoft"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.submit(Request::Login, "Starting Microsoft sign-in…", cx);
                            })),
                    ),
                )
        };
        page(
            intro,
            None,
            div().flex().flex_col().gap_4().child(body).child(
                div()
                    .w(px(620.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("The sign-in is stored in your system's credential store. Downloaded add-ons and servers stay on this computer."),
            ),
        )
        .into_any_element()
    }
}

fn sign_in_step(number: u8, text: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_none()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(rgb(MOSS_SOFT))
                .text_color(rgb(MOSS))
                .font_weight(FontWeight::SEMIBOLD)
                .child(number.to_string()),
        )
        .child(div().flex_1().child(text))
}
