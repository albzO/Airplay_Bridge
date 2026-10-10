<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, nextTick, watch } from 'vue';
import { getVersion } from '@tauri-apps/api/app';
import { listen } from '@tauri-apps/api/event';
import { invokeCommand } from './commands';
import { useStreamSession } from './useStreamSession';
import { useDiagnostics, nativeFields } from './useDiagnostics';
import SourcePicker from './SourcePicker.vue';
import DeviceCard from './DeviceCard.vue';
import GeneralSettings from './GeneralSettings.vue';
import RuntimeStats from './RuntimeStats.vue';
import TechnicalDetails from './TechnicalDetails.vue';
import LogView from './LogView.vue';
import { level, formatAudioFormat } from './display';
import {
  decodeDevices,
  decodeInitialization,
  decodeSourceLevel,
  decodeStreamEvent,
} from './protocol';
import type { Device, DeviceCardData, Input, Settings, StreamEvent } from './types';

/**
 * 阅读顺序：页面状态 → computed 设备/来源视图 → 用户操作 → event → onMounted。
 * 音频采集和协议连接由桌面端负责；页面只发送命令并显示经过校验的事件。
 * 命令成功表示请求已被接受，是否开始播放、是否结束要以异步事件为准。
 * Read in this order: state, computed device/source views, user actions, event, onMounted.
 * The desktop owns capture and protocol connections; the page sends commands and displays
 * validated events. Command success accepts a request; events determine playback and completion.
 */
// 设置是下一次连接的配置；activeDetailedLogs/activeDiagnostics 是本次会话的快照。
// Settings configure the next connection; activeDetailedLogs/activeDiagnostics snapshot this session.
const authNotice = ref('');
const appVersion = ref('');
const displayVersion = computed(() => appVersion.value.replace(/^(\d+\.\d+)\.0$/, '$1'));
const settingsTab = ref('常规');
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
function copySettings(value: Settings): Settings {
  return { ...value, mapping: [...value.mapping] };
}
let confirmedSettings = copySettings(settings.value);
const settingsSaving = ref(false),
  sourceChanging = ref(false);
const diagnostics = useDiagnostics(() => devices.value);
const {
  activeDetailedLogs,
  activeDiagnostics,
  pipelinePath,
  diagnosticPath,
  diagnosticLogs,
  telemetry,
  report,
  stats,
  statsNames,
  technical,
  logs,
  logPath,
  clearDiagnostics,
} = diagnostics;
const selection = ref<string[]>([]),
  expanded = ref(''),
  refreshing = ref(false),
  ready = ref(false);
const {
  busy,
  playing,
  stopping,
  connected,
  phase,
  error,
  session,
  pending,
  password,
  sending,
  retry,
  retrySending,
  start,
  stop,
  submit,
  event,
  clearRetry,
} = useStreamSession({
  startArgs: () => ({ names: selection.value, settings: settings.value }),
  reset: resetSessionView,
  showPassword: (isRetry) => {
    expanded.value = isRetry ? attemptCard.value : chosen.value?.id || '';
    void focusPassword();
  },
  ready: () => {
    deviceStates.value[attemptCard.value] = 'green';
  },
  startFailed: () => {
    deviceStates.value[attemptCard.value] = 'red';
  },
  render: renderSessionEvent,
});
const volume = ref<number | null>(null),
  draft = ref(50),
  lastVolume = ref(50);
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
const savedTheme = localStorage.getItem('theme');
const theme = ref(savedTheme === 'light' || savedTheme === 'dark' ? savedTheme : 'system');
const autostart = ref(false),
  startupSaving = ref(false),
  captureEnabled = ref(false),
  captureChanging = ref(false);
const settingsLocked = computed(
  () => settingsSaving.value || sourceChanging.value || captureChanging.value,
);
const tone = computed(() => (playing.value ? 'green' : busy.value ? 'yellow' : 'red'));
const deviceStates = ref<Record<string, string>>({}),
  attemptCard = ref('');
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
  // Keep both paired and individual cards; create a pair only for exactly two members sharing tsid.
  // Order the igl primary first; swapping left/right affects playback without changing discovery data.
  const list: DeviceCardData[] = [];
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

// 操作方法：设置、设备选择、会话控制与事件处理。
// User actions: settings, device selection, session control and event handling.
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
  initialCapturePending = false;
  await setCaptureEnabled(!captureEnabled.value);
}

