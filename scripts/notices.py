#!/usr/bin/env python3
"""Regenerate THIRD-PARTY.txt on a Fedora cross-build host after cargo fetch and npm ci.

Provide a writable cache with --cache. Missing upstream artifacts are fetched over
HTTPS and verified by SHA-256. Existing checked-in notices suffice for building.
"""
import argparse
import hashlib
import io
import json
import re
import subprocess
import tarfile
import urllib.request
import zipfile
from collections import defaultdict
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc")
SUPPLEMENTS = {
    "dlopen": (
        "https://raw.githubusercontent.com/OpenByteDev/dlopen2/cc80e4a0a90d499b677fdf7743699b4b3a43a989/LICENSE",
        "39fa265207450e77c62e90c5594a06c085b655d8374c7ced4bf7894b6bd95dd2",
    ),
    "webview-rust": (
        "https://raw.githubusercontent.com/wravery/webview2-rs/edc2caf886175ccaebe86078c9cfe1ae2a187328/LICENSE",
        "0dcf41516e608bbcb6cdc5229feb7b86fe4a643b85e7df251133c93408fdac73",
    ),
    "sdk.zip": (
        "https://api.nuget.org/v3-flatcontainer/microsoft.web.webview2/1.0.3800.47/microsoft.web.webview2.1.0.3800.47.nupkg",
        "56c9f26bdd07916a2d1949fb58a5c7e434dfa1173577dca879206050c4e718db",
    ),
    "nsis-utils-src": (
        "https://codeload.github.com/tauri-apps/nsis-tauri-utils/tar.gz/refs/tags/nsis_tauri_utils-v0.5.3",
        "34150c4cee737baca022a18d4b0483c5e8f684924249f0580fbef3d6dfe970d4",
    ),
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def supplement(cache, name):
    url, expected = SUPPLEMENTS[name]
    file = cache / name
    if not file.is_file():
        with urllib.request.urlopen(url, timeout=40) as response:
            data = response.read()
        if digest(data) != expected:
            raise ValueError(f"Unexpected upstream contents for {name}")
        file.write_bytes(data)
    data = file.read_bytes()
    if digest(data) != expected:
        raise ValueError(f"Unexpected cached contents for {name}")
    return data


def unpack_text(data):
    if data.startswith((b"\xff\xfe", b"\xfe\xff")):
        data = data.decode("utf-16")
    else:
        data = data.decode("utf-8-sig")
    return data.replace("\r\n", "\n").replace("\r", "\n").strip()


class Collection:
    def __init__(self):
        self.entries = defaultdict(list)
        self.texts = {}

    def add(self, component, source, data):
        content = unpack_text(data)
        if not content:
            raise ValueError(f"Empty license for {component}")
        code = digest(content.encode())[:16]
        if code in self.texts and self.texts[code] != content:
            raise ValueError("Signature collision")
        self.texts[code] = content
        self.entries[component].append((source, code))

    def write(self, path):
        header = [
            "Repo Uploader and Manager — third-party licenses and notices",
            "",
            "Component index",
            "",
            "The index covers the Linux and Windows Rust dependency graphs, frontend",
            "runtime packages, and installer components. Each entry identifies its",
            "license and notices by their identifiers in the text section below.",
            "Some components are only used during the build or on one platform.",
            "Original upstream copyrights and contact details are reproduced here.",
            "The linked versioned package pages provide the unmodified upstream source.",
            "The Windows installer includes Microsoft's Evergreen WebView2 bootstrapper.",
            "Microsoft distributes the WebView2 Runtime under its own terms.",
            "https://developer.microsoft.com/microsoft-edge/webview2/",
            "https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution",
            "",
        ]
        for component, notices in sorted(self.entries.items()):
            header.append(component)
            header.extend(f"  {file}  [{code}]" for file, code in sorted(set(notices)))
        header += ["", "License and notice texts", ""]
        for code, content in sorted(self.texts.items()):
            header += [f"[{code}]", content, ""]
        path.write_text("\n".join(header), encoding="utf-8")


def licenses_in(folder, recursive=True):
    candidates = folder.rglob("*") if recursive else folder.iterdir()
    legal = re.compile(r"^(licen[cs]es?|copying|copyright|notices?|unlicense)(?:$|[._-])", re.I)
    return sorted(x for x in candidates
                  if x.is_file() and legal.match(x.name)
                  and x.suffix.lower() not in (".rs", ".js", ".ts", ".py", ".go"))


def cargo_packages():
    packages = {}
    for target in TARGETS:
        cmd = ["cargo", "metadata", "--locked", "--offline", "--format-version", "1",
               "--filter-platform", target, "--features", "custom-protocol",
               "--manifest-path", str(ROOT / "src-tauri/Cargo.toml")]
        metadata = json.loads(subprocess.check_output(cmd))
        nodes = {node["id"] for node in metadata["resolve"]["nodes"]}
        packages.update((p["id"], p) for p in metadata["packages"] if p["source"] and p["id"] in nodes)
    return list(packages.values())


def cargo_notices(book, cache, packages):
    by_name = {p["name"]: Path(p["manifest_path"]).parent for p in packages}
    for package in packages:
        name, version = package["name"], package["version"]
        folder = Path(package["manifest_path"]).parent
        title = f"Rust {name} {version} | {package['license'] or 'see license file'} | https://crates.io/crates/{name}/{version}"
        files = licenses_in(folder)
        if package["license_file"]:
            files.append(folder / package["license_file"])
        for file in set(files):
            book.add(title, file.relative_to(folder).as_posix(), file.read_bytes())
        if files:
            continue
        # These published workspaces place their notices in a sibling crate.
        if name in ("alloc-stdlib", "libappindicator-sys"):
            sibling = by_name[{"alloc-stdlib": "alloc-no-stdlib", "libappindicator-sys": "libappindicator"}[name]]
            for file in licenses_in(sibling, recursive=False):
                book.add(title, f"Workspace {file.name}", file.read_bytes())
        elif name in ("dlopen2", "dlopen2_derive") or name.startswith("webview2-com"):
            key = "dlopen" if name.startswith("dlopen") else "webview-rust"
            book.add(title, "Upstream workspace LICENSE", supplement(cache, key))
        elif name == "selectors":
            book.add(title, "MPL 2.0", (ROOT / "src-tauri/templates/licenses/mpl-2.0.txt").read_bytes())
        else:
            raise ValueError(f"No notice source for {name} {version}")


def js_notices(book):
    packages = json.loads((ROOT / "package-lock.json").read_text())["packages"]
    for key, record in packages.items():
        if not key or record.get("dev"):
            continue
        folder = ROOT / key
        if not folder.is_dir():
            raise ValueError(f"Install frontend dependencies before collecting notices for {key}")
        package = json.loads((folder / "package.json").read_text())
        name, version = package["name"], record["version"]
        title = f"JavaScript {name} {version} | {record['license']} | https://www.npmjs.com/package/{name}/v/{version}"
        files = licenses_in(folder, recursive=False)
        for file in files:
            book.add(title, file.name, file.read_bytes())
        if files:
            continue
        if name not in ("is-reference", "locate-character") or package["license"] != "MIT":
            raise ValueError(f"No notice source for {name} {version}")
        # These releases state MIT and name their author in package.json. No
        # copyright year or separate license text was included in the tarball.
        body = (ROOT / "src-tauri/templates/licenses/mit.txt").read_text().split("Permission is hereby", 1)[1]
        book.add(title, "package.json author and MIT terms",
                 f"Author: {package['author']}\n\nPermission is hereby{body}".encode())
    # Vite can emit its modulepreload polyfill into the browser bundle.
    vite = ROOT / "node_modules/vite"
    book.add(f"Vite {packages['node_modules/vite']['version']} | MIT | https://github.com/vitejs/vite",
             "LICENSE.md", (vite / "LICENSE.md").read_bytes())


class HTMLText(HTMLParser):
    """Preserve the toolchain's legal texts, path tables and external links."""
    def __init__(self):
        super().__init__()
        self.parts = []

    def handle_data(self, data):
        self.parts.append(data)

    def handle_starttag(self, tag, attrs):
        if tag in ("br", "p", "li", "h1", "h2", "h3", "pre"):
            self.parts.append("\n")
        if tag == "a":
            href = dict(attrs).get("href", "")
            if href.startswith(("https://", "http://")):
                self.parts.append(f"<{href}> ")


def runtime_notices(book, cache, packages, nsis_license):
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    docs = sysroot / "share/doc/rust"
    rust = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
    component = f"Rust standard library {rust} | https://github.com/rust-lang/rust/releases/tag/{rust}"
    html = HTMLText()
    html.feed((docs / "COPYRIGHT-library.html").read_text())
    info = re.sub(r"(\n[ \t]*){3,}", "\n\n", "".join(html.parts))
    book.add(component, "COPYRIGHT-library.html (text)", info.encode())
    for file in sorted((docs / "licenses").glob("*.txt")):
        book.add(component, file.name, file.read_bytes())

    sdk_name = "Microsoft WebView2 SDK loader 1.0.3800.47 | https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3800.47"
    webview = next(Path(p["manifest_path"]).parent for p in packages if p["name"] == "webview2-com-sys")
    with zipfile.ZipFile(io.BytesIO(supplement(cache, "sdk.zip"))) as sdk:
        if sdk.read("build/native/x64/WebView2LoaderStatic.lib") != (webview / "x64/WebView2LoaderStatic.lib").read_bytes():
            raise ValueError("The WebView2 loader no longer matches the pinned SDK")
        for file in ("LICENSE.txt", "NOTICE.txt"):
            book.add(sdk_name, file, sdk.read(file))

    nsis_version = subprocess.check_output(["makensis", "-VERSION"], text=True).strip()
    book.add(f"NSIS {nsis_version} | https://nsis.sourceforge.io/Download", "COPYING", nsis_license.read_bytes())
    component = "Tauri NSIS utilities 0.5.3 | https://github.com/tauri-apps/nsis-tauri-utils/releases/tag/nsis_tauri_utils-v0.5.3"
    with tarfile.open(fileobj=io.BytesIO(supplement(cache, "nsis-utils-src")), mode="r:gz") as utils:
        for info in utils.getmembers():
            if Path(info.name).name in ("LICENSE_APACHE-2.0", "LICENSE_MIT"):
                book.add(component, Path(info.name).name, utils.extractfile(info).read())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", required=True, type=Path)
    parser.add_argument("--nsis-license", type=Path, default=Path("/usr/share/licenses/mingw-nsis-base/COPYING"))
    parser.add_argument("--out", type=Path, default=ROOT / "THIRD-PARTY.txt")
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    collection = Collection()
    packages = cargo_packages()
    cargo_notices(collection, args.cache, packages)
    js_notices(collection)
    runtime_notices(collection, args.cache, packages, args.nsis_license)
    collection.write(args.out)
    print(f"{len(collection.entries)} components, {len(collection.texts)} distinct notice texts.")


if __name__ == "__main__":
    main()
