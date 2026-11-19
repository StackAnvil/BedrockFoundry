# Bedrock Foundry contributor instructions

Stay on the current branch unless the user requests another branch. Preserve unrelated work.
Use Conventional Commits and do not bypass hooks.

## Architecture

Use Rust and GPUI for the desktop. Keep service and filesystem behavior in the
library so the CLI and desktop share one implementation. Serialize account
refresh and persistence. Store credentials in the OS credential store. Keep
service errors free of tokens, headers, signed URLs, and receipt bodies.

Keep downloads and server instances in the application data directory. Never
commit accounts, content keys, Marketplace packs, BDS distributions, captures,
or private live-test evidence. Use synthetic fixtures for repository tests.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked
python3 -m unittest discover -s tools -p 'test_*.py'
```

Install cargo-deny 0.20.2 for the pre-push checks. The hook checks both Cargo
projects when a Cargo manifest, lockfile, or deny.toml changes.

Run focused tests first. Test observable archive admission, entitlement checks,
world activation, recovery, configuration, and process ownership. A unit test does
not establish native rendering or multiplayer compatibility.

Release tags must match Cargo.toml and the changelog. Keep release archives limited
to the executable, documented public files, provenance, and dependency licenses.
Run actionlint when workflows change and the packaging tests when release tools change.

## Licensing

Keep this project MIT licensed. Do not copy GPL or LGPL implementation code into
it. Protocol field names and behavior need versioned evidence. Preserve notices
for permissively licensed dependencies and vendored code. The xattr patch is a
compatibility fix for the published GPUI dependency, not an application feature.

## UI and documentation

Use the tagline: Set up Bedrock servers with your Marketplace add-ons.
Use plain language and explicit action names. Avoid decorative metrics or
placeholder behavior. Keep build instructions in docs/development.md, CLI usage
in docs/cli.md, and hosting behavior in docs/hosting.md. Replace compatibility
status when evidence changes instead of appending an experiment diary.
