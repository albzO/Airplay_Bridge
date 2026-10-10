import { createRequire } from 'node:module';

const require = createRequire(new URL('../../../airplay-frontend/package.json', import.meta.url));
const { test: base, expect } = require('@playwright/test');
const test = base.extend({
  page: async ({ page }, use) => {
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await use(page);
    expect(errors, 'uncaught browser errors').toEqual([]);
  },
});
const inputs = createRequire(import.meta.url)('../fixtures/inputs.json');
const host = '192.0.2.10';

const start = (page) => page.getByRole('button', { name: '开始串流', exact: true });
const refresh = (page) => page.getByRole('button', { name: '刷新', exact: true });
const source = (page) => page.locator('.source-trigger');
const password = (page) =>
  page.getByRole('dialog', { name: 'AirPlay 密码验证' }).getByLabel(/^输入 .* 的 AirPlay 密码$/);
const calls = (page, name) => page.evaluate((name) => window.__airplayTest.calls(name), name);
const hold = (page, name) => page.evaluate((name) => window.__airplayTest.hold(name), name);
const resolve = (page, name, value) =>
  page.evaluate(([name, value]) => window.__airplayTest.resolve(name, value), [name, value]);
const reject = (page, name, message) =>
  page.evaluate(([name, message]) => window.__airplayTest.reject(name, message), [name, message]);
const send = (page, id, payload) =>
  page.evaluate(([id, payload]) => window.__airplayTest.send(id, payload), [id, payload]);
const telemetry = {
  kind: 'telemetry',
  elapsed_seconds: 1,
  water_ms: 100,
  lead_ms: 300,
  capture_frames: 48000,
  output_frames: 44100,
  peaks: [0.1, 0.2],
  controller: { correction_ppm: 0 },
};

// 点击/输入都经过真实模板；evaluate 只准备 IPC 夹具、注入桌面事件和读取命令记录。
// Click/type through the real template; evaluate only configures IPC fixtures, sends desktop events and reads commands.
async function open(page, fixture = {}) {
  await page.addInitScript((fixture) => {
    window.__airplayFixture = fixture;
  }, fixture);
  await page.goto('/dom.html');
  await expect(refresh(page)).toBeEnabled();
  await expect(page.locator('.alert')).toHaveCount(0);
}
async function playing(page, id) {
  await send(page, id, { kind: 'native', line: '[PROBE] PCM_READY' });
  await send(page, id, telemetry);
  await expect(page.getByRole('button', { name: '静音', exact: true })).toBeEnabled();
}

test('no source keeps start disabled after initialization', async ({ page }) => {
  await open(page, { inputs: [] });
  await expect(start(page)).toBeDisabled();
  await expect(page.getByRole('switch')).toBeDisabled();
  await expect(source(page)).toContainText('请选择音频流来源');
  expect(await calls(page, 'start_stream')).toHaveLength(0);
  expect(await calls(page, 'monitor_source')).toHaveLength(0);
});

test('no receiver keeps start disabled while refresh remains available', async ({ page }) => {
  await open(page, { devices: [] });
  await expect(start(page)).toBeDisabled();
  await expect(page.getByText('未发现设备。点击刷新', { exact: false })).toBeVisible();
  await refresh(page).click();
  await expect(refresh(page)).toBeEnabled();
  await expect(start(page)).toBeDisabled();
  expect(await calls(page, 'start_stream')).toHaveLength(0);
});

