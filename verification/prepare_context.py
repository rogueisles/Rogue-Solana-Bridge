import argparse
import hashlib
from pathlib import Path
import shutil
import urllib.request

from pins import PUBLIC_SOURCE_SHA256, AGAVE_ARCHIVE, AGAVE_SHA256, AGAVE_URL, PLATFORM_ARCHIVE, PLATFORM_SHA256, PLATFORM_URL

repository = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('destination', type=Path)
destination = parser.parse_args().destination.resolve()
if destination.exists() or destination.is_relative_to(repository):
    raise ValueError('Choose a new build directory outside this repository')
destination.mkdir(parents=True)
for name, expected in PUBLIC_SOURCE_SHA256.items():
    original = repository / name
    if original.is_symlink() or original.resolve() != original:
        raise ValueError('Linked build source is not supported')
    raw = original.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('Source checksum differs: ' + name)
    target = destination / 'public-source/solana' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(raw)
for name in ['Dockerfile.verifiable', '.dockerignore', 'pins.py', 'image_setup.py', 'run_rebuild_v2.py']:
    shutil.copyfile(Path(__file__).parent / name, destination / name)
downloads = destination / 'downloads'
downloads.mkdir()
for name, url, expected in [(AGAVE_ARCHIVE, AGAVE_URL, AGAVE_SHA256), (PLATFORM_ARCHIVE, PLATFORM_URL, PLATFORM_SHA256)]:
    size = 0
    digest = hashlib.sha256()
    with urllib.request.urlopen(url, timeout=45) as response, (downloads / name).open('xb') as output:
        while chunk := response.read(1024 * 1024):
            size += len(chunk)
            if size > 2 * 1024**3:
                raise ValueError('Compiler archive size limit exceeded')
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != expected:
        raise ValueError('Compiler archive checksum differs')
print(destination)
