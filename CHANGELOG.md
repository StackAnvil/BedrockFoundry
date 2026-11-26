# Changelog

## [Unreleased]

## [0.2.0] - 2026-11-26

- Browse and search public Marketplace listings with thumbnails, details, and paging.
- Review Minecoin balances and confirm standalone add-on purchases in the desktop or CLI.
- Re-check purchase prices, accounts, and packs, and retain uncertain attempts across restarts.
- Redesign the desktop with guided setup, a server rail, dedicated management tabs, and purchase dialogs.
- Add `server list --json` and `server run --json` so other programs can start, join, and stop servers.
- Update `url` to 2.5.9 for RUSTSEC-2026-0592.

## [0.1.0] - 2026-11-14

Initial public release of Bedrock Foundry.

- GPUI desktop app and CLI for Bedrock server instances.
- Microsoft account connection with OS credential storage.
- Owned Marketplace add-on discovery, download, and decryption.
- BDS download or ZIP import, pack installation, and interrupted-install recovery.
- Server settings, NetherNet access, console commands, shutdown, and backups.

Poisonous Potato Add-On 1.0.8 passed server-side tests on Linux BDS 1.26.60.6
and 1.26.63.1. Native client joins, rendering, and public exposure remain unverified.
Automatic BDS upgrades, add-on updates, and external tunnels are not available
in this release. Read [compatibility](docs/compatibility.md) for the current limits.
