"""Pinned extraction output and fail-closed adapter boundaries; no native build needed."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'airplay-backend'))
from upstream_guard import PatchGuard

UPSTREAM = ROOT / 'upstream/airplay-cli'
MANIFEST = ROOT / 'airplay-backend/upstream-manifest.json'
SCRIPT = ROOT / 'airplay-backend/select_upstream.py'


class UpstreamTests(unittest.TestCase):
    def test_pinned_extraction_matches_reviewed_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            out = pathlib.Path(temporary) / 'generated'
            subprocess.run([sys.executable, str(SCRIPT), str(UPSTREAM), str(out)], check=True,
                           capture_output=True, text=True)
            manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
            report = json.loads((out / 'selection-report.json').read_text(encoding='utf-8'))
            self.assertEqual(report['outputs'], manifest['outputs'])
            self.assertEqual(report['patches'], manifest['patches'])
            self.assertEqual(report['inputs'], manifest['inputs'])

    def test_crlf_checkout_and_changed_input(self):
        manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for name in manifest['inputs']:
                destination = root / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                normalized = (UPSTREAM / name).read_text(encoding='utf-8')
                destination.write_bytes(normalized.replace('\n', '\r\n').encode('utf-8'))
            PatchGuard(root, MANIFEST)
            changed = root / 'src/ap2_io.c'
            changed.write_bytes(changed.read_bytes()+b'\r\n/* upstream changed */\r\n')
            out = root / 'generated'
            result = subprocess.run([sys.executable, str(SCRIPT), str(root), str(out)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('src/ap2_io.c', result.stderr)
            self.assertFalse(out.exists(), 'input hashes must be checked before generating files')

    def test_missing_extra_or_repeated_matches_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({'inputs': {}, 'patches': {'one': [1]}, 'outputs': {}}))
            for text in ('missing', 'hit hit'):
                guard = PatchGuard(root, manifest)
                with self.assertRaisesRegex(ValueError, 'one'):
                    guard.replace(text, 'hit', 'new', site='one')
            guard = PatchGuard(root, manifest)
            self.assertEqual(guard.replace('hit', 'hit', 'new', site='one'), 'new')
            with self.assertRaisesRegex(ValueError, 'one'):
                guard.replace('hit', 'hit', 'new', site='one')
            with self.assertRaisesRegex(ValueError, 'Patch applications changed'):
                PatchGuard(root, manifest).finish(root)


if __name__ == '__main__':
    unittest.main()
