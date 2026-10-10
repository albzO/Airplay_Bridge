import assert from 'node:assert/strict';
import test from 'node:test';
import { diagnosticsModule } from './load-diagnostics.mjs';
const { useDiagnostics, DIAGNOSTIC_LIMITS: limits } = diagnosticsModule;
const event = (kind, fields = {}) => ({ session_id: 1, kind, ...fields });
const native = (d, line, fields = {}) => d.accept(event('native', { line, ...fields }));

test('all log producers enforce count and text limits including diagnostic end and faults', () => {
  const d = useDiagnostics(() => []);
  d.beginSession({ detailedLogs: true, captureDiagnostics: true });
  for (let i = 0; i < 310; i++) native(d, 'raw secret', { safe_line: `safe-${i}` });
  assert.equal(d.logs.value.length, limits.logs);
  assert.equal(d.logs.value[0], 'safe-10');
  d.recordSourceWarning('w'.repeat(8000));
  d.accept(event('finished', { error: 'raw secret', safe_error: 'safe error' }));
  assert.equal(d.logs.value.length, limits.logs);
  assert.equal(d.logs.value.at(-1), 'safe error');
  assert.ok(
    d.logs.value.every((text) => text.length <= limits.text && !text.includes('raw secret')),
  );
  for (let i = 0; i < 125; i++)
    d.accept(
      event('capture_diagnostic', {
        elapsed_seconds: 1.5,
        capture_frames: i,
        pending_pcm_ms: 0,
        water_ms: null,
        correction_ppm: 0,
        capture: {
          device_position: 10,
          packets: i,
          packet_qpc_100ns: 20,
          discontinuities: 0,
          timestamp_errors: 0,
        },
        error: 'e'.repeat(8000),
      }),
    );
  d.accept(event('diagnostic_end', { status: { error: 'e'.repeat(8000) } }));
  assert.equal(d.diagnosticLogs.value.length, limits.summaries);
  assert.ok(d.diagnosticLogs.value.every((text) => text.length <= limits.text));
  assert.match(d.diagnosticLogs.value[0], /1.5s.*采集 6 帧.*水位 — ms/);
  assert.match(d.diagnosticLogs.value.at(-1), /^诊断结束/);
});

test('snapshot reset and diagnostic clear preserve totals and log path; report display is bounded', () => {
  const d = useDiagnostics(() => []);
  d.setDataMode('portable');
  d.beginSession({ detailedLogs: true, captureDiagnostics: true });
  native(d, '[PACKET_STATS] host=fixture sent=3');
  d.accept(
    event('log_path', {
      path: 'private/session.log',
      detailed_logs: true,
      pipeline_path: 'private/pipeline.jsonl',
    }),
  );
  const report = { device: 'fixture', capture: null };
  d.accept(event('report', { report }));
  d.accept(event('diagnostic_end', { status: {} }));
  d.clearDiagnostics();
  assert.deepEqual(d.diagnosticLogs.value, []);
  assert.strictEqual(d.report.value, report);
  assert.equal(d.logs.value.length, 1);
  assert.equal(d.pipelinePath.value, '[程序目录]/data/logs/pipeline.jsonl');
  d.accept(event('report', { report: { device: 'fixture', extra: '中'.repeat(limits.report) } }));
  assert.match(d.report.value.display_error, /上限/);
  d.beginSession({ detailedLogs: false, captureDiagnostics: false });
  assert.equal(d.logPath.value, '[程序目录]/data/logs/session.log');
  assert.equal(d.pipelinePath.value, '');
  assert.deepEqual(d.report.value, {});
  native(d, '[PACKET_STATS] host=fixture sent=2');
  assert.equal(d.stats.value.fixture.sent, '5');
  assert.equal(d.logs.value.length, 0);
});

test('partial, repeated, invalid and decreasing counters remain isolated per device and session', () => {
  const d = useDiagnostics(() => [{ name: 'A', addresses: ['a'] }]);
  native(d, '[PACKET_STATS] host=a sent=10 rtx_resent=4', { safe_line: 'host=hidden sent=10' });
  native(d, '[PACKET_STATS] host=a sent=10');
  native(d, '[PACKET_STATS] host=a sent=NaN rtx_resent=-1');
  native(d, '[PACKET_STATS] host=a sent=12 rtx_resent=5');
  assert.deepEqual(d.stats.value.a, { sent: '12', rtx_resent: '5' });
  native(d, '[PACKET_STATS] host=a sent=1');
  native(d, '[PACKET_STATS] host=a sent=3');
  assert.equal(d.stats.value.a.sent, '14');
  native(d, '[PACKET_STATS] host=b sent=2');
  assert.equal(d.statsNames.value.a, 'A');
  assert.equal(d.statsNames.value.b, 'b');
  d.beginSession({ detailedLogs: false, captureDiagnostics: false });
  native(d, '[PACKET_STATS] host=a sent=1');
  assert.equal(d.stats.value.a.sent, '15');
  assert.equal(d.stats.value.b.sent, '2');
});

test('device history and technical values are bounded and prototype-like hosts are data', () => {
  const d = useDiagnostics(() => []);
  for (let i = 0; i < 140; i++) native(d, `[PACKET_STATS] host=device-${i} sent=1`);
  assert.equal(Object.keys(d.stats.value).length, limits.devices);
  assert.equal(Object.keys(d.statsNames.value).length, limits.devices);
  assert.equal(d.stats.value['device-0'], undefined);
  native(d, '[PACKET_STATS] host=__proto__ sent=2');
  assert.equal(d.stats.value.__proto__.sent, '2');
  native(d, '[PROBE] TIMING value=' + 't'.repeat(8000));
  assert.ok(d.technical.value['时钟协议'].length <= limits.text);
});
