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
const captureDiagnostic = {
  kind: 'capture_diagnostic',
  elapsed_seconds: 1.5,
  capture_frames: 48000,
  pending_pcm_ms: 10,
  water_ms: 100,
  correction_ppm: -2,
  capture: {
    device_position: 48000,
    packets: 100,
    packet_qpc_100ns: 15000000,
    discontinuities: 0,
    timestamp_errors: 0,
  },
};
const logPanel = (page, name) =>
  page.locator('section.panel').filter({ has: page.getByRole('heading', { name, exact: true }) });

async function openLogView(page) {
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '日志', exact: true }).click();
}

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
async function readyEvents(page, id) {
  await send(page, id, { kind: 'native', line: '[PROBE] PCM_READY' });
  await send(page, id, telemetry);
}
async function playing(page, id) {
  await readyEvents(page, id);
  await expect(page.getByRole('button', { name: '静音', exact: true })).toBeEnabled();
}

test('failed source save restores endpoint and mapping without previewing; retry stays locked through monitoring', async ({
  page,
}) => {
  const mono = {
    ...inputs[0],
    id: 'fixture-retry-mono',
    name: 'Retry Mono Fixture',
    channels: 1,
  };
  await open(page, { inputs: [...inputs, mono] });
  const initial = (await calls(page, 'save_settings')).at(-1).args.settings;
  const monitors = (await calls(page, 'monitor_source')).length;
  await hold(page, 'save_settings');
  await source(page).click();
  await page.getByRole('button', { name: mono.name, exact: true }).click();
  await expect(source(page)).toBeDisabled();
  await expect(start(page)).toBeDisabled();
  await expect(page.getByRole('switch')).toBeDisabled();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: '避免系统自动睡眠' })).toBeDisabled();
  await expect(page.getByLabel('左输出取样', { exact: true })).toBeDisabled();
  expect(await calls(page, 'monitor_source')).toHaveLength(monitors);
  await reject(page, 'save_settings', 'fixture source save failure');
  await expect(page.locator('.alert')).toContainText('fixture source save failure');
  await expect(page.locator('#source-name')).toHaveText(inputs[0].name);
  await expect(page.getByLabel('左输出取样', { exact: true })).toHaveValue('0');
  await expect(page.getByLabel('右输出取样', { exact: true })).toHaveValue('1');
  await expect(source(page)).toBeEnabled();
  expect(await calls(page, 'monitor_source')).toHaveLength(monitors);
  await hold(page, 'monitor_source');
  await source(page).click();
  await page.getByRole('button', { name: mono.name, exact: true }).click();
  await expect.poll(async () => (await calls(page, 'monitor_source')).length).toBe(monitors + 1);
  await expect(source(page)).toBeDisabled();
  await expect(start(page)).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: '避免系统自动睡眠' })).toBeDisabled();
  await page.evaluate(
    (endpoint) => window.__airplayTest.sourceLevel({ endpoint, error: 'stale source error' }),
    initial.endpoint,
  );
  await expect(page.locator('.source-notice')).toHaveCount(0);
  await resolve(page, 'monitor_source');
  await expect(source(page)).toBeEnabled();
  await expect(page.locator('#source-name')).toHaveText(mono.name);
  await expect(page.getByLabel('右输出取样', { exact: true })).toHaveValue('0');
  await start(page).click();
  expect((await calls(page, 'start_stream')).at(-1).args.settings).toEqual({
    ...initial,
    endpoint: mono.id,
    mapping: [0, 0],
  });
});

test('mapping failure rolls back both selectors and retries without restarting the session', async ({
  page,
}) => {
  await open(page);
  await start(page).click();
  await playing(page, 101);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const left = page.getByLabel('左输出取样', { exact: true });
  const right = page.getByLabel('右输出取样', { exact: true });
  const saves = (await calls(page, 'save_settings')).length;
  await hold(page, 'set_mapping');
  await left.selectOption('1');
  await expect(left).toBeDisabled();
  await expect(right).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: '避免系统自动睡眠' })).toBeDisabled();
  await reject(page, 'set_mapping', 'fixture mapping save failure');
  await expect(page.locator('.alert')).toContainText('fixture mapping save failure');
  await expect(left).toHaveValue('0');
  await expect(right).toHaveValue('1');
  await expect(left).toBeEnabled();
  await left.selectOption('1');
  await expect.poll(async () => (await calls(page, 'set_mapping')).length).toBe(2);
  await expect(left).toBeEnabled();
  await expect(left).toHaveValue('1');
  expect((await calls(page, 'set_mapping')).map((call) => call.args.mapping)).toEqual([
    [1, 1],
    [1, 1],
  ]);
  expect(await calls(page, 'save_settings')).toHaveLength(saves);
  expect(await calls(page, 'start_stream')).toHaveLength(1);
  expect(await calls(page, 'stop_stream')).toHaveLength(0);
});

