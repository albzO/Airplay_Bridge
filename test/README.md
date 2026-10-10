# 测试 / Tests

项目测试源码统一放在根目录 `test/`，随 Git 提交。生产目录只保留测试模块挂接及诊断入口；已有测试的私有模块访问关系和忽略标记保持不变。

Test sources are committed under root `test/`. Production modules retain attachment/diagnostic entry points; existing private-module access and ignored hardware checks are preserved.

Source 缓冲池阶段：核心 97 通过 / 9 默认忽略（7 项设备/本机环境检查及 2 项显式基线）、桌面 9、前端脚本 23 通过。新增 `core/unit/source_buffers.rs` 的位值/所有权/扩容和保留上限回归，`source.rs` 的非法/超预算/大包及跨线程变长包/采样率回归；原失败、停止、重连清理测试继续通过。`source_baseline.rs` 使用真实 send_audio 路径，显式 Release 执行；产物忽略、默认覆盖固定文件且每份小于 64 KiB。方法、命令和前后结果见 [Source 性能记录](../docs/source-performance.md)。本阶段没有改变前端状态机或重跑 DOM/设备播放。

自动熄屏缓解验证：核心 93 通过 / 8 默认忽略、桌面 9 通过、前端脚本 23 通过、DOM 28 通过。`frontend/desktop/awake.rs` 验证系统/显示器两项请求顺序、失败撤销与错误保留，并实际创建请求、检查重复开关及关闭/析构后的 Windows 句柄；没有等待真实显示器超时。核心检查未启用逐包诊断时的故障字段及倒退数值，DOM 检查开关说明、失败类别、回退与重试。证据与实机复测见[排查记录](../docs/display-off-capture.md)。

