<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, shallowRef, nextTick, watch } from 'vue';
import { getVersion } from '@tauri-apps/api/app';
import { listen } from '@tauri-apps/api/event';
import { invokeCommand } from './commands';
import { describeError } from './errors';
import {
  decodeDevices,
  decodeInitialization,
  decodeSourceLevel,
  decodeStreamEvent,
} from './protocol';
import type {
  Device,
  AudioFormat,
  Input,
  Settings,
  SessionReport,
  StreamEvent,
  Telemetry,
} from './types';

/**
 * 阅读顺序：页面状态 → computed 设备/来源视图 → 用户操作 → event → onMounted。
 * 音频采集和协议连接由桌面端负责；页面只发送命令并显示经过校验的事件。
 * 命令成功表示请求已被接受，是否开始播放、是否结束要以异步事件为准。
 */
// 设置是下一次连接的配置；activeDetailedLogs/activeDiagnostics 是本次会话的快照。
const authNotice = ref('');
const appVersion = ref('');
const displayVersion = computed(() => appVersion.value.replace(/^(\d+\.\d+)\.0$/, '$1'));
const settingsTab = ref('常规');
const activeDetailedLogs = ref(false),
  pipelinePath = ref('');
const diagnosticPath = ref(''),
  diagnosticLogs = ref<string[]>([]),
  activeDiagnostics = ref(false);
const previewPeaks = ref<number[]>([0, 0]),
  previewError = ref(''),
  sourceWarning = ref('');
const page = ref('播放'),
  devices = ref<Device[]>([]),
  inputs = ref<Input[]>([]),
  settings = ref<Settings>({
    endpoint: '',
    latency: 300,
    buffer: 128,
    mapping: [0, 1],
    detailedLogs: false,
    captureDiagnostics: false,
    closeAction: 'tray',
    speakersSwapped: false,
    keepAwake: true,
  });
const selection = ref<string[]>([]),
  expanded = ref(''),
  busy = ref(false),
  playing = ref(false),
  stopping = ref(false),
  refreshing = ref(false),
  ready = ref(false),
  phase = ref('尚未连接'),
  error = ref('');
// busy 覆盖准备、认证和播放；playing 收到遥测后置真；stopping 等 finished 才清除。
// connected 仅表示原生后端已完成握手（PCM_READY），并不保证已经收到音频遥测。
const session = ref<number | null>(null),
  pending = ref(''),
  password = ref(''),
  sending = ref(false),
  connected = ref(false);
// 连接前没有遥测，保留空快照让界面显示“—”；报告只在串流收尾时到达。
const telemetry = ref<Partial<Telemetry>>({}),
  // 报告按整份快照替换，不修改内部字段；无需把任意深度 JSON 转为响应式代理。
  report = shallowRef<SessionReport>({}),
  stats = ref<Record<string, Record<string, string>>>({}),
  technical = ref<Record<string, string>>({}),
  logs = ref<string[]>([]),
  logPath = ref('');
const volume = ref<number | null>(null),
  draft = ref(50),
  lastVolume = ref(50);
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
const savedTheme = localStorage.getItem('theme');
const theme = ref(savedTheme === 'light' || savedTheme === 'dark' ? savedTheme : 'system');
const autostart = ref(false),
  startupSaving = ref(false),
  captureEnabled = ref(true),
  captureChanging = ref(false);
const tone = computed(() => (playing.value ? 'green' : busy.value ? 'yellow' : 'red'));
const deviceStates = ref<Record<string, string>>({}),
  attemptCard = ref(''),
  lastPasswordHost = ref(''),
  retry = ref(false),
  retrySending = ref(false);
// sessionStats 保存后端本次累计计数，stats 累加每次增量，避免重连后重复计入。
const sessionStats = ref<Record<string, Record<string, string>>>({});
const statsNames = ref<Record<string, string>>({});
const statsRows = computed(() =>
  Object.entries(stats.value).map(([host, values]) => ({
    host,
    values,
    name: statsNames.value[host] || host,
  })),
);
const sourceOpen = ref(false);
const passwordDevice = computed(
  () => devices.value.find((d) => d.addresses.includes(pending.value))?.name || pending.value,
);
const totalPeak = computed(() => Math.max(0, ...previewPeaks.value));
const sourceLineWidth = computed(() =>
  totalPeak.value > 0
    ? Math.max(0, Math.min(100, ((20 * Math.log10(totalPeak.value) + 60) / 60) * 100))
    : 0,
);
const source = computed(() => inputs.value.find((i) => i.id === settings.value.endpoint));
const cards = computed(() => {
  // 配对卡片和单设备卡片同时保留；仅同 tsid 且恰好两台时生成立体声卡片。
  // igl 主设备排在前面；“左右互换”控制实际声道顺序，不修改发现结果。
  const list: { id: string; members: Device[]; title: string }[] = [];
  const groups = new Map<string, Device[]>();
  for (const d of devices.value) {
    if (d.properties.tsid) {
      const group = groups.get(d.properties.tsid) || [];
      group.push(d);
      groups.set(d.properties.tsid, group);
    }
  }
  for (const [id, members] of groups) {
    if (members.length === 2) {
      members.sort(
        (a, b) =>
          Number(b.properties.igl === '1') - Number(a.properties.igl === '1') ||
          a.service.localeCompare(b.service),
      );
      list.push({ id: 'pair:' + id, members, title: members.map((d) => d.name).join(' + ') });
    }
  }
  for (const d of devices.value) list.push({ id: d.service, members: [d], title: d.name });
  return list;
});
const chosen = computed(() =>
  cards.value.find((c) => c.members.map((d) => d.name).join('|') === selection.value.join('|')),
);
const peaks = computed(() => previewPeaks.value);
const recordingInputs = computed(() =>
  inputs.value.filter((i) => i.flow === 'recording').sort(sortSources),
);
const playbackInputs = computed(() =>
  inputs.value.filter((i) => i.flow === 'playback').sort(sortSources),
);
const sourceGroups = computed(() => [
  {
    flow: 'playback',
    title: '播放设备',
    english: 'Playback',
    description: '采集此设备正在播放的声音',
    items: playbackInputs.value,
  },
  {
    flow: 'recording',
    title: '录音设备',
    english: 'Recording',
    description: '采集此设备的输入声音',
    items: recordingInputs.value,
  },
]);
const channels = computed(() => Array.from({ length: source.value?.channels || 0 }, (_, i) => i));