for (const [dataMode, root] of [
  ['installed', '%APPDATA%/AirPlay Hub/logs'],
  ['portable', '[程序目录]/data/logs'],
]) {
  test(`${dataMode} log locations preserve filenames, hide private roots and accept empty paths`, async ({
    page,
  }) => {
    await open(page, { dataMode });
    await openLogView(page);
    const sessionLog = logPanel(page, '会话日志');
    const diagnostics = logPanel(page, '采集逐包诊断');
    await expect(sessionLog.locator('.log-location')).toHaveText(root);
    await start(page).click();
    await readyEvents(page, 101);
    await send(page, 101, {
      kind: 'log_path',
      path: 'C:\\private-fixture\\data\\logs\\session.log',
      detailed_logs: false,
      pipeline_path: 'C:/private-fixture/data/logs/pipeline.jsonl',
    });
    await send(page, 101, {
      kind: 'diagnostic_path',
      path: 'C:/private-fixture/data/logs/capture.jsonl',
      pipeline_path: 'C:/private-fixture/data/logs/pipeline.jsonl',
    });
    await expect(sessionLog.locator('.log-location')).toHaveText([
      root + '/session.log',
      '流水线：' + root + '/pipeline.jsonl',
    ]);
    await expect(diagnostics.locator('.log-location')).toHaveText(
      '逐包：' + root + '/capture.jsonl',
    );
    await expect(page.locator('main')).not.toContainText('private-fixture');
    const report = {
      device: 'Receiver A',
      report_write_error: '[REPORT_WRITE_FAILED] fixture write failure',
    };
    await send(page, 101, { kind: 'report', report });
    await sessionLog.getByText('完整会话报告', { exact: true }).click();
    await expect(sessionLog.locator('details pre')).toHaveText(JSON.stringify(report, null, 2));
    await send(page, 101, {
      kind: 'log_path',
      path: '',
      detailed_logs: false,
      pipeline_path: null,
    });
    await send(page, 101, { kind: 'diagnostic_path', path: '', pipeline_path: '' });
    await expect(sessionLog.locator('.log-location')).toHaveText('');
    await expect(diagnostics.locator('.log-location')).toHaveCount(0);
    await expect(sessionLog.locator('details pre')).toHaveText(JSON.stringify(report, null, 2));
  });
}

test('no source keeps start disabled after initialization', async ({ page }) => {
  await open(page, { inputs: [] });
  await expect(start(page)).toBeDisabled();
  await expect(page.getByRole('switch')).toBeDisabled();
  await expect(source(page)).toContainText('请选择音频流来源');
  expect(await calls(page, 'start_stream')).toHaveLength(0);
  expect(await calls(page, 'monitor_source')).toHaveLength(0);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '技术详情', exact: true }).click();
  await expect(page.locator('.capture-panel dd').nth(0)).toHaveText('设备未公布格式');
  await expect(page.locator('.capture-panel dd').nth(1)).toHaveText('设备未公布格式');
});

test('failed initial settings save skips capture until a successful source retry', async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__airplayFixture = { captureEnabled: false, holdCommands: ['save_settings'] };
  });
  await page.goto('/dom.html');
  await expect.poll(async () => (await calls(page, 'save_settings')).length).toBe(1);
  await expect(start(page)).toBeDisabled();
  await reject(page, 'save_settings', 'fixture initial save failure');
  await expect(refresh(page)).toBeEnabled();
  await expect(page.locator('.alert')).toContainText('fixture initial save failure');
  expect(await calls(page, 'set_capture_enabled')).toHaveLength(0);
  expect(await calls(page, 'monitor_source')).toHaveLength(0);
  expect(await calls(page, 'ui_ready')).toHaveLength(1);
  await source(page).click();
  await page.getByRole('button', { name: inputs[1].name, exact: true }).click();
  await expect.poll(async () => (await calls(page, 'set_capture_enabled')).length).toBe(1);
  expect((await calls(page, 'set_capture_enabled'))[0].args).toEqual({ enabled: true });
  await expect(start(page)).toBeEnabled();
  await expect(page.getByRole('switch')).toHaveAttribute('aria-checked', 'true');
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
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '技术详情', exact: true }).click();
  await expect(page.getByRole('button', { name: '重新检测所选设备', exact: true })).toBeDisabled();
  expect(await calls(page, 'forget_auth_policy')).toHaveLength(0);
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

