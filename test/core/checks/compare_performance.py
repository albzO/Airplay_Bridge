"""Compare synthetic release artifacts; optional threshold, no device/CPU claims."""
import argparse
import json
import pathlib
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('before', type=pathlib.Path)
parser.add_argument('after', type=pathlib.Path)
parser.add_argument('--max-regression-percent', type=float, default=None)
args = parser.parse_args()
before, after = [json.loads(p.read_text(encoding='utf-8')) for p in (args.before, args.after)]
assert before['profile'] == after['profile'] == 'release', 'compare release with release'
assert before['fixture'] == after['fixture'], 'benchmark fixture changed'
rates = {row['input_rate'] for row in before['results']}
assert rates == {row['input_rate'] for row in after['results']}, 'sample rates changed'
print('| Rate | p95 before (us) | p95 after (us) | Change | Wall before/after (ms) |')
print('|---:|---:|---:|---:|---:|')
failed = False
for rate in sorted(rates):
    old, new = [[row for row in obj['results'] if row['input_rate'] == rate] for obj in (before, after)]
    assert len(old) >= 3 and len(new) >= 3, 'need at least three measured rounds'
    p95 = [statistics.median(row['process_us']['p95'] for row in rows) for rows in (old, new)]
    wall = [statistics.median(row['wall_ms'] for row in rows) for rows in (old, new)]
    change = (p95[1] / p95[0] - 1) * 100
    print(f'| {rate} | {p95[0]:.1f} | {p95[1]:.1f} | {change:+.1f}% | {wall[0]:.2f} / {wall[1]:.2f} |')
    failed |= args.max_regression_percent is not None and change > args.max_regression_percent
raise SystemExit(1 if failed else 0)
