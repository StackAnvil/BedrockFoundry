import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.1.0"\n')
        for name in release.PUBLIC_FILES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"Public file: {name}\n")
        (self.root / "licenses.html").write_text(
            "<html>Dependency license texts</html>"
        )
        (self.root / "CHANGELOG.md").write_text(
            "# Changes\n\n## [Unreleased]\n\nFuture work\n\n"
            "## [0.1.0] - 2026-11-14\n\nInitial release.\n\n## [0.0.1]\n\nOld notes.\n"
        )
        (self.root / ".private").mkdir()
        (self.root / ".private/account.json").write_text("private-account-sentinel")
        self.commit = "a" * 40

    def package(self, platform):
        binary = self.root / release.PLATFORMS[platform][0]
        binary.write_bytes(b"executable-fixture")
        return release.package(
            platform,
            binary,
            self.root / "licenses.html",
            self.root / "dist",
            self.commit,
            self.root,
        )

    def test_mismatched_or_injected_tags_cannot_select_a_release(self):
        for tag in [
            "v0.2.0",
            "0.1.0",
            "main",
            "v0.1.0\nextra=value",
            "v0.1.0; echo bad",
        ]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.metadata(tag, self.root)
        self.assertEqual(release.metadata("v0.1.0", self.root)["prerelease"], "false")
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.1.0-rc.1"\n')
        self.assertEqual(
            release.metadata("v0.1.0-rc.1", self.root)["prerelease"], "true"
        )

    def test_release_notes_exclude_unreleased_and_other_versions(self):
        self.assertEqual(release.notes(self.root), "Initial release.\n")
        path = self.root / "CHANGELOG.md"
        path.write_text(
            path.read_text().replace(
                "Initial release.", "Read [compatibility](docs/compatibility.md)."
            )
        )
        self.assertEqual(
            release.notes(self.root),
            "Read [compatibility](https://github.com/StackAnvil/BedrockFoundry/blob/v0.1.0/docs/compatibility.md).\n",
        )

    def test_archives_include_executable_provenance_and_licenses_without_private_files(
        self,
    ):
        for platform in release.PLATFORMS:
            with self.subTest(platform=platform):
                path = self.package(platform)
                if path.suffix == ".zip":
                    with zipfile.ZipFile(path) as archive:
                        contents = {
                            name: archive.read(name) for name in archive.namelist()
                        }
                else:
                    with tarfile.open(path, "r:gz") as archive:
                        contents = {
                            member.name: archive.extractfile(member).read()
                            for member in archive.getmembers()
                        }
                        binary = next(
                            m
                            for m in archive.getmembers()
                            if m.name.endswith("/bedrock-foundry")
                        )
                        self.assertEqual(binary.mode, 0o755)
                folder = f"bedrock-foundry-0.1.0-{platform}/"
                self.assertEqual(
                    contents[folder + release.PLATFORMS[platform][0]],
                    b"executable-fixture",
                )
                self.assertIn(folder + "LICENSE", contents)
                self.assertIn(folder + "third-party-licenses.html", contents)
                self.assertEqual(
                    json.loads(contents[folder + "BUILD.json"])["commit"], self.commit
                )
                self.assertNotIn(
                    b"private-account-sentinel", b"".join(contents.values())
                )
                self.assertFalse(any(".private" in name for name in contents))

    def test_missing_or_tampered_platform_archives_prevent_publication(self):
        linux = self.package("linux-x86_64")
        with self.assertRaises(ValueError):
            release.checksums(linux.parent, self.root)
        windows = self.package("windows-x86_64")
        sums = release.checksums(linux.parent, self.root).read_text()
        self.assertIn(linux.name, sums)
        self.assertIn(windows.name, sums)
        windows.write_bytes(b"tampered archive")
        with self.assertRaisesRegex(ValueError, "checksum"):
            release.checksums(linux.parent, self.root)


if __name__ == "__main__":
    unittest.main()
