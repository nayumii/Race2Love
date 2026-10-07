#!/usr/bin/env python3
"""Check package integrity and optionally run native/mock Demo (or Wine) smoke tests.

Use only on artifacts you trust: --smoke executes the bundled application.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

TARGETS = {
    "linux-x86_64": ("x86_64-unknown-linux-gnu", "race2love", ".tar.gz"),
    "windows-x86_64": ("x86_64-pc-windows-gnu", "race2love.exe", ".zip"),
    "windows-msvc-x86_64": ("x86_64-pc-windows-msvc", "race2love.exe", ".zip"),
}


def verify(archive, expected_hash, expected_commit=None):
    """Read an explicit file list; never extract archive paths or symlinks."""
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected_hash:
        raise ValueError(f"Checksum mismatch: {archive.name}")
    match = re.fullmatch(r"Race2Love-([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)-(linux-x86_64|windows-x86_64|windows-msvc-x86_64)(\.tar\.gz|\.zip)", archive.name)
    if not match:
        raise ValueError(f"Unexpected archive name: {archive.name}")
    version, label, suffix = match.groups()
    target, binary, extension = TARGETS[label]
    if suffix != extension:
        raise ValueError("Archive extension does not match target")
    folder = archive.name.removesuffix(suffix)
    required = {binary, "LICENSE", "NOTICE", "config.example.toml", "README.txt", "INSTALL.md", "THIRD_PARTY_NOTICES.txt", "build-info.json"}
    if label.startswith("windows"):
        required |= {"Launch-LMU.cmd", "Launch-with-logs.cmd"}
    expected = {f"{folder}/{name}" for name in required}
    if suffix == ".zip":
        with zipfile.ZipFile(archive) as bundle:
            members = bundle.infolist()
            if {m.filename for m in members} != expected or len(members) != len(expected):
                raise ValueError("Unexpected or duplicate ZIP members")
            if any(m.file_size > 100_000_000 or (m.external_attr >> 16) & 0o170000 == 0o120000 for m in members):
                raise ValueError("Oversized or linked ZIP member")
            content = {name: bundle.read(f"{folder}/{name}") for name in required}
    else:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            files = [m for m in members if m.isfile()]
            if {m.name for m in files} != expected or len(files) != len(expected):
                raise ValueError("Unexpected or duplicate TAR files")
            if any(not m.isfile() and not (m.isdir() and m.name == folder) or m.size > 100_000_000 for m in members):
                raise ValueError("Unsafe TAR member")
            if not bundle.getmember(f"{folder}/{binary}").mode & 0o111:
                raise ValueError("Linux executable permissions missing")
            content = {name: bundle.extractfile(f"{folder}/{name}").read() for name in required}
    info = json.loads(content["build-info.json"])
    if info.get("application") != "Race2Love" or info.get("version") != version or info.get("target") != target:
        raise ValueError("Build metadata does not match package")
    if info.get("profile") != "release" or hashlib.sha256(content[binary]).hexdigest() != info.get("executable_sha256"):
        raise ValueError("Executable hash/profile mismatch")
    if b"PolyForm Noncommercial License 1.0.0" not in content["LICENSE"]:
        raise ValueError("PolyForm license text missing")
    if b"Race2Love" not in content["NOTICE"]:
        raise ValueError("Race2Love attribution notice missing")
    if not content["THIRD_PARTY_NOTICES.txt"].startswith(b"Race2Love third-party notices"):
        raise ValueError("Third-party notice header missing")
    if expected_commit and info.get("source_commit") != expected_commit:
        raise ValueError("Package source commit mismatch")
    if not content[binary].startswith(b"MZ" if suffix == ".zip" else b"\x7fELF"):
        raise ValueError("Executable format does not match target")
    config = tomllib.loads(content["config.example.toml"].decode())
    if config["lovense"]["output_mode"] != "vibrate":
        raise ValueError("Example must retain the tested Direct Vibrate default")
    return label, binary, content[binary], info


def smoke(label, binary, data, wine=False):
    native = (label.startswith("linux") and platform.system() == "Linux") or (label.startswith("windows") and platform.system() == "Windows")
    use_wine = label.startswith("windows") and platform.system() == "Linux" and wine
    if not native and not use_wine:
        raise ValueError(f"Cannot smoke-test {label} on this host; use --wine on Linux for Windows GNU")
    with tempfile.TemporaryDirectory(prefix="race2love-package-test-") as temporary:
        temporary = Path(temporary)
        executable = temporary / binary
        executable.write_bytes(data)
        executable.chmod(0o700)
        env = os.environ.copy()
        # The test always starts Demo + MockDevice; no saved config or physical output.
        config = temporary / "isolated-config.toml"
        env.update(RACE2LOVE_CONFIG=str(config), NO_COLOR="1", RUST_LOG="race2love=info,race2love_core=info,race2love_lovense=info")
        command = [str(executable), "--demo-seconds", "5"]
        if use_wine:
            prefix = temporary / "wine"
            prefix.mkdir()
            env.update(WINEPREFIX=str(prefix), WINEDEBUG="-all", WINEDLLOVERRIDES="mscoree,mshtml=", DISPLAY="", WAYLAND_DISPLAY="")
            env["RACE2LOVE_CONFIG"] = "Z:" + str(config).replace("/", "\\")
            command.insert(0, "wine")
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90)
        output = re.sub(r"\x1b\[[0-9;]*m", "", result.stdout + result.stderr)
        if result.returncode or "Demo: connected=true" not in output or "Race2Love shutdown complete output=0.0" not in output:
            raise ValueError(f"Demo smoke failed ({result.returncode}):\n{output}")
        levels = re.search(r"mixed=([0-9.]+), mock_output=([0-9.]+)", output)
        if not levels or not all(float(level) > 0 for level in levels.groups()):
            raise ValueError("Demo did not produce nonzero mock output")
        print(f"  {label}: Demo connected, nonzero mock output, shutdown output 0", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=Path("dist"))
    parser.add_argument("--expected-commit")
    parser.add_argument("--smoke", action="store_true", help="Execute trusted packaged binaries with isolated mock output")
    parser.add_argument("--wine", action="store_true", help="Permit Windows smoke tests through Wine on Linux")
    args = parser.parse_args()
    lines = (args.directory / "SHA256SUMS.txt").read_text().splitlines()
    if not lines:
        raise ValueError("Empty checksum manifest")
    seen = set()
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+)", line)
        if not match or match[2] in seen:
            raise ValueError("Malformed/duplicate checksum entry")
        digest, name = match.groups()
        seen.add(name)
        label, binary, data, info = verify(args.directory / name, digest, args.expected_commit)
        print(f"Verified {name} (commit {info.get('source_commit')}, dirty={info.get('source_dirty')})", flush=True)
        if args.smoke:
            smoke(label, binary, data, args.wine)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, tarfile.TarError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        print(f"Verification failed: {error}", file=sys.stderr)
        sys.exit(1)