test('source groups keep natural ordering and outside clicks dismiss the picker', async ({
  page,
}) => {
  const playback = ['Output 10', 'output 2', 'Alpha'].map((name, index) => ({
    ...inputs[0],
    id: `fixture-output-${index}`,
    name,
    flow: 'playback',
  }));
  await open(page, { inputs: [...inputs, ...playback] });
  const saves = (await calls(page, 'save_settings')).length;
  await source(page).click();
  await expect(
    page.getByRole('group', { name: '播放设备', exact: true }).locator('button > span:first-child'),
  ).toHaveText(['Alpha', 'output 2', 'Output 10', 'Playback Device']);
  await expect(page.getByRole('group', { name: '录音设备', exact: true })).toBeVisible();
  await page.locator('.source-dismiss').click();
  await expect(source(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.source-popup')).toHaveCount(0);
  await source(page).click();
  await expect(page.locator('.source-popup')).toBeVisible();
  await source(page).press('Escape');
  expect(await calls(page, 'save_settings')).toHaveLength(saves);
});

test('choosing mono resets channel mapping and saves before monitoring the new source', async ({
  page,
}) => {
  const mono = {
    ...inputs[0],
    id: 'fixture-mono',
    name: 'Mono Fixture',
    channels: 1,
    device_format: { ...inputs[0].device_format, channels: 1, block_align: 4, channel_mask: 4 },
    mix_format: { ...inputs[0].mix_format, channels: 1, block_align: 4, channel_mask: 4 },
  };
  await open(page, { inputs: [...inputs, mono] });
  const monitors = (await calls(page, 'monitor_source')).length;
  await hold(page, 'save_settings');
  await source(page).click();
  await page.getByRole('button', { name: mono.name, exact: true }).click();
  await expect(page.locator('#source-name')).toHaveText(mono.name);
  await expect(page.locator('.source-popup')).toHaveCount(0);
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings)
    .toMatchObject({ endpoint: mono.id, mapping: [0, 0] });
  expect(await calls(page, 'monitor_source')).toHaveLength(monitors);
  await resolve(page, 'save_settings');
  await expect.poll(async () => (await calls(page, 'monitor_source')).length).toBe(monitors + 1);
});

test('stereo card applies speaker order after success and preserves it after failure', async ({
  page,
}) => {
  await open(page);
  const card = page.locator('.device.selected');
  await card.locator('.device-row').click();
  await start(page).click();
  await playing(page, 101);
  const names = card.locator('.channel-pair > div > strong');
  const swap = card.getByTitle('交换扬声器位置', { exact: true });
  await expect(names).toHaveText(['Receiver A', 'Receiver B']);
  await hold(page, 'set_speaker_order');
  await swap.click();
  await expect.poll(async () => (await calls(page, 'set_speaker_order')).length).toBe(1);
  expect((await calls(page, 'set_speaker_order'))[0].args).toEqual({ swapped: true });
  await expect(names).toHaveText(['Receiver A', 'Receiver B']);
  await expect(swap).toBeDisabled();
  await resolve(page, 'set_speaker_order');
  await expect(names).toHaveText(['Receiver B', 'Receiver A']);
  await expect(swap).toBeEnabled();
  await expect(card.locator('.channel-pair > div > small:last-child')).toHaveText([
    '−∞ dBFS · 输入 1',
    '−∞ dBFS · 输入 2',
  ]);
  await hold(page, 'set_speaker_order');
  await swap.click();
  await expect.poll(async () => (await calls(page, 'set_speaker_order')).length).toBe(2);
  expect((await calls(page, 'set_speaker_order'))[1].args).toEqual({ swapped: false });
  await reject(page, 'set_speaker_order', 'fixture speaker order failure');
  await expect(page.locator('.alert')).toContainText('fixture speaker order failure');
  await expect(names).toHaveText(['Receiver B', 'Receiver A']);
  await expect(swap).toBeEnabled();
  expect(await calls(page, 'start_stream')).toHaveLength(1);
});

test('selecting an individual card starts only that receiver and shows both channels', async ({
  page,
}) => {
  await open(page);
  const card = page
    .locator('.device')
    .filter({ has: page.locator('.device-name > strong', { hasText: /^Receiver B$/ }) });
  await card.locator('.device-row').click();
  await expect(card).toHaveClass(/selected/);
  await expect(card.locator('.device-info')).toContainText('Receiver B · 192.0.2.11:7000');
  await start(page).click();
  await expect.poll(async () => (await calls(page, 'start_stream')).length).toBe(1);
  expect((await calls(page, 'start_stream'))[0].args.names).toEqual(['Receiver B']);
  await playing(page, 101);
  await expect(card.locator('.channel-pair')).toHaveClass(/single/);
  await expect(card.locator('.channel-pair > div > strong')).toHaveText([
    'Receiver B',
    'Receiver B',
  ]);
  await expect(card.getByTitle('交换扬声器位置', { exact: true })).toHaveCount(0);
});

test('autostart waits for the command, rolls back failure and allows retry', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const autostart = page.getByRole('checkbox', { name: '登录 Windows 后启动 AirPlay Hub' });
  await hold(page, 'set_autostart');
  await autostart.check();
  await expect(autostart).toBeDisabled();
  await expect.poll(async () => (await calls(page, 'set_autostart')).length).toBe(1);
  expect((await calls(page, 'set_autostart'))[0].args).toEqual({ enabled: true });
  await reject(page, 'set_autostart', 'fixture autostart failure');
  await expect(autostart).not.toBeChecked();
  await expect(autostart).toBeEnabled();
  await expect(page.locator('.alert')).toContainText('开机自启设置失败：');
  await hold(page, 'set_autostart');
  await autostart.check();
  await expect.poll(async () => (await calls(page, 'set_autostart')).length).toBe(2);
  await resolve(page, 'set_autostart');
  await expect(autostart).toBeEnabled();
  await expect(autostart).toBeChecked();
  expect((await calls(page, 'set_autostart')).map((call) => call.args.enabled)).toEqual([
    true,
    true,
  ]);
});

