import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.path.insert(0, '/rgn-build')
from pins import BUILD_ROOT, PROGRAM_BYTES, PROGRAM_SHA256, PUBLIC_SOURCE_SHA256, TOOL_SHA256


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def exclusive(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    try:
        with os.fdopen(descriptor, 'wb', closefd=False) as output:
            output.write(data)
            output.flush()
            os.fsync(descriptor)
    finally:
        os.close(descriptor)


def main():
    root = Path(BUILD_ROOT)
    incoming = Path('/rgn-public-input')
    source = root / 'solana'
    output = Path('/rgn-output')
    require(source.is_dir() and not any(source.iterdir()), 'Container build source must begin empty')
    require(output.is_dir() and not any(output.iterdir()), 'Reproduction output directory must begin empty')
    directories = {str(Path(n).parent) for n in PUBLIC_SOURCE_SHA256}
    directories.discard('.')
    for item in incoming.rglob('*'):
        name = item.relative_to(incoming).as_posix()
        require(not item.is_symlink() and (name in directories if item.is_dir() else name in PUBLIC_SOURCE_SHA256),
                'Unexpected public input entry')
    for name, expected in PUBLIC_SOURCE_SHA256.items():
        original = incoming / name
        require(original.is_file() and digest(original) == expected, 'Publication source checksum mismatch: ' + name)
        target = source / name
        target.parent.mkdir(parents=True, exist_ok=True)
        exclusive(target, original.read_bytes())
    for name, expected in TOOL_SHA256.items():
        require(digest(root / name) == expected, 'Qualified compiler checksum mismatch: ' + name)
    env = os.environ.copy()
    env.update(CARGO_HOME=str(root / 'cargo-home'), RUSTUP_HOME=str(root / 'cache/rustup'),
               CARGO_TARGET_DIR=str(source / 'combined/target'), CARGO_NET_OFFLINE='true',
               CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_OPT_LEVEL='s',
               SOLANA_PLATFORM_TOOLS_DIR=str(root / 'build-tools'))
    args = [str(root / 'tools/solana-release/bin/cargo-build-sbf'), '--manifest-path',
            str(source / 'combined/Cargo.toml'), '--tools-version', 'v1.57', '--skip-tools-install',
            '--no-rustup-override', '--arch', 'v0', '--jobs', '1', '--offline', '--', '--locked']
    started = int(time.time())
    subprocess.run(args, cwd=source, env=env, timeout=1800, check=True)
    for name, expected in PUBLIC_SOURCE_SHA256.items():
        require(digest(source / name) == expected and digest(incoming / name) == expected,
                'Compiler changed qualified source')
    elf = source / 'combined/target/deploy/rgn_spl_bridge.so'
    data = elf.read_bytes()
    actual = hashlib.sha256(data).hexdigest()
    identical = actual == PROGRAM_SHA256 and len(data) == PROGRAM_BYTES
    report = {'containerBuild': True, 'networkDuringBuild': False, 'startedAt': started,
              'completedAt': int(time.time()), 'programBytes': len(data), 'programSha256': actual,
              'expectedSha256': PROGRAM_SHA256, 'identicalQualifiedArtifact': identical,
              'solanaVerifyHash': hashlib.sha256(data.rstrip(b'\0')).hexdigest(),
              'sourceSha256': PUBLIC_SOURCE_SHA256, 'compilerSha256': TOOL_SHA256,
              'architecture': 'v0', 'optimization': 's', 'lto': 'fat', 'overflowChecks': True,
              'buildArguments': args, 'cargoHome': env['CARGO_HOME'], 'rustupHome': env['RUSTUP_HOME'],
              'runnerSha256': digest(Path(__file__)), 'explorerVerified': False,
              'mainnetTransactionsSent': False}
    exclusive(output / 'rgn_spl_bridge.so', data)
    exclusive(output / 'container-reproduction.json', (json.dumps(report, indent=2) + '\n').encode())
    print(json.dumps(report, indent=2), flush=True)
    require(identical, 'Container ELF differs from the qualified artifact. Deployment remains blocked.')


if __name__ == '__main__':
    main()
