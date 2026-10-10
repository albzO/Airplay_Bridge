import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(new URL('../../airplay-frontend/package.json', import.meta.url));
const { defineConfig } = require('@playwright/test');

export default defineConfig({
  testDir: './dom',
  outputDir: '../.artifacts/frontend-dom',
  workers: 1,
  retries: 0,
  forbidOnly: true,
  timeout: 15_000,
  globalTimeout: 120_000,
  reporter: 'list',
  use: {
    browserName: 'chromium',
    channel:
      process.env.AIRPLAY_TEST_BROWSER_CHANNEL ||
      (process.platform === 'win32' ? 'msedge' : undefined),
    headless: true,
    viewport: { width: 1280, height: 900 },
    baseURL: 'http://127.0.0.1:4178',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  // 测试自己启动/回收服务，不复用可能过期的人工测试页面。
  // Own the server lifecycle instead of reusing a potentially stale manual test page.
  webServer: {
    command:
      'node node_modules/vite/bin/vite.js --config ../test/frontend/vite.config.mjs --port 4178 --strictPort',
    cwd: fileURLToPath(new URL('../../airplay-frontend', import.meta.url)),
    url: 'http://127.0.0.1:4178/dom.html',
    reuseExistingServer: false,
    timeout: 20_000,
  },
});
