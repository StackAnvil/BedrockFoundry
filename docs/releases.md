# Publish a release

Releases contain Linux and Windows executables, user guides, dependency license
texts, and SHA-256 checksums. The workflow publishes a GitHub Release only after
both platforms pass CI and both archives pass checksum validation.

## Prepare the source

1. Set the release version in `Cargo.toml`.
2. Add that version's notes to `CHANGELOG.md`.
3. Run the checks in [the contribution guide](../CONTRIBUTING.md).
4. Merge the changes into `main` and make sure that CI passes.

The tag must match the package version exactly. For version 0.1.0, publish this tag:

```sh
git switch main
git pull --ff-only
git tag -a v0.1.0 -m "Release 0.1.0"
git push origin v0.1.0
```

Replace 0.1.0 with the prepared version for later releases. A tag such as
`v0.2.0-rc.1` publishes a prerelease. Tags must point to commits on `main`.
The workflow refuses a version mismatch or an already published release.

## Read the workflow result

The Release workflow validates the tag, runs general CI, builds optimized
executables, and collects dependency license texts with cargo-about. It packages
Linux as `.tar.gz` and Windows as `.zip`. Each archive includes `BUILD.json`,
which records its source commit, target, version, and executable checksum.

The publishing job downloads both archives, verifies their individual checksums,
and writes one `SHA256SUMS` file. It uploads all assets to a draft release before
publishing that release. A build failure cannot publish a release with one
platform missing.

## Retry a failed release

Open the failed workflow run and rerun its failed jobs. You can also use the
Release workflow's manual action with the existing version tag. Do not move the
tag to different code. Publish a new version when the source needs a fix.

Retries can replace assets on an unfinished draft. They cannot replace an
already published release. If publication succeeds, the release is complete even
when a later URL display step reports an error. Read the release page before
starting another publishing attempt.

## Verify a download

Download the archive for your platform and the release's `SHA256SUMS` file.
On Linux, run this command from the download directory:

```sh
sha256sum --ignore-missing --check SHA256SUMS
```

On Windows, run `Get-FileHash ARCHIVE.zip -Algorithm SHA256` in PowerShell and
compare its value with the corresponding line in `SHA256SUMS`. The checksum
must match before you extract or run the executable.