test('keep-awake save failure restores the checkbox and preserves other settings', async ({
  page,
}) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  const awake = page.getByRole('checkbox', { name: '避免系统自动睡眠' });
  await expect(awake).toBeChecked();
  const initial = (await calls(page, 'save_settings')).at(-1).args.settings;
  await hold(page, 'save_settings');
  await awake.uncheck();
  await expect(awake).toBeDisabled();
  await awake.click({ force: true });
  await expect(awake).not.toBeChecked();
  expect(await calls(page, 'save_settings')).toHaveLength(2);
  await expect(page.getByLabel('窗口关闭动作', { exact: true })).toBeDisabled();
  await expect(page.getByLabel('左输出取样', { exact: true })).toBeDisabled();
  await expect(source(page)).toBeDisabled();
  await expect(start(page)).toBeDisabled();
  await page.getByRole('tab', { name: '技术详情', exact: true }).click();
  await expect(page.getByLabel('采集 Buffer', { exact: true })).toBeDisabled();
  await page.getByRole('tab', { name: '日志', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: '保存详细日志', exact: true })).toBeDisabled();
  await expect(
    page.getByRole('checkbox', { name: '启用额外采集诊断', exact: true }),
  ).toBeDisabled();
  await page.getByRole('tab', { name: '常规', exact: true }).click();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings.keepAwake)
    .toBe(false);
  expect((await calls(page, 'save_settings')).at(-1).args.settings).toEqual({
    ...initial,
    keepAwake: false,
  });
  await reject(page, 'save_settings', 'fixture keep-awake failure');
  await expect(awake).toBeChecked();
  await expect(awake).toBeEnabled();
  await expect(page.locator('.alert')).toContainText('fixture keep-awake failure');
  await hold(page, 'save_settings');
  await awake.uncheck();
  await expect.poll(async () => (await calls(page, 'save_settings')).length).toBe(3);
  await expect(awake).toBeDisabled();
  expect(await calls(page, 'set_mapping')).toHaveLength(0);
  await resolve(page, 'save_settings');
  await expect(awake).toBeEnabled();
  await expect(awake).not.toBeChecked();
  await page.getByRole('button', { name: '返回播放', exact: true }).click();
  await start(page).click();
  await expect.poll(async () => (await calls(page, 'start_stream')).length).toBe(1);
  expect((await calls(page, 'start_stream'))[0].args.settings).toEqual({
    ...initial,
    keepAwake: false,
  });
});

test('general settings persist and runtime mapping keeps its desktop command', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByLabel('窗口关闭动作', { exact: true }).selectOption('quit');
  await expect(page.getByRole('button', { name: '退出应用', exact: true })).toBeVisible();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings.closeAction)
    .toBe('quit');
  const latency = page.getByRole('spinbutton', { name: '播放提前量', exact: true });
  await latency.fill('450');
  await latency.blur();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings.latency)
    .toBe(450);
  const saves = (await calls(page, 'save_settings')).length;
  const left = page.getByLabel('左输出取样', { exact: true });
  const right = page.getByLabel('右输出取样', { exact: true });
  await left.selectOption('1');
  await right.selectOption('0');
  await expect.poll(async () => (await calls(page, 'set_mapping')).length).toBe(2);
  expect((await calls(page, 'set_mapping')).map((call) => call.args.mapping)).toEqual([
    [1, 1],
    [1, 0],
  ]);
  expect(await calls(page, 'save_settings')).toHaveLength(saves);
  await page.getByLabel('外观', { exact: true }).selectOption('dark');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  expect(await page.evaluate(() => localStorage.getItem('theme'))).toBe('dark');
  await page.getByRole('button', { name: '返回播放', exact: true }).click();
  await start(page).click();
  await expect.poll(async () => (await calls(page, 'start_stream')).length).toBe(1);
  expect((await calls(page, 'start_stream'))[0].args.settings).toMatchObject({
    latency: 450,
    closeAction: 'quit',
    mapping: [1, 0],
  });
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(latency).toBeDisabled();
  await expect(left).toBeEnabled();
  await left.selectOption('0');
  await expect.poll(async () => (await calls(page, 'set_mapping')).length).toBe(3);
  expect((await calls(page, 'set_mapping')).at(-1).args.mapping).toEqual([0, 0]);
  expect(await calls(page, 'start_stream')).toHaveLength(1);
});

