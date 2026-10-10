# 测试 / Tests

项目测试源码统一放在根目录 `test/`，随 Git 提交。生产目录只保留测试模块挂接及诊断入口；已有测试的私有模块访问关系和忽略标记保持不变。

Test sources are committed under root `test/`. Production modules retain attachment/diagnostic entry points; existing private-module access and ignored hardware checks are preserved.

`core/unit/live/protocol.rs` 使用实际协议读线程和可注入 writer，分别阻塞/失败文件与控制台输出，验证队列溢出时就绪、认证错误及首个故障仍被处理、取消不等待日志、收尾超时后不输出积压，并检查详细/故障日志脱敏及密码提示。夹具在失败时也解除故意阻塞，无需声卡或接收设备。完整串流报告中的 `protocol_log_status` 单独表示日志健康；没有据此断言整个应用有严格停止上限。

`core/unit/capture/decode.rs` 的 8 项测试直接调用生产解码模块，用固定字节验证 PCM 8/16/24/32 位、24-in-32、float32 特殊值、默认/重复/多声道映射、帧裁剪与非法输入，检查跨包输出缓冲复用及完整原始包摘要。无需声卡或模拟接收端；不验证 WASAPI 驱动或实际播放。可单独运行 `cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib capture::decode::`。

`core/unit/capture/metrics.rs` 的 6 项测试调用纯统计/时序模块，覆盖电平与信号帧阈值、NaN/Infinity、固定指纹与负零/NaN 位差、可选指纹、空/不完整帧、首条及跨日志暂停的诊断间隔、有符号回退及 100 ns 单位换算。无需设备、不改写音频；运行过滤器为 `capture::metrics::`。Windows 时钟读取及实际驱动时序不在此纯逻辑测试范围内。

| 目录 | 内容 |
|---|---|
| `frontend/` | JSON 协议、会话脚本、真实模板 DOM、模拟 UI、合成设备样例和桌面 Rust 测试 |
| `backend/` | 本机模拟接收端、认证、密码管道、立体声、延迟 SETUP 和原生 nonce 检查 |
| `core/unit/` | 采集解码/统计/时间线/健康状态、转换、漂移、来源生命周期、串流管线与脱敏单元测试 |
| `core/checks/` | 已采集 WAV 与转换 PCM 的独立校验脚本 |
| `.artifacts/` | 截图、模拟 PCM 和检查输出；不提交 Git |

## 运行 / Run

以下命令在项目根目录执行。前端测试只需 `pnpm --dir airplay-frontend install --frozen-lockfile` 与相应浏览器，无需原生后端或声卡。原生/Python 协议检查先按 [构建指南](../docs/building.md) 构建后端；Python 另需 `backend/requirements.txt` 中的依赖。

Run commands from the project root. Frontend tests require frontend dependencies and, for DOM tests, a browser; native/Python receiver checks additionally require the backend and Python requirements.

```powershell
# Rust 单元测试 / Rust unit tests
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib
cargo test --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 前端协议及模拟页面 / Frontend protocol and mocked UI
pnpm --dir airplay-frontend test:protocol
pnpm --dir airplay-frontend test:session
pnpm --dir airplay-frontend test:dom
pnpm --dir airplay-frontend test:ui
pnpm --dir airplay-frontend test:ui:build

# 本机模拟接收端 / Local mock receivers
python -m pip install -r test/backend/requirements.txt
python test/backend/check_auth.py
python test/backend/check_gui_pipe.py
python test/backend/check_gui_pipe.py --direct
python test/backend/check_stereo.py
python test/backend/check_setup.py
python test/backend/mock_receiver.py
python test/backend/test_harness.py
python test/backend/test_upstream.py

# 约 9 分钟，完整传输跨 16-bit RTP 序号回绕 / ~9-minute full transport RTP wrap
python test/backend/check_wrap.py

# 无声卡的 Release 合成基线 / Synthetic release baseline without audio hardware
$env:AIRPLAY_BASELINE_NAME = 'performance-baseline.json'
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --release --lib synthetic_performance_baseline -- --ignored --nocapture
Remove-Item Env:AIRPLAY_BASELINE_NAME
python test/core/checks/compare_performance.py test/.artifacts/core/performance-before.json test/.artifacts/core/performance-after.json

# 已有采集/转换文件的校验 / Validate existing capture/conversion artifacts
python test/core/checks/check_capture.py "path/to/capture.wav"
python test/core/checks/check_conversion.py "path/to/capture.pcm"
```

`test:session` 自动执行真实页面脚本和 `useStreamSession.ts`，覆盖命令返回前的事件、旧会话事件、密码重试、停止、启动失败、事件积压和卸载；替换桌面 IPC 与浏览器接口，不覆盖 DOM 点击、布局或真实 WebView。

`test:session` executes the actual page script for early/stale events, password retries, stopping, startup failure, overflow and unmount. Desktop IPC/browser APIs are mocked; DOM clicks, layout and the real WebView are not covered.

`test:dom` 使用固定版本 `@playwright/test 1.64.0`，加载生产 `main.ts`、`App.vue`、`SourcePicker.vue` 和样式，模拟 Tauri IPC/事件，执行 10 项真实点击/输入回归：无来源、无设备、来源选择及 Escape、分组排序/外部关闭、单声道保存后预览、连接/停止时控件禁用、Enter 密码重试与新会话归属、旧事件隔离、启动失败恢复、重复/迟到密码提交。未捕获浏览器异常也会失败。夹具没有定时协议事件，测试可暂停/完成/拒绝命令来确定异步顺序，不读取或改写组件内部状态。

