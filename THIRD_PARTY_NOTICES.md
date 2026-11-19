# Third-party notices

Bedrock Foundry's original source code is MIT licensed. Cargo dependencies retain
the licenses declared by their authors. `Cargo.lock` records the exact dependency
versions used by this application. Release archives include
`third-party-licenses.html` with the dependency license texts collected by cargo-about. No GPL or LGPL source from StackAnvil's Java
patches is included in this repository.

## Bedrock Services

Authentication and Minecraft service sessions use
[bedrock-services](https://github.com/cinnabar-mc/bedrock-services), pinned to
revision `cf4bfc5f59eaade09b0d075463e5f77b7d8c2e63`. It is MIT licensed. Its source
includes its own LICENSE and THIRD_PARTY_NOTICES.md in Cargo's dependency checkout.

## GPUI

[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 0.2.2 and
[GPUI Component](https://github.com/longbridge/gpui-component) 0.5.1 are
Apache-2.0 licensed. They and their dependencies provide windowing, text, and UI
components. They are dependencies, not vendored application source.

## Vendored xattr compatibility fix

`vendor/xattr` contains xattr 0.2.3 by Steven Allen, distributed under MIT or
Apache-2.0. The original license texts remain in that directory. Foundry changes
its missing-attribute constant on Linux and Android to libc::ENODATA. Current
libc releases removed their historical ENOATTR alias. This preserves the old
xattr API required by GPUI 0.2.2's tar dependency.

Foundry also applies its Clippy checks to this copy. The attribute iterator
stops if the operating system returns a name without a null terminator.
Each unsafe block documents the conditions its operation requires. Calls to the
operating system and the buffer allocator use separate unsafe blocks.

## Minecraft content

BDS executables and Marketplace assets are downloaded separately from Microsoft
services. They are not part of this MIT-licensed source distribution. They
retain the licenses and terms of Mojang, Microsoft, and their respective creators.
