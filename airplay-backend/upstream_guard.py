"""Verify pinned inputs and apply reviewed context patches to a temporary copy."""
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
from contextlib import contextmanager


def digest(path):
    # 接受 Git 的 CRLF 检出差异，其他字符变化必须重新审查。
    # Accept Git CRLF checkout differences; every other content change requires review.
    return hashlib.sha256(path.read_text(encoding='utf-8').encode('utf-8')).hexdigest()


class UpstreamGuard:
    def __init__(self, root, manifest, git='git'):
        self.git = str(git)
        self.root = root.resolve()
        self.adapter = manifest.resolve().parent
        self.manifest = json.loads(manifest.read_text(encoding='utf-8'))
        for name, expected in self.manifest['inputs'].items():
            if digest(self.root / name) != expected:
                raise ValueError(f'Upstream input changed; review adapter before building: {name}')
        for patch in self.manifest['patches']:
            if digest(self.adapter / patch['path']) != patch['sha256']:
                raise ValueError(f'Reviewed patch changed: {patch["path"]}')

    def verify_revisions(self):
        for name, expected in self.manifest['revisions'].items():
            path = (self.root / name).resolve()
            result = subprocess.run([self.git, '-C', str(path), 'rev-parse', '--show-toplevel', 'HEAD'],
                                    capture_output=True, text=True, encoding='utf-8', check=True)
            top, revision = result.stdout.strip().splitlines()
            if pathlib.Path(top).resolve() != path or revision != expected:
                raise ValueError(f'Upstream revision changed; expected {expected}: {name}')

    @contextmanager
    def patched_sources(self):
        # 子模块只读；补丁失败时临时副本整体丢弃，生成目录尚未写入。
        # Submodules stay read-only; failed patches discard the copy before any output is written.
        with tempfile.TemporaryDirectory(prefix='airplay-upstream-') as temporary:
            work = pathlib.Path(temporary)
            for name in self.manifest['inputs']:
                target = work / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text((self.root / name).read_text(encoding='utf-8'),
                                  encoding='utf-8', newline='\n')
            env = os.environ.copy()
            # 即使临时目录位于仓库内，也按独立目录应用，不触碰父仓库索引。
            # Apply outside repository discovery even when TEMP is inside a checkout.
            for name in ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE'):
                env.pop(name, None)
            env['GIT_CEILING_DIRECTORIES'] = str(work.parent)
            for patch in self.manifest['patches']:
                data = (self.adapter / patch['path']).read_text(encoding='utf-8').encode('utf-8')
                for check in (True, False):
                    command = [self.git, '-C', str(work), 'apply', '--whitespace=error-all']
                    if check:
                        command.append('--check')
                    result = subprocess.run(command + ['-'], input=data, capture_output=True, env=env)
                    if result.returncode:
                        detail = result.stderr.decode('utf-8', errors='replace').strip()
                        raise ValueError(f'Cannot apply {patch["path"]}: {detail}')
            yield work

    def finish(self, out):
        report = {key: self.manifest[key] for key in ('revisions', 'inputs', 'patches')}
        report['outputs'] = {path.name: digest(path) for path in sorted(out.iterdir())
                             if path.suffix in ('.c', '.h', '.cpp')}
        (out / 'selection-report.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
