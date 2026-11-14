# Compatibility

Poisonous Potato Add-On 1.0.8 from Jigarbov Productions passed server-side live
tests on Linux BDS 1.26.60.6 and 1.26.63.1. Foundry restored the account, read owned entitlements,
downloaded the official add-on archive, and decrypted both packs. It preserved
the original manifests, JavaScript, resources, identifiers, and dependencies.

The server test created a fresh world and passed custom block placement and
readback, custom item insertion into a chest, custom entity spawning and lookup,
and normal shutdown. A CPU profile recorded the original add-on JavaScript
executing. The accepted run contained no content or script errors. Native client
pack delivery, rendering, guide forms, and multiplayer were not tested.

The automatic installer also downloaded and installed official BDS 1.26.63.1.
Each add-on still needs a matching server version and working script modules.
One successful free add-on does not establish compatibility with every paid
add-on, subscription product, or world template.

## Supported operations

The desktop and CLI provide account sign-in, owned add-on discovery, free sample
acquisition, download and decryption, server creation, BDS installation, pack
activation, process control, console input, configuration, and stopped-server
backups. The desktop can import an official BDS ZIP when automatic download is
unavailable. Local BDS hosting targets Windows and Linux.

Archive validation bounds download size, expanded size, nested pack count, and
file count. It rejects traversal paths, symbolic links, ambiguous duplicate
payloads, unexpected pack identities, and missing entitlement keys. The native
legacy duplicate `contents.json` entry remains supported. Installation preserves
external pack activation entries and rolls back an unfinished journal on startup.

## Current limits

- World templates and skin-only products do not appear in the add-on library.
- Automatic BDS upgrades, add-on updates, add-on removal, and backup restoration
  are not implemented in the desktop.
- Pack-to-pack dependency versions are checked. Script API dependencies remain
  visible in BDS content logs, which are the authority for runtime compatibility.
- Public port forwarding, tunnels, and native client joins require separate tests.
- macOS does not have an official local BDS distribution.
- The first release uses one connected account per application data directory.

Account receipts, content keys, copyrighted downloads, and raw test evidence stay
in private local storage. The repository contains synthetic regression fixtures
and source code only.
