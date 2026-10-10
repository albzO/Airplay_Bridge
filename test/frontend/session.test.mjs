import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';

const require = createRequire(new URL('../../airplay-frontend/package.json', import.meta.url));
const ts = require('typescript');
const vue = require('vue');
const source = await readFile(
  new URL('../../airplay-frontend/src/App.vue', import.meta.url),
  'utf8',
);
const script = source.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
const parsed = ts.createSourceFile('App.ts', script, ts.ScriptTarget.Latest, true);
const body = parsed.statements
  .filter((statement) => !ts.isImportDeclaration(statement))
  .map((statement) => statement.getText(parsed))
  .join('\n');
const { outputText } = ts.transpileModule(body, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
});
const sessionSource = await readFile(
  new URL('../../airplay-frontend/src/useStreamSession.ts', import.meta.url),
  'utf8',
);
const sessionParsed = ts.createSourceFile(
  'session.ts',
  sessionSource,
  ts.ScriptTarget.Latest,
  true,
);
const sessionBody = sessionParsed.statements
  .filter((s) => !ts.isImportDeclaration(s))
  .map((s) => s.getText(sessionParsed))
  .join('\n');
const sessionJs = ts.transpileModule(sessionBody, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;
const displayJs = ts.transpileModule(
  await readFile(new URL('../../airplay-frontend/src/display.ts', import.meta.url), 'utf8'),
  { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } },
).outputText;
const display = {};
new Function('exports', displayJs)(display);

