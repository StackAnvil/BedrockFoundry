# Contribute to Bedrock Foundry

Use the pinned Rust toolchain and read the [build guide](docs/development.md).
Keep each change focused. The desktop and CLI share the library backend, so put
service, filesystem, and process behavior there.

## Prepare a change

Search existing issues before reporting a problem. Include the Foundry version,
operating system, BDS version, and a small reproduction. Use synthetic packs and
accounts for repository tests. Keep real credentials, receipt keys, copyrighted
content, and traffic captures out of issues, commits, and pull requests.

Use Conventional Commits such as `fix(server): preserve external pack activation`.
Explain the problem, resulting behavior, and validation in the pull request.
Include a screenshot for a visible change. State what remains unverified.

## Run checks

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo check --locked --no-default-features
python3 -m unittest discover -s tools -p 'test_*.py'
```

The packaging tests need Python 3.11 or newer. They use only the standard library.
Workflow validation uses [actionlint](https://github.com/rhysd/actionlint).
CI runs the Rust tests and desktop build on Linux and Windows. Live Marketplace
or game tests are separate from CI and require private local evidence.

If dependencies change, generate the release license report:

```sh
cargo install cargo-about --version 0.9.3 --locked --features cli
cargo about generate --locked --fail tools/licenses.hbs --output-file third-party-licenses.html
```

Keep `Cargo.lock` committed. Do not remove license checks or loosen archive
validation to accept a broken fixture. See [the release guide](docs/releases.md)
for version tags and publishing.

## Community and security

Follow the [StackAnvil code of conduct](https://github.com/StackAnvil/.github/blob/main/CODE_OF_CONDUCT.md).
Report vulnerabilities through the repository's Security tab and follow the
[organization's private reporting instructions](https://github.com/StackAnvil/.github/blob/main/SECURITY.md).