test('runtime snapshots distinguish missing values from zero and reset for the next session', async ({
  page,
}) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '运行统计', exact: true }).click();
  const metrics = page.locator('.metrics h2');
  const pipeline = page
    .locator('section.panel')
    .filter({ has: page.getByRole('heading', { name: '音频流水线', exact: true }) })
    .locator('dd');
  const empty = ['—', '—', '— ms', '— ms', '停止后汇总 / —', '停止后汇总'];
  await expect(metrics).toHaveText(['— s', '— ms', '— ppm']);
  await expect(pipeline).toHaveText(empty);
  await start(page).click();
  await readyEvents(page, 101);
  await send(page, 101, {
    ...telemetry,
    elapsed_seconds: 0,
    water_ms: 0,
    lead_ms: 0,
    capture_frames: 0,
    output_frames: 0,
    controller: { correction_ppm: 0 },
  });
  await expect(metrics).toHaveText(['0 s', '0.0 ms', '0.0 ppm']);
  await expect(pipeline).toHaveText(['0', '0', '0 ms', '0.0 ms', '停止后汇总 / —', '停止后汇总']);
  await send(page, 101, {
    ...telemetry,
    elapsed_seconds: 12.25,
    water_ms: 125.25,
    controller: { correction_ppm: -4.25 },
  });
  await send(page, 101, {
    kind: 'report',
    report: {
      device: 'Receiver A',
      capture: { discontinuities: 0, timestamp_errors: 2 },
      conversion: { frames: 44100 },
    },
  });
  await expect(metrics).toHaveText(['12 s', '125.3 ms', '-4.3 ppm']);
  await expect(pipeline.nth(3)).toHaveText('425.3 ms');
  await expect(pipeline.nth(4)).toHaveText('0 / 2');
  await expect(pipeline.nth(5)).toHaveText('{"frames":44100}');
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await expect(pipeline.nth(4)).toHaveText('0 / 2');
  await start(page).click();
  await expect(metrics).toHaveText(['— s', '— ms', '— ppm']);
  await expect(pipeline).toHaveText(empty);
});

test('transport statistics survive tab changes and reconnects without double-counting or stale events', async ({
  page,
}) => {
  await open(page);
  await start(page).click();
  await playing(page, 101);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '运行统计', exact: true }).click();
  const rows = page.locator('table tbody tr');
  await expect(rows).toHaveCount(0);
  const first =
    '[PACKET_STATS] host=192.0.2.10 sent=10 send_dropped=1 rtx_requested=4 rtx_resent=3 rtx_expired=0';
  await send(page, 101, { kind: 'native', line: first });
  await send(page, 101, { kind: 'native', line: first });
  await expect(rows.nth(0).locator('td')).toHaveText(['Receiver A', '10', '1', '4', '3', '0']);
  await send(page, 101, {
    kind: 'native',
    line: '[PACKET_STATS] host=192.0.2.10 sent=15 send_dropped=2 rtx_requested=6 rtx_resent=4 rtx_expired=1',
  });
  await send(page, 101, { kind: 'native', line: '[PACKET_STATS] host=192.0.2.11 sent=4' });
  await send(page, 101, { kind: 'native', line: '[PACKET_STATS] host=192.0.2.99 sent=3' });
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0).locator('td')).toHaveText(['Receiver A', '15', '2', '6', '4', '1']);
  await expect(rows.nth(1).locator('td')).toHaveText(['Receiver B', '4', '—', '—', '—', '—']);
  await expect(rows.nth(2).locator('td')).toHaveText(['192.0.2.99', '3', '—', '—', '—', '—']);
  await page.getByRole('tab', { name: '常规', exact: true }).click();
  await page.getByRole('tab', { name: '运行统计', exact: true }).click();
  await expect(rows.nth(0).locator('td')).toHaveText(['Receiver A', '15', '2', '6', '4', '1']);
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await start(page).click();
  await readyEvents(page, 102);
  await send(page, 101, { kind: 'native', line: '[PACKET_STATS] host=192.0.2.10 sent=999' });
  await expect(rows.nth(0).locator('td')).toHaveText(['Receiver A', '15', '2', '6', '4', '1']);
  await send(page, 102, {
    kind: 'native',
    line: '[PACKET_STATS] host=192.0.2.10 sent=2 send_dropped=0 rtx_requested=1 rtx_resent=1 rtx_expired=0',
  });
  await expect(rows.nth(0).locator('td')).toHaveText(['Receiver A', '17', '2', '7', '5', '1']);
  await expect(rows).toHaveCount(3);
});