// 自动启动与手动开关走同一命令，页面状态在命令成功后更新；失败时保留关闭状态供重试。
// Auto-start and manual toggles share one command; update state only on success and keep failures retryable.
async function setCaptureEnabled(enabled: boolean) {
  if (busy.value || captureChanging.value || settingsSaving.value) return;
  captureChanging.value = true;
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

// 首次开启必须同时满足页面已显示和来源有效；无来源时保留一次自动开启机会。
// 消耗机会后不再干预手动关闭，切换设备不会再次强制打开采集。
// The first enable requires a rendered UI and valid source; defer it until selection if needed.
// Consume that opportunity once, then respect manual disabling across source changes.
async function startInitialCapture() {
  if (disposed || !uiInitialized || !initialCapturePending || !source.value) return;
  initialCapturePending = false;
  if (captureEnabled.value) await call('monitor_source');
  else await setCaptureEnabled(true);
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
  if (busy.value || settingsLocked.value || !input.channels) return;
  sourceChanging.value = true;
  settings.value.endpoint = input.id;
  sourceOpen.value = false;
  try {
    await sourceChanged();
  } finally {
    sourceChanging.value = false;
  }
}

function enterPassword(e: KeyboardEvent) {
  if (!e.isComposing) {
    e.preventDefault();
    void submit();
  }
}

async function call<T>(name: string, args?: Record<string, unknown>) {
  try {
    return await invokeCommand<T>(name, args);
  } catch (e) {
    error.value = String(e);
    return undefined;
  }
}

// 所有持久化设置共用一个事务；禁止携带另一条未确认的更改提交整份快照。
// Serialize persisted edits so another command cannot save an unconfirmed full snapshot.
async function commitSettings(name: string, args: Record<string, unknown>, next: Settings) {
  if (disposed || settingsSaving.value) return false;
  settingsSaving.value = true;
  try {
    await invokeCommand(name, args);
    if (disposed) return false;
    confirmedSettings = copySettings(next);
    settings.value = copySettings(next);
    return true;
  } catch (e) {
    if (!disposed) {
      settings.value = copySettings(confirmedSettings);
      error.value = String(e);
    }
    return false;
  } finally {
    settingsSaving.value = false;
  }
}

async function persist() {
  const next = copySettings(settings.value);
  return commitSettings('save_settings', { settings: next }, next);
}

async function awakeChanged() {
  await persist();
}

async function sourceChanged() {
  // 换来源后旧声道下标可能越界，先恢复合法映射，再保存并启动该端点的预览。
  // A new source may invalidate old channel indices; reset the mapping before saving and previewing it.
  previewPeaks.value = [0, 0];
  previewError.value = '';
  sourceWarning.value = '';
  settings.value.mapping = [0, (source.value?.channels || 0) > 1 ? 1 : 0];
  if (!(await persist()) || disposed) return;
  if (uiInitialized && initialCapturePending) await startInitialCapture();
  else await call('monitor_source');
}

function choose(card: (typeof cards.value)[number]) {
  if (!busy.value) {
    if (chosen.value?.id !== card.id) {
      deviceStates.value = {};
      clearRetry();
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

// 页面只清空本次展示快照；会话生命周期、密码和事件归属由 composable 管理。
// Reset presentation snapshots here; the composable owns session lifetime, passwords and attribution.
function resetSessionView() {
  diagnostics.beginSession(settings.value);
  previewPeaks.value = [0, 0];
  previewError.value = '';
  attemptCard.value = chosen.value?.id || '';
  deviceStates.value[attemptCard.value] = 'yellow';
  volume.value = null;
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
  const next = copySettings(settings.value);
  await commitSettings('set_mapping', { mapping: next.mapping }, next);
}

async function swap() {
  if (settingsLocked.value) return;
  const next = copySettings(settings.value);
  next.speakersSwapped = !next.speakersSwapped;
  await commitSettings('set_speaker_order', { swapped: next.speakersSwapped }, next);
}

/**
 * 仅接收 protocol.ts 已验证的事件；kind 收窄后只能读取该事件拥有的字段。
 * Accept only events validated by protocol.ts; narrowing kind exposes only that variant's fields.
 */
function renderSessionEvent(e: StreamEvent) {
  diagnostics.accept(e);
  if (
    e.kind === 'native' &&
    (e.line.includes('VOLUME_CURRENT') || e.line.includes('VOLUME_APPLIED'))
  ) {
    const line = e.line;
    const f = nativeFields(line);
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
    deviceStates.value[attemptCard.value] = e.error ? 'red' : '';
  }
}

// 响应式监听只协调页面；桌面事件在 onMounted 注册，并在卸载时统一释放。
// Reactive watchers coordinate the UI; register desktop events on mount and release them on unmount.
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
let uiInitialized = false;
let initialCapturePending = true;
onUnmounted(() => {
  disposed = true;
  systemTheme.removeEventListener('change', applyTheme);
  unlisteners.forEach((unlisten) => unlisten());
});

// 监听注册是异步的：若组件先卸载，返回的监听必须立即释放，避免回调重复累积。
// Registration is asynchronous; immediately release late listeners if the component already unmounted.
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
    // 先订阅再初始化及开启采集，避免错过首次创建来源时的电平和故障事件。
    // Subscribe before initialization and enabling capture to catch the first source's levels and faults.
    await registerListener('stream-event', (payload) => {
      const decoded = decodeStreamEvent(payload);
      if (decoded) event(decoded);
    });
    await registerListener('source-level', (payload) => {
      const e = decodeSourceLevel(payload);
      // 旧来源切换过程中可能还有回调，只更新当前选中端点。
      // A replaced source can still emit callbacks; update only the currently selected endpoint.
      if (e.endpoint === settings.value.endpoint) {
        if (e.captureEnabled !== undefined) captureEnabled.value = e.captureEnabled;
        if (e.peaks) previewPeaks.value = e.peaks;
        if (e.error !== undefined) previewError.value = e.error || '';
        if (e.warning) {
          sourceWarning.value = e.warning;
          diagnostics.recordSourceWarning(e.warning);
        }
      }
    });
    const s = decodeInitialization(await invokeCommand<unknown>('initialize'));
    if (disposed) return;
    devices.value = s.devices;
    inputs.value = s.inputs;
    settings.value = copySettings(s.settings);
    confirmedSettings = copySettings(s.settings);
    autostart.value = !!s.autostart;
    captureEnabled.value = s.captureEnabled;
    diagnostics.setDataMode(s.dataMode);
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
    const settingsSaved = await persist();
    // 冷启动时桌面采集保持关闭。先提交页面状态，再跨过一次绘制，之后才创建 WASAPI 来源。
    // 两次动画帧回调之间浏览器可完成首次绘制；页面卸载后不再自动开启采集。
    // Keep desktop capture off on cold start. Commit UI state and allow a paint before creating WASAPI.
    // Two animation frames allow the first paint in between; never auto-enable after the page unmounts.
    await nextTick();
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
    if (disposed) return;
    await invokeCommand('ui_ready', { inputs: inputs.value });
    if (disposed) return;
    uiInitialized = true;
    if (settingsSaved) await startInitialCapture();
    ready.value = s.backendAvailable;
    if (ready.value) {
      const initializationError = error.value;
      await refresh();
      // 自动发现成功不应清除初始化保存/采集失败；新的发现错误仍优先显示。
      // Successful automatic discovery must retain initialization failures; a discovery error takes precedence.
      if (!error.value) error.value = initializationError;
    }
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
          <DeviceCard
            v-for="card in cards"
            :key="card.id"
            :card="card"
            :state="deviceStates[card.id]"
            :selected="chosen?.id === card.id"
            :expanded="expanded === card.id"
            :disabled="busy && chosen?.id !== card.id"
            :active="connected && busy && chosen?.id === card.id"
            :swapped="settings.speakersSwapped"
            :swap-disabled="settingsLocked"
            :mapping="settings.mapping"
            :peaks="peaks"
            :level-labels="[level(peaks[0]), level(peaks[1])]"
            @choose="choose(card)"
            @swap="swap"
          />
        </section>
        <p class="footnote">窗口关闭动作可在设置中选择；托盘右键“退出”始终停止串流并关闭软件。</p>
      </template>
      <RuntimeStats
        v-if="page === '设置' && settingsTab === '运行统计'"
        :telemetry="telemetry"
        :report="report"
        :stats="stats"
        :names="statsNames"
      />
      <TechnicalDetails
        v-if="page === '设置' && settingsTab === '技术详情'"
        v-model:buffer="settings.buffer"
        :source="source"
        :busy="busy"
        :saving="settingsLocked"
        :can-reset-auth="selection.length > 0"
        :auth-notice="authNotice"
        :peaks="peaks"
        :technical="technical"
        :devices="devices"
        @persist="persist"
        @reset-auth="resetAuth"
      /><GeneralSettings
        v-if="page === '设置' && settingsTab === '常规'"
        v-model:settings="settings"
        v-model:autostart="autostart"
        v-model:theme="theme"
        :startup-saving="startupSaving"
        :channel-count="source?.channels || 0"
        :busy="busy"
        :saving="settingsLocked"
        @startup-change="startupChanged"
        @awake-change="awakeChanged"
        @persist="persist"
        @mapping-change="mapping"
      /><LogView
        v-if="page === '设置' && settingsTab === '日志'"
        v-model:detailed-logs="settings.detailedLogs"
        v-model:capture-diagnostics="settings.captureDiagnostics"
        :busy="busy || settingsLocked"
        :active-detailed-logs="activeDetailedLogs"
        :active-diagnostics="activeDiagnostics"
        :logs="logs"
        :diagnostic-logs="diagnosticLogs"
        :log-path="logPath"
        :pipeline-path="pipelinePath"
        :diagnostic-path="diagnosticPath"
        :report="report"
        @persist="persist"
        @open-logs="call('open_logs')"
        @clear-diagnostics="clearDiagnostics"
      />
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
        :disabled="busy || settingsLocked || !ready || !source"
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
            : captureChanging || sourceChanging
              ? '切换中…'
              : captureEnabled
                ? '采集中'
                : '采集已关闭'
        }}</span>
      </button>
      <div class="dock-source">
        <label id="source-label">音频来源</label>
        <SourcePicker
          v-model:open="sourceOpen"
          :inputs="inputs"
          :endpoint="settings.endpoint"
          :disabled="busy || settingsLocked"
          :meter-width="sourceLineWidth"
          @select="selectSource"
        />
        <div class="source-format">
          <small>{{ source ? formatAudioFormat(source.device_format) : '未选择音频来源' }}</small
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
          :disabled="!ready || !selection.length || !source || refreshing || settingsLocked"
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
