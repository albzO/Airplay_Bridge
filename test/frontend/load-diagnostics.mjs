import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../../airplay-frontend/package.json', import.meta.url));
const ts = require('typescript');
function compile(path, dependencies = {}) {
  const source = readFileSync(
    new URL('../../airplay-frontend/src/' + path, import.meta.url),
    'utf8',
  );
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  new Function('exports', 'require', code)(exports, (name) => {
    if (Object.hasOwn(dependencies, name)) return dependencies[name];
    throw new Error('Unexpected diagnostic dependency: ' + name);
  });
  return exports;
}
// 使用真实 Vue 与展示函数，不复制被测试的汇总逻辑。
// Load actual Vue and display code instead of duplicating aggregation logic.
export const diagnosticsModule = compile('useDiagnostics.ts', {
  vue: require('vue'),
  './display': compile('display.ts'),
});
