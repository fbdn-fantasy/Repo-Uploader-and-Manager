#!/usr/bin/env python3
"""Package the compiled Linux application and the Tauri Windows NSIS installer.

Build the frontend and Linux release with npm run tauri build -- --no-bundle.
Build Windows with npm run tauri build -- --runner cargo-xwin
--target x86_64-pc-windows-msvc --bundles nsis (one command).
Run this script on Linux with GNU ar and rpmbuild.
"""
import argparse
import json
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
VERSION = CONFIG['version']
PRODUCT = CONFIG['productName']
NAME = 'repo-uploader-and-manager'
BINARY = f'Repo-Uploader-and-Manager-V{VERSION}'
LICENSE = CONFIG['bundle']['license']
SOURCE_DIRS = ('src', 'docs', 'scripts', 'tests', 'src-tauri/src', 'src-tauri/capabilities',
               'src-tauri/icons', 'src-tauri/templates')
ROOT_FILES = ('LICENSE', 'NOTICE', 'README.md', 'THIRD-PARTY.txt', '.gitignore', 'index.html',
              'package.json', 'package-lock.json', 'tsconfig.json', 'vite.config.ts',
              'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs',
              'src-tauri/tauri.conf.json')
EXTENSIONS = {'.svelte', '.ts', '.css', '.md', '.mjs', '.js', '.py', '.rs', '.json',
              '.png', '.ico', '.txt'}