test('technical details preserve formats, preview levels, Buffer persistence and busy restrictions', async ({
  page,
}) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '技术详情', exact: true }).click();
  const values = page.locator('.capture-panel dd');
  await expect(values.nth(0)).toHaveText('48000 Hz · 24-bit PCM · 2 声道');
  await expect(values.nth(1)).toHaveText('48000 Hz · 32-bit float · 2 声道');
  await expect(values.nth(2)).toHaveText('44100 Hz · 16-bit PCM → ALAC');
  await expect(values.nth(4)).toHaveText('−∞ dBFS / −∞ dBFS');
  await page.evaluate(
    (endpoint) => window.__airplayTest.sourceLevel({ endpoint, peaks: [0.1, 0.5] }),
    inputs[0].id,
  );
  await expect(values.nth(4)).toHaveText('-20.0 dBFS / -6.0 dBFS');
  await expect(values.nth(4).locator('.meter > i')).toHaveAttribute('style', 'width: 50%;');
  const capabilities = page
    .locator('section.panel')
    .filter({ has: page.getByRole('heading', { name: '设备能力', exact: true }) });
  await expect(capabilities.locator('summary')).toHaveText([
    'Receiver A · 192.0.2.10:7000',
    'Receiver B · 192.0.2.11:7000',
  ]);
  await capabilities.locator('summary').first().click();
  await expect(capabilities.locator('pre').first()).toBeVisible();
  await expect(capabilities.locator('pre').first()).toContainText('"igl": "1"');
  const buffer = page.getByRole('spinbutton', { name: '采集 Buffer', exact: true });
  await expect(buffer).toHaveAttribute('min', '64');
  await expect(buffer).toHaveAttribute('max', '512');
  await buffer.fill('256');
  await buffer.blur();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings.buffer)
    .toBe(256);
  const saved = (await calls(page, 'save_settings')).at(-1).args.settings;
  await start(page).click();
  await expect.poll(async () => (await calls(page, 'start_stream')).length).toBe(1);
  expect((await calls(page, 'start_stream'))[0].args.settings).toEqual(saved);
  await expect(buffer).toBeDisabled();
  await expect(page.getByRole('button', { name: '重新检测所选设备', exact: true })).toBeDisabled();
  await readyEvents(page, 101);
  await send(page, 101, { kind: 'native', line: '[AUTH_METHOD] value=pair-verify' });
  await send(page, 101, { kind: 'native', line: '[TIMING] value=PTP' });
  await expect(values.nth(6)).toHaveText('{"认证方式":"pair-verify","时钟协议":"PTP"}');
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await expect(buffer).toBeEnabled();
  await expect(page.getByRole('button', { name: '重新检测所选设备', exact: true })).toBeEnabled();
});

test('auth-policy reset reports failure and targets the current selection on success', async ({
  page,
}) => {
  await open(page);
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('tab', { name: '技术详情', exact: true }).click();
  const reset = page.getByRole('button', { name: '重新检测所选设备', exact: true });
  const notice = page.locator('.capture-panel dd').nth(5).locator('small');
  const initial = '只记住密码要求，不保存密码。取消设备密码后可重新检测。';
  await hold(page, 'forget_auth_policy');
  await reset.click();
  await expect.poll(async () => (await calls(page, 'forget_auth_policy')).length).toBe(1);
  expect((await calls(page, 'forget_auth_policy'))[0].args).toEqual({
    names: ['Receiver A', 'Receiver B'],
  });
  await expect(notice).toHaveText(initial);
  await reject(page, 'forget_auth_policy', 'fixture auth reset failure');
  await expect(page.locator('.alert')).toContainText('fixture auth reset failure');
  await expect(notice).toHaveText(initial);
  await page.getByRole('button', { name: '返回播放', exact: true }).click();
  const receiver = page
    .locator('.device')
    .filter({ has: page.locator('.device-name > strong', { hasText: /^Receiver B$/ }) });
  await receiver.locator('.device-row').click();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await hold(page, 'forget_auth_policy');
  await reset.click();
  await expect.poll(async () => (await calls(page, 'forget_auth_policy')).length).toBe(2);
  expect((await calls(page, 'forget_auth_policy'))[1].args).toEqual({ names: ['Receiver B'] });
  await expect(notice).toHaveText(initial);
  await resolve(page, 'forget_auth_policy');
  await expect(notice).toHaveText('已清除所选设备记录，下次连接重新检测。');
  expect(await calls(page, 'start_stream')).toHaveLength(0);
});

test('log switches persist independently for the next session and opening logs handles command failures', async ({
  page,
}) => {
  await open(page);
  await openLogView(page);
  const sessionLog = logPanel(page, '会话日志');
  const diagnostics = logPanel(page, '采集逐包诊断');
  const detailed = page.getByRole('checkbox', { name: '保存详细日志', exact: true });
  const capture = page.getByRole('checkbox', { name: '启用额外采集诊断', exact: true });
  await detailed.check();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings)
    .toMatchObject({ detailedLogs: true, captureDiagnostics: false });
  await capture.check();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings)
    .toMatchObject({ detailedLogs: true, captureDiagnostics: true });
  await detailed.uncheck();
  await expect
    .poll(async () => (await calls(page, 'save_settings')).at(-1).args.settings)
    .toMatchObject({ detailedLogs: false, captureDiagnostics: true });
  await expect(sessionLog.locator('pre.logs')).toHaveText('暂无关键故障。');
  await expect(diagnostics.locator('pre.logs')).toHaveText('未启用采集诊断。');
  await start(page).click();
  await expect.poll(async () => (await calls(page, 'start_stream')).length).toBe(1);
  expect((await calls(page, 'start_stream'))[0].args.settings).toMatchObject({
    detailedLogs: false,
    captureDiagnostics: true,
  });
  await expect(detailed).toBeDisabled();
  await expect(capture).toBeDisabled();
  await expect(diagnostics.locator('pre.logs')).toHaveText('等待采集诊断输出…');
  const folder = page.getByRole('button', { name: '打开日志目录', exact: true });
  await hold(page, 'open_logs');
  await folder.click();
  await expect.poll(async () => (await calls(page, 'open_logs')).length).toBe(1);
  expect((await calls(page, 'open_logs'))[0].args).toEqual({});
  await reject(page, 'open_logs', 'fixture open logs failure');
  await expect(page.locator('.alert')).toContainText('fixture open logs failure');
  await folder.click();
  await expect.poll(async () => (await calls(page, 'open_logs')).length).toBe(2);
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await expect(detailed).toBeEnabled();
  await expect(capture).toBeEnabled();
});

