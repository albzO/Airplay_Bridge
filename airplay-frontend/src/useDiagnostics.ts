import { ref, shallowRef } from 'vue';
import { num } from './display';
import type {
  Device,
  Initialization,
  SessionReport,
  Settings,
  StreamEvent,
  Telemetry,
} from './types';

// 显示内存独立限量；不会截断用于协议控制、音量和设备归属的原始事件。
// Bound presentation memory independently; never truncate raw control, volume or attribution events.
export const DIAGNOSTIC_LIMITS = {
  logs: 300,
  summaries: 120,
  text: 4096,
  devices: 128,
  report: 1024 * 1024,
};
export function nativeFields(line: string): Record<string, string> {
  return Object.fromEntries([...line.matchAll(/([a-zA-Z_]+)=([^\s]+)/g)].map((m) => [m[1], m[2]]));
}
function bounded(text: string) {
  const limit = DIAGNOSTIC_LIMITS.text;
  return text.length <= limit ? text : text.slice(0, limit - 6) + '…[截断]';
}
function append(list: string[], text: string, limit: number) {
  list.push(bounded(text));
  if (list.length > limit) list.splice(0, list.length - limit);
}

/**
 * 只管理诊断快照及汇总；调用方先验证事件并过滤 session_id/endpoint。
 * 重连清空会话基线，保留应用累计统计；不发命令、不管理密码或会话生命周期。
 * Own diagnostic snapshots and aggregation after caller validation/session/endpoint filtering.
 * Reconnect resets session baselines, retaining app totals; no IPC, passwords or lifecycle ownership.
 */