test('source picker closes with Escape and rejects unusable sources', async ({ page }) => {
  const playback = {
    ...inputs[0],
    id: 'fixture-playback',
    name: 'Fixture Playback',
    flow: 'playback',
  };
  const invalid = {
    ...playback,
    id: 'fixture-unavailable',
    name: 'Unavailable Fixture',
    channels: null,
  };
  await open(page, { inputs: [...inputs, playback, invalid] });
  await source(page).click();
  await expect(source(page)).toHaveAttribute('aria-expanded', 'true');
  await source(page).press('Escape');
  await expect(page.locator('.source-popup')).toHaveCount(0);
  await source(page).click();
  await expect(
    page.getByRole('button', { name: 'Unavailable Fixture', exact: true }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Fixture Playback', exact: true }).click();
  await expect(page.locator('#source-name')).toHaveText('Fixture Playback');
  await expect(page.locator('.source-popup')).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings.endpoint)
    .toBe(playback.id);
  await expect(start(page)).toBeEnabled();
});

test('connecting and stopping lock controls and suppress late password/ready events', async ({
  page,
}) => {
  await open(page);
  await hold(page, 'start_stream');
  await start(page).click();
  const stop = page.getByRole('button', { name: '停止串流', exact: true });
  await expect(stop).toBeEnabled();
  await expect(source(page)).toBeDisabled();
  await expect(refresh(page)).toBeDisabled();
  await expect(page.getByRole('switch')).toBeDisabled();
  const otherDevices = page.locator('.device:not(.selected) .device-row');
  await expect(otherDevices).toHaveCount(2);
  for (const button of await otherDevices.all()) await expect(button).toBeDisabled();
  await stop.click();
  await expect(page.getByRole('button', { name: '停止中…', exact: true })).toBeDisabled();
  await send(page, 101, { kind: 'password_required', host });
  await resolve(page, 'start_stream', 101);
  await send(page, 101, { kind: 'native', line: '[PROBE] PCM_READY' });
  await send(page, 101, telemetry);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.status')).toHaveText('正在停止');
  await expect(page.getByRole('button', { name: '静音', exact: true })).toBeDisabled();
  expect(await calls(page, 'stop_stream')).toHaveLength(1);
  expect(await calls(page, 'submit_password')).toHaveLength(0);
  await send(page, 101, { kind: 'finished', cancelled: true });
  await expect(start(page)).toBeEnabled();
  await expect(source(page)).toBeEnabled();
  await expect(refresh(page)).toBeEnabled();
});

test('password retry via Enter waits for the new command-confirmed session', async ({ page }) => {
  await open(page);
  await start(page).click();
  await send(page, 101, { kind: 'password_required', host });
  const confirm = page.getByRole('button', { name: '确认连接', exact: true });
  await expect(password(page)).toBeFocused();
  await expect(confirm).toBeDisabled();
  await password(page).fill('bad');
  await confirm.click();
  await expect.poll(async () => (await calls(page, 'submit_password')).length).toBe(1);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await send(page, 101, { kind: 'finished', error: 'PASSWORD_REJECTED fixture' });
  await expect(page.getByRole('alert')).toHaveText('密码不正确，请重新输入。');
  await expect(password(page)).toHaveValue('');
  await password(page).fill('fixture-secret');
  await hold(page, 'start_stream');
  await password(page).press('Enter');
  await send(page, 101, { kind: 'password_required', host: '192.0.2.99' });
  await send(page, 102, { kind: 'password_required', host });
  expect(await calls(page, 'submit_password')).toHaveLength(1);
  await resolve(page, 'start_stream', 102);
  await expect.poll(async () => (await calls(page, 'submit_password')).length).toBe(2);
  expect((await calls(page, 'submit_password')).map((call) => call.args)).toEqual([
    { sessionId: 101, host, password: 'bad' },
    { sessionId: 102, host, password: 'fixture-secret' },
  ]);
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await playing(page, 102);
  await expect(page.locator('.status')).toHaveText('串流中');
});

test('old session completion cannot unlock or overwrite a newer session', async ({ page }) => {
  await open(page);
  await start(page).click();
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await start(page).click();
  await send(page, 101, { kind: 'finished', error: 'old fixture failure' });
  await send(page, 101, { kind: 'password_required', host });
  await send(page, 101, { kind: 'native', line: '[PROBE] PCM_READY' });
  await send(page, 101, telemetry);
  await expect(page.getByRole('button', { name: '停止串流', exact: true })).toBeEnabled();
  await expect(source(page)).toBeDisabled();
  await expect(refresh(page)).toBeDisabled();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.alert')).toHaveCount(0);
  await expect(page.locator('.status')).toHaveText('正在连接');
  await playing(page, 102);
  await expect(page.locator('.status')).toHaveText('串流中');
});

test('startup failure drops queued password events and restores controls', async ({ page }) => {
  await open(page);
  await hold(page, 'start_stream');
  await start(page).click();
  await send(page, 101, { kind: 'password_required', host });
  await reject(page, 'start_stream', 'fixture startup failure');
  await expect(start(page)).toBeEnabled();
  await expect(source(page)).toBeEnabled();
  await expect(refresh(page)).toBeEnabled();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.locator('.alert')).toContainText('fixture startup failure');
});

test('pending password submission cannot submit twice or alter a new session after stop', async ({
  page,
}) => {
  await open(page);
  await start(page).click();
  await send(page, 101, { kind: 'password_required', host });
  await hold(page, 'submit_password');
  await password(page).fill('fixture-secret');
  await password(page).press('Enter');
  await expect(password(page)).toHaveValue('');
  await expect(page.getByRole('button', { name: '验证中…', exact: true })).toBeDisabled();
  await password(page).press('Enter');
  expect(await calls(page, 'submit_password')).toHaveLength(1);
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await send(page, 101, { kind: 'finished', cancelled: true });
  await start(page).click();
  await reject(page, 'submit_password', 'old fixture submission failure');
  await expect(page.locator('.alert')).toHaveCount(0);
  await expect(page.locator('.status')).toHaveText('正在连接');
  await send(page, 102, { kind: 'password_required', host });
  await expect(password(page)).toBeFocused();
  await password(page).fill('next-fixture-secret');
  await password(page).press('Enter');
  await expect.poll(async () => (await calls(page, 'submit_password')).length).toBe(2);
  expect((await calls(page, 'submit_password'))[1].args.sessionId).toBe(102);
});
