"""Pinned source, ordered context patches and extraction; no native build needed."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'airplay-backend'))
from upstream_guard import UpstreamGuard, digest
from select_upstream import function, select_sources

UPSTREAM = ROOT / 'upstream/airplay-cli'
MANIFEST = ROOT / 'airplay-backend/upstream-manifest.json'
SCRIPT = ROOT / 'airplay-backend/select_upstream.py'


class UpstreamTests(unittest.TestCase):
    def fixture(self, temporary, crlf=False):
        base = pathlib.Path(temporary)
        root, adapter = base / 'source', base / 'adapter'
        manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
        paths = [(UPSTREAM / name, root / name) for name in manifest['inputs']]
        paths += [(MANIFEST.parent / patch['path'], adapter / patch['path'])
                  for patch in manifest['patches']]
        for source, destination in paths:
            destination.parent.mkdir(parents=True, exist_ok=True)
            text = source.read_text(encoding='utf-8')
            destination.write_bytes(text.replace('\n', '\r\n').encode('utf-8') if crlf else text.encode('utf-8'))
        path = adapter / MANIFEST.name
        path.write_text(json.dumps(manifest), encoding='utf-8')
        return root, path, manifest

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
            self.assertEqual(report['revisions'], manifest['revisions'])

    def test_crlf_sources_and_patches_preserve_submodule_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, path, manifest = self.fixture(temporary, crlf=True)
            before = {name: (root / name).read_bytes() for name in manifest['inputs']}
            guard = UpstreamGuard(root, path)
            out = pathlib.Path(temporary) / 'generated'
            out.mkdir()
            with guard.patched_sources() as patched:
                select_sources(patched, out)
            self.assertFalse(patched.exists(), 'temporary patched sources must be removed')
            self.assertEqual({p.name: digest(p) for p in out.iterdir()}, manifest['outputs'])
            self.assertEqual(before, {name: (root / name).read_bytes() for name in manifest['inputs']})

    def test_changed_input_fails_before_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, _, _ = self.fixture(temporary)
            changed = root / 'src/ap2_io.c'
            changed.write_bytes(changed.read_bytes()+b'\r\n/* upstream changed */\r\n')
            out = root / 'generated'
            result = subprocess.run([sys.executable, str(SCRIPT), str(root), str(out)],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('src/ap2_io.c', result.stderr)
            self.assertFalse(out.exists(), 'input hashes must be checked before generating files')

    def test_changed_patch_requires_review(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, path, manifest = self.fixture(temporary)
            patch = path.parent / manifest['patches'][0]['path']
            patch.write_bytes(patch.read_bytes() + b'\n')
            with self.assertRaisesRegex(ValueError, 'Reviewed patch changed'):
                UpstreamGuard(root, path)

    def test_invalid_context_discards_copy_and_preserves_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, path, manifest = self.fixture(temporary)
            patch = path.parent / manifest['patches'][1]['path']
            # 模拟已审查哈希但上下文仍不匹配的补丁；第一组已应用也不能泄漏到原目录。
            # Simulate a reviewed hash with invalid context after the first patch has applied.
            text = patch.read_text(encoding='utf-8').replace('     uint8_t shared_secret[32];',
                                                          '     uint8_t missing_context[32];', 1)
            self.assertNotEqual(text, patch.read_text(encoding='utf-8'))
            patch.write_text(text, encoding='utf-8')
            manifest['patches'][1]['sha256'] = digest(patch)
            path.write_text(json.dumps(manifest), encoding='utf-8')
            out = pathlib.Path(temporary) / 'generated'
            out.mkdir()
            sentinel = out / 'existing.c'
            sentinel.write_text('previous successful output', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'Cannot apply patches/0002'):
                with UpstreamGuard(root, path).patched_sources() as patched:
                    select_sources(patched, out)
            self.assertEqual(list(out.iterdir()), [sentinel])
            self.assertEqual(sentinel.read_text(), 'previous successful output')
            self.assertTrue(all(digest(root / name) == expected for name, expected in manifest['inputs'].items()))

    def test_revision_mismatch_is_rejected_even_with_identical_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            _, path, manifest = self.fixture(temporary)
            manifest['revisions']['.'] = '0' * 40
            path.write_text(json.dumps(manifest), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'Upstream revision changed'):
                UpstreamGuard(UPSTREAM, path).verify_revisions()

    def test_function_selection_handles_literals_comments_and_duplicates(self):
        source = 'static bool chosen(void) { const char *s = "}"; /* } */ char c = \'{\'; // }\n return true; }'
        self.assertEqual(function('chosen', source), source)
        for invalid in ('', source + '\n' + source):
            with self.assertRaisesRegex(ValueError, 'expected one definition'):
                function('chosen', invalid)


if __name__ == '__main__':
    unittest.main()
