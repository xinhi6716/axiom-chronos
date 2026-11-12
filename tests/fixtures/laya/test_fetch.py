import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / 'tools/laya/fetch.py'
spec = importlib.util.spec_from_file_location('laya_fetch', SCRIPT)
fetch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fetch)


class FetchTests(unittest.TestCase):
    def test_sha256_known_vector(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'test.txt'
            path.write_bytes(b'abc')
            self.assertEqual(fetch.sha256_file(path),
                             'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')

    def test_record_and_refuse_tamper(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'test.txt'
            path.write_bytes(b'abc')
            entry = {'url': 'https://example.test/test.txt', 'revision': 'v1', 'sha256': None, 'size': None}
            digest = fetch.verify_or_record(path, entry)
            self.assertEqual(entry['sha256'], digest)
            self.assertEqual(entry['size'], 3)
            path.write_bytes(b'abd')
            with self.assertRaisesRegex(ValueError, 'test.txt'):
                fetch.verify_or_record(path, entry)
            self.assertEqual(entry['sha256'], digest)

    def test_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            fetch.write_manifest(tmp, 'rev', 'fused', 'source', {'z': '2', 'a': '1'})
            text = (Path(tmp) / 'axiom-laya.json').read_text(encoding='utf-8')
            value = json.loads(text)
            self.assertEqual(value, {'format': 1, 'model_version': 'rev', 'layout': 'fused',
                                     'source': 'source', 'revision': 'rev', 'files': {'a': '1', 'z': '2'}})
            self.assertEqual(text, json.dumps(value, indent=2, sort_keys=True) + '\n')

    def test_runtime_fetch_and_tamper_leave_existing_destination(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pin_path = root / 'pins.json'
            archive = root / 'source.zip'
            with zipfile.ZipFile(archive, 'w') as z:
                z.writestr('onnxruntime-win-x64-1.20.1/lib/onnxruntime.dll', b'runtime')
            pins = {'runtime': {'archives': {'win-x64': {'url': 'https://example.test/runtime.zip',
                    'revision': 'v1', 'sha256': fetch.sha256_file(archive), 'size': archive.stat().st_size}},
                    'files': {'win-x64': {}}}, 'model': {}}
            pin_path.write_text(json.dumps(pins), encoding='utf-8')
            destination = root / 'dest'
            def copy_download(url, path):
                path.write_bytes(archive.read_bytes())
            with patch.object(fetch, 'PINS', pin_path), patch.object(fetch.platform, 'system', return_value='Windows'), \
                 patch.object(fetch.platform, 'machine', return_value='AMD64'), patch.object(fetch, 'download', side_effect=copy_download):
                self.assertEqual(fetch.main(['--runtime', '--dest', str(destination)]), 0)
                dll = destination / 'runtime/onnxruntime.dll'
                self.assertEqual(dll.read_bytes(), b'runtime')
                recorded = json.loads(pin_path.read_text(encoding='utf-8'))
                self.assertEqual(recorded['runtime']['files']['win-x64']['onnxruntime.dll']['sha256'], fetch.sha256_file(dll))
                dll.write_bytes(b'existing')
                archive.write_bytes(b'tampered')
                self.assertEqual(fetch.main(['--runtime', '--dest', str(destination)]), 1)
                self.assertEqual(dll.read_bytes(), b'existing')
                self.assertEqual(list(destination.glob('.fetch-*')), [])

    def test_linux_archive_symlinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source.tgz'
            with tarfile.open(source, 'w:gz') as archive:
                member = tarfile.TarInfo('package/lib/libonnxruntime.so.1.20.1')
                member.size = 3
                archive.addfile(member, io.BytesIO(b'ort'))
                for name, target in [('libonnxruntime.so.1', 'libonnxruntime.so.1.20.1'),
                                     ('libonnxruntime.so', 'libonnxruntime.so.1')]:
                    link = tarfile.TarInfo('package/lib/' + name)
                    link.type, link.linkname = tarfile.SYMTYPE, target
                    archive.addfile(link)
            entry = {'url': 'https://example.test/runtime.tgz', 'revision': 'v1',
                     'sha256': fetch.sha256_file(source), 'size': source.stat().st_size}
            pins = {'runtime': {'archives': {'linux-x64': entry}, 'files': {'linux-x64': {}}}}
            with patch.object(fetch.platform, 'system', return_value='Linux'), \
                 patch.object(fetch.platform, 'machine', return_value='x86_64'), \
                 patch.object(fetch, 'download', side_effect=lambda url, path: path.write_bytes(source.read_bytes())):
                target = fetch.fetch_runtime(pins, root)
            self.assertEqual((target / 'libonnxruntime.so').read_bytes(), b'ort')
            self.assertEqual(len(pins['runtime']['files']['linux-x64']), 3)

    def test_model_api_resolves_commit_and_onnx_layout(self):
        revision = 'a' * 40
        response = json.dumps({'sha': revision, 'siblings': [
            {'rfilename': name} for name in ('onnx/model.onnx', 'onnx/model.onnx.data',
                                               'tokenizer/tokenizer.json', 'rl_agent_config.json')]}).encode()
        model = {}
        with patch.object(fetch.urllib.request, 'urlopen', return_value=io.BytesIO(response)):
            fetch.discover_model(model)
        self.assertEqual(model['revision'], revision)
        self.assertEqual(model['layout'], 'fused')
        self.assertEqual(set(model['files']), {'model.onnx', 'model.onnx.data',
                                                'tokenizer/tokenizer.json', 'rl_agent_config.json'})
        self.assertTrue(all('/resolve/' + revision + '/' in pin['url'] for pin in model['files'].values()))

    def test_help_and_pins(self):
        result = subprocess.run([sys.executable, str(SCRIPT), '--help'], capture_output=True)
        self.assertEqual(result.returncode, 0)
        pins = json.loads((SCRIPT.parent / 'pins.json').read_text(encoding='utf-8'))
        self.assertIn('runtime', pins)
        self.assertIn('model', pins)


if __name__ == '__main__':
    unittest.main()
