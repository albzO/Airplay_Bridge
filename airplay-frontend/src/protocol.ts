import type {
  AudioFormat,
  Device,
  Initialization,
  Input,
  JsonObject,
  JsonValue,
  SessionReport,
  Settings,
  SourceLevel,
  StreamEvent,
} from './types';

/**
 * Tauri 的泛型只约束编译，不验证运行时 JSON。因此先接收 unknown，再在此
 * 检查页面真正使用的字段。异常只携带字段路径，避免把密码或设备数据写入错误。
 * 新增事件可返回 null 供旧页面忽略；已知事件的损坏字段必须报错，不能默默当成成功。
 * Tauri generics constrain compilation but do not validate runtime JSON. Accept unknown
 * and validate the fields consumed by the UI. Errors contain field paths, not sensitive values.
 * Ignore new event kinds via null; malformed known events must fail rather than imply success.
 */
type Reader<T> = (value: unknown, path: string) => T;
function invalid(path: string): never {
  throw new Error(`桌面数据格式不正确：${path}`);
}
const object: Reader<Record<string, unknown>> = (value, path) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid(path);
  return value as Record<string, unknown>;
};
const text: Reader<string> = (value, path) => (typeof value === 'string' ? value : invalid(path));
const number: Reader<number> = (value, path) =>
  typeof value === 'number' && Number.isFinite(value) ? value : invalid(path);
const boolean: Reader<boolean> = (value, path) =>
  typeof value === 'boolean' ? value : invalid(path);
const integer: Reader<number> = (value, path) => {
  const result = number(value, path);
  return Number.isSafeInteger(result) && result >= 0 ? result : invalid(path);
};
function optional<T>(value: unknown, path: string, read: Reader<T>): T | undefined {
  return value === undefined ? undefined : read(value, path);
}
function nullable<T>(value: unknown, path: string, read: Reader<T>): T | null {
  return value === null ? null : read(value, path);
}
function array<T>(value: unknown, path: string, read: Reader<T>): T[] {
  if (!Array.isArray(value)) invalid(path);
  return value.map((item, index) => read(item, `${path}[${index}]`));
}
function pair(value: unknown, path: string, read: Reader<number>): [number, number] {
  const items = array(value, path, read);
  if (items.length !== 2) invalid(path);
  return [items[0], items[1]];
}
const nullableText: Reader<string | null> = (value, path) => nullable(value, path, text);

/**
 * 报告是可扩展 JSON；递归限制避免损坏的桥接数据导致页面无限递归。
 * Reports allow extensible JSON; the depth limit prevents unbounded recursion on malformed data.
 */
function json(value: unknown, path: string, depth = 0): JsonValue {
  if (depth > 32) invalid(path);
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') return number(value, path);
  if (Array.isArray(value)) return value.map((item, i) => json(item, `${path}[${i}]`, depth + 1));
  const entries = object(value, path);
  return Object.fromEntries(
    Object.entries(entries).map(([key, item]) => [key, json(item, `${path}.${key}`, depth + 1)]),
  );
}
function jsonObject(value: unknown, path: string): JsonObject {
  object(value, path);
  // json 已递归验证为 JSON；object 的检查保证根节点不是数组或基本值。
  // json validates the entire value; object ensures the root is neither an array nor a primitive.
  return json(value, path) as JsonObject;
}

