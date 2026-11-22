#!/usr/bin/env python3
"""Collect finished Tauri bundles and produce the matching installation and source archives."""
import argparse
from pathlib import Path
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SLUG = "Repo-Uploader-and-Manager-V3"
ROOT_FILES = (
    "LICENSE", "README.md", "THIRD-PARTY.txt", ".gitignore", "index.html",
    "package.json", "package-lock.json", "tsconfig.json", "vite.config.ts",
    "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/build.rs",
    "src-tauri/tauri.conf.json",
)
SOURCE_DIRS = ("src", "docs", "scripts", "tests", "src-tauri/src", "src-tauri/capabilities",
               "src-tauri/icons", "src-tauri/templates")
SOURCE_EXTENSIONS = {".svelte", ".ts", ".css", ".md", ".mjs", ".js", ".py", ".rs", ".json", ".png", ".ico", ".txt"}


def sources():
    result = [ROOT / name for name in ROOT_FILES]
    for directory in SOURCE_DIRS:
        for path in sorted((ROOT / directory).rglob("*")):
            if path.is_symlink():
                raise ValueError(f"Unexpected symbolic link: {path.relative_to(ROOT)}")
            if path.is_dir():
                continue
            if path.suffix not in SOURCE_EXTENSIONS and path.parent != ROOT / "src-tauri/templates/ignores":
                raise ValueError(f"Review unexpected source file: {path.relative_to(ROOT)}")
            result.append(path)
    return sorted(result)


def source_zip(destination):
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sources():
            name = path.relative_to(ROOT).as_posix()
            entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, path.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def bundle(base, kind, pattern):
    matches = list((base/"bundle"/kind).glob(pattern))
    if len(matches) != 1:
        raise ValueError(f"Expected one {kind} bundle. Found {len(matches)}.")
    return matches[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux-release", type=Path, required=True)
    parser.add_argument("--windows-release", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="V3 output directory outside the source tree")
    args = parser.parse_args()
    out = args.output.resolve()
    if out == ROOT or ROOT in out.parents:
        parser.error("Choose a release directory outside the public source.")
    deb = bundle(args.linux_release, "deb", "*amd64.deb")
    packages = {
        "Windows": (out/"Windows", bundle(args.windows_release, "nsis", "*x64-setup.exe"), "-Setup.exe"),
        "DEB": (out/"Linux/DEB", deb, ".deb"),
        "RPM": (out/"Linux/RPM", bundle(args.linux_release, "rpm", "*x86_64.rpm"), ".rpm"),
        "TAR": (out/"Linux/TAR", deb, ".tar.gz"),
    }
    first = None
    for kind, (folder, path, ext) in packages.items():
        folder.mkdir(parents=True, exist_ok=True)
        destination = folder/(SLUG+ext)
        if kind == "TAR":
            # Use the installed filesystem layout, including the desktop entry and icons.
            payload = subprocess.check_output(["ar", "p", path, "data.tar.gz"])
            if not payload.startswith(b"\x1f\x8b"):
                raise ValueError("Expected a gzip-compressed data.tar.gz in the DEB.")
            destination.write_bytes(payload)
        else:
            shutil.copyfile(path, destination)
        archive = folder/f"{SLUG}-{kind}-Source.zip"
        if first is None:
            source_zip(archive)
            first = archive
        else:
            shutil.copyfile(first, archive)


if __name__ == "__main__":
    main()