F1–F5 修复后的快速验证为核心 80 通过 / 8 默认忽略、桌面 7 通过、前端协议 7 + 会话/页面脚本 12 通过、真实模板 DOM 27 通过。范围与剩余边界见 [自证记录](../docs/review-self-check.md#f1f5-修复与正式回归)。

`frontend/desktop/routing.rs` 的 2 项回归调用生产路由模块，在暂存设置文件路径创建目录触发真实写入失败，检查磁盘、内存和活动 Mutex/AtomicBool 不变，解除故障后可重试。它们没有创建 WASAPI Source 或完整 Tauri 会话。

`core/unit/live/report.rs` 的 2 项回归调用生产结果选择/报告模块，以目录占用 JSON 目标路径，验证协议、采集、管道、传输/退出及仅报告失败的优先级；即使无法落盘仍收到包含 `report_write_error` 的内存报告。可写报告与 GUI 快照一致，已有故障仍保留；报告失败使用 `REPORT_WRITE_FAILED` 并保留操作和 OS 错误来源。

DOM 新增来源失败/重试、等待预览、冷启动保存失败、映射回退以及两种日志模式；加强保持唤醒重复点击、跨标签禁用与互换等待。IPC 夹具仅在成功完成时应用保存/路由副作用，拒绝不会先修改模拟设置。脚本另外覆盖重复来源选择及卸载后的迟到保存成功/失败；这些边界模拟不等于实机播放验证。

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

`frontend/diagnostics.test.mjs` 直接执行生产 `useDiagnostics.ts`，覆盖所有日志入口的数量/文本限制、结束摘要、报告大小、清空/重连、重复/部分/非法统计、设备淘汰及原型式键名。DOM 突发事件回归验证真实页面的 300/120 上限、截断、脱敏和清空范围。

`core/unit/capture/{clock,state,diagnostics}.rs` 覆盖 QPC 换算、首包断续、观察与成功交付计数、可选进度、诊断暂停时间差、端点字段、sink 错误和最终 JSON 字段；无需声卡。`core/unit/log_store.rs` 用真实临时文件验证容量、目录保留、活动预留、未知文件/目录保护及 Windows 打开句柄。Source 临界轮转及超大报告首因回归在 `core/unit/live/`。完整保留规则见 [日志限制](../docs/log-retention.md)。

## 运行 / Run

以下命令在项目根目录执行。前端测试只需 `pnpm --dir airplay-frontend install --frozen-lockfile` 与相应浏览器，无需原生后端或声卡。原生/Python 协议检查先按 [构建指南](../docs/building.md) 构建后端；Python 另需 `backend/requirements.txt` 中的依赖。

Run commands from the project root. Frontend tests require frontend dependencies and, for DOM tests, a browser; native/Python receiver checks additionally require the backend and Python requirements.

```powershell
# Rust 单元测试 / Rust unit tests
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib
# Tauri 测试嵌入生产产物，先完成构建再运行，避免与资源替换并行。
# Build production assets before Tauri tests; do not rebuild them concurrently.
pnpm --dir airplay-frontend build
cargo test --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 前端协议及模拟页面 / Frontend protocol and mocked UI
pnpm --dir airplay-frontend test:protocol
pnpm --dir airplay-frontend test:session
pnpm --dir airplay-frontend test:diagnostics
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

`test:dom` 使用固定版本 `@playwright/test 1.64.0`，加载生产 `main.ts`、`App.vue`、`SourcePicker.vue`、`DeviceCard.vue`、`GeneralSettings.vue`、`RuntimeStats.vue`、`TechnicalDetails.vue`、`LogView.vue` 和样式，模拟 Tauri IPC/事件，执行 27 项真实点击/输入回归：无来源、无设备、来源选择及 Escape、分组排序/外部关闭、单声道保存后预览、设备卡片互换成功/失败及单设备选择、自启/保持唤醒失败回退与重试、常规设置同步及映射、统计快照/累计计数、技术详情/Buffer/认证策略重检、日志开关/清空/报告/目录命令、连接/停止时控件禁用、Enter 密码重试与新会话归属、旧事件隔离、启动失败恢复、重复/迟到密码提交。未捕获浏览器异常也会失败。夹具没有定时协议事件，测试可暂停/完成/拒绝命令来确定异步顺序，不读取或改写组件内部状态。

Windows 默认使用已安装 Edge 的无头模式，无需下载浏览器；已有 Chrome 可设置 `$env:AIRPLAY_TEST_BROWSER_CHANNEL = 'chrome'` 后运行，完成后 `Remove-Item Env:AIRPLAY_TEST_BROWSER_CHANNEL`。其他系统默认使用 Playwright Chromium，先运行 `pnpm --dir airplay-frontend exec playwright install chromium`。浏览器选择依据见 [Playwright 文档](https://playwright.dev/docs/browsers)。

测试自己启动并回收 `127.0.0.1:4178` 的 Vite 服务与浏览器上下文，端口已占用则失败，不复用人工页面。每项最多 15 秒、整组最多 120 秒，不自动重试；失败截图/trace 和运行状态只写入忽略的 `test/.artifacts/frontend-dom/`，Vite 缓存位于 `.artifacts/frontend-vite-cache/`。`test:ui` 仍是人工模拟入口；`test:ui:build` 同时构建人工和 DOM 夹具页面。DOM 回归不等于真实 Tauri WebView、布局截图、IME、托盘或声卡/HomePod 测试。

来源组件提取后 DOM 共 10 项：新增分组自然排序/外部点击关闭，以及单声道选择重置为 `[0, 0]`、保存完成后才启动预览的回归。测试通过真实 `App.vue` 与 `SourcePicker.vue` 的按钮操作，只在桌面 IPC 边界暂停/完成命令。

设备卡片组件再新增 2 项，DOM 共 12 项：互换成功前保持原名称、成功后更新、失败保留顺序且不重启串流/改变输入映射；单设备只连接所选接收端、展示两个声道且没有互换按钮。点击经过真实 `DeviceCard.vue`，暂停/拒绝发生在 `set_speaker_order` IPC 边界。

常规设置组件再新增 3 项，DOM 共 15 项：自启等待时禁用及失败回退/重试；保持唤醒保存失败回退并保留其他设置；关闭动作、提前量、主题和映射同步，以及连接期间提前量禁用而映射仍发送原有命令、不重启会话。测试仅模拟桌面 IPC，不修改 Windows 自启或电源策略。

运行统计组件再新增 2 项，DOM 共 17 项：缺失/零值与精度、报告和下一会话快照重置；重复统计去重、切页保留、设备名称/地址回退、缺失列占位、重连后的累计值与旧事件隔离。会话脚本测试读取实际 `display.ts`，与页面使用相同数值格式化实现。

技术详情组件再新增 2 项，DOM 共 19 项，并扩展无来源/无接收端检查：设备/WASAPI 格式、电平与设备能力、Buffer 保存/启动参数/忙碌禁用、协议展示；认证策略重检命令等待/失败/成功反馈及所选设备范围。`source-level` 从模拟桌面事件注入，不修改组件状态；重检只检查请求和反馈，不操作真实认证记录。

日志组件再新增 3 项，DOM 共 22 项：开关独立保存/下次连接生效/忙碌禁用、打开目录失败/重试；脱敏文本、路径、报告展开、清空只影响诊断摘要、新摘要继续接收及切页保留；故障过滤、重连重置与旧日志/诊断/报告事件隔离。保留既有会话日志文件名收到新 `log_path` 后才更新的行为。`open_logs` 只模拟 IPC，不打开真实 Windows 目录或操作日志文件。

`test:dom` mounts the actual production UI in headless Edge on Windows (Chromium elsewhere), mocks desktop boundaries and checks twenty-two click/keyboard/lifecycle scenarios, including source grouping/dismissal, save-before-monitor ordering, successful/failed speaker swaps, individual receiver selection, settings rollback, runtime mapping, snapshot resets, cumulative statistics, technical details, auth-policy reset requests, log controls, diagnostic clearing, expandable reports and stale log isolation. It owns its local server/browser, fails on uncaught browser errors and writes only ignored failure artifacts; it does not exercise native WebView or audio hardware or open real log folders.

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
