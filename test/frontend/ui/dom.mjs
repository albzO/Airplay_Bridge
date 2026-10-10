import { mockIPC } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import devices from '../fixtures/devices.json';
import inputs from '../fixtures/inputs.json';
import metadata from '../../../airplay-frontend/package.json';

// 仅控制桌面边界，不访问组件内部状态。每次导航重新创建夹具，无定时协议事件。
// Control desktop boundaries only, never component internals. Each navigation creates a fixture without timed events.
const copy = (value) => JSON.parse(JSON.stringify(value));
const data = {
  devices,
  inputs,
  settings: {
    endpoint: inputs[0].id,
    latency: 300,
    buffer: 128,
    mapping: [0, 1],
    keepAwake: true,
    closeAction: 'tray',
    detailedLogs: false,
    captureDiagnostics: false,
    speakersSwapped: false,
  },
  autostart: false,
  captureEnabled: true,
  dataPath: 'DOM fixture / no hardware connection',
  backendAvailable: true,
  ...window.__airplayFixture,
};
let sessionId = 100;
let settings = copy(data.settings);
const calls = [];
const holds = new Map();
const pending = new Map();

window.__airplayTest = {
  calls: (name) => copy(calls.filter((call) => call.name === name)),
  hold(name) {
    holds.set(name, (holds.get(name) || 0) + 1);
  },
  resolve(name, value) {
    take(name).resolve(value);
  },
  reject(name, message) {
    take(name).reject(message);
  },
  send: (id, payload) => emit('stream-event', { ...payload, session_id: id }),
};
function take(name) {
  const command = pending.get(name)?.shift();
  if (!command) throw new Error(`No held fixture command: ${name}`);
  return command;
}
function reply(name, args) {
  switch (name) {
    case 'plugin:app|version':
      return metadata.version;
    case 'initialize':
      return copy(data);
    case 'discover_devices':
      return copy(data.devices);
    case 'save_settings':
      settings = copy(args.settings);
      return;
    case 'set_capture_enabled':
      return emit('source-level', {
        endpoint: settings.endpoint,
        captureEnabled: args.enabled,
        peaks: [0, 0],
      });
    case 'start_stream':
      return ++sessionId;
    case 'ui_ready':
    case 'monitor_source':
    case 'stop_stream':
    case 'submit_password':
    case 'set_speaker_order':
      return;
    default:
      throw new Error(`Unexpected fixture command: ${name}`);
  }
}
mockIPC(
  (name, args) => {
    calls.push({ name, args: copy(args || {}) });
    // 先计算默认结果（包括会话编号），但由测试决定何时返回或失败。
    // Allocate default results, including ids, but let tests decide when held commands complete or fail.
    const result = reply(name, args);
    if (!holds.get(name)) return result;
    holds.set(name, holds.get(name) - 1);
    return new Promise((resolve, reject) => {
      const queue = pending.get(name) || [];
      queue.push({ resolve, reject });
      pending.set(name, queue);
    });
  },
  { shouldMockEvents: true },
);

await import('../../../airplay-frontend/src/main.ts');
