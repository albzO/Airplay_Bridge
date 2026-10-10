import type { AudioFormat } from './types';

// 展示有限数值；缺失或非有限值显示破折号，合法的零值继续显示为零。
// Format finite numbers; show a dash for missing/nonfinite values while preserving valid zeroes.
export function num(v: unknown, d = 1) {
  return typeof v === 'number' && Number.isFinite(v) ? v.toFixed(d) : '—';
}

// 电平展示保留原有 dBFS 精度与静音占位。
// Keep the existing dBFS precision and silence placeholder.
export function level(v: number) {
  return v > 0 ? `${(20 * Math.log10(v)).toFixed(1)} dBFS` : '−∞ dBFS';
}

// 优先显示有效位宽，缺失格式仍使用原有说明。
// Prefer valid bit depth and retain the existing missing-format text.
export function formatAudioFormat(f: AudioFormat | null | undefined) {
  return f
    ? `${f.rate} Hz · ${f.valid_bits || f.bits}-bit ${f.encoding === 'pcm' ? 'PCM' : f.encoding === 'float32' ? 'float' : f.encoding} · ${f.channels} 声道`
    : '设备未公布格式';
}
