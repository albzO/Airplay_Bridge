import { onUnmounted, ref } from 'vue';
import { invokeCommand } from './commands';
import { describeError } from './errors';
import type { Settings, StreamEvent } from './types';

interface SessionOptions {
  startArgs: () => { names: string[]; settings: Settings };
  reset: () => void;
  showPassword: (retry: boolean) => void;
  ready: () => void;
  startFailed: () => void;
  render: (event: StreamEvent) => void;
}

/**
 * 管理一次串流的命令、事件归属与密码重试；页面负责设备视图和诊断快照。
 * busy 覆盖准备/认证/播放，connected 表示 PCM_READY，playing 表示收到遥测。
 * 命令返回前最多暂存 512 条事件；只信任命令返回的 session_id。
 * Own session commands, event attribution and password retries; the page owns views/snapshots.
 * busy covers preparation/auth/playback; connected means PCM_READY; playing means telemetry.
 * Buffer at most 512 early events and trust only the command-returned session_id.
 */
export function useStreamSession(options: SessionOptions) {
  const busy = ref(false),
    playing = ref(false),
    stopping = ref(false),
    connected = ref(false),
    phase = ref('尚未连接'),
    error = ref(''),
    session = ref<number | null>(null),
    pending = ref(''),
    password = ref(''),
    sending = ref(false),
    retry = ref(false),
    retrySending = ref(false);
  let lastPasswordHost = '';
  let request = 0;
  let starting = false;
  let early: StreamEvent[] = [];
  let overflow = false;
  let disposed = false;

  onUnmounted(() => {
    disposed = true;
    request++;
    starting = false;
    early = [];
    password.value = '';
  });

  function clearRetry() {
    pending.value = '';
    password.value = '';
    retry.value = false;
    retrySending.value = false;
    lastPasswordHost = '';
  }

  async function start(passwordFirst = false) {
    if (busy.value || disposed) return;
    const ticket = ++request;
    starting = true;
    early = [];
    overflow = false;
    options.reset();
    error.value = '';
    busy.value = true;
    playing.value = false;
    stopping.value = false;
    connected.value = false;
    phase.value = '正在连接';
    session.value = null;
    pending.value = '';
    if (!passwordFirst) {
      password.value = '';
      retry.value = false;
    }
    const id = await invokeCommand<number>('start_stream', {
      ...options.startArgs(),
      passwordFirst,
    }).catch((e: unknown) => {
      if (!disposed && ticket === request) error.value = String(e);
      return undefined;
    });
    if (disposed || ticket !== request) return;
    starting = false;
    const queued = early;
    early = [];
    if (id === undefined) {
      busy.value = false;
      stopping.value = false;
      phase.value = '连接失败';
      retrySending.value = false;
      options.startFailed();
      return;
    }
    session.value = id;
    if (overflow) {
      error.value = '连接期间事件积压过多，已请求停止；请重新连接';
      await stop();
      // 溢出后仍处理收尾：已结束的后端不会再次发送 finished。
      // Finalize on overflow too; an exited backend will not emit finished again.
      for (const entry of queued) if (entry.kind === 'finished') event(entry);
    } else {
      for (const entry of queued) event(entry);
    }
  }

  async function stop() {
    if (!busy.value || disposed) return;
    const ticket = request;
    stopping.value = true;
    phase.value = '正在停止';
    password.value = '';
    pending.value = '';
    try {
      await invokeCommand('stop_stream');
    } catch (e) {
      if (!disposed && ticket === request && busy.value) error.value = String(e);
    }
  }

  async function submit() {
    if (!pending.value || !password.value || sending.value) return;
    if (retry.value && !busy.value) {
      retrySending.value = true;
      await start(true);
      return;
    }
    sending.value = true;
    const ticket = request;
    const submittedSession = session.value;
    const secret = password.value;
    password.value = '';
    lastPasswordHost = pending.value;
    try {
      await invokeCommand('submit_password', {
        sessionId: session.value,
        host: pending.value,
        password: secret,
      });
      if (disposed || ticket !== request || !busy.value || stopping.value) return;
      pending.value = '';
      phase.value = '正在验证密码';
    } catch (e) {
      if (!disposed && ticket === request && busy.value && !stopping.value) error.value = String(e);
    } finally {
      if (ticket === request && submittedSession === session.value) sending.value = false;
    }
  }

  // 仅接收 protocol.ts 已验证的事件；finished 后迟到事件不能重新打开会话。
  // Accept protocol.ts-validated events only; late events cannot reopen a finished session.
  function event(e: StreamEvent) {
    if (disposed) return;
    if (starting) {
      if (early.length < 512) early.push(e);
      else {
        overflow = true;
        if (e.kind === 'finished') {
          const replace = early.findIndex((entry) => entry.kind !== 'finished');
          early[replace < 0 ? early.length - 1 : replace] = e;
        }
      }
      return;
    }
    if (!busy.value || session.value === null || e.session_id !== session.value) return;
    if (e.kind === 'preparing_source' && !stopping.value) phase.value = '等待采集稳定';
    if (e.kind === 'password_required') {
      if (stopping.value) return;
      pending.value = e.host;
      phase.value = '需要 AirPlay 密码';
      options.showPassword(false);
      if (retrySending.value) {
        retrySending.value = false;
        void submit();
      }
    }
    if (e.kind === 'auth_memory_error' || e.kind === 'auth_pipe_error')
      error.value = describeError(e.error, e.kind);
    if (e.kind === 'telemetry') {
      if (stopping.value) return;
      playing.value = true;
      phase.value = '串流中';
    }
    if (e.kind === 'native' && e.line.includes('PCM_READY') && !stopping.value) {
      connected.value = true;
      phase.value = '启动音频采集';
      password.value = '';
      retry.value = false;
      options.ready();
    }
    if (e.kind === 'finished') {
      password.value = '';
      busy.value = false;
      playing.value = false;
      connected.value = false;
      stopping.value = false;
      sending.value = false;
      retrySending.value = false;
      pending.value = '';
      phase.value = e.error ? '串流失败' : '已停止';
      if (e.error) error.value = e.error;
      options.render(e);
      retry.value = !!e.error?.includes('PASSWORD_REJECTED') && !!lastPasswordHost;
      if (retry.value) {
        pending.value = lastPasswordHost;
        options.showPassword(true);
      }
      return;
    }
    options.render(e);
  }

  return {
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
  };
}