export function useDiagnostics(devices: () => readonly Device[]) {
  const activeDetailedLogs = ref(false),
    activeDiagnostics = ref(false);
  const logs = ref<string[]>([]),
    diagnosticLogs = ref<string[]>([]);
  const logPath = ref(''),
    pipelinePath = ref(''),
    diagnosticPath = ref('');
  const telemetry = ref<Partial<Telemetry>>({});
  // 报告整体替换，不为任意深度 JSON 创建响应式代理。
  // Replace reports wholesale without deep reactive proxies for arbitrary JSON.
  const report = shallowRef<SessionReport>({});
  const technical = ref<Record<string, string>>({});
  const stats = ref<Record<string, Record<string, string>>>({});
  const statsNames = ref<Record<string, string>>({});
  let sessionStats = new Map<string, Record<string, number>>();
  let directory = '%APPDATA%/AirPlay Hub/logs';

  function setDataMode(mode: Initialization['dataMode']) {
    directory = mode === 'portable' ? '[程序目录]/data/logs' : '%APPDATA%/AirPlay Hub/logs';
    logPath.value = directory;
  }
  function displayPath(path?: string | null) {
    return path ? directory + '/' + bounded(path.split(/[\\/]/).pop() || '') : '';
  }
  function clearDiagnostics() {
    diagnosticLogs.value = [];
  }
  function beginSession(settings: Pick<Settings, 'detailedLogs' | 'captureDiagnostics'>) {
    activeDetailedLogs.value = settings.detailedLogs;
    activeDiagnostics.value = settings.captureDiagnostics;
    logs.value = [];
    clearDiagnostics();
    pipelinePath.value = '';
    diagnosticPath.value = '';
    telemetry.value = {};
    report.value = {};
    technical.value = {};
    sessionStats = new Map();
  }
  function recordSourceWarning(warning: string) {
    append(logs.value, '[WARN] ' + warning, DIAGNOSTIC_LIMITS.logs);
  }
  function recordStats(line: string) {
    const fields = nativeFields(line);
    if (!fields.host) return;
    const host = bounded(fields.host);
    if (
      !Object.hasOwn(stats.value, host) &&
      Object.keys(stats.value).length >= DIAGNOSTIC_LIMITS.devices
    ) {
      // 新设备过多时淘汰最早记录，连同名称和会话基线一起移除。
      // Evict the oldest device together with its name and session baseline.
      const oldest = Object.keys(stats.value)[0];
      delete stats.value[oldest];
      delete statsNames.value[oldest];
      sessionStats.delete(oldest);
    }
    const previous = sessionStats.get(host) || {};
    const total = Object.hasOwn(stats.value, host) ? stats.value[host]! : {};
    for (const key of [
      'sent',
      'send_dropped',
      'sync_dropped',
      'rtx_requested',
      'rtx_resent',
      'rtx_expired',
    ]) {
      const value = Number(fields[key]);
      if (Number.isFinite(value) && value >= 0) {
        total[key] = String(Number(total[key] || 0) + Math.max(0, value - (previous[key] || 0)));
        previous[key] = value;
      }
    }
    stats.value = { ...stats.value, [host]: total };
    sessionStats.set(host, previous);
    const name = devices().find((d) => d.addresses.includes(fields.host))?.name || fields.host;
    statsNames.value = { ...statsNames.value, [host]: bounded(name) };
  }
  function accept(e: StreamEvent) {
    switch (e.kind) {
      case 'log_path':
        logPath.value = displayPath(e.path);
        activeDetailedLogs.value = e.detailed_logs;
        pipelinePath.value = displayPath(e.pipeline_path);
        break;
      case 'diagnostic_path':
        diagnosticPath.value = displayPath(e.path);
        pipelinePath.value = displayPath(e.pipeline_path);
        break;
      case 'telemetry':
        telemetry.value = e;
        break;
      case 'report': {
        const text = JSON.stringify(e.report);
        report.value =
          new TextEncoder().encode(text).length <= DIAGNOSTIC_LIMITS.report
            ? e.report
            : {
                device: bounded(e.report.device || '会话'),
                display_error: '报告超过 1 MiB 显示上限，请缩小诊断范围后重试。',
              };
        break;
      }
      case 'capture_diagnostic': {
        const c = e.capture;
        append(
          diagnosticLogs.value,
          `${num(e.elapsed_seconds, 1)}s · 采集 ${e.capture_frames} 帧 · 待发送 ${num(e.pending_pcm_ms)} ms · 水位 ${num(e.water_ms)} ms · 校正 ${num(e.correction_ppm)} ppm\n设备位置 ${c.device_position} · 包 ${c.packets} · 时间戳 ${c.packet_qpc_100ns} · 不连续 ${c.discontinuities} · 时间戳错误 ${c.timestamp_errors}${e.error ? '\n错误：' + e.error : ''}`,
          DIAGNOSTIC_LIMITS.summaries,
        );
        break;
      }
      case 'diagnostic_end':
        append(
          diagnosticLogs.value,
          '诊断结束：' + JSON.stringify(e.status),
          DIAGNOSTIC_LIMITS.summaries,
        );
        break;
      case 'native': {
        if (e.is_fault || activeDetailedLogs.value)
          append(logs.value, e.safe_line || e.line, DIAGNOSTIC_LIMITS.logs);
        if (e.line.includes('PACKET_STATS')) recordStats(e.line);
        if (e.line.includes('AUTH_METHOD') || e.line.includes('TIMING')) {
          const fields = nativeFields(e.line);
          technical.value[e.line.includes('TIMING') ? '时钟协议' : '认证方式'] = bounded(
            fields.value || e.line,
          );
        }
        break;
      }
      case 'finished':
        telemetry.value.peaks = [0, 0];
        if (e.error) append(logs.value, e.safe_error || e.error, DIAGNOSTIC_LIMITS.logs);
        break;
    }
  }
  return {
    activeDetailedLogs,
    activeDiagnostics,
    logs,
    diagnosticLogs,
    logPath,
    pipelinePath,
    diagnosticPath,
    telemetry,
    report,
    technical,
    stats,
    statsNames,
    setDataMode,
    beginSession,
    recordSourceWarning,
    clearDiagnostics,
    accept,
  };
}
