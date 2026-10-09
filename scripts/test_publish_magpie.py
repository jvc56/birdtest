import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publish', Path(__file__).with_name('publish-magpie.py'))
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)
VERSION = 'wasm-preview-v0.1.0'


class ReleaseTests(unittest.TestCase):
    def bundle(self, path, corrupt=False, extra=None):
        files = {name: b'asset' for name in [
            'wasmentry/index.html', 'wasmentry/magpie_wasm.mjs', 'wasmentry/magpie_wasm.wasm',
            'data/layouts/standard15.txt', 'data/letterdistributions/english.csv',
            'data/strategy/winpct_english.csv',
            *[f'data/lexica/{lexicon}.{extension}' for lexicon in ('CSW24', 'NWL23')
              for extension in ('kwg', 'klv2')]]}
        manifest = {'version': VERSION, 'source_revision': 'a' * 40,
                    'files': {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}}
        if corrupt:
            files['wasmentry/magpie_wasm.wasm'] = b'corrupt'
        files['release.json'] = json.dumps(manifest).encode()
        if extra:
            files[extra] = b'unsafe'
        with tarfile.open(path, 'w:gz') as archive:
            for name, data in files.items():
                member = tarfile.TarInfo('./' + name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_checksums_and_version_before_upload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.bundle(root / 'app.tgz')
            manifest = publish.unpack(root / 'app.tgz', root / 'assets', VERSION)
            self.assertEqual(manifest['version'], VERSION)
            with self.assertRaises(ValueError):
                publish.unpack(root / 'app.tgz', root / 'assets', 'wasm-preview-v0.2.0')
            self.bundle(root / 'app.tgz', corrupt=True)
            with self.assertRaisesRegex(ValueError, 'Checksum mismatch'):
                publish.unpack(root / 'app.tgz', root / 'assets', VERSION)
            self.bundle(root / 'app.tgz', extra='../escape')
            with self.assertRaisesRegex(ValueError, 'Unsafe archive'):
                publish.unpack(root / 'app.tgz', root / 'assets', VERSION)

    def test_upload_retries_without_replacing_existing_content(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'engine.wasm'
            path.write_bytes(b'wasm')
            missing = subprocess.CalledProcessError(1, 'aws', stderr='(404) Not Found')
            with patch.object(publish, 'run', side_effect=[missing, '{}']) as run:
                publish.upload_file('bucket', VERSION, path, 'wasmentry/engine.wasm')
                put = run.call_args.args
                self.assertIn('--if-none-match', put)
                self.assertIn('application/wasm', put)
            digest = hashlib.sha256(b'wasm').hexdigest()
            with patch.object(publish, 'run', return_value=json.dumps({'Metadata': {'sha256': digest}})) as run:
                publish.upload_file('bucket', VERSION, path, 'wasmentry/engine.wasm')
                self.assertEqual(run.call_count, 1)
            with patch.object(publish, 'run', return_value='{}'):
                with self.assertRaisesRegex(ValueError, 'Refusing to overwrite'):
                    publish.upload_file('bucket', VERSION, path, 'wasmentry/engine.wasm')
            denied = subprocess.CalledProcessError(1, 'aws', stderr='(403) Forbidden')
            with patch.object(publish, 'run', side_effect=denied) as run:
                with self.assertRaises(subprocess.CalledProcessError):
                    publish.upload_file('bucket', VERSION, path, 'wasmentry/engine.wasm')
                self.assertEqual(run.call_count, 1)


if __name__ == '__main__':
    unittest.main()
