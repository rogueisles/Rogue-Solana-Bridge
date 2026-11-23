import hashlib
import os
from pathlib import Path
import posixpath
import shutil
import subprocess
import tarfile

from pins import (AGAVE_ARCHIVE, AGAVE_SHA256, BUILD_ROOT, PLATFORM_ARCHIVE,
                  PLATFORM_SHA256, PUBLIC_SOURCE_SHA256, TOOL_SHA256)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while chunk := stream.read(1024 * 1024):
            value.update(chunk)
    return value.hexdigest()


def archive_name(name):
    require(isinstance(name, str) and 0 < len(name) < 4096 and '\x00' not in name and '\\' not in name,
            'Unexpected archive path encoding')
    require(not name.startswith('/'), 'Absolute archive path')
    while name.startswith('./'):
        name = name[2:]
    if name in ('', '.'):
        return ''
    name = name.rstrip('/')
    require(all(part not in ('', '.', '..') for part in name.split('/')), 'Archive traversal or ambiguous path')
    return name


def extract(path, target, expected, prefix):
    require(digest(path) == expected, 'Pinned public archive checksum mismatch')
    target.mkdir(parents=True, exist_ok=False)
    with tarfile.open(path, 'r:bz2') as archive:
        members = archive.getmembers()
        require(len(members) < 100000, 'Archive member limit')
        require(sum(m.size for m in members if m.isfile()) < 4 * 1024**3, 'Expanded archive limit')
        if prefix == 'platform-tools' and any(archive_name(m.name) == 'rust/bin/rustc' for m in members):
            prefix = ''
        names = set()
        links = []
        for member in members:
            name = archive_name(member.name)
            if not name:
                require(member.isdir(), 'Archive root marker must be a directory')
                continue
            require(not prefix or name == prefix or name.startswith(prefix + '/'), 'Unexpected archive root')
            relative = name[len(prefix):].lstrip('/') if prefix else name
            if not relative:
                continue
            require(relative not in names, 'Duplicate archive path')
            names.add(relative)
            out = target / relative
            require(out.is_relative_to(target), 'Archive target escape')
            out.parent.mkdir(parents=True, exist_ok=True)
            if member.isdir():
                out.mkdir(exist_ok=True)
            elif member.isfile():
                with archive.extractfile(member) as source, out.open('xb') as output:
                    shutil.copyfileobj(source, output)
                os.chmod(out, member.mode & 0o755)
            elif member.issym() or member.islnk():
                links.append((out, member))
            else:
                raise ValueError('Unexpected archive special file')
        for out, member in links:
            require(not member.linkname.startswith('/'), 'Absolute archive link')
            if member.issym():
                linked = posixpath.normpath(str(out.parent / member.linkname))
                require(Path(linked).is_relative_to(target), 'Archive link escape')
                out.symlink_to(member.linkname)
            else:
                linkname = archive_name(member.linkname)
                require(not prefix or linkname.startswith(prefix + '/'), 'Archive hardlink root')
                relative_link = linkname[len(prefix) + 1:] if prefix else linkname
                origin = target / relative_link
                require(origin.is_relative_to(target) and origin.is_file(), 'Archive hardlink target')
                os.link(origin, out)


def main():
    root = Path(BUILD_ROOT)
    require(not root.exists(), 'Build root must not preexist in the base image')
    extract(Path('/rgn-downloads') / AGAVE_ARCHIVE, root / 'tools/solana-release',
            AGAVE_SHA256, 'solana-release')
    extract(Path('/rgn-downloads') / PLATFORM_ARCHIVE, root / 'build-tools/v1.57/platform-tools',
            PLATFORM_SHA256, 'platform-tools')
    for name, sha in TOOL_SHA256.items():
        require(digest(root / name) == sha, 'Qualified compiler binary mismatch: ' + name)
    source = Path('/rgn-public-source')
    for name, sha in PUBLIC_SOURCE_SHA256.items():
        require(digest(source / name) == sha, 'Qualified Rust source mismatch')
    env = os.environ.copy()
    env.update(CARGO_HOME=str(root / 'cargo-home'),
               RUSTUP_HOME=str(root / 'cache/rustup'), CARGO_BUILD_JOBS='1')
    platform = root / 'build-tools/v1.57/platform-tools/rust'
    subprocess.run(['rustup', 'toolchain', 'link', 'rgn-sbf', str(platform)], env=env, check=True)
    subprocess.run(['rustup', 'default', 'rgn-sbf'], env=env, check=True)
    subprocess.run([str(platform / 'bin/cargo'), 'fetch', '--locked', '--manifest-path',
                    str(source / 'combined/Cargo.toml')], env=env, check=True, timeout=900)
    for name, sha in PUBLIC_SOURCE_SHA256.items():
        require(digest(source / name) == sha, 'Cargo changed qualified source')
    (root / 'solana').mkdir()


if __name__ == '__main__':
    main()