const audioFormat: Reader<AudioFormat> = (value, path) => {
  const v = object(value, path);
  return {
    rate: integer(v.rate, `${path}.rate`),
    channels: integer(v.channels, `${path}.channels`),
    bits: integer(v.bits, `${path}.bits`),
    valid_bits: integer(v.valid_bits, `${path}.valid_bits`),
    encoding: text(v.encoding, `${path}.encoding`),
    block_align: integer(v.block_align, `${path}.block_align`),
    channel_mask: integer(v.channel_mask, `${path}.channel_mask`),
  };
};
const device: Reader<Device> = (value, path) => {
  const v = object(value, path);
  const properties = object(v.properties, `${path}.properties`);
  return {
    name: text(v.name, `${path}.name`),
    service: text(v.service, `${path}.service`),
    host: text(v.host, `${path}.host`),
    addresses: array(v.addresses, `${path}.addresses`, text),
    port: integer(v.port, `${path}.port`),
    properties: Object.fromEntries(
      Object.entries(properties).map(([key, item]) => [
        key,
        text(item, `${path}.properties.${key}`),
      ]),
    ),
  };
};
const input: Reader<Input> = (value, path) => {
  const v = object(value, path);
  if (v.flow !== 'recording' && v.flow !== 'playback') invalid(`${path}.flow`);
  return {
    flow: v.flow,
    id: text(v.id, `${path}.id`),
    name: text(v.name, `${path}.name`),
    description: text(v.description, `${path}.description`),
    device_format: nullable(v.device_format, `${path}.device_format`, audioFormat),
    mix_format: nullable(v.mix_format, `${path}.mix_format`, audioFormat),
    channels: nullable(v.channels, `${path}.channels`, integer),
    rate: nullable(v.rate, `${path}.rate`, integer),
    encoding: nullableText(v.encoding, `${path}.encoding`),
  };
};
const settings: Reader<Settings> = (value, path) => {
  const v = object(value, path);
  if (v.closeAction !== 'tray' && v.closeAction !== 'quit') invalid(`${path}.closeAction`);
  const latency = integer(v.latency, `${path}.latency`);
  const buffer = integer(v.buffer, `${path}.buffer`);
  if (latency < 250 || latency > 2000) invalid(`${path}.latency`);
  if (buffer < 64 || buffer > 512) invalid(`${path}.buffer`);
  return {
    endpoint: text(v.endpoint, `${path}.endpoint`),
    latency,
    buffer,
    mapping: pair(v.mapping, `${path}.mapping`, integer),
    closeAction: v.closeAction,
    detailedLogs: boolean(v.detailedLogs, `${path}.detailedLogs`),
    captureDiagnostics: boolean(v.captureDiagnostics, `${path}.captureDiagnostics`),
    speakersSwapped: boolean(v.speakersSwapped, `${path}.speakersSwapped`),
    keepAwake: boolean(v.keepAwake, `${path}.keepAwake`),
  };
};

export function decodeDevices(value: unknown): Device[] {
  return array(value, 'devices', device);
}
export function decodeInitialization(value: unknown): Initialization {
  const v = object(value, 'initialize');
  const dataMode = v.dataMode === undefined ? 'installed' : v.dataMode;
  if (dataMode !== 'installed' && dataMode !== 'portable') invalid('dataMode');
  return {
    devices: decodeDevices(v.devices),
    inputs: array(v.inputs, 'inputs', input),
    settings: settings(v.settings, 'settings'),
    dataPath: text(v.dataPath, 'dataPath'),
    dataMode,
    backendAvailable: boolean(v.backendAvailable, 'backendAvailable'),
    autostart: boolean(v.autostart, 'autostart'),
    captureEnabled:
      v.captureEnabled === undefined ? true : boolean(v.captureEnabled, 'captureEnabled'),
    awakeActive: optional(v.awakeActive, 'awakeActive', boolean),
    awakeError: optional(v.awakeError, 'awakeError', nullableText),
    autostartError: optional(v.autostartError, 'autostartError', nullableText),
  };
}

export function decodeSourceLevel(value: unknown): SourceLevel {
  const v = object(value, 'source-level');
  return {
    endpoint: text(v.endpoint, 'endpoint'),
    captureEnabled: optional(v.captureEnabled, 'captureEnabled', boolean),
    peaks: optional(v.peaks, 'peaks', (item, path) => pair(item, path, number)),
    error: optional(v.error, 'error', nullableText),
    warning: optional(v.warning, 'warning', text),
  };
}