def source_zip(destination):
    files = [ROOT / name for name in ROOT_FILES]
    for directory in SOURCE_DIRS:
        for path in sorted((ROOT / directory).rglob('*')):
            if '__pycache__' in path.parts:
                continue
            if path.is_symlink():
                raise ValueError(f'Unexpected source symlink: {path.relative_to(ROOT)}')
            if path.is_file():
                if path.suffix not in EXTENSIONS and path.parent != ROOT / 'src-tauri/templates/ignores':
                    raise ValueError(f'Review unexpected source file: {path.relative_to(ROOT)}')
                files.append(path)
    with zipfile.ZipFile(destination, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for file in sorted(files):
            entry = zipfile.ZipInfo(file.relative_to(ROOT).as_posix())
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, file.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def linux_tree(root, elf):
    bindir = root / 'usr/bin'
    bindir.mkdir(parents=True)
    compiled = bindir / BINARY
    shutil.copyfile(elf, compiled)
    compiled.chmod(0o755)
    launcher = bindir / NAME
    launcher.write_text(f'#!/bin/sh\nexec "$(dirname -- "$0")/{BINARY}" "$@"\n')
    launcher.chmod(0o755)
    resources = root / 'usr/lib' / PRODUCT
    resources.mkdir(parents=True)
    for name in ('LICENSE', 'NOTICE', 'THIRD-PARTY.txt'):
        shutil.copyfile(ROOT / name, resources / name)
    applications = root / 'usr/share/applications'
    applications.mkdir(parents=True)
    (applications / (CONFIG['identifier'] + '.desktop')).write_text(
        f'[Desktop Entry]\nType=Application\nName={PRODUCT}\n'
        f'Comment=Manage local Git working trees and GitHub repositories\n'
        f'Exec={NAME}\nIcon={NAME}\nTerminal=false\nCategories=Development;RevisionControl;\n'
        f'StartupNotify=true\nStartupWMClass={NAME}\n')
    for size in ('32x32', '128x128', '256x256'):
        icons = root / 'usr/share/icons/hicolor' / size / 'apps'
        icons.mkdir(parents=True)
        shutil.copyfile(ROOT / f'src-tauri/icons/{size}.png', icons / f'{NAME}.png')


def tar_gz(folder, archive):
    def normalize(member):
        member.uid = member.gid = member.mtime = 0
        member.uname = member.gname = 'root'
        return member
    with tarfile.open(archive, 'w:gz', compresslevel=9) as tar:
        for path in sorted(folder.rglob('*')):
            tar.add(path, arcname='./' + path.relative_to(folder).as_posix(), recursive=False, filter=normalize)


def deb_package(work, tree, data_archive, output):
    control = work / 'deb-control'
    control.mkdir()
    deps = ', '.join(CONFIG['bundle']['linux']['deb']['depends'])
    installed_size = sum(p.stat().st_size for p in tree.rglob('*') if p.is_file())
    # V1.0.0 starts the new numbering scheme after historical version 3.0.0.
    (control / 'control').write_text(
        f'Package: {NAME}\nVersion: 1:{VERSION}\nArchitecture: amd64\nSection: devel\n'
        f'Priority: optional\nMaintainer: Forbidden_Fantasy\nLicense: {LICENSE}\n'
        f'Installed-Size: {(installed_size+1023)//1024}\nDepends: {deps}\n'
        f'Description: {CONFIG["bundle"]["shortDescription"]}\n {CONFIG["bundle"]["longDescription"]}\n')
    (work / 'debian-binary').write_bytes(b'2.0\n')
    tar_gz(control, work / 'control.tar.gz')
    subprocess.run(['ar', 'rcD', str(output), 'debian-binary', 'control.tar.gz', data_archive.name],
                   check=True, cwd=work)


def rpm_package(work, tree, output):
    # The staging directory has no spaces. Quote paths in the generated spec anyway.
    requires = '\n'.join('Requires: ' + dep for dep in CONFIG['bundle']['linux']['rpm']['depends'])
    paths = [p for p in sorted(tree.rglob('*')) if p.is_file()]
    entries = [f'"/{p.relative_to(tree).as_posix()}"' for p in paths]
    spec = work / 'application.spec'
    spec.write_text(
        f'Name: {NAME}\nVersion: {VERSION}\nRelease: 1\nEpoch: 1\nLicense: {LICENSE}\n'
        f'Summary: {CONFIG["bundle"]["shortDescription"]}\nBuildArch: x86_64\n'
        f'Prefix: /usr\n{requires}\n%global debug_package %{{nil}}\n'
        f'%description\n{CONFIG["bundle"]["longDescription"]}\n'
        f'%install\nmkdir -p "%{{buildroot}}"\ncp -a {shlex.quote(str(tree))}/usr "%{{buildroot}}/"\n'
        f'%files\n%dir "/usr/lib/{PRODUCT}"\n' + '\n'.join(entries) + '\n')
    subprocess.run(['rpmbuild', '-bb', '--quiet', '--define', f'_topdir {work}/rpm',
                    '--define', f'_tmppath {work}', '--define', '_buildhost reproducible', '--define', '__os_install_post %{nil}', str(spec)],
                   check=True)
    shutil.copyfile(next((work / 'rpm/RPMS/x86_64').glob('*.rpm')), output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--linux-binary', type=Path)
    parser.add_argument('--windows-installer', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.linux_binary and not args.windows_installer:
        parser.error('Supply a Linux release binary, a Windows installer, or both.')
    if BINARY != CONFIG['mainBinaryName']:
        parser.error('Match mainBinaryName to the release version before packaging.')
    out = args.output.resolve()
    if ROOT == out or ROOT in out.parents:
        parser.error('Choose a release folder outside the public source.')
    dirs = {kind: out / 'Linux' / kind for kind in ('RPM', 'DEB', 'TAR')}
    dirs['Windows'] = out / 'Windows'
    with tempfile.TemporaryDirectory(prefix='repo-manager-package-') as temp:
        work = Path(temp)
        packages = []
        if args.linux_binary:
            tree = work / 'tree'
            linux_tree(tree, args.linux_binary)
            data = work / 'data.tar.gz'
            tar_gz(tree, data)
            deb = work / 'application.deb'
            deb_package(work, tree, data, deb)
            rpm = work / f'{NAME}-{VERSION}-1.x86_64.rpm'
            rpm_package(work, tree, rpm)
            packages.extend([('DEB', deb, BINARY+'.deb'), ('RPM', rpm, rpm.name), ('TAR', data, BINARY+'.tar.gz')])
        if args.windows_installer:
            packages.append(('Windows', args.windows_installer, BINARY+'-Setup.exe'))
        sources = work / 'source.zip'
        source_zip(sources)
        for kind, source, name in packages:
            dirs[kind].mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, dirs[kind] / name)
            shutil.copyfile(sources, dirs[kind] / f'{BINARY}-{kind}-Source.zip')


if __name__ == '__main__':
    main()
