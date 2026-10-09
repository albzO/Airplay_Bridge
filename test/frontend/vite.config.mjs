import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

// 复用生产界面的依赖和源码，仅在测试页面模拟桌面命令。
// Reuse production UI dependencies/source; mock desktop commands only in the test page.
const require = createRequire(new URL('../../airplay-frontend/package.json', import.meta.url));
const vue = (await import(pathToFileURL(require.resolve('@vitejs/plugin-vue')).href)).default;
export default {
  root: fileURLToPath(new URL('./ui', import.meta.url)),
  plugins: [vue()],
  build: {
    outDir: fileURLToPath(new URL('../.artifacts/frontend-ui-build', import.meta.url)),
    emptyOutDir: false,
  },
  resolve: {
    alias: {
      vue: require.resolve('vue/dist/vue.runtime.esm-bundler.js'),
      '@tauri-apps/api': fileURLToPath(
        new URL('../../airplay-frontend/node_modules/@tauri-apps/api', import.meta.url),
      ),
    },
  },
  server: {
    host: '127.0.0.1',
    fs: { allow: [fileURLToPath(new URL('../..', import.meta.url))] },
    watch: { ignored: ['**/src-tauri/**'] },
  },
};
