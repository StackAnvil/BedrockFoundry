# Build and test Bedrock Foundry

Use the Rust toolchain pinned in `rust-toolchain.toml`. GPUI requires native
windowing, font, and graphics libraries. A desktop session with a working GPU
driver is required to run the window. The CLI can run without a desktop build.

## Install Linux dependencies

On Ubuntu 24.04 or later, install these packages:

```sh
sudo apt-get update
sudo apt-get install -y build-essential cmake clang libclang-dev pkg-config \
  libasound2-dev libfontconfig1-dev libfreetype6-dev libx11-dev libxcb1-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev
```

On Fedora, install the matching development packages:

```sh
sudo dnf install gcc gcc-c++ cmake clang clang-devel pkgconf-pkg-config \
  alsa-lib-devel fontconfig-devel freetype-devel libX11-devel libxcb-devel \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel
```

Linux credential storage uses Secret Service, usually provided by GNOME Keyring
or KWallet. Run Foundry in a session where that service is available and unlocked.
Foundry does not fall back to an unencrypted credential file.

## Build on Windows

Install Rust through rustup and the Visual Studio C++ Build Tools. Use the MSVC
toolchain. Run `cargo build --locked --release` from the repository directory.
Windows account storage uses Credential Manager. BDS runs as a child process
without opening a separate console window.

## Run checks

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked
```

For the CLI and core tests without GPUI, use:

```sh
cargo run --locked --no-default-features -- --help
cargo test --locked --no-default-features
```

Tests use small synthetic packs and temporary server directories. No test needs
a Microsoft account or downloads paid content. See [compatibility](compatibility.md)
for the separate live test boundary.

## Change the code

`account.rs` owns sign-in and protected checkpoints. `marketplace.rs` resolves
entitlements and catalog downloads. `archive.rs` validates and decodes pack
archives. `server.rs` owns instance files, process handles, and backups.
`backend.rs` joins those operations. The desktop and CLI share that backend.

A single process holds the application directory lease. The desktop serializes
account operations on its worker thread. Add-on installs stage files before
activation and retain a recovery journal until the operation commits. Startup
restores an unfinished installation before exposing the server list.

The small xattr compatibility patch in `vendor/` supports the published GPUI
release with current libc. Keep its upstream license files and explain any
future changes in `THIRD_PARTY_NOTICES.md`.
