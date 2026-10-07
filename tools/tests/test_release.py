"""Packaging contract tests use synthetic executables; no hardware/compiler needed."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

TOOLS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("verify_release", TOOLS / "verify-release.py")
verify_release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify_release)


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def package(self, label="linux-x86_64", corrupt_binary=False, extra=None, target=None):
        triple, binary, extension = verify_release.TARGETS[label]
        folder = "Race2Love-0.1.0-" + label
        data = b"MZfixture" if extension == ".zip" else b"\x7fELFfixture"
        info = {"application": "Race2Love", "version": "0.1.0", "target": target or triple,
                "profile": "release", "source_commit": "abc", "executable_sha256": hashlib.sha256(data).hexdigest()}
        files = {binary: data + (b"changed" if corrupt_binary else b""), "LICENSE": b"PolyForm Noncommercial License 1.0.0", "NOTICE": b"Race2Love attribution", "README.txt": b"fixture",
                 "INSTALL.md": b"fixture", "THIRD_PARTY_NOTICES.txt": b"Race2Love third-party notices\nfixture", "build-info.json": json.dumps(info).encode(),
                 "config.example.toml": b'[lovense]\noutput_mode = "vibrate"\n'}
        if extension == ".zip":
            files.update({"Launch-LMU.cmd": b"fixture", "Launch-with-logs.cmd": b"fixture"})
        path = self.root / (folder + extension)
        if extension == ".zip":
            with zipfile.ZipFile(path, "w") as bundle:
                for name, value in files.items():
                    bundle.writestr(folder + "/" + name, value)
                if extra:
                    bundle.writestr(extra, b"unwanted")
        else:
            with tarfile.open(path, "w:gz") as bundle:
                for name, value in files.items():
                    member = tarfile.TarInfo(folder + "/" + name)
                    member.size = len(value)
                    member.mode = 0o755 if name == binary else 0o644
                    bundle.addfile(member, io.BytesIO(value))
                if extra:
                    member = tarfile.TarInfo(extra)
                    member.type = tarfile.SYMTYPE
                    member.linkname = "../../outside"
                    bundle.addfile(member)
        return path, hashlib.sha256(path.read_bytes()).hexdigest()

    def test_valid_native_and_cross_packages(self):
        for label in verify_release.TARGETS:
            with self.subTest(label=label):
                path, digest = self.package(label)
                self.assertEqual(verify_release.verify(path, digest, "abc")[0], label)

    def test_corrupt_archive_and_executable_rejected(self):
        path, digest = self.package()
        with self.assertRaisesRegex(ValueError, "Checksum"):
            verify_release.verify(path, "0" * 64)
        path, digest = self.package(corrupt_binary=True)
        with self.assertRaisesRegex(ValueError, "Executable hash"):
            verify_release.verify(path, digest)

    def test_wrong_commit_or_target_rejected(self):
        path, digest = self.package()
        with self.assertRaisesRegex(ValueError, "commit mismatch"):
            verify_release.verify(path, digest, "different")
        path, digest = self.package(target="x86_64-pc-windows-msvc")
        with self.assertRaisesRegex(ValueError, "metadata"):
            verify_release.verify(path, digest)

    def test_unexpected_paths_and_links_rejected(self):
        for label in verify_release.TARGETS:
            path, digest = self.package(label, extra="../../unexpected")
            with self.assertRaises(ValueError):
                verify_release.verify(path, digest)


if __name__ == "__main__":
    unittest.main()
