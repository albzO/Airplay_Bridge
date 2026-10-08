"""检查已提交源码与待新增文件，不打印敏感值。
Check committed source and pending additions without printing sensitive values.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
# 只报告规则名称和位置；不要在检查输出中再次泄露命中的内容。
# Report only rule names and locations; never expose matched values in scanner output.
RULES = {
    "personal-windows-path": re.compile(r"[A-Za-z]:[\\/]Users[\\/][^\\/\s]+", re.I),
    "private-key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"),
    "github-token": re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b"),
    "aws-access-key": re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "openai-key": re.compile(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{32,}\b"),
    "private-lan-address": re.compile(
        r"\b(?:192\.168|10\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01]))\.\d{1,3}\.\d{1,3}\b"
    ),
}
LOCAL_FILES = {
    "devices.json", "inputs.json", "audio-inputs.json", "settings.json",
    "settings.pending.json", "auth-policy.json", "auth-policy.pending.json",
    "ui-smoke.json", "instance-smoke.json",
}
FIXTURES = {
    "airplay-frontend/checks/devices.json", "airplay-frontend/checks/inputs.json",
}
LOCAL_PARTS = {".local", "node_modules", "target", "dist", "Release", "releases", "captures", "logs"}


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", *args], cwd=ROOT, stderr=subprocess.PIPE)


def findings(path: str, data: bytes) -> list[tuple[int, str]]:
    result = []
    parts = Path(path).parts
    if path not in FIXTURES and (
        Path(path).name in LOCAL_FILES or any(part in LOCAL_PARTS for part in parts)
        or Path(path).suffix.lower() in {".log", ".jsonl", ".pcm", ".wav", ".pfx", ".p12", ".key"}
    ):
        result.append((0, "local-runtime-or-secret-file"))
    if b"\0" in data:
        return result
    for number, line in enumerate(data.decode("utf-8", errors="replace").splitlines(), 1):
        for name, pattern in RULES.items():
            if pattern.search(line):
                result.append((number, name))
    return result


def check_revision(revision: str) -> int:
    count = 0
    # 不递归扫描第三方子模块；其固定引用与审查边界另行记录。
    # Do not recurse into third-party submodules; document their pinned revisions and review scope separately.
    records = git("ls-tree", "-r", "-z", revision).split(b"\0")
    for record in filter(None, records):
        metadata, encoded_path = record.split(b"\t", 1)
        _, kind, blob = metadata.split()
        if kind != b"blob":
            continue
        path = encoded_path.decode("utf-8")
        for line, rule in findings(path, git("cat-file", "blob", blob.decode())):
            print(f"{revision[:12]}:{path}:{line}: {rule}")
            count += 1
    return count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--history", action="store_true", help="Read all local revisions; never rewrite history")
    args = parser.parse_args()
    count = 0
    if args.history:
        for revision in git("rev-list", "--all").decode().splitlines():
            count += check_revision(revision)
    else:
        paths = set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0"))
        for encoded_path in sorted(filter(None, paths)):
            path = encoded_path.decode("utf-8")
            file = ROOT / path
            if not file.is_file():
                continue
            for line, rule in findings(path, file.read_bytes()):
                print(f"{path}:{line}: {rule}")
                count += 1
    print(f"Sensitive-data check: {count} finding(s). Values are never printed.")
    return 1 if count else 0


if __name__ == "__main__":
    sys.exit(main())