// 执行真实页面脚本和 Vue ref；只替换桌面 IPC 与浏览器接口，不复制事件处理逻辑。
// Execute the actual page script and Vue refs; replace desktop IPC/browser APIs, not event logic.
function page(t, invokeCommand) {
  const scope = vue.effectScope();
  const unmounts = [];
  const deps = {
    ref: vue.ref,
    shallowRef: vue.shallowRef,
    computed: vue.computed,
    num: display.num,
    level: display.level,
    formatAudioFormat: display.formatAudioFormat,
    nextTick: vue.nextTick,
    watch: vue.watch,
    onMounted: () => {},
    onUnmounted: (callback) => unmounts.push(callback),
    invokeCommand,
    describeError: String,
    matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
    localStorage: { getItem: () => null, setItem() {} },
    document: { documentElement: { dataset: {} }, querySelector: () => null },
  };
  const sessionExports = {};
  new Function('exports', ...Object.keys(deps), sessionJs)(sessionExports, ...Object.values(deps));
  deps.useStreamSession = sessionExports.useStreamSession;
  const names = Object.keys(deps);
  const create = new Function(
    ...names,
    outputText +
      '\nreturn { start, stop, submit, event, session, busy, playing, stopping, connected, pending, password, sending, retry, retrySending, phase, error };',
  );
  const state = scope.run(() => create(...names.map((name) => deps[name])));
  state.unmount = () => {
    unmounts.forEach((callback) => callback());
    scope.stop();
  };
  t.after(state.unmount);
  return state;
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const event = (id, kind, fields = {}) => ({ session_id: id, kind, ...fields });
const telemetry = (id) => event(id, 'telemetry', { peaks: [0.1, 0.2], water_ms: 100 });

test('old events before command completion cannot claim or finish the new session', async (t) => {
  const command = deferred();
  const p = page(t, () => command.promise);
  const started = p.start();
  p.event(event(100, 'finished'));
  p.event(event(101, 'preparing_source'));
  p.event(telemetry(101));
  assert.equal(p.session.value, null);
  assert.equal(p.playing.value, false);
  command.resolve(101);
  await started;
  assert.equal(p.session.value, 101);
  assert.equal(p.busy.value, true);
  assert.equal(p.playing.value, true);
  p.event(event(100, 'finished'));
  assert.equal(p.busy.value, true);
});

test('an early finished event remains terminal after the command returns', async (t) => {
  const command = deferred();
  const p = page(t, () => command.promise);
  const started = p.start();
  p.event(event(101, 'finished'));
  p.event(event(101, 'password_required', { host: 'fixture' }));
  p.event(telemetry(101));
  command.resolve(101);
  await started;
  assert.equal(p.busy.value, false);
  assert.equal(p.playing.value, false);
  assert.equal(p.pending.value, '');
});

test('failed startup discards buffered events and releases stopping state', async (t) => {
  const command = deferred();
  const p = page(t, (name) => (name === 'start_stream' ? command.promise : Promise.resolve()));
  const started = p.start();
  await p.stop();
  p.event(event(101, 'password_required', { host: 'fixture' }));
  command.reject(new Error('fixture startup failure'));
  await started;
  assert.equal(p.busy.value, false);
  assert.equal(p.stopping.value, false);
  assert.equal(p.pending.value, '');
  assert.match(p.error.value, /fixture startup failure/);
});

test('stop suppresses late password, ready and telemetry events until finished', async (t) => {
  const p = page(t, async (name) => (name === 'start_stream' ? 101 : undefined));
  await p.start();
  await p.stop();
  p.event(event(101, 'password_required', { host: 'fixture' }));
  p.event(event(101, 'native', { line: '[PROBE] PCM_READY' }));
  p.event(telemetry(101));
  assert.equal(p.phase.value, '正在停止');
  assert.equal(p.pending.value, '');
  assert.equal(p.connected.value, false);
  assert.equal(p.playing.value, false);
  assert.equal(p.busy.value, true);
  p.event(event(101, 'finished', { cancelled: true }));
  assert.equal(p.busy.value, false);
  assert.equal(p.stopping.value, false);
});

test('password retry submits only after the new session identity is confirmed', async (t) => {
  const retryCommand = deferred();
  const submissions = [];
  let starts = 0;
  const p = page(t, async (name, args) => {
    if (name === 'start_stream') return ++starts === 1 ? 101 : retryCommand.promise;
    if (name === 'submit_password') submissions.push({ ...args });
  });
  await p.start();
  p.event(event(101, 'password_required', { host: 'fixture' }));
  p.password.value = 'bad';
  await p.submit();
  p.event(event(101, 'finished', { error: 'PASSWORD_REJECTED' }));
  assert.equal(p.retry.value, true);
  p.password.value = 'fixture-secret';
  const retried = p.submit();
  p.event(event(101, 'password_required', { host: 'old' }));
  p.event(event(102, 'password_required', { host: 'fixture' }));
  assert.equal(submissions.length, 1);
  retryCommand.resolve(102);
  await retried;
  await Promise.resolve();
  assert.deepEqual(submissions[1], { sessionId: 102, host: 'fixture', password: 'fixture-secret' });
  assert.equal(p.password.value, '');
  assert.equal(p.retrySending.value, false);
});

test('late password command completion cannot overwrite a stopped or newer session', async (t) => {
  const submitted = deferred();
  let id = 100;
  const p = page(t, (name) =>
    name === 'start_stream' ? Promise.resolve(++id) : submitted.promise,
  );
  await p.start();
  p.event(event(101, 'password_required', { host: 'fixture' }));
  p.password.value = 'fixture-secret';
  const sending = p.submit();
  p.event(event(101, 'finished'));
  await p.start();
  submitted.resolve();
  await sending;
  assert.equal(p.session.value, 102);
  assert.equal(p.phase.value, '正在连接');
});

test('startup buffer overflow requests stop and retains an early terminal event', async (t) => {
  const command = deferred();
  let stops = 0;
  const p = page(t, (name) => {
    if (name === 'start_stream') return command.promise;
    if (name === 'stop_stream') stops++;
    return Promise.resolve();
  });
  const started = p.start();
  for (let i = 0; i < 600; i++) p.event(event(101, 'native', { line: 'fixture' }));
  p.event(event(101, 'finished'));
  command.resolve(101);
  await started;
  assert.equal(stops, 1);
  assert.equal(p.busy.value, false);
  assert.match(p.error.value, /事件积压/);
});

test('unmount invalidates pending startup and ignores later events', async (t) => {
  const command = deferred();
  const p = page(t, () => command.promise);
  const started = p.start();
  p.unmount();
  command.resolve(101);
  await started;
  p.event(event(101, 'password_required', { host: 'fixture' }));
  assert.equal(p.session.value, null);
  assert.equal(p.pending.value, '');
});

test('late password completion during stop cannot restore the authentication phase', async (t) => {
  const submitted = deferred();
  const p = page(t, (name) => {
    if (name === 'start_stream') return Promise.resolve(101);
    if (name === 'submit_password') return submitted.promise;
    return Promise.resolve();
  });
  await p.start();
  p.event(event(101, 'password_required', { host: 'fixture' }));
  p.password.value = 'fixture-secret';
  const sending = p.submit();
  await p.stop();
  submitted.resolve();
  await sending;
  assert.equal(p.phase.value, '正在停止');
  assert.equal(p.pending.value, '');
  assert.equal(p.sending.value, false);
});

test('late stop failure cannot overwrite the next session and duplicate starts are ignored', async (t) => {
  const stopped = deferred();
  let starts = 0;
  const p = page(t, (name) => {
    if (name === 'start_stream') return Promise.resolve(100 + ++starts);
    return stopped.promise;
  });
  await p.start();
  await p.start();
  assert.equal(starts, 1);
  const stopping = p.stop();
  p.event(event(101, 'finished'));
  await p.start();
  stopped.reject(new Error('old stop failure'));
  await stopping;
  assert.equal(p.session.value, 102);
  assert.equal(p.error.value, '');
  assert.equal(p.stopping.value, false);
});
