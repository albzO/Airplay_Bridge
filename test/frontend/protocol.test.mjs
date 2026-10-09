import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createRequire } from 'node:module';

// 复用前端锁定的 TypeScript，无需在测试目录重复安装依赖。
// Reuse the frontend's pinned TypeScript without a second dependency installation.
const require = createRequire(new URL('../../airplay-frontend/package.json', import.meta.url));
const ts = require('typescript');

// 仅将不依赖运行时模块的校验器转为 JS，使用 Node 内置测试，不额外引入测试框架。
// 编译错误由 build 中的 vue-tsc 检查；此处验证实际 JSON 边界和失败行为。
// Transpile the validator, which has no runtime imports, and use Node's built-in test runner.
// vue-tsc checks types during build; these tests exercise JSON boundaries and failure behavior.
const source = await readFile(
  new URL('../../airplay-frontend/src/protocol.ts', import.meta.url),
  'utf8',
);
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
});
const { decodeDevices, decodeInitialization, decodeSourceLevel, decodeStreamEvent } = await import(
  'data:text/javascript;base64,' + Buffer.from(outputText).toString('base64')
);
const devices = JSON.parse(
  await readFile(new URL('./fixtures/devices.json', import.meta.url), 'utf8'),
);
const inputs = JSON.parse(
  await readFile(new URL('./fixtures/inputs.json', import.meta.url), 'utf8'),
);
const initialization = {
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
  dataPath: 'fixture',
  backendAvailable: true,
};
const emit = (payload) => decodeStreamEvent({ session_id: 100, ...payload });
const telemetry = {
  kind: 'telemetry',
  elapsed_seconds: 20,
  capture_frames: 960000,
  output_frames: 882000,
  water_ms: 130,
  lead_ms: 300,
  peaks: [0.12, 0.08],
  controller: { correction_ppm: 25 },
};

test('existing UI fixtures initialize; omitted optional fields use existing defaults', () => {
  const result = decodeInitialization(initialization);
  assert.deepEqual(result.devices, devices);
  assert.deepEqual(result.inputs, inputs);
  assert.equal(result.captureEnabled, true);
  assert.equal(result.awakeError, undefined);
  assert.deepEqual(decodeDevices(devices), devices);
});

test('malformed initialization cannot partially populate page state', () => {
  for (const patch of [
    { devices: {} },
    { backendAvailable: 'true' },
    { inputs: [{ ...inputs[0], channels: '2' }] },
    { settings: { ...initialization.settings, mapping: [0] } },
    { settings: { ...initialization.settings, buffer: 0 } },
    { settings: { ...initialization.settings, closeAction: 'unknown' } },
  ])
    assert.throws(() => decodeInitialization({ ...initialization, ...patch }), /格式不正确/);
});

test('all known stream variants preserve fields used by the page', () => {
  const capture = {
    device_position: 120,
    packets: 3,
    packet_qpc_100ns: 999,
    discontinuities: 0,
    timestamp_errors: 0,
  };
  const events = [
    { kind: 'preparing_source' },
    { kind: 'password_required', host: 'fixture-host' },
    { kind: 'auth_memory_error', error: 'fixture' },
    { kind: 'auth_pipe_error', error: 'fixture' },
    { kind: 'log_path', path: 'fixture.log', detailed_logs: true, pipeline_path: null },
    telemetry,
    {
      kind: 'report',
      report: { device: 'device-1', capture: null, conversion: null, extra: [1, 'x'] },
    },
    { kind: 'diagnostic_path', path: 'capture.log', pipeline_path: 'pipeline.log' },
    {
      kind: 'capture_diagnostic',
      elapsed_seconds: null,
      capture_frames: 120,
      pending_pcm_ms: 10,
      water_ms: null,
      correction_ppm: 0,
      capture,
      error: null,
    },
    { kind: 'diagnostic_end', status: { dropped_records: 2, error: null } },
    { kind: 'native', line: '[PROBE] PCM_READY', safe_line: '[PROBE] PCM_READY', is_fault: false },
    { kind: 'finished', error: null, safe_error: null, error_code: null, cancelled: true },
  ];
  for (const payload of events) {
    const decoded = emit(payload);
    for (const [key, value] of Object.entries(payload)) assert.deepEqual(decoded[key], value);
    assert.equal(decoded.session_id, 100);
  }
  assert.equal(decodeStreamEvent({ kind: 'future_event' }), null);
});

test('telemetry rejects nonfinite numbers, string counts, invalid pairs and wrong session ids', () => {
  for (const patch of [
    { water_ms: Infinity },
    { elapsed_seconds: NaN },
    { capture_frames: '960000' },
    { output_frames: -1 },
    { peaks: [0] },
    { peaks: [0, '0'] },
    { controller: null },
    { session_id: 1.5 },
    { session_id: Number.MAX_SAFE_INTEGER + 1 },
  ])
    assert.throws(() => emit({ ...telemetry, ...patch }), /格式不正确/);
});

test('source preview accepts partial updates but validates optional fields', () => {
  assert.deepEqual(
    decodeSourceLevel({ endpoint: 'fixture', captureEnabled: false }).captureEnabled,
    false,
  );
  assert.equal(decodeSourceLevel({ endpoint: 'fixture', error: null }).error, null);
  for (const patch of [{ peaks: null }, { captureEnabled: 'false' }, { error: {} }, { warning: 1 }])
    assert.throws(() => decodeSourceLevel({ endpoint: 'fixture', ...patch }), /格式不正确/);
});

test('report validates displayed fields, preserves extension JSON, limits nesting and hides values in errors', () => {
  const report = {
    device: 'device-1',
    capture: { discontinuities: 2, timestamp_errors: 0 },
    extra: { values: [null, true, 3] },
  };
  assert.deepEqual(emit({ kind: 'report', report }).report, report);
  for (const value of [
    [],
    { capture: [] },
    { capture: { discontinuities: '2' } },
    { device: 3 },
    { extra: undefined },
  ])
    assert.throws(() => emit({ kind: 'report', report: value }), /格式不正确/);
  let deep = {};
  for (let i = 0; i < 40; i++) deep = { child: deep };
  assert.throws(() => emit({ kind: 'report', report: deep }), /格式不正确/);
  assert.throws(
    () => emit({ kind: 'password_required', host: { secret: 'do-not-print-me' } }),
    (e) => {
      assert.equal(e.message.includes('do-not-print-me'), false);
      return e.message.endsWith('host');
    },
  );
});
