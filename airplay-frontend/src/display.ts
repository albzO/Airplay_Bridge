// 展示有限数值；缺失或非有限值显示破折号，合法的零值继续显示为零。
// Format finite numbers; show a dash for missing/nonfinite values while preserving valid zeroes.
export function num(v: unknown, d = 1) {
  return typeof v === 'number' && Number.isFinite(v) ? v.toFixed(d) : '—';
}