test('diagnostic history remains bounded after event bursts and end records', async ({ page }) => {
  await open(page);
  await openLogView(page);
  await page.getByRole('checkbox', { name: '保存详细日志', exact: true }).check();
  await page.getByRole('checkbox', { name: '启用额外采集诊断', exact: true }).check();
  await start(page).click();
  await readyEvents(page, 101);
  await page.evaluate(async (diagnostic) => {
    for (let i = 0; i < 310; i++) {
      await window.__airplayTest.send(101, {
        kind: 'native',
        line: 'private raw output',
        safe_line: `safe-${i}` + (i === 309 ? 'x'.repeat(8000) : ''),
      });
    }
    for (let i = 0; i < 125; i++) {
      await window.__airplayTest.send(101, { ...diagnostic, capture_frames: i });
    }
    await window.__airplayTest.send(101, {
      kind: 'diagnostic_end',
      status: { error: 'end-marker' },
    });
  }, captureDiagnostic);
  const logs = logPanel(page, '会话日志').locator('pre.logs');
  const diagnostics = logPanel(page, '采集逐包诊断');
  await expect(logs).toContainText('safe-10');
  const lines = (await logs.textContent()).split('\n');
  expect(lines).toHaveLength(300);
  expect(lines[0]).toBe('safe-10');
  expect(lines.at(-1)).toMatch(/^safe-309.*截断/);
  expect(lines.every((line) => line.length <= 4096)).toBe(true);
  expect(lines.join('\n')).not.toContain('private raw output');
  const summaries = (await diagnostics.locator('pre.logs').textContent()).split('\n\n');
  expect(summaries).toHaveLength(120);
  expect(summaries[0]).toContain('采集 6 帧');
  expect(summaries.at(-1)).toContain('end-marker');
  await diagnostics.getByRole('button', { name: '清空显示', exact: true }).click();
  await expect(diagnostics.locator('pre.logs')).toHaveText('等待采集诊断输出…');
  await expect(logs).toContainText('safe-309');
});

test('clearing diagnostics preserves session logs, paths and expandable reports while new summaries continue', async ({
  page,
}) => {
  await open(page);
  await openLogView(page);
  await page.getByRole('checkbox', { name: '保存详细日志', exact: true }).check();
  await page.getByRole('checkbox', { name: '启用额外采集诊断', exact: true }).check();
  await start(page).click();
  await readyEvents(page, 101);
  const sessionLog = logPanel(page, '会话日志');
  const diagnostics = logPanel(page, '采集逐包诊断');
  await send(page, 101, {
    kind: 'log_path',
    path: 'fixture-root/session.log',
    detailed_logs: true,
    pipeline_path: 'fixture-root/pipeline.jsonl',
  });
  await send(page, 101, {
    kind: 'diagnostic_path',
    path: 'fixture-root/capture.jsonl',
    pipeline_path: 'fixture-root/pipeline.jsonl',
  });
  await send(page, 101, {
    kind: 'native',
    line: 'fixture raw output',
    safe_line: 'fixture safe output',
  });
  await send(page, 101, captureDiagnostic);
  const report = {
    device: 'Receiver A',
    capture: { discontinuities: 0, timestamp_errors: 1 },
    conversion: { frames: 44100 },
  };
  await send(page, 101, { kind: 'report', report });
  await expect(sessionLog.locator('pre.logs')).toContainText('fixture safe output');
  await expect(sessionLog.locator('pre.logs')).not.toContainText('fixture raw output');
  await expect(sessionLog.locator('.log-location')).toHaveText([
    '%APPDATA%/AirPlay Hub/logs/session.log',
    '流水线：%APPDATA%/AirPlay Hub/logs/pipeline.jsonl',
  ]);
  await expect(diagnostics.locator('.log-location')).toHaveText(
    '逐包：%APPDATA%/AirPlay Hub/logs/capture.jsonl',
  );
  await expect(diagnostics.locator('pre.logs')).toContainText('1.5s · 采集 48000 帧');
  await sessionLog.getByText('完整会话报告', { exact: true }).click();
  await expect(sessionLog.locator('details pre')).toBeVisible();
  await expect(sessionLog.locator('details pre')).toHaveText(JSON.stringify(report, null, 2));
  const saves = (await calls(page, 'save_settings')).length;
  await diagnostics.getByRole('button', { name: '清空显示', exact: true }).click();
  await expect(diagnostics.locator('pre.logs')).toHaveText('等待采集诊断输出…');
  await expect(sessionLog.locator('pre.logs')).toContainText('fixture safe output');
  await expect(sessionLog.locator('details pre')).toHaveText(JSON.stringify(report, null, 2));
  await expect(diagnostics.locator('.log-location')).toHaveText(
    '逐包：%APPDATA%/AirPlay Hub/logs/capture.jsonl',
  );
  expect(await calls(page, 'save_settings')).toHaveLength(saves);
  expect(await calls(page, 'stop_stream')).toHaveLength(0);
  await send(page, 101, { ...captureDiagnostic, elapsed_seconds: 2.5, capture_frames: 96000 });
  await expect(diagnostics.locator('pre.logs')).toContainText('2.5s · 采集 96000 帧');
  await expect(diagnostics.locator('pre.logs')).not.toContainText('1.5s');
  await page.getByRole('tab', { name: '常规', exact: true }).click();
  await page.getByRole('tab', { name: '日志', exact: true }).click();
  await expect(diagnostics.locator('pre.logs')).toContainText('2.5s · 采集 96000 帧');
  await expect(sessionLog.locator('pre.logs')).toContainText('fixture safe output');
  await sessionLog.getByText('完整会话报告', { exact: true }).click();
  await expect(sessionLog.locator('details pre')).toHaveText(JSON.stringify(report, null, 2));
});

