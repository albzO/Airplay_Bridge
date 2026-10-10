"""Fail closed when pinned inputs or reviewed patch match counts change."""
import hashlib
import json
import re
from collections import defaultdict


def digest(path):
    # 接受 Git 的 CRLF 检出差异，其他字符变化必须重新审查。
    # Accept Git CRLF checkout differences; every other content change requires review.
    return hashlib.sha256(path.read_text(encoding='utf-8').encode('utf-8')).hexdigest()


class PatchGuard:
    def __init__(self, root, manifest):
        self.manifest = json.loads(manifest.read_text(encoding='utf-8'))
        self.matches = defaultdict(list)
        for name, expected in self.manifest['inputs'].items():
            if digest(root / name) != expected:
                raise ValueError(f'Upstream input changed; review adapter before building: {name}')

    def check(self, site, actual):
        index = len(self.matches[site])
        expected = self.manifest['patches'].get(site, [])
        if index >= len(expected) or actual != expected[index]:
            reviewed = expected[index] if index < len(expected) else 'no additional application'
            raise ValueError(f'{site}[{index}]: expected {reviewed}, found {actual} matches')
        self.matches[site].append(actual)

    def replace(self, text, old, new, count=-1, *, site):
        actual = text.count(old)
        self.check(site, actual)
        return text.replace(old, new, count)

    def sub(self, pattern, replacement, text, count=0, flags=0, *, site):
        result, actual = re.subn(pattern, replacement, text, count=count, flags=flags)
        self.check(site, actual)
        return result

    def finish(self, out):
        if dict(self.matches) != self.manifest['patches']:
            raise ValueError('Patch applications changed; review every adapter boundary')
        report = {'inputs': self.manifest['inputs'], 'patches': self.matches,
                  'outputs': {path.name: digest(path) for path in sorted(out.iterdir())
                              if path.suffix in ('.c', '.h', '.cpp')}}
        (out / 'selection-report.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
