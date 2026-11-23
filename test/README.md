# 测试 / Tests

项目测试源码统一放在根目录 `test/`，随 Git 提交。生产目录只保留测试模块挂接及诊断入口；已有测试的私有模块访问关系和忽略标记保持不变。

Test sources are committed under root `test/`. Production modules retain attachment/diagnostic entry points; existing private-module access and ignored hardware checks are preserved.

核心子模块测试归入 `core/unit/capture/`、`live/`、`source/`；后端定位测试为 `core/unit/backend.rs`。挂接统一位于生产模块尾部。当前实现及资源边界见[架构](../docs/architecture.md)。

会话测试覆盖阻塞准备时取消、迟到编号、配置互斥及真实设置写入的取消/失败；界面会话测试检查迟到事件隔离。停止测试覆盖 stderr 原始 Win32 错误、统一期限、本地进程/管道、资源保留和停止后的真实错误。

Source/会话通知测试使用阻塞回调，检查最新快照、设备统计、控制和终态顺序，以及容量/字节限制、异常和超时保留。认证策略测试覆盖去重、慢写入、保存失败后重试和超时禁止新会话。

Source 缓冲测试检查位值、所有权、扩容/回收上限、非法或超预算包及跨线程变长包/采样率。显式 Release 合成基线使用真实 send_audio 路径；方法与产物限制见[性能基线](../docs/performance-baseline.md)。

保持唤醒测试检查系统/显示器请求顺序、失败撤销和实际句柄释放；它不等待真实熄屏。采集时间戳有核心回归，适用边界见[熄屏排查](../docs/display-off-capture.md)。

本版实际执行结果见[交付简报](../docs/release-v1.0.3.md)。本页维护测试范围和运行方法，逐轮执行记录从 Git 历史查阅。

`frontend/desktop/routing.rs` 的 2 项回归调用生产路由模块，在暂存设置文件路径创建目录触发真实写入失败，检查磁盘、内存和活动 Mutex/AtomicBool 不变，解除故障后可重试。它们没有创建 WASAPI Source 或完整桌面会话。

`core/unit/live/report.rs` 的 2 项回归调用生产结果选择/报告模块，以目录占用 JSON 目标路径，验证协议、采集、管道、传输/退出及仅报告失败的优先级；即使无法落盘仍收到包含 `report_write_error` 的内存报告。可写报告与 GUI 快照一致，已有故障仍保留；报告失败使用 `REPORT_WRITE_FAILED` 并保留操作和 OS 错误来源。

`core/unit/live/protocol.rs` 使用实际协议读线程和可注入 writer，分别阻塞/失败文件与控制台输出，验证队列溢出时就绪、认证错误及首个故障仍被处理、取消不等待日志、收尾超时后不输出积压，并检查详细/故障日志脱敏及密码提示。夹具在失败时也解除故意阻塞，无需声卡或接收设备。完整串流报告中的 `protocol_log_status` 单独表示日志健康；没有据此断言整个应用有严格停止上限。

`core/unit/capture/decode.rs` 的 8 项测试直接调用生产解码模块，用固定字节验证 PCM 8/16/24/32 位、24-in-32、float32 特殊值、默认/重复/多声道映射、帧裁剪与非法输入，检查跨包输出缓冲复用及完整原始包摘要。无需声卡或模拟接收端；不验证 WASAPI 驱动或实际播放。可单独运行 `cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib capture::decode::`。

`core/unit/capture/metrics.rs` 的 6 项测试调用纯统计/时序模块，覆盖电平与信号帧阈值、NaN/Infinity、固定指纹与负零/NaN 位差、可选指纹、空/不完整帧、首条及跨日志暂停的诊断间隔、有符号回退及 100 ns 单位换算。无需设备、不改写音频；运行过滤器为 `capture::metrics::`。Windows 时钟读取及实际驱动时序不在此纯逻辑测试范围内。

| 目录           | 内容                                                                            |
| -------------- | ------------------------------------------------------------------------------- |
| `frontend/desktop/` | 桌面服务与 GPUI 界面逻辑的 Rust 测试                                       |
| `backend/`     | 本机模拟接收端、认证、密码管道、立体声、延迟 SETUP 和原生 nonce 检查            |
| `core/unit/`   | 采集解码/统计/时间线/健康状态、转换、漂移、来源生命周期、串流管线与脱敏单元测试 |
| `core/checks/` | 已采集 WAV 与转换 PCM 的独立校验脚本                                            |
| `.artifacts/`  | 截图、模拟 PCM 和检查输出；不提交 Git                                           |

`frontend/desktop/gpui_diagnostics.rs` 直接调用 GPUI 的诊断批次和汇总，覆盖日志与摘要容量、UTF-16 截断、报告大小、迟到报告隔离、清空范围，以及设备统计跨会话累计。

`frontend/desktop/gpui_jobs.rs` 在 MTA 调用线程提交作业，验证独立作业线程仍能初始化 WASAPI 所需的 STA。

`core/unit/capture/{clock,state,diagnostics}.rs` 覆盖 QPC 换算、首包断续、观察与成功交付计数、可选进度、诊断暂停时间差、端点字段、sink 错误和最终 JSON 字段；无需声卡。`core/unit/log_store.rs` 用真实临时文件验证容量、目录保留、活动预留、未知文件/目录保护及 Windows 打开句柄。Source 临界轮转及超大报告首因回归在 `core/unit/live/`。完整保留规则见 [日志限制](../docs/log-retention.md)。

## 运行 / Run

以下命令在项目根目录执行。Rust 测试无需原生后端或声卡。原生/Python 协议检查先按 [构建指南](../docs/building.md) 构建后端；Python 另需 `backend/requirements.txt` 中的依赖。

Run commands from the project root. Rust tests need no backend or audio hardware; native/Python receiver checks require the backend and Python requirements.

```powershell
# Rust 单元测试 / Rust unit tests
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib
cargo test --manifest-path airplay-desktop/Cargo.toml --locked --offline
cargo test --manifest-path airplay-frontend/src-ely/Cargo.toml --locked --offline

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