test('log snapshots reset on reconnect and reject stale log, diagnostic and report events', async ({
  page,
}) => {
  await open(page);
  await openLogView(page);
  const sessionLog = logPanel(page, '会话日志');
  const diagnostics = logPanel(page, '采集逐包诊断');
  await start(page).click();
  await readyEvents(page, 101);
  await send(page, 101, { kind: 'native', line: 'fixture ordinary output' });
  await expect(sessionLog.locator('pre.logs')).toHaveText('暂无关键故障。');
  await send(page, 101, {
    kind: 'native',
    line: 'fixture raw fault',
    safe_line: 'fixture safe fault',
    is_fault: true,
  });
  await expect(sessionLog.locator('pre.logs')).toHaveText('fixture safe fault');
  await send(page, 101, {
    kind: 'log_path',
    path: 'fixture-root/session.log',
    detailed_logs: false,
    pipeline_path: 'fixture-root/old-pipeline.jsonl',
  });
  await send(page, 101, { kind: 'report', report: { device: 'Receiver A' } });
  await expect(sessionLog.locator('details')).toHaveCount(1);
  await page.getByRole('button', { name: '停止串流', exact: true }).click();
  await send(page, 101, { kind: 'finished', cancelled: true });
  await expect(sessionLog.locator('pre.logs')).toHaveText('fixture safe fault');
  await page.getByRole('checkbox', { name: '启用额外采集诊断', exact: true }).check();
  await expect(diagnostics.locator('pre.logs')).toHaveText('未启用采集诊断。');
  await start(page).click();
  await readyEvents(page, 102);
  await expect(sessionLog.locator('pre.logs')).toHaveText('暂无关键故障。');
  await expect(sessionLog.locator('details')).toHaveCount(0);
  await expect(sessionLog.locator('.log-location')).toHaveText([
    '%APPDATA%/AirPlay Hub/logs/session.log',
  ]);
  await expect(diagnostics.locator('.log-location')).toHaveCount(0);
  await expect(diagnostics.locator('pre.logs')).toHaveText('等待采集诊断输出…');
  await send(page, 101, { kind: 'log_path', path: 'fixture-root/stale.log', detailed_logs: true });
  await send(page, 101, { kind: 'diagnostic_path', path: 'fixture-root/stale.jsonl' });
  await send(page, 101, { kind: 'native', line: 'stale fault', is_fault: true });
  await send(page, 101, { ...captureDiagnostic, elapsed_seconds: 9.5 });
  await send(page, 101, { kind: 'report', report: { device: 'stale receiver' } });
  await expect(sessionLog.locator('pre.logs')).toHaveText('暂无关键故障。');
  await expect(sessionLog.locator('details')).toHaveCount(0);
  await expect(sessionLog.locator('.log-location')).toHaveText([
    '%APPDATA%/AirPlay Hub/logs/session.log',
  ]);
  await expect(diagnostics.locator('.log-location')).toHaveCount(0);
  await expect(diagnostics.locator('pre.logs')).toHaveText('等待采集诊断输出…');
  await send(page, 102, { ...captureDiagnostic, elapsed_seconds: 2.5 });
  await expect(diagnostics.locator('pre.logs')).toContainText('2.5s · 采集 48000 帧');
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
