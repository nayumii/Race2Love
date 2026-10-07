#!/usr/bin/env python3
"""Build portable Linux/Windows archives locally or on native CI runners."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "linux-x86_64": ("x86_64-unknown-linux-gnu", "race2love", ".tar.gz"),
    "windows-x86_64": ("x86_64-pc-windows-gnu", "race2love.exe", ".zip"),
    "windows-msvc-x86_64": ("x86_64-pc-windows-msvc", "race2love.exe", ".zip"),
}


def capture(command, env):
    return subprocess.check_output(command, cwd=ROOT, env=env, text=True).strip()


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def dependency_notices(target, cargo_flags, env):
    """Preserve upstream notices from Cargo's resolved sources, including bundled fonts.

    This intentionally includes build/test crates too; an inventory is not a
    declaration that every resolved crate is linked into the final executable.
    """
    metadata = json.loads(capture(["cargo", "metadata", "--format-version=1", "--filter-platform", target, *cargo_flags], env))
    blocks = ["Race2Love third-party notices\n\nResolved Cargo dependencies (including build/test dependencies). "
              "Each dependency retains its own license; Race2Love's license does not replace these terms.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if not package.get("source"):
            continue
        root = Path(package["manifest_path"]).parent
        files = {p for p in root.rglob("*") if p.is_file() and re.fullmatch(r"(?i)(?:licen[cs]e|copying|notice|copyright)(?:[-_.].*)?|ofl\.txt", p.name)}
        if package.get("license_file"):
            files.add(root / package["license_file"])
        heading = f"\n{'=' * 72}\n{package['name']} {package['version']}\nLicense: {package.get('license') or 'See license file'}\nRepository: {package.get('repository') or package['source']}\n"
        if not files:
            fallback = ROOT / "packaging" / "licenses"
            sources = json.loads((fallback / "sources.json").read_text(encoding="utf-8"))
            group = sources["packages"].get(f"{package['name']}@{package['version']}")
            if group:
                root = fallback / group
                files = {path for path in root.iterdir() if path.is_file()}
                heading += "Notice sources: " + ", ".join(url for name, url in sources["sources"].items() if name.startswith(group + "/")) + "\n"
            else:
                # Cargo's published manifest still records the governing SPDX
                # expression and repository. Keep that attribution visible when
                # a crate omits a standalone text file; do not invent license text.
                files = set()
                heading += "Published crate omitted a standalone notice file; see the Cargo license expression and repository above.\n"
        blocks.append(heading)
        for path in sorted(files):
            if path.stat().st_size > 2_000_000:
                raise RuntimeError(f"Unexpectedly large notice: {path}")
            blocks.append(f"\n--- {path.relative_to(root)} ---\n" + path.read_text(encoding="utf-8") + "\n")
    return "\n".join(blocks)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Use cached Rust dependencies only")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist", help="Archive directory (default: dist/)")
    parser.add_argument("--target", action="append", choices=TARGETS, help="Build only this target; repeat to select several. Default: both Linux/GNU from Linux; MSVC from Windows.")
    parser.add_argument("--expected-version", help="Require this Cargo version (used to validate release tags)")
    args = parser.parse_args()
    if sys.version_info < (3, 11):
        parser.error("Python 3.11 or later is required.")
    host = platform.system()
    if host not in ("Linux", "Windows") or platform.machine().lower() not in ("x86_64", "amd64"):
        parser.error("Run this command on x86-64 Linux or Windows.")
    labels = list(dict.fromkeys(args.target or (["linux-x86_64", "windows-x86_64"] if host == "Linux" else ["windows-msvc-x86_64"])))
    if (host == "Linux" and "windows-msvc-x86_64" in labels) or (host == "Windows" and labels != ["windows-msvc-x86_64"]):
        parser.error("MSVC requires Windows; Linux and MinGW packaging require Linux.")
    selected = {label: TARGETS[label] for label in labels}

    env = os.environ.copy()
    for tool in ("cargo", "rustup"):
        if not shutil.which(tool):
            parser.error(f"Missing {tool}; install Rust with rustup first.")
    installed = capture(["rustup", "target", "list", "--installed"], env).splitlines()
    missing = [target for target, _, _ in selected.values() if target not in installed]
    if missing:
        parser.error("Install missing targets first: rustup target add " + " ".join(missing))
    if "windows-x86_64" in selected:
        for key, default in (
            ("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER", "x86_64-w64-mingw32-gcc"),
            ("CC_x86_64_pc_windows_gnu", "x86_64-w64-mingw32-gcc"),
            ("AR_x86_64_pc_windows_gnu", "x86_64-w64-mingw32-ar"),
        ):
            env.setdefault(key, default)
            if not shutil.which(env[key]):
                parser.error(f"Missing {env[key]}; install the MinGW-w64 GCC toolchain (see README.md).")

    cargo_flags = ["--locked"] + (["--offline"] if args.offline else [])
    metadata = json.loads(capture(["cargo", "metadata", "--no-deps", "--format-version=1", *cargo_flags], env))
    version = next(package["version"] for package in metadata["packages"] if package["name"] == "race2love")
    if args.expected_version is not None and args.expected_version != version:
        parser.error(f"Release version {args.expected_version!r} does not match Cargo version {version!r}")
    target_dir = Path(metadata["target_directory"])
    for target, _, _ in selected.values():
        command = ["cargo", "build", "--release", "-p", "race2love", "--bin", "race2love", "--target", target, *cargo_flags]
        print("\n+ " + " ".join(command), flush=True)
        subprocess.run(command, cwd=ROOT, env=env, check=True)

    # Package an explicit file list, never Cargo's dependency/build directories.
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    archives = []
    rustc = capture(["rustc", "--version"], env)
    commit = capture(["git", "rev-parse", "HEAD"], env) if shutil.which("git") and (ROOT / ".git").exists() else None
    dirty = bool(capture(["git", "status", "--porcelain"], env)) if commit else None
    with tempfile.TemporaryDirectory(prefix=".race2love-release-", dir=output) as temporary:
        temporary = Path(temporary)
        for label, (target, binary_name, extension) in selected.items():
            name = f"Race2Love-{version}-{label}"
            staging = temporary / name
            staging.mkdir()
            binary = target_dir / target / "release" / binary_name
            shutil.copy2(binary, staging / binary_name)
            for filename in ("LICENSE", "NOTICE", "config.example.toml"):
                shutil.copy2(ROOT / filename, staging / filename)
            os_name = label.split("-")[0]
            shutil.copy2(ROOT / "packaging" / f"README-{os_name}.txt", staging / "README.txt")
            if os_name == "windows":
                for filename in ("Launch-LMU.cmd", "Launch-with-logs.cmd"):
                    shutil.copy2(ROOT / "packaging" / filename, staging / filename)
                # CMD files need Windows line endings even in a Linux checkout.
                for path in staging.iterdir():
                    if path.suffix in (".cmd", ".txt", ".toml"):
                        path.write_bytes(path.read_text().replace("\r\n", "\n").replace("\n", "\r\n").encode())
            shutil.copy2(ROOT / "docs" / "INSTALL.md", staging / "INSTALL.md")
            (staging / "THIRD_PARTY_NOTICES.txt").write_text(dependency_notices(target, cargo_flags, env), encoding="utf-8")
            info = {
                "application": "Race2Love", "version": version, "target": target,
                "profile": "release", "rustc": rustc, "source_commit": commit,
                "source_dirty": dirty,
                "build_host": {"system": host, "release": platform.release(), "libc": platform.libc_ver() if host == "Linux" else None},
                "built_at_utc": datetime.now(timezone.utc).isoformat(),
                "executable_sha256": sha256(binary),
            }
            (staging / "build-info.json").write_text(json.dumps(info, indent=2) + "\n")
            archive = temporary / (name + extension)
            if extension == ".zip":
                with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
                    for path in sorted(staging.iterdir()):
                        bundle.write(path, f"{name}/{path.name}")
                with zipfile.ZipFile(archive) as bundle:
                    if bundle.testzip() is not None:
                        raise RuntimeError("ZIP integrity check failed")
            else:
                with tarfile.open(archive, "w:gz", compresslevel=9) as bundle:
                    bundle.add(staging, arcname=name)
            destination = output / archive.name
            os.replace(archive, destination)
            archives.append(destination)
    checksums = "".join(sha256(path) + "  " + path.name + "\n" for path in archives)
    (output / "SHA256SUMS.txt").write_text(checksums)
    print("\nReady to share:")
    for path in archives:
        print(f"  {path} ({path.stat().st_size / 1024 / 1024:.1f} MiB)")
    print(f"  {output / 'SHA256SUMS.txt'}")
    print("Validate before distribution: python3 tools/verify-release.py --directory " + str(output))


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, OSError, RuntimeError) as error:
        print(f"Release failed: {error}", file=sys.stderr)
        sys.exit(1)
