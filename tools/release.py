#!/usr/bin/env python3
"""Validate release versions and package only explicit public release inputs."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
import re
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLATFORMS = {
    "linux-x86_64": ("bedrock-foundry", "x86_64-unknown-linux-gnu", ".tar.gz"),
    "windows-x86_64": ("bedrock-foundry.exe", "x86_64-pc-windows-msvc", ".zip"),
}
PUBLIC_FILES = (
    "LICENSE",
    "README.md",
    "THIRD_PARTY_NOTICES.md",
    "CONTRIBUTING.md",
    "CHANGELOG.md",
    "docs/cli.md",
    "docs/compatibility.md",
    "docs/development.md",
    "docs/hosting.md",
    "docs/releases.md",
    "docs/media/create-server.mp4",
    "docs/media/create-server.gif",
    "docs/media/install-addon.mp4",
    "docs/media/install-addon.gif",
    "docs/media/manage-server.mp4",
    "docs/media/manage-server.gif",
)
VERSION = re.compile(
    r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def version(root: Path = ROOT) -> str:
    value = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"][
        "version"
    ]
    if not VERSION.fullmatch(value):
        raise ValueError(
            "Cargo.toml must contain a release version such as 0.1.0 or 0.1.0-rc.1"
        )
    return value


def metadata(tag: str, root: Path = ROOT) -> dict[str, str]:
    current = version(root)
    if tag != f"v{current}":
        raise ValueError(f"Tag {tag!r} does not match Cargo.toml version v{current}")
    return {"version": current, "tag": tag, "prerelease": str("-" in current).lower()}


def notes(root: Path = ROOT) -> str:
    current = version(root)
    text = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    sections = re.split(r"^## ", text, flags=re.MULTILINE)
    for section in sections[1:]:
        heading, _, body = section.partition("\n")
        if heading.startswith(f"[{current}] ") or heading == f"[{current}]":
            if not body.strip():
                raise ValueError(f"CHANGELOG.md has no notes for {current}")
            return (
                body.strip().replace(
                    "](docs/",
                    f"](https://github.com/StackAnvil/BedrockFoundry/blob/v{current}/docs/",
                )
                + "\n"
            )
    raise ValueError(f"CHANGELOG.md has no release section for {current}")


def _write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temporary:
        pending = Path(temporary.name)
        try:
            temporary.write(data)
            temporary.flush()
            os.fsync(temporary.fileno())
        except BaseException:
            pending.unlink(missing_ok=True)
            raise
    try:
        os.replace(pending, path)
    finally:
        pending.unlink(missing_ok=True)


def package(
    platform: str,
    binary: Path,
    licenses: Path,
    output: Path,
    commit: str,
    root: Path = ROOT,
) -> Path:
    executable, target, suffix = PLATFORMS[platform]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("A complete source commit SHA is required")
    if binary.name != executable or not binary.is_file() or binary.is_symlink():
        raise ValueError(f"Expected a regular {executable} executable")
    if not licenses.is_file() or not licenses.stat().st_size:
        raise ValueError("Generate the third-party license report before packaging")
    current = version(root)
    folder = f"bedrock-foundry-{current}-{platform}"
    files = {name: (root / name).read_bytes() for name in PUBLIC_FILES}
    files[executable] = binary.read_bytes()
    files["third-party-licenses.html"] = licenses.read_bytes()
    files["BUILD.json"] = (
        json.dumps(
            {
                "version": current,
                "target": target,
                "commit": commit,
                "binary_sha256": hashlib.sha256(files[executable]).hexdigest(),
                "repository": "https://github.com/StackAnvil/BedrockFoundry",
            },
            indent=2,
        )
        + "\n"
    ).encode()
    content = io.BytesIO()
    if suffix == ".zip":
        with zipfile.ZipFile(
            content, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9
        ) as archive:
            for name, data in sorted(files.items()):
                entry = zipfile.ZipInfo(
                    f"{folder}/{name}", date_time=(1980, 1, 1, 0, 0, 0)
                )
                entry.create_system = 3
                entry.external_attr = (
                    0o100755 if name == executable else 0o100644
                ) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(entry, data)
    else:
        with gzip.GzipFile(
            fileobj=content, mode="wb", filename="", mtime=0
        ) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for name, data in sorted(files.items()):
                    entry = tarfile.TarInfo(f"{folder}/{name}")
                    entry.size = len(data)
                    entry.mode = 0o755 if name == executable else 0o644
                    entry.mtime = 0
                    archive.addfile(entry, io.BytesIO(data))
    path = output / f"{folder}{suffix}"
    data = content.getvalue()
    _write(path, data)
    _write(
        path.with_name(path.name + ".sha256"),
        f"{hashlib.sha256(data).hexdigest()}  {path.name}\n".encode(),
    )
    return path


def checksums(directory: Path, root: Path = ROOT) -> Path:
    current = version(root)
    expected = {
        f"bedrock-foundry-{current}-{platform}{suffix}"
        for platform, (_, _, suffix) in PLATFORMS.items()
    }
    found = {
        p.name for p in directory.iterdir() if p.name.endswith((".zip", ".tar.gz"))
    }
    if found != expected:
        raise ValueError(
            f"Release archives do not match the complete platform set: {sorted(found)}"
        )
    records = []
    for name in sorted(expected):
        digest = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        record = f"{digest}  {name}\n"
        if (directory / f"{name}.sha256").read_text(encoding="utf-8") != record:
            raise ValueError(f"Archive checksum does not match: {name}")
        records.append(record)
    path = directory / "SHA256SUMS"
    _write(path, "".join(records).encode())
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    meta = commands.add_parser("metadata")
    meta.add_argument("--tag", required=True)
    meta.add_argument("--github-output", type=Path)
    commands.add_parser("notes")
    pack = commands.add_parser("package")
    pack.add_argument("--platform", choices=PLATFORMS, required=True)
    pack.add_argument("--binary", type=Path, required=True)
    pack.add_argument("--licenses", type=Path, required=True)
    pack.add_argument("--out", type=Path, default=ROOT / "dist")
    pack.add_argument("--commit", required=True)
    sums = commands.add_parser("checksums")
    sums.add_argument("--directory", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if args.command == "metadata":
        values = metadata(args.tag)
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8", newline="\n") as output:
                for key, value in values.items():
                    output.write(f"{key}={value}\n")
        print(json.dumps(values))
    elif args.command == "notes":
        print(notes(), end="")
    elif args.command == "package":
        print(package(args.platform, args.binary, args.licenses, args.out, args.commit))
    else:
        print(checksums(args.directory))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(str(error)) from None