Windows 默认使用已安装 Edge 的无头模式，无需下载浏览器；已有 Chrome 可设置 `$env:AIRPLAY_TEST_BROWSER_CHANNEL = 'chrome'` 后运行，完成后 `Remove-Item Env:AIRPLAY_TEST_BROWSER_CHANNEL`。其他系统默认使用 Playwright Chromium，先运行 `pnpm --dir airplay-frontend exec playwright install chromium`。浏览器选择依据见 [Playwright 文档](https://playwright.dev/docs/browsers)。

测试自己启动并回收 `127.0.0.1:4178` 的 Vite 服务与浏览器上下文，端口已占用则失败，不复用人工页面。每项最多 15 秒、整组最多 120 秒，不自动重试；失败截图/trace 和运行状态只写入忽略的 `test/.artifacts/frontend-dom/`，Vite 缓存位于 `.artifacts/frontend-vite-cache/`。`test:ui` 仍是人工模拟入口；`test:ui:build` 同时构建人工和 DOM 夹具页面。DOM 回归不等于真实 Tauri WebView、布局截图、IME、托盘或声卡/HomePod 测试。

来源组件提取后 DOM 共 10 项：新增分组自然排序/外部点击关闭，以及单声道选择重置为 `[0, 0]`、保存完成后才启动预览的回归。测试通过真实 `App.vue` 与 `SourcePicker.vue` 的按钮操作，只在桌面 IPC 边界暂停/完成命令。

`test:dom` mounts the actual production UI in headless Edge on Windows (Chromium elsewhere), mocks desktop boundaries and checks ten click/keyboard/lifecycle scenarios, including source grouping/dismissal and save-before-monitor ordering. It owns its local server/browser, fails on uncaught browser errors and writes only ignored failure artifacts; it does not exercise native WebView or audio hardware.

`test_upstream.py` 无需原生编译或声卡，需要 Python、Git 和固定子模块。7 项测试核对输出快照、三层提交、输入/补丁哈希、CRLF、只在临时副本应用补丁、上下文失败保留旧输出及函数选择边界；补丁和输入变更须先审查再更新清单，见 [上游适配变更](../docs/building.md#上游适配变更)。

`test_upstream.py` requires Python, Git and the pinned submodules, without a native build or audio hardware. It checks snapshots, revisions, hashes, CRLF, temporary-copy patching, failed-context isolation and function selection.

后端默认读取 `dist/runtime/airplay-backend.exe`。设置 `AIRPLAY_TEST_BACKEND` 可检查另一个构建；`check_setup.py --baseline <旧后端路径>` 可重现旧的 2 秒 SETUP 超时。模拟检查只连接 localhost，不连接真实接收端。

Backend checks default to `dist/runtime/airplay-backend.exe`; `AIRPLAY_TEST_BACKEND` selects another build. `check_setup.py --baseline <old-backend-path>` reproduces the earlier 2-second timeout. Mock checks connect only to localhost.

`mock_receiver.py`、`check_stereo.py` 和 `check_setup.py` 使用统一资源守卫：失败时终止并回收已启动子进程、关闭 socket，并限时等线程退出。`test_harness.py` 注入断言失败验证收尾，同时分别检查 16-bit 序号、32-bit 时间戳模运算及独立 64-bit nonce。接收端只保留最近 512 个原始包用于重传核对。

`check_wrap.py` 以实际节奏发送 526 秒合成 PCM，加上 EOF 尾部静音，自然跨过 65535→0；校验原生后端的包顺序、nonce、解密 PCM 及跨界两包重传。32-bit RTP 时间戳回绕由合成边界测试覆盖，此脚本没有运行约 27 小时来覆盖实际时间戳回绕。PTP 模拟占用本机 UDP 319/320，请串行运行各 PTP 脚本。

The shared fixture guard reaps children, closes sockets and bounds thread joins on failures. The wrap check sends paced synthetic PCM through the native backend and checks decryption, nonce progression and retransmissions across 65535→0. Full-chain 32-bit timestamp wrap and physical devices are outside this check.

性能入口每种采样率运行 1 轮预热和 3 轮测量，每轮 5 秒合成双声道、10 ms 输入包，经过真实管线和 PCM 写线程，终点为 `io::sink`；没有节奏等待、WASAPI、协议时钟、日志或接收端。记录每次 `process` 的 p50/p95/p99/max、总墙钟时间、PCM 队列峰值与新建/复用缓冲数。默认忽略，产物写入 `test/.artifacts/core/`；名称限制为该目录内的 `.json` 文件。比较前后时分别设置名称保留两份结果；比较脚本默认报告变化，可用 `--max-regression-percent 25` 对各采样率三轮中位 p95 设置退出码阈值。阈值是调用者选定的策略，未作为项目固定性能承诺；方法与本轮记录见 [性能基线](../docs/performance-baseline.md)。

The explicit release benchmark measures synthetic pipeline wall time and buffer reuse, not real-device latency or total application CPU. Compare artifacts produced on the same host/profile; the optional regression threshold is caller-selected.

需要真实声卡、局域网监听或接收端的 Rust 检查保留 `#[ignore]`，默认不会运行。明确选择检查名称后才使用 `--ignored`；这些检查可能采集或连接真实设备。测试和编译不能替代实机听感验证。

Rust checks needing real audio hardware, network listeners or receivers remain ignored by default. Select an individual check before using `--ignored`; these checks can capture or connect to real devices. Automated checks do not establish real-device playback quality.

设备样例只使用文档保留地址和合成标识。测试源码、模拟样例可以提交；真实配置、设备清单、录音、截图输出、日志和缓存仍忽略，不加入发行包。

Fixtures use documentation addresses and synthetic identities. Commit test sources and synthetic fixtures; exclude real configuration, discovery data, recordings, screenshot output, logs and caches from Git and release packages.
