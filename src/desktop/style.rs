//! Palette and small layout primitives shared by the desktop views.
//!
//! The palette follows bedrock strata: a deepslate rail, pale stone surfaces,
//! moss for actions that move setup forward, and gold reserved for Minecoins.

use gpui::{prelude::*, *};

pub const DEEPSLATE: u32 = 0x2b3134;
pub const DEEPSLATE_RAISED: u32 = 0x373e42;
pub const DEEPSLATE_TEXT: u32 = 0xc9d0cc;
pub const DEEPSLATE_MUTED: u32 = 0x8d9893;
pub const STONE: u32 = 0xebede8;
pub const SURFACE: u32 = 0xfbfbf9;
pub const INK: u32 = 0x1f2527;
pub const MUTED: u32 = 0x5d676a;
pub const LINE: u32 = 0xd3d8d1;
pub const MOSS: u32 = 0x2f6a4a;
pub const MOSS_SOFT: u32 = 0xdde9df;
pub const GOLD: u32 = 0x8a6512;
pub const GOLD_SOFT: u32 = 0xf4eacd;
pub const AMBER: u32 = 0xb87613;
pub const AMBER_SOFT: u32 = 0xf6e7cc;
pub const RED: u32 = 0xa83a2c;
pub const RED_SOFT: u32 = 0xf6dedb;
pub const CONSOLE: u32 = 0x22282a;
pub const CONSOLE_TEXT: u32 = 0xd5dcd6;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Busy,
    Bad,
    Coin,
}

impl Tone {
    pub fn colors(self) -> (u32, u32) {
        match self {
            Tone::Neutral => (MUTED, 0xe2e5e0),
            Tone::Good => (MOSS, MOSS_SOFT),
            Tone::Busy => (AMBER, AMBER_SOFT),
            Tone::Bad => (RED, RED_SOFT),
            Tone::Coin => (GOLD, GOLD_SOFT),
        }
    }
}

/// A short state label, such as Running or Owned.
pub fn pill(text: impl Into<SharedString>, tone: Tone) -> Div {
    let (fg, bg) = tone.colors();
    div()
        .flex_none()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// A small square marker for process state in lists.
pub fn dot(tone: Tone) -> Div {
    let (fg, _) = tone.colors();
    div().flex_none().size(px(8.)).rounded(px(2.)).bg(rgb(fg))
}

pub fn panel() -> Div {
    div()
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(LINE))
        .rounded(px(8.))
}

pub fn muted(text: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(rgb(MUTED)).child(text.into())
}

pub fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(26.))
        .line_height(px(32.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

pub fn section(text: impl Into<SharedString>) -> Div {
    div()
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// A labeled form field with optional help text below the control.
pub fn field(label: &str, control: impl IntoElement, help: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(label.to_owned()),
        )
        .child(control)
        .when_some(help, |this, help| {
            this.child(div().text_xs().text_color(rgb(MUTED)).child(help))
        })
}

/// A highlighted note inside a page, such as a blocked action's reason.
pub fn callout(tone: Tone, text: impl Into<SharedString>) -> Div {
    let (fg, bg) = tone.colors();
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .rounded(px(6.))
        .bg(rgb(bg))
        .border_l_4()
        .border_color(rgb(fg))
        .text_sm()
        .text_color(rgb(INK))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// Strata mark: three offset layers of stone over a moss layer.
pub fn mark(scale: f32) -> Div {
    let layer = |width: f32, offset: f32, color: u32| {
        div()
            .ml(px(offset * scale))
            .w(px(width * scale))
            .h(px(5. * scale))
            .rounded(px(1.5 * scale))
            .bg(rgb(color))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(2. * scale))
        .child(layer(22., 6., 0x9aa6a0))
        .child(layer(26., 0., 0xc3cbc5))
        .child(layer(20., 4., 0x6e7a75))
        .child(layer(28., 0., 0x5c9a6f))
}

pub fn minecoins(amount: u64) -> String {
    let digits = amount.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!(
        "{grouped} {}",
        if amount == 1 { "Minecoin" } else { "Minecoins" }
    )
}
