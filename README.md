# Bedrock Foundry

Set up Bedrock servers with your Marketplace add-ons.

Bedrock Foundry is a Rust desktop application built with [GPUI](https://gpui.rs/).
Connect your Microsoft account, download add-ons that you own, and install them
in a Bedrock Dedicated Server (BDS) instance. Start and stop servers, send console
commands, change settings, and create backups from the same window.

This is an early release. Local hosting targets Windows and Linux. See the
[compatibility notes](docs/compatibility.md) for tested behavior and current limits.

## Run the app

Install the native dependencies in the [build guide](docs/development.md), then run:

```sh
git clone https://github.com/StackAnvil/BedrockFoundry.git
cd BedrockFoundry
cargo run --locked
```

The pinned Rust toolchain is installed through rustup. For a release build, run
`cargo build --locked --release`. The executable is `target/release/bedrock-foundry`
on Linux and `target/release/bedrock-foundry.exe` on Windows.

## Set up a server

1. Open **Microsoft account** and select **Connect Microsoft account**.
2. Complete sign-in on the Microsoft page with the displayed code.
3. Open **Marketplace add-ons** and select **Refresh library**.
4. Download an add-on that you own. **Try a free add-on** offers a free sample.
5. Open **Servers**, enter a name and port, and select **Create server**.
6. Select **Download and install BDS**. You can also import an official BDS ZIP.
7. Select the server, return to the library, and install the downloaded add-on.
8. Return to **Servers** and select **Start server**.

BDS downloads come from Mojang. Review the [Minecraft EULA](https://www.minecraft.net/eula)
before downloading or running the server software. Foundry does not include a server binary.

Account credentials stay in the OS credential store. Downloads, servers, and
backups stay in the application data directory. The [CLI reference](docs/cli.md)
describes commands for terminal use and separate data directories.

## Connect other players

New instances bind to this computer only. Stop the server and select
**Allow network connections** to accept connections from other computers.
[The hosting guide](docs/hosting.md) explains NetherNet ports, router forwarding,
and connections behind carrier-grade NAT.

## License

Bedrock Foundry source code is [MIT licensed](LICENSE). Dependencies retain their
own licenses; see [third-party notices](THIRD_PARTY_NOTICES.md). Minecraft server
software and Marketplace content retain their original licenses and are not
redistributed by this project.
