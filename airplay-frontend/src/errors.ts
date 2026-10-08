import catalog from '../../docs/error-codes.json';

const commandCodes: Record<string, string> = {
  initialize: 'APP_INITIALIZE_FAILED',
  monitor_source: 'CAPTURE_INIT_FAILED',
  set_capture_enabled: 'CAPTURE_INIT_FAILED',
  discover_devices: 'DISCOVERY_FAILED',
  save_settings: 'SETTINGS_SAVE_FAILED',
  start_stream: 'STREAM_FAILED',
  submit_password: 'PASSWORD_SUBMIT_FAILED',
  set_volume: 'VOLUME_UNAVAILABLE',
  set_mapping: 'INVALID_ARGUMENT',
  set_speaker_order: 'STEREO_PAIR_INVALID',
  set_autostart: 'AUTOSTART_FAILED',
  window_action: 'WINDOW_ACTION_FAILED',
  open_logs: 'LOG_DIRECTORY_FAILED',
  forget_auth_policy: 'AUTH_CACHE_FAILED',
  auth_pipe_error: 'AUTH_PIPE_FAILED',
  auth_memory_error: 'AUTH_CACHE_FAILED',
};
function errorOrigin(detail: string): string {
  const os = detail.match(/os error (\d+)/);
  if (os) {
    const code = Number(os[1]);
    return `Windows / ${code >= 10000 && code < 12000 ? 'Winsock' : 'Win32'} ${code}`;
  }
  const hr = detail.match(/0x([0-9a-f]{8})(?![0-9a-f])/i);
  if (hr) {
    const code = parseInt(hr[1], 16),
      hex = hr[1].toUpperCase();
    return ((code >>> 16) & 0x1fff) === 7
      ? `Windows / Win32 ${code & 0xffff}（HRESULT 0x${hex}）`
      : `Windows / HRESULT 0x${hex}`;
  }
  return 'AirPlay Hub / 应用或协议处理';
}
export function describeError(error: unknown, command = ''): string {
  const detail = error instanceof Error ? error.message : String(error);
  const existing = catalog.find(
    (row) => detail.includes(`[${row.code}]`) || detail.includes(`code=${row.code}`),
  );
  if (existing)
    return detail.includes('来源：') ? detail : `${detail} 来源：${errorOrigin(detail)}`;
  const row =
    catalog.find((row) => row.patterns.some((pattern) => detail.includes(pattern))) ||
    catalog.find((row) => row.code === (commandCodes[command] || 'INTERNAL_ERROR'))!;
  return `[${row.code}] ${row.message} 来源：${errorOrigin(detail)}；详情：${detail}`;
}
