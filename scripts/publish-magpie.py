#!/usr/bin/env python3
"""Upload one verified MAGPIE GitHub release; activating it is a separate Terraform apply."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile

ARCHIVE = 'magpie-wasm-preview.tar.gz'
VERSION = r'wasm-preview-v\d+\.\d+\.\d+(?:-[a-z0-9.]+)?'
TYPES = {'.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript',
         '.wasm': 'application/wasm', '.css': 'text/css', '.json': 'application/json',
         '.csv': 'text/csv', '.txt': 'text/plain', '.kwg': 'application/octet-stream',
         '.klv2': 'application/octet-stream'}


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def unpack(archive, destination, version):
    """Validate the entire bundle before writing anything to S3. Never extract links."""
    with tarfile.open(archive, 'r:gz') as bundle:
        members = {}
        for member in bundle.getmembers():
            path = PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not (member.isfile() or member.isdir()):
                raise ValueError(f'Unsafe archive member: {member.name}')
            if member.isfile():
                if str(path) in members:
                    raise ValueError(f'Duplicate archive member: {member.name}')
                members[str(path)] = member
        manifest_bytes = bundle.extractfile(members['release.json']).read()
        manifest = json.loads(manifest_bytes)
        if manifest['version'] != version or not re.fullmatch(r'[0-9a-f]{40}', manifest['source_revision']):
            raise ValueError('Release version or source revision does not match the release contract')
        files = manifest['files']
        required = {'wasmentry/index.html', 'wasmentry/magpie_wasm.mjs', 'wasmentry/magpie_wasm.wasm',
                    'data/layouts/standard15.txt', 'data/letterdistributions/english.csv',
                    'data/strategy/winpct_english.csv'}
        required.update(f'data/lexica/{lexicon}.{extension}' for lexicon in ('CSW24', 'NWL23')
                        for extension in ('kwg', 'klv2'))
        if not required <= files.keys():
            raise ValueError('Incomplete preview bundle')
        for name, expected in files.items():
            path = PurePosixPath(name)
            if (str(path) != name or path.is_absolute() or '..' in path.parts
                    or path.parts[0] not in ('wasmentry', 'data') or path.suffix not in TYPES):
                raise ValueError(f'Unexpected public asset: {name}')
            data = bundle.extractfile(members[name]).read()
            if hashlib.sha256(data).hexdigest() != expected:
                raise ValueError(f'Checksum mismatch: {name}')
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        (destination / 'release.json').write_bytes(manifest_bytes)
    return manifest


def upload_file(bucket, version, path, name):
    key = f'releases/{version}/{name}'
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    try:
        existing = json.loads(run('aws', 's3api', 'head-object', '--bucket', bucket, '--key', key,
                                  '--output', 'json', '--no-cli-pager'))
    except subprocess.CalledProcessError as error:
        # AccessDenied, expired credentials and network failures are not absence.
        if '(404)' not in error.stderr and '(NotFound)' not in error.stderr:
            raise
    else:
        if existing.get('Metadata', {}).get('sha256') != digest:
            raise ValueError(f'Refusing to overwrite a different published file: {key}')
        return
    run('aws', 's3api', 'put-object', '--bucket', bucket, '--key', key, '--body', str(path),
        '--if-none-match', '*', '--metadata', f'sha256={digest}',
        '--content-type', TYPES[path.suffix], '--cache-control', 'public,max-age=31536000,immutable',
        '--no-cli-pager')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('version')
    parser.add_argument('--bucket', required=True, help='infra/magpie Terraform bucket output')
    args = parser.parse_args()
    if not re.fullmatch(VERSION, args.version):
        parser.error('Expected a wasm-preview-vMAJOR.MINOR.PATCH tag')
    with tempfile.TemporaryDirectory(prefix='magpie-release-') as directory:
        work = Path(directory)
        run('gh', 'release', 'download', args.version, '--repo', 'jvc56/MAGPIE', '--dir', directory,
            '--pattern', ARCHIVE, '--pattern', ARCHIVE + '.sha256')
        expected, filename = (work / (ARCHIVE + '.sha256')).read_text().split()
        if filename != ARCHIVE or hashlib.sha256((work / ARCHIVE).read_bytes()).hexdigest() != expected:
            raise ValueError('Release archive checksum mismatch')
        manifest = unpack(work / ARCHIVE, work / 'assets', args.version)
        for name in manifest['files']:
            upload_file(args.bucket, args.version, work / 'assets' / name, name)
        # This is the completion marker Terraform checks before activation.
        upload_file(args.bucket, args.version, work / 'assets/release.json', 'release.json')
        print(f"Uploaded {args.version} from MAGPIE {manifest['source_revision']}")
        print('Set release in infra/magpie/prod.tfvars, then review and apply its Terraform plan.')


if __name__ == '__main__':
    main()