// 操作方法：设置、设备选择、会话控制与事件处理。
function applyTheme() {
  document.documentElement.dataset.theme =
    theme.value === 'system' ? (systemTheme.matches ? 'dark' : 'light') : theme.value;
}

async function startupChanged() {
  startupSaving.value = true;
  try {
    await invokeCommand('set_autostart', { enabled: autostart.value });
  } catch (e) {
    autostart.value = !autostart.value;
    error.value = '开机自启设置失败：' + String(e);
  } finally {
    startupSaving.value = false;
  }
}

async function toggleCapture() {
  if (busy.value || captureChanging.value) return;
  captureChanging.value = true;
  const enabled = !captureEnabled.value;
  try {
    await invokeCommand('set_capture_enabled', { enabled });
    captureEnabled.value = enabled;
    previewPeaks.value = [0, 0];
    previewError.value = '';
    sourceWarning.value = '';
  } catch (e) {
    error.value = String(e);
  } finally {
    captureChanging.value = false;
  }
}

function logDisplayPath(path: unknown) {
  if (!path) return '';
  return '%APPDATA%/AirPlay Hub/logs/' + String(path).split(/[\\/]/).pop();
}

function format(f: AudioFormat | null | undefined) {
  return f
    ? `${f.rate} Hz · ${f.valid_bits || f.bits}-bit ${f.encoding === 'pcm' ? 'PCM' : f.encoding === 'float32' ? 'float' : f.encoding} · ${f.channels} 声道`
    : '设备未公布格式';
}

async function focusPassword() {
  await nextTick();
  const input = document.querySelector<HTMLInputElement>('.password input');
  input?.focus();
  input?.select();
}

async function resetAuth() {
  try {
    await invokeCommand('forget_auth_policy', { names: selection.value });
    authNotice.value = '已清除所选设备记录，下次连接重新检测。';
  } catch (e) {
    error.value = String(e);
  }
}

async function windowAction(action: string) {
  await call('window_action', { action });
}

async function selectSource(input: Input) {
  if (busy.value || !input.channels) return;
  settings.value.endpoint = input.id;
  sourceOpen.value = false;
  await sourceChanged();
}

function speakerFor(card: (typeof cards.value)[number], side: number) {
  return (
    card.members[card.members.length === 2 && settings.value.speakersSwapped ? 1 - side : side] ||
    card.members[0]
  );
}

function sortSources(a: Input, b: Input) {
  return (
    a.name.localeCompare(b.name, 'en', { sensitivity: 'base', numeric: true }) ||
    a.id.localeCompare(b.id)
  );
}

function sourceIcon(flow: string) {
  return flow === 'playback'
    ? 'M11 4 6 8H3v8h3l5 4V4Zm4 4a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14'
    : 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM6 11v1a6 6 0 0 0 12 0v-1m-6 7v3m-3 0h6';
}

function enterPassword(e: KeyboardEvent) {
  if (!e.isComposing) {
    e.preventDefault();
    void submit();
  }
}

function num(v: unknown, d = 1) {
  return typeof v === 'number' && Number.isFinite(v) ? v.toFixed(d) : '—';
}

function level(v: number) {
  return v > 0 ? `${(20 * Math.log10(v)).toFixed(1)} dBFS` : '−∞ dBFS';
}

async function call<T>(name: string, args?: Record<string, unknown>) {
  try {
    return await invokeCommand<T>(name, args);
  } catch (e) {
    error.value = String(e);
    return undefined;
  }
}

async function persist() {
  await call('save_settings', { settings: settings.value });
}

async function awakeChanged() {
  try {
    await invokeCommand('save_settings', { settings: settings.value });
  } catch (e) {
    settings.value.keepAwake = !settings.value.keepAwake;
    error.value = String(e);
  }
}

async function sourceChanged() {
  // 换来源后旧声道下标可能越界，先恢复合法映射，再保存并启动该端点的预览。
  previewPeaks.value = [0, 0];
  previewError.value = '';
  sourceWarning.value = '';
  settings.value.mapping = [0, (source.value?.channels || 0) > 1 ? 1 : 0];
  await persist();
  await call('monitor_source');
}

function choose(card: (typeof cards.value)[number]) {
  if (!busy.value) {
    if (chosen.value?.id !== card.id) {
      deviceStates.value = {};
      pending.value = '';
      password.value = '';
      retry.value = false;
      retrySending.value = false;
      lastPasswordHost.value = '';
      error.value = '';
    }
    selection.value = card.members.map((d) => d.name);
  }
  expanded.value = expanded.value === card.id ? '' : card.id;
}

