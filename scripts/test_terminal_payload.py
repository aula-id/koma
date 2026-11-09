import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
import urllib.error
import zipfile

spec = importlib.util.spec_from_file_location('prepare_terminal', Path(__file__).with_name('prepare-terminal.py'))
payload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(payload)


class PayloadValidation(unittest.TestCase):
    def test_missing_stale_wrong_target_and_corrupt(self):
        original = payload.ROOT
        try:
            with tempfile.TemporaryDirectory() as temporary:
                payload.ROOT = Path(temporary)
                target = payload.host_target()
                with self.assertRaises(FileNotFoundError):
                    payload.check(target)
                (payload.ROOT / 'scripts').mkdir()
                (payload.ROOT / 'scripts/prepare-terminal.py').write_text('fixture')
                term = payload.ROOT / 'src-term'
                term.mkdir()
                for name in ['package.json', 'bun.lock', 'tsconfig.json']:
                    (term / name).write_text('{}')
                dest = payload.ROOT / 'target/terminal' / target
                dest.mkdir(parents=True)
                (dest / 'payload.zip').write_bytes(b'archive')
                manifest = dict(target=target, inputs=payload.inputs(), bun_version=payload.VERSION,
                                archive_sha256=payload.digest(dest / 'payload.zip'))
                def save():
                    (dest / 'manifest.json').write_text(json.dumps(manifest))
                save()
                payload.check(target)
                manifest['target'] = 'wrong-target'
                save()
                with self.assertRaisesRegex(ValueError, 'wrong target'):
                    payload.check(target)
                manifest['target'] = target
                save()
                (term / 'bun.lock').write_text('changed')
                with self.assertRaisesRegex(ValueError, 'stale'):
                    payload.check(target)
                manifest['inputs'] = payload.inputs()
                save()
                (dest / 'payload.zip').write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError, 'corrupt'):
                    payload.check(target)
        finally:
            payload.ROOT = original

    def test_five_distinct_pinned_archives(self):
        self.assertEqual(len(payload.TARGETS), 5)
        assets = set()
        for target, (os_name, arch, asset, checksum) in payload.TARGETS.items():
            self.assertIn(os_name, ['linux', 'darwin', 'win32'])
            self.assertIn(arch, ['x64', 'arm64'])
            self.assertEqual(len(checksum), 64)
            int(checksum, 16)
            assets.add(asset)
            github, npm = payload.bun_sources(target)
            self.assertEqual(github, (
                f'https://github.com/oven-sh/bun/releases/download/bun-v{payload.VERSION}/{asset}.zip',
                'sha256',
                checksum,
            ))
            self.assertEqual(npm[0], f'https://registry.npmjs.org/@oven/{asset}/-/{asset}-{payload.VERSION}.tgz')
            self.assertEqual(npm[1], 'sha512')
            self.assertTrue(npm[2].endswith('=='))
        self.assertEqual(set(payload.NPM_SHA512), assets)

    def test_archive_binary_reads_zip_and_npm_tarball(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            zip_path = root / 'bun.zip'
            with zipfile.ZipFile(zip_path, 'w') as bundle:
                bundle.writestr('bun-linux-x64-baseline/bun', b'from-zip')
            self.assertEqual(payload.archive_binary(zip_path, 'bun'), b'from-zip')
            tar_path = root / 'bun.tgz'
            data = b'from-npm'
            with tarfile.open(tar_path, 'w:gz') as bundle:
                info = tarfile.TarInfo('package/bin/bun')
                info.size = len(data)
                bundle.addfile(info, io.BytesIO(data))
            self.assertEqual(payload.archive_binary(tar_path, 'bun'), b'from-npm')
            with self.assertRaisesRegex(ValueError, 'missing bun.exe'):
                payload.archive_binary(tar_path, 'bun.exe')

    def test_download_keeps_the_github_zip_when_it_verifies(self):
        sources = payload.bun_sources('x86_64-unknown-linux-gnu')
        calls = []

        def fake_fetch(url, destination, algo, checksum):
            del algo, checksum
            calls.append(url)
            destination.write_bytes(b'zip')

        name, body = self._download_with(fake_fetch, 'x86_64-unknown-linux-gnu')
        self.assertEqual(calls, [sources[0][0]])
        self.assertTrue(name.endswith('.zip'))
        self.assertEqual(body, b'zip')

    def test_download_uses_npm_after_github_fails(self):
        sources = payload.bun_sources('x86_64-unknown-linux-gnu')
        calls = []

        def fake_fetch(url, destination, algo, checksum):
            del algo, checksum
            calls.append(url)
            if url == sources[0][0]:
                raise urllib.error.URLError('HTTP Error 504: Gateway Time-out')
            destination.write_bytes(b'npm')

        name, body = self._download_with(fake_fetch, 'x86_64-unknown-linux-gnu')
        self.assertEqual(calls, [sources[0][0]] * payload.ATTEMPTS_PER_SOURCE + [sources[1][0]])
        self.assertTrue(name.endswith('.tgz'))
        self.assertEqual(body, b'npm')

    def _download_with(self, fetch, target):
        original_fetch = payload._fetch_url
        original_sleep = payload.time.sleep
        payload._fetch_url = fetch
        payload.time.sleep = lambda _delay: None
        try:
            with tempfile.TemporaryDirectory() as temporary:
                archive = payload.download_bun(target, Path(temporary))
                return archive.name, archive.read_bytes()
        finally:
            payload._fetch_url = original_fetch
            payload.time.sleep = original_sleep