function report(value: unknown): SessionReport {
  const result = jsonObject(value, 'report');
  const device = optional(result.device, 'report.device', text);
  const capture = optional(result.capture, 'report.capture', (item, path) =>
    nullable(item, path, jsonObject),
  );
  if (capture) {
    optional(capture.discontinuities, 'report.capture.discontinuities', integer);
    optional(capture.timestamp_errors, 'report.capture.timestamp_errors', integer);
  }
  // 已校验可选字段；不额外填入 undefined，以保持整个报告仍是有效 JSON。
  // Optional fields are validated; omit undefined values to keep the report valid JSON.
  return {
    ...result,
    ...(device === undefined ? {} : { device }),
    ...(capture === undefined ? {} : { capture }),
  };
}

export function decodeStreamEvent(value: unknown): StreamEvent | null {
  const v = object(value, 'stream-event');
  const kind = text(v.kind, 'kind');
  // 新版本桌面端可能增加事件。先识别 kind，未知事件不影响现有会话。
  // Recognize kind first so events added by newer desktop versions do not disrupt this session.
  if (
    ![
      'preparing_source',
      'password_required',
      'auth_memory_error',
      'auth_pipe_error',
      'log_path',
      'telemetry',
      'report',
      'diagnostic_path',
      'capture_diagnostic',
      'diagnostic_end',
      'native',
      'finished',
    ].includes(kind)
  )
    return null;
  const session_id = integer(v.session_id, 'session_id');
  switch (kind) {
    case 'preparing_source':
      return { kind, session_id };
    case 'password_required':
      return { kind, session_id, host: text(v.host, 'host') };
    case 'auth_memory_error':
    case 'auth_pipe_error':
      return { kind, session_id, error: text(v.error, 'error') };
    case 'log_path':
      return {
        kind,
        session_id,
        path: text(v.path, 'path'),
        detailed_logs: boolean(v.detailed_logs, 'detailed_logs'),
        pipeline_path: optional(v.pipeline_path, 'pipeline_path', nullableText),
      };
    case 'telemetry':
      return {
        kind,
        session_id,
        elapsed_seconds: number(v.elapsed_seconds, 'elapsed_seconds'),
        capture_frames: integer(v.capture_frames, 'capture_frames'),
        output_frames: integer(v.output_frames, 'output_frames'),
        water_ms: number(v.water_ms, 'water_ms'),
        lead_ms: number(v.lead_ms, 'lead_ms'),
        peaks: pair(v.peaks, 'peaks', number),
        controller: {
          correction_ppm: number(
            object(v.controller, 'controller').correction_ppm,
            'controller.correction_ppm',
          ),
        },
      };
    case 'report':
      return { kind, session_id, report: report(v.report) };
    case 'diagnostic_path':
      return {
        kind,
        session_id,
        path: text(v.path, 'path'),
        pipeline_path: optional(v.pipeline_path, 'pipeline_path', nullableText),
      };
    case 'capture_diagnostic': {
      const capture = jsonObject(v.capture, 'capture');
      for (const key of [
        'device_position',
        'packets',
        'packet_qpc_100ns',
        'discontinuities',
        'timestamp_errors',
      ])
        integer(capture[key], `capture.${key}`);
      return {
        kind,
        session_id,
        capture,
        elapsed_seconds: nullable(v.elapsed_seconds, 'elapsed_seconds', number),
        capture_frames: integer(v.capture_frames, 'capture_frames'),
        pending_pcm_ms: number(v.pending_pcm_ms, 'pending_pcm_ms'),
        water_ms: nullable(v.water_ms, 'water_ms', number),
        correction_ppm: number(v.correction_ppm, 'correction_ppm'),
        error: optional(v.error, 'error', nullableText),
      };
    }
    case 'diagnostic_end':
      return { kind, session_id, status: jsonObject(v.status, 'status') };
    case 'native':
      return {
        kind,
        session_id,
        line: text(v.line, 'line'),
        safe_line: optional(v.safe_line, 'safe_line', text),
        is_fault: optional(v.is_fault, 'is_fault', boolean),
      };
    case 'finished':
      return {
        kind,
        session_id,
        error: optional(v.error, 'error', nullableText),
        safe_error: optional(v.safe_error, 'safe_error', nullableText),
        error_code: optional(v.error_code, 'error_code', nullableText),
        cancelled: optional(v.cancelled, 'cancelled', boolean),
      };
    default:
      return null;
  }
}