async function refresh() {
  refreshing.value = true;
  error.value = '';
  try {
    const d = await call<unknown>('discover_devices');
    if (d !== undefined) {
      devices.value = decodeDevices(d);
      if (!chosen.value) selection.value = [];
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    refreshing.value = false;
  }
}

/**
 * 先清空本次会话的快照，再请求桌面线程启动。密码重试会建立新会话，
 * 暂存输入直到新会话再次发出 password_required，随后立即提交并清空。
 */
async function start(passwordFirst = false) {
  activeDiagnostics.value = settings.value.captureDiagnostics;
  diagnosticLogs.value = [];
  diagnosticPath.value = '';
  previewPeaks.value = [0, 0];
  previewError.value = '';
  activeDetailedLogs.value = settings.value.detailedLogs;
  pipelinePath.value = '';
  error.value = '';
  busy.value = true;
  phase.value = '正在连接';
  session.value = null;
  pending.value = '';
  if (!passwordFirst) {
    password.value = '';
    retry.value = false;
  }
  attemptCard.value = chosen.value?.id || '';
  deviceStates.value[attemptCard.value] = 'yellow';
  connected.value = false;
  telemetry.value = {};
  report.value = {};
  sessionStats.value = {};
  technical.value = {};
  logs.value = [];
  volume.value = null;
  const id = await call<number>('start_stream', {
    names: selection.value,
    settings: settings.value,
    passwordFirst,
  });
  if (id === undefined) {
    busy.value = false;
    phase.value = '连接失败';
    deviceStates.value[attemptCard.value] = 'red';
    retrySending.value = false;
  } else {
    // 工作线程可能在命令返回前发事件；event 已认领 id 时不能再次覆盖它。
    if (session.value === null) session.value = id;
  }
}

async function stop() {
  // 停止是异步请求：禁用遥测更新，但让 finished 负责最终状态和失败信息。
  stopping.value = true;
  phase.value = '正在停止';
  password.value = '';
  pending.value = '';
  await call('stop_stream');
}

async function submit() {
  if (!pending.value || !password.value || sending.value) return;
  if (retry.value && !busy.value) {
    // 密码失败后旧会话已经结束，不能把密码发给旧 session_id。
    retrySending.value = true;
    await start(true);
    return;
  }
  sending.value = true;
  const secret = password.value;
  // 交给桌面命令后不在响应式页面状态中保留密码。
  password.value = '';
  lastPasswordHost.value = pending.value;
  try {
    await invokeCommand('submit_password', {
      sessionId: session.value,
      host: pending.value,
      password: secret,
    });
    pending.value = '';
    phase.value = '正在验证密码';
  } catch (e) {
    error.value = String(e);
  } finally {
    sending.value = false;
  }
}

async function setVolume(value: number) {
  await call('set_volume', { percent: value });
}

async function mute() {
  if (volume.value === 0) await setVolume(lastVolume.value);
  else {
    lastVolume.value = volume.value ?? draft.value;
    await setVolume(0);
  }
}

async function mapping() {
  await call('set_mapping', { mapping: settings.value.mapping });
}

async function swap() {
  const swapped = !settings.value.speakersSwapped;
  try {
    await invokeCommand('set_speaker_order', { swapped });
    settings.value.speakersSwapped = swapped;
  } catch (e) {
    error.value = String(e);
  }
}

/** 仅接收 protocol.ts 已验证的事件；kind 收窄后只能读取该事件拥有的字段。 */
function event(e: StreamEvent) {
  // 新会话启动后旧线程可能仍有排队事件；先按 id 过滤，防止旧 finished 关闭新播放。
  if (session.value !== null && e.session_id !== session.value) return;
  if (session.value === null && busy.value) session.value = e.session_id;
  if (e.kind === 'preparing_source') {
    phase.value = '等待采集稳定';
  }
  if (e.kind === 'password_required') {
    pending.value = e.host;
    phase.value = '需要 AirPlay 密码';
    expanded.value = chosen.value?.id || '';
    if (retrySending.value) {
      retrySending.value = false;
      void submit();
    } else void focusPassword();
  }
  if (e.kind === 'auth_memory_error' || e.kind === 'auth_pipe_error')
    error.value = describeError(e.error, e.kind);
  if (e.kind === 'log_path') {
    logPath.value = logDisplayPath(e.path);
    activeDetailedLogs.value = !!e.detailed_logs;
    pipelinePath.value = logDisplayPath(e.pipeline_path);
  }
  if (e.kind === 'telemetry') {
    if (!busy.value || stopping.value) return;
    telemetry.value = e;
    playing.value = true;
    phase.value = '串流中';
  }
  if (e.kind === 'report') report.value = e.report;
  if (e.kind === 'diagnostic_path') {
    diagnosticPath.value = logDisplayPath(e.path);
    pipelinePath.value = logDisplayPath(e.pipeline_path);
  }
  if (e.kind === 'capture_diagnostic') {
    const c = e.capture || {};
    diagnosticLogs.value.push(
      `${num(e.elapsed_seconds, 1)}s · 采集 ${e.capture_frames} 帧 · 待发送 ${num(e.pending_pcm_ms)} ms · 水位 ${num(e.water_ms)} ms · 校正 ${num(e.correction_ppm)} ppm\n设备位置 ${c.device_position} · 包 ${c.packets} · 时间戳 ${c.packet_qpc_100ns} · 不连续 ${c.discontinuities} · 时间戳错误 ${c.timestamp_errors}${e.error ? '\n错误：' + e.error : ''}`,
    );
    if (diagnosticLogs.value.length > 120) diagnosticLogs.value.shift();
  }
  if (e.kind === 'diagnostic_end')
    diagnosticLogs.value.push('诊断结束：' + JSON.stringify(e.status));
  if (e.kind === 'native') {
    // 原始协议行用于识别标记；展示日志优先用桌面端脱敏后的 safe_line。
    // 不对脱敏文本解析 host，否则多个设备别名会破坏计数归属。
    const line = String(e.line);
    if (e.is_fault || activeDetailedLogs.value) {
      logs.value.push(String(e.safe_line || line));
      if (logs.value.length > 300) logs.value.shift();
    }
    const f = Object.fromEntries(
      [...line.matchAll(/([a-zA-Z_]+)=([^\s]+)/g)].map((m) => [m[1], m[2]]),
    );
    if (line.includes('PACKET_STATS') && f.host) {
      const previous = sessionStats.value[f.host] || {};
      const total = stats.value[f.host] || {};
      for (const key of [
        'sent',
        'send_dropped',
        'sync_dropped',
        'rtx_requested',
        'rtx_resent',
        'rtx_expired',
      ]) {
        // 后端发送累计计数；首次从 0 算增量，下降按 0 处理，避免出现负统计。
        const value = Number(f[key]);
        if (Number.isFinite(value)) {
          total[key] = String(
            Number(total[key] || 0) + Math.max(0, value - Number(previous[key] || 0)),
          );
        }
      }
      stats.value[f.host] = total;
      sessionStats.value[f.host] = f;
      const device = devices.value.find((d) => d.addresses.includes(f.host));
      statsNames.value[f.host] = device?.name || f.host;
    }
    if (line.includes('AUTH_METHOD') || line.includes('TIMING'))
      technical.value[line.includes('TIMING') ? '时钟协议' : '认证方式'] = f.value || line;
    if (line.includes('PCM_READY')) {
      connected.value = true;
      phase.value = '启动音频采集';
      deviceStates.value[attemptCard.value] = 'green';
      password.value = '';
      retry.value = false;
    }
    if (line.includes('VOLUME_CURRENT') || (line.includes('VOLUME_APPLIED') && f.http === '200')) {
      const value =
        f.percent !== undefined
          ? Number(f.percent)
          : f.db !== undefined
            ? Number(f.db) <= -144
              ? 0
              : Math.max(0, Math.min(100, (Number(f.db) + 30) / 0.3))
            : NaN;
      if (Number.isFinite(value)) {
        volume.value = value;
        draft.value = value;
      }
    }
  }
  if (e.kind === 'finished') {
    // finished 是本次会话唯一收尾入口；仍保留最后的遥测和报告供用户诊断。
    password.value = '';
    busy.value = false;
    playing.value = false;
    connected.value = false;
    telemetry.value.peaks = [0, 0];
    stopping.value = false;
    pending.value = '';
    phase.value = e.error ? '串流失败' : '已停止';
    deviceStates.value[attemptCard.value] = e.error ? 'red' : '';
    if (e.error) {
      error.value = e.error;
      logs.value.push(String(e.safe_error || e.error));
      if (logs.value.length > 300) logs.value.shift();
      if (String(e.error).includes('PASSWORD_REJECTED') && lastPasswordHost.value) {
        retry.value = true;
        pending.value = lastPasswordHost.value;
        expanded.value = attemptCard.value;
        void focusPassword();
      } else {
        password.value = '';
        retry.value = false;
      }
    } else {
      password.value = '';
      retry.value = false;
    }
  }
}

// 响应式监听只协调页面；桌面事件在 onMounted 注册，并在卸载时统一释放。
watch(
  theme,
  (value) => {
    applyTheme();
    localStorage.setItem('theme', value);
  },
  { immediate: true },
);

systemTheme.addEventListener('change', applyTheme);

const unlisteners: (() => void)[] = [];
let disposed = false;
onUnmounted(() => {
  disposed = true;
  systemTheme.removeEventListener('change', applyTheme);
  unlisteners.forEach((unlisten) => unlisten());
});

// 监听注册是异步的：若组件先卸载，返回的监听必须立即释放，避免回调重复累积。
async function registerListener(name: string, handler: (payload: unknown) => void) {
  const unlisten = await listen<unknown>(name, (e) => {
    if (disposed) return;
    try {
      handler(e.payload);
    } catch (e) {
      error.value = String(e);
    }
  });
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}

watch(pending, () => {
  if (pending.value) void focusPassword();
});

watch(pending, () => {
  if (pending.value) sourceOpen.value = false;
});

watch(page, () => (sourceOpen.value = false));

watch(busy, () => (sourceOpen.value = false));

onMounted(async () => {
  try {
    appVersion.value = await getVersion();
    // 先订阅再 initialize/monitor_source，避免错过初始化期间的采集事件。
    await registerListener('stream-event', (payload) => {
      const decoded = decodeStreamEvent(payload);
      if (decoded) event(decoded);
    });
    await registerListener('source-level', (payload) => {
      const e = decodeSourceLevel(payload);
      // 旧来源切换过程中可能还有回调，只更新当前选中端点。
      if (e.endpoint === settings.value.endpoint) {
        if (e.captureEnabled !== undefined) captureEnabled.value = e.captureEnabled;
        if (e.peaks) previewPeaks.value = e.peaks;
        if (e.error !== undefined) previewError.value = e.error || '';
        if (e.warning) {
          sourceWarning.value = e.warning;
          logs.value.push('[WARN] ' + e.warning);
          if (logs.value.length > 300) logs.value.shift();
        }
      }
    });
    const s = decodeInitialization(await invokeCommand<unknown>('initialize'));
    if (disposed) return;
    devices.value = s.devices;
    inputs.value = s.inputs;
    settings.value = s.settings;
    autostart.value = !!s.autostart;
    captureEnabled.value = s.captureEnabled !== false;
    logPath.value = '%APPDATA%/AirPlay Hub/logs';
    if (s.autostartError) error.value = '无法读取开机自启状态：' + s.autostartError;
    if (s.awakeError) {
      settings.value.keepAwake = !!s.awakeActive;
      error.value = s.awakeError;
    }
    if (!inputs.value.some((i) => i.id === settings.value.endpoint && i.channels)) {
      settings.value.endpoint = '';
    }
    selection.value = cards.value[0]?.members.map((d) => d.name) || [];
    if (!s.backendAvailable) error.value = '缺少原生后端，请从完整 dist 文件夹启动';
    await persist();
    if (source.value) await call('monitor_source');
    ready.value = s.backendAvailable;
    await invokeCommand('ui_ready', { inputs: inputs.value });
    if (ready.value) await refresh();
  } catch (e) {
    error.value = `桌面后端未就绪：${String(e)}`;
  }
});
</script>
<template>
  <svg width="0" height="0" aria-hidden="true" style="position: absolute; pointer-events: none">
    <defs>
      <mask id="stereo-icon-gap" maskUnits="userSpaceOnUse" x="0" y="0" width="44" height="36">
        <rect width="44" height="36" fill="white" />
        <circle cx="28" cy="23" r="13" fill="black" />
      </mask>
    </defs>
  </svg>
  <div class="window-buttons">
    <button
      aria-label="设置"
      title="设置"
      :class="{ active: page === '设置' }"
      @click="page = page === '设置' ? '播放' : '设置'"
    >
      <svg viewBox="0 0 24 24">
        <path
          d="m10 3-.6 3-2.6 1.5-2.9-1-2 3.5 2.3 2v3l-2.3 2 2 3.5 2.9-1 2.6 1.5.6 3h4l.6-3 2.6-1.5 2.9 1 2-3.5-2.3-2v-3l2.3-2-2-3.5-2.9 1L14.6 6 14 3Z"
        />
        <circle cx="12" cy="13.5" r="3" />
      </svg></button
    ><button aria-label="最小化" @click="windowAction('minimize')">
      <svg viewBox="0 0 16 16"><path d="M3 8h10" /></svg></button
    ><button aria-label="最大化" @click="windowAction('maximize')">
      <svg viewBox="0 0 16 16"><rect x="3.5" y="3.5" width="9" height="9" /></svg></button
    ><button
      :aria-label="settings.closeAction === 'quit' ? '退出应用' : '收起到托盘'"
      class="window-close"
      @click="windowAction('close')"
    >
      <svg viewBox="0 0 16 16"><path d="m4 4 8 8M12 4l-8 8" /></svg>
    </button>
  </div>
  <div class="shell">
    <main>
      <header>
        <div
          class="window-drag"
          @mousedown.left="windowAction('drag')"
          @dblclick="windowAction('maximize')"
        ></div>
        <div class="heading">
          <div>
            <h1>{{ page === '播放' ? 'AirPlay Hub' : '设置' }}</h1>
            <p>
              {{
                page === '播放'
                  ? 'Windows → AirPlay Hub'
                  : `AirPlay Hub · v${displayVersion || '—'}`
              }}
            </p>
          </div>
        </div>
      </header>
      <div v-if="error" class="alert">{{ error }}<button @click="error = ''">×</button></div>
      <div v-if="page === '设置'" class="settings-toolbar">
        <button class="back-play" aria-label="返回播放" title="返回播放" @click="page = '播放'">
          <svg viewBox="0 0 24 24"><path d="m14 6-6 6 6 6M8 12h12" /></svg>
        </button>
        <div class="settings-tabs" role="tablist" aria-label="设置子标签">
          <button
            v-for="tab in ['常规', '运行统计', '技术详情', '日志']"
            :key="tab"
            role="tab"
            :aria-selected="settingsTab === tab"
            :class="{ current: settingsTab === tab }"
            @click="settingsTab = tab"
          >
            {{ tab }}
          </button>
        </div>
      </div>
      <template v-if="page === '播放'">
        <section class="panel">
          <div class="section-title">
            <h2>AirPlay 设备</h2>
            <div class="actions">
              <span class="status"><i :class="['status-light', tone]"></i>{{ phase }}</span>
            </div>
          </div>
          <div class="volume">
            <button :disabled="!playing" @click="mute">
              {{ volume === 0 ? '取消静音' : '静音' }}</button
            ><span>音量</span
            ><input
              v-model.number="draft"
              type="range"
              min="0"
              max="100"
              :disabled="!playing"
              aria-label="HomePod 音量"
              @change="setVolume(draft)"
            /><strong>{{ volume === null ? '—' : Math.round(volume) + '%' }}</strong>
          </div>
          <p v-if="!cards.length" class="empty">
            未发现设备。点击刷新，确保 HomePod 与电脑在同一局域网。
          </p>
          <article
            v-for="card in cards"
            :key="card.id"
            :class="['device', deviceStates[card.id], { selected: chosen?.id === card.id }]"
          >
            <button
              class="device-row"
              :disabled="busy && chosen?.id !== card.id"
              @click="choose(card)"
            >
              <span class="radio">{{ chosen?.id === card.id ? '●' : '○' }}</span
              ><span class="speaker"
                ><svg class="device-speaker-icon" viewBox="0 0 44 36" aria-hidden="true">
                  <template v-if="card.members.length === 2">
                    <circle cx="16" cy="14" r="11" mask="url(#stereo-icon-gap)" />
                    <circle cx="28" cy="23" r="11" />
                  </template>
                  <circle v-else cx="22" cy="18" r="12" /></svg
              ></span>
              <div class="device-name">
                <strong>{{ card.title }}</strong
                ><small>{{
                  card.members.length === 2 ? '立体声对' : card.members[0]?.addresses[0]
                }}</small>
              </div>
              <span class="badge">{{ card.members.length === 2 ? 'Stereo' : 'AirPlay 2' }}</span
              ><span>{{ expanded === card.id ? '⌃' : '⌄' }}</span>
            </button>
            <div v-if="expanded === card.id" class="device-details">
              <div
                v-if="connected && busy && chosen?.id === card.id"
                :class="['channel-pair', { single: card.members.length === 1 }]"
              >
                <div>
                  <small>左{{ card.members.length === 2 ? '扬声器' : '声道' }}</small
                  ><strong>{{ speakerFor(card, 0)?.name }}</strong>
                  <div class="meter">
                    <i :style="{ width: Math.min(100, peaks[0] * 100) + '%' }"></i>
                  </div>
                  <small>{{ level(peaks[0]) }} · 输入 {{ settings.mapping[0] + 1 }}</small>
                </div>
                <button v-if="card.members.length === 2" title="交换扬声器位置" @click="swap">
                  ⇄
                </button>
                <div>
                  <small>右{{ card.members.length === 2 ? '扬声器' : '声道' }}</small
                  ><strong>{{ speakerFor(card, 1)?.name }}</strong>
                  <div class="meter">
                    <i :style="{ width: Math.min(100, peaks[1] * 100) + '%' }"></i>
                  </div>
                  <small>{{ level(peaks[1]) }} · 输入 {{ settings.mapping[1] + 1 }}</small>
                </div>
              </div>
              <div v-else class="device-info">
                {{ card.members.map((d) => `${d.name} · ${d.addresses[0]}:${d.port}`).join(' / ') }}
              </div>
            </div>
          </article>
        </section>
        <p class="footnote">窗口关闭动作可在设置中选择；托盘右键“退出”始终停止串流并关闭软件。</p>
      </template>
      <template v-if="page === '设置' && settingsTab === '运行统计'"
        ><div class="metrics">
          <section class="panel">
            <small>运行时长</small>
            <h2>{{ num(telemetry.elapsed_seconds, 0) }} <em>s</em></h2>
          </section>
          <section class="panel">
            <small>流水线水位</small>
            <h2>{{ num(telemetry.water_ms) }} <em>ms</em></h2>
          </section>
          <section class="panel">
            <small>漂移校正</small>
            <h2>{{ num(telemetry.controller?.correction_ppm) }} <em>ppm</em></h2>
          </section>
        </div>
        <section class="panel">
          <h2>音频流水线</h2>
          <dl>
            <dt>采集帧数</dt>
            <dd>{{ telemetry.capture_frames?.toLocaleString() || '—' }}</dd>
            <dt>输出帧数</dt>
            <dd>{{ telemetry.output_frames?.toLocaleString() || '—' }}</dd>
            <dt>协议提前量</dt>
            <dd>{{ telemetry.lead_ms ?? '—' }} ms</dd>
            <dt>估计发送端延迟</dt>
            <dd>{{ num((telemetry.lead_ms ?? NaN) + (telemetry.water_ms ?? NaN)) }} ms</dd>
            <dt>采集不连续 / 时间戳错误</dt>
            <dd>
              {{ report.capture?.discontinuities ?? '停止后汇总' }} /
              {{ report.capture?.timestamp_errors ?? '—' }}
            </dd>
            <dt>转换汇总</dt>
            <dd>
              <code>{{
                report.conversion ? JSON.stringify(report.conversion) : '停止后汇总'
              }}</code>
            </dd>
          </dl>
          <small>延迟估计不包含应用和扬声器实际发声耗时。</small>
        </section>
        <section class="panel">
          <h2>设备传输</h2>
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>设备</th>
                  <th>发送包</th>
                  <th>本地丢包</th>
                  <th>重传请求</th>
                  <th>已重传</th>
                  <th>过期</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in statsRows" :key="row.host">
                  <td>{{ row.name }}</td>
                  <td>{{ row.values.sent ?? '—' }}</td>
                  <td>{{ row.values.send_dropped ?? '—' }}</td>
                  <td>{{ row.values.rtx_requested ?? '—' }}</td>
                  <td>{{ row.values.rtx_resent ?? '—' }}</td>
                  <td>{{ row.values.rtx_expired ?? '—' }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <small>约每 5 秒更新，累计本次软件运行中所有设备的统计，退出软件后清空。</small>
        </section></template
      >
      <template v-if="page === '设置' && settingsTab === '技术详情'"
        ><section class="panel capture-panel">
          <h2>采集与处理</h2>
          <dl class="processing">
            <dt>设备格式</dt>
            <dd>{{ format(source?.device_format) }}</dd>
            <dt>WASAPI 采集格式</dt>
            <dd>{{ format(source?.mix_format) }}</dd>
            <dt>发送格式</dt>
            <dd>44100 Hz · 16-bit PCM → ALAC</dd>
            <dt>Buffer<small>下次连接生效</small></dt>
            <dd>
              <div class="unit small-unit" title="采集流水线预缓冲">
                <input
                  aria-label="采集 Buffer"
                  v-model.number="settings.buffer"
                  type="number"
                  min="64"
                  max="512"
                  :disabled="busy"
                  @change="persist"
                /><span>ms</span>
              </div>
            </dd>
            <dt>左右电平</dt>
            <dd>
              {{ level(peaks[0]) }} / {{ level(peaks[1]) }}
              <div class="meter">
                <i :style="{ width: Math.min(100, Math.max(...peaks) * 100) + '%' }"></i>
              </div>
            </dd>
            <dt>密码要求记录</dt>
            <dd>
              <button :disabled="busy || !selection.length" @click="resetAuth">
                重新检测所选设备</button
              ><small>{{
                authNotice || '只记住密码要求，不保存密码。取消设备密码后可重新检测。'
              }}</small>
            </dd>
            <dt>会话协议</dt>
            <dd>{{ JSON.stringify(technical) }}</dd>
          </dl>
        </section>
        <section class="panel">
          <h2>设备能力</h2>
          <details v-for="d in devices">
            <summary>{{ d.name }} · {{ d.addresses.join(', ') }}:{{ d.port }}</summary>
            <pre>{{ JSON.stringify(d.properties, null, 2) }}</pre>
          </details>
        </section> </template
      ><template v-if="page === '设置' && settingsTab === '常规'"
        ><section class="panel general-panel">
          <h2>应用行为</h2>
          <dl class="processing">
            <dt>开机自启</dt>
            <dd>
              <label class="log-switch"
                ><input
                  type="checkbox"
                  v-model="autostart"
                  :disabled="startupSaving"
                  @change="startupChanged"
                />登录 Windows 后启动 AirPlay Hub</label
              ><small>默认关闭。启用后请保留软件所在位置。</small>
            </dd>
            <dt>保持系统唤醒</dt>
            <dd>
              <label class="log-switch"
                ><input
                  type="checkbox"
                  v-model="settings.keepAwake"
                  @change="awakeChanged"
                />避免系统自动睡眠</label
              ><small>开启后在应用运行期间生效，包括托盘状态；允许屏幕熄灭，退出应用后恢复。</small>
            </dd>
            <dt>右上角关闭按钮</dt>
            <dd>
              <select aria-label="窗口关闭动作" v-model="settings.closeAction" @change="persist">
                <option value="tray">收起到托盘</option>
                <option value="quit">退出应用</option></select
              ><small>立即生效。托盘右键“退出”始终关闭应用。</small>
            </dd>
            <dt>外观</dt>
            <dd>
              <select aria-label="外观" v-model="theme">
                <option value="system">跟随系统</option>
                <option value="light">浅色模式</option>
                <option value="dark">深色模式</option>
              </select>
            </dd>
          </dl>
        </section>
        <section class="panel general-panel">
          <h2>播放设置</h2>
          <dl class="processing">
            <dt>播放提前量<small>下次连接生效</small></dt>
            <dd>
              <div class="unit small-unit">
                <input
                  aria-label="播放提前量"
                  v-model.number="settings.latency"
                  type="number"
                  min="250"
                  max="2000"
                  :disabled="busy"
                  @change="persist"
                /><span>ms</span>
              </div>
            </dd>
          </dl>
        </section>
        <section class="panel general-panel">
          <h2>输入声道映射</h2>
          <small>选择音频来源中的左右输入。交换扬声器位置不会改变这里的设置。</small>
          <dl class="processing">
            <dt>左输出取样</dt>
            <dd>
              <select
                aria-label="左输出取样"
                v-model.number="settings.mapping[0]"
                @change="mapping"
              >
                <option v-for="ch in channels" :value="ch">输入声道 {{ ch + 1 }}</option>
              </select>
            </dd>
            <dt>右输出取样</dt>
            <dd>
              <select
                aria-label="右输出取样"
                v-model.number="settings.mapping[1]"
                @change="mapping"
              >
                <option v-for="ch in channels" :value="ch">输入声道 {{ ch + 1 }}</option>
              </select>
            </dd>
          </dl>
        </section></template
      ><template v-if="page === '设置' && settingsTab === '日志'"
        ><section class="panel">
          <div class="section-title">
            <h2>会话日志</h2>
            <button @click="call('open_logs')">打开日志目录</button>
          </div>
          <label class="log-switch"
            ><input
              type="checkbox"
              v-model="settings.detailedLogs"
              :disabled="busy"
              @change="persist"
            />保存详细日志</label
          ><small
            >下次连接生效。开启后保存完整协议日志和每秒采集、重采样、队列、发送进度；关闭时只保存关键故障。</small
          ><small class="log-location">{{ logPath }}</small
          ><small v-if="pipelinePath" class="log-location">流水线：{{ pipelinePath }}</small
          ><small
            >显示最近 300 行{{
              activeDetailedLogs ? '详细输出，完整内容持续保存到文件。' : '关键故障。'
            }}</small
          >
          <pre class="logs">{{
            logs.join('\n') || (activeDetailedLogs ? '等待详细日志输出…' : '暂无关键故障。')
          }}</pre>
          <details v-if="report.device">
            <summary>完整会话报告</summary>
            <pre>{{ JSON.stringify(report, null, 2) }}</pre>
          </details>
        </section>
        <section class="panel">
          <div class="section-title">
            <h2>采集逐包诊断</h2>
            <button @click="diagnosticLogs = []">清空显示</button>
          </div>
          <label class="log-switch"
            ><input
              type="checkbox"
              v-model="settings.captureDiagnostics"
              :disabled="busy"
              @change="persist"
            />启用额外采集诊断</label
          ><small
            >下次连接生效，独立于“保存详细日志”。保存逐包时序、帧数、内容指纹、处理耗时和队列进度；不保存原始音频。滚动保存当前和前
            3 个片段，每片段约 8 MiB，故障时保留最近现场。</small
          ><small v-if="diagnosticPath" class="log-location">逐包：{{ diagnosticPath }}</small
          ><small>窗口每秒更新，保留最近 120 条摘要；逐包数据保存到文件。</small>
          <pre class="logs">{{
            diagnosticLogs.join('\n\n') ||
            (activeDiagnostics ? '等待采集诊断输出…' : '未启用采集诊断。')
          }}</pre>
        </section></template
      >
    </main>
    <footer class="audio-dock">
      <div
        v-if="pending"
        class="password password-popup"
        role="dialog"
        aria-label="AirPlay 密码验证"
      >
        <label
          ><input
            v-model="password"
            type="password"
            autocomplete="off"
            :placeholder="'输入 ' + passwordDevice + ' 的 AirPlay 密码'"
            :aria-label="'输入 ' + passwordDevice + ' 的 AirPlay 密码'"
            maxlength="1023"
            @keydown.enter="enterPassword" /></label
        ><button class="primary" :disabled="!password || sending || retrySending" @click="submit">
          {{ sending || retrySending ? '验证中…' : '确认连接' }}</button
        ><small v-if="retry" class="password-error" role="alert">密码不正确，请重新输入。</small
        ><small>仅在本次会话使用，不保存密码。</small>
      </div>
      <button
        class="capture-toggle"
        role="switch"
        :aria-checked="!!source && captureEnabled"
        :disabled="busy || captureChanging || !ready || !source"
        :title="
          !source
            ? '请先选择音频流来源'
            : busy
              ? '串流期间需要采集，请先停止串流'
              : captureEnabled
                ? '停止采集，释放音频设备'
                : '开启音频采集'
        "
        @click="toggleCapture"
      >
        <span class="capture-switch"><i></i></span
        ><span>{{
          !source
            ? '待选择来源'
            : captureChanging
              ? '切换中…'
              : captureEnabled
                ? '采集中'
                : '采集已关闭'
        }}</span>
      </button>
      <div class="dock-source">
        <label id="source-label">音频来源</label>
        <div class="source-picker" @keydown.esc="sourceOpen = false">
          <div v-if="sourceOpen" class="source-popup" id="source-options" aria-label="音频来源选项">
            <section
              v-for="group in sourceGroups"
              :key="group.flow"
              :class="['source-section', group.flow]"
              role="group"
              :aria-label="group.title"
            >
              <div class="source-group">
                <svg class="source-icon" viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="sourceIcon(group.flow)" />
                </svg>
                <div>
                  <strong
                    >{{ group.title }} <small>{{ group.english }}</small></strong
                  ><small>{{ group.description }}</small>
                </div>
              </div>
              <button
                v-for="input in group.items"
                :key="input.id"
                :disabled="!input.channels"
                :aria-pressed="settings.endpoint === input.id"
                @click="selectSource(input)"
              >
                <span>{{ input.name }}</span
                ><span class="source-check" aria-hidden="true">{{
                  settings.endpoint === input.id ? '✓' : ''
                }}</span></button
              ><small v-if="!group.items.length" class="source-empty">没有可用设备</small>
            </section>
          </div>
          <button
            class="source-trigger"
            :disabled="busy"
            :aria-expanded="sourceOpen"
            aria-controls="source-options"
            :aria-labelledby="
              source ? 'source-label source-kind source-name' : 'source-label source-name'
            "
            @click="sourceOpen = !sourceOpen"
          >
            <span class="source-selection"
              ><span v-if="source" id="source-kind" :class="['source-type', source.flow]"
                ><svg class="source-icon" viewBox="0 0 24 24" aria-hidden="true">
                  <path :d="sourceIcon(source.flow)" /></svg
                >{{ source.flow === 'playback' ? '播放' : '录音' }}</span
              ><span id="source-name">{{ source?.name || '请选择音频流来源' }}</span></span
            ><svg viewBox="0 0 16 16">
              <path :d="sourceOpen ? 'm4 6 4 4 4-4' : 'm4 10 4-4 4 4'" /></svg
            ><span class="source-total-meter"
              ><i :style="{ width: sourceLineWidth + '%' }"></i
            ></span>
          </button>
        </div>
        <div class="source-format">
          <small>{{ source ? format(source.device_format) : '未选择音频来源' }}</small
          ><small>{{ level(totalPeak) }}</small>
        </div>
        <small v-if="previewError" class="source-notice">电平预览：{{ previewError }}</small
        ><small v-if="sourceWarning" class="source-notice">{{ sourceWarning }}</small>
      </div>
      <div class="dock-actions">
        <button v-if="busy" class="primary" :disabled="stopping" @click="stop">
          {{ stopping ? '停止中…' : '停止串流' }}</button
        ><button
          v-else
          class="primary"
          :disabled="!ready || !selection.length || !settings.endpoint || refreshing"
          @click="start()"
        >
          开始串流</button
        ><button :disabled="busy || refreshing || !ready" @click="refresh">
          {{ refreshing ? '发现中…' : '刷新' }}
        </button>
      </div>
    </footer>
    <div
      v-if="sourceOpen"
      class="source-dismiss"
      @click="sourceOpen = false"
      aria-hidden="true"
    ></div>
  </div>
</template>
