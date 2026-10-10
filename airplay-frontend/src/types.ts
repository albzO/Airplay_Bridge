/**
 * 桌面端序列化给页面的数据约定。字段名与 Rust 的 serde 输出一致，
 * 不在页面私自改成另一套命名；新增字段时同步修改 protocol.ts 的输入校验。
 * Desktop-to-UI data contracts use the field names produced by Rust serde.
 * Keep those names consistent and update protocol.ts when adding fields.
 */
export type Device = {
  /**
   * 显示名；连接时传给 start_stream，由桌面端重新查找真实设备。
   * Display name passed to start_stream; the desktop resolves the actual device again.
   */
  name: string;
  /**
   * mDNS 服务标识，用于区分同名设备和构建设备卡片。
   * mDNS service identity used to distinguish duplicate names and build device cards.
   */
  service: string;
  host: string;
  addresses: string[];
  port: number;
  /**
   * mDNS TXT 字段；tsid 相同的两台设备可组成立体声，igl 标记主设备。
   * mDNS TXT fields: matching tsid identifies a stereo pair; igl marks its primary member.
   */
  properties: Record<string, string>;
};
// 页面构建的设备卡片，不属于桌面端 IPC 数据约定。
// UI-only card model, separate from the desktop IPC contracts.
export type DeviceCardData = {
  id: string;
  members: Device[];
  title: string;
};

export type AudioFormat = {
  /**
   * 每秒采样帧数（Hz）；一帧包含全部声道的一个采样。
   * Sample frames per second (Hz); each frame contains one sample per channel.
   */
  rate: number;
  channels: number;
  /**
   * 存储位宽与有效位宽可能不同，例如 32 位容器中的 24 位 PCM。
   * Storage width can differ from valid bits, such as 24-bit PCM in a 32-bit container.
   */
  bits: number;
  valid_bits: number;
  encoding: string;
  block_align: number;
  channel_mask: number;
};
export type Input = {
  /**
   * recording 采集输入，playback 通过 WASAPI loopback 采集设备播放的声音。
   * recording captures input; playback captures rendered audio through WASAPI loopback.
   */
  flow: 'recording' | 'playback';
  device_format: AudioFormat | null;
  mix_format: AudioFormat | null;
  /**
   * Windows 端点标识；切换来源或重新连接都以此定位设备。
   * Windows endpoint identity used to locate the source when switching or reconnecting.
   */
  id: string;
  name: string;
  description: string;
  /**
   * null 表示系统未公布该字段，不能据此启用声道选择。
   * null means the system did not expose this field; it cannot enable channel selection.
   */
  channels: number | null;
  rate: number | null;
  encoding: string | null;
};
export type Settings = {
  endpoint: string;
  /**
   * 播放提前量（ms，250–2000），不是采集端缓冲帧数。
   * Playback lead time (250–2000 ms), distinct from capture buffer frame counts.
   */
  latency: number;
  /**
   * 后端预缓冲时长（ms，64–512）；控制器另按实际启动水位建立基准。
   * Backend prebuffer duration (64–512 ms); the controller derives its startup baseline separately.
   */
  buffer: number;
  /**
   * 输出左右声道对应的来源声道下标（从 0 开始）；单声道可用 [0, 0]。
   * Zero-based source indices for output left/right channels; mono can use [0, 0].
   */
  mapping: [number, number];
  detailedLogs: boolean;
  captureDiagnostics: boolean;
  closeAction: 'tray' | 'quit';
  speakersSwapped: boolean;
  keepAwake: boolean;
};

/**
 * 报告保留可扩展 JSON，但不允许函数、undefined 或非有限数进入页面。
 * Reports allow extensible JSON, excluding functions, undefined and nonfinite numbers.
 */
export type JsonValue = string | number | boolean | null | JsonValue[] | JsonObject;
export type JsonObject = { [key: string]: JsonValue };
export type SessionReport = JsonObject & {
  device?: string;
  capture?: JsonObject | null;
  conversion?: JsonValue;
};

/**
 * 串流工作线程的快照；缺少字段时显示“—”，不要把缺失数值当作 0。
 * Stream worker snapshot; display “—” for missing fields rather than treating them as zero.
 */
export type Telemetry = {
  elapsed_seconds: number;
  capture_frames: number;
  output_frames: number;
  water_ms: number;
  lead_ms: number;
  peaks: [number, number];
  /**
   * ppm 是输出采样率相对基准速率的微调，不是音量。
   * ppm adjusts the output sample rate relative to its baseline; it does not control volume.
   */
  controller: { correction_ppm: number };
};

/**
 * 独立于播放会话的采集预览；按 endpoint 过滤，不能按 session_id 过滤。
 * Capture preview is independent of playback sessions; filter by endpoint, not session_id.
 */
export type SourceLevel = {
  endpoint: string;
  captureEnabled?: boolean;
  peaks?: [number, number];
  error?: string | null;
  warning?: string;
};

export type Initialization = {
  devices: Device[];
  inputs: Input[];
  settings: Settings;
  dataPath: string;
  dataMode: 'installed' | 'portable';
  backendAvailable: boolean;
  autostart: boolean;
  captureEnabled: boolean;
  awakeActive?: boolean;
  awakeError?: string | null;
  autostartError?: string | null;
};

/**
 * kind 决定可以读取的字段；session_id 隔离已经结束的工作线程回调。
 * kind determines the available fields; session_id isolates callbacks from finished workers.
 */
export type StreamEvent = { session_id: number } & (
  | { kind: 'preparing_source' }
  | { kind: 'password_required'; host: string }
  | { kind: 'auth_memory_error' | 'auth_pipe_error'; error: string }
  | { kind: 'log_path'; path: string; detailed_logs: boolean; pipeline_path?: string | null }
  | ({ kind: 'telemetry' } & Telemetry)
  | { kind: 'report'; report: SessionReport }
  | { kind: 'diagnostic_path'; path: string; pipeline_path?: string | null }
  | {
      kind: 'capture_diagnostic';
      elapsed_seconds: number | null;
      capture_frames: number;
      pending_pcm_ms: number;
      water_ms: number | null;
      correction_ppm: number;
      capture: JsonObject;
      error?: string | null;
    }
  | { kind: 'diagnostic_end'; status: JsonObject }
  | { kind: 'native'; line: string; safe_line?: string; is_fault?: boolean }
  | {
      kind: 'finished';
      error?: string | null;
      safe_error?: string | null;
      error_code?: string | null;
      cancelled?: boolean;
    }
);
