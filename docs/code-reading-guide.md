# 代码阅读与修改指南

维护分支沿用现有命名和排版，代码注释使用中英双语，中文在前、英文在后，在同一注释块解释数据来源、参数单位、状态条件和设计原因。两种语言保持字段名、数值及单位一致，修改逻辑时同步更新。本指南串联关键入口；准确行为仍以入口实现和对应测试为准。

测试实现统一位于根目录 `test/`，按 `frontend/`、`backend/`、`core/` 分类。Rust 测试通过原模块的 `#[path]` 挂接，生产文件只保留挂接声明；运行入口和合成样例说明见 [测试说明](../test/README.md)。

## 建议阅读顺序

| 顺序 | 文件 | 阅读时要解决的问题 |
|---|---|---|
| 1 | `airplay-frontend/src/types.ts` | 页面接收什么数据？哪些数值是帧、毫秒、Hz 或 ppm？ |
| 2 | `airplay-frontend/src/protocol.ts` | 桌面 JSON 怎样从 `unknown` 变成可读取的数据？缺失或错误字段如何处理？ |
| 3 | `airplay-frontend/src/useStreamSession.ts`、`useDiagnostics.ts`、`App.vue` | 会话控制怎样独立于设备视图？事件怎样改变页面状态？ |
| 4 | `airplay-frontend/src-tauri/src/main.rs` | 命令怎样校验设备、来源和会话，并启动工作线程？ |
| 5 | `airplay-core/src/source.rs` | 谁持有持续采集线程？预览与串流如何共用它？ |
| 6 | `airplay-core/src/live.rs` | 采集、转换、管道、协议日志和最终报告怎样连接？ |
| 7 | `capture.rs`、`convert.rs`、`drift.rs` | 音频格式、资源释放、重采样状态和时钟校正为什么这样实现？ |
| 8 | 桌面的 `auth.rs`、`settings.rs`、`routing.rs`、`window.rs`，核心的 `privacy.rs` | 密码、配置、运行路由、退出和日志的边界在哪里？ |
| 9 | `airplay-backend/patches/README.md`、`upstream_guard.py`、`select_upstream.py` | 固定源码怎样校验、在临时副本应用补丁并选择编译？平台实现与协议修改分别在哪里？ |

## 从点击播放到实际串流

1. `useStreamSession/start` 调用页面快照重置，设置 `busy`，发送 `start_stream`。设置中的 `latency` 是播放提前量，`buffer` 是后端预缓冲时长；两者单位都是 ms。`mapping` 是来源声道下标，从 0 开始。
2. 桌面 `start_stream` 检查有没有活动会话或设备发现、设备名称是否唯一、两台设备是否同一个立体声对、端点是否存在以及映射是否越界。校验通过后保存偏好并创建会话控制和密码管道。
3. `ensure_source` 复用同端点的健康采集线程；来源改变时先结束旧线程。工作线程先发 `preparing_source`，再调用 `wait_ready`。连续采集至少稳定 500 ms、至少收到 3 包、最近一包不超过 250 ms 才算就绪；故障或长间隔会重置窗口，最长等待 8 秒。静音也可就绪。
4. `live::run_gui` 启动原生后端进行协议连接。`PCM_READY` 表示握手完成，页面进入启动采集阶段；遥测到达后才置 `playing` 并显示串流中。
5. `Source::consume` 挂接一个订阅者，交付交错 float32 立体声。`Converter` 跨音频包保存滤波器历史，转换为 44.1 kHz 的 i16。写管道线程将 PCM 发送给后端。
6. 遥测约每 250 ms 更新一次。控制器以实际启动水位为基准，按水位偏差调整转换比例。`correction_ppm` 是采样率微调量：水位偏高时减慢输出，不能用它表示音量。
7. 工作线程收尾生成脱敏报告、关闭密码管道、释放会话占用，最后发 `finished`；前端据此结束忙碌状态，并保留本次诊断数据。

命令返回和事件是独立异步通道，事件可能先于命令返回。每条串流事件带 `session_id`；页面先暂存最多 512 条事件，等启动命令返回确认本次编号后，按顺序处理匹配编号的事件。不能从第一条事件认领编号，否则旧事件可能冒充新会话。超量时明确请求停止并保留收尾事件；启动失败或卸载时丢弃暂存。`finished` 后不再处理该会话的迟到事件，停止期间不再打开密码框或更新播放状态。

播放端点另有原始音频启动检查：仅在首次稳定之前，端点电平大于 0.01、确认未静音且音量非零，但全部原始声道持续全零时，暂停就绪判定；矛盾持续 500 ms 后释放旧 WASAPI 客户端，再创建新客户端。尚未挂接串流时最多重建三次，每次重置采集计数与稳定窗口，并在 `source-startup.jsonl` 记录 `loopback_reopen`。连续 500 ms 无矛盾后，当前客户端永久结束这项启动检查，后续暂停/恢复不重新开启；电平和原始包不一致不能单独证明运行中的回环失败。正常安静来源也可完成检查。挂接订阅前仍核对就绪；已有串流不自动重建或重置时钟。

左侧采集开关关闭时会停止并等待旧 Source 退出，释放音频客户端、采集服务和事件句柄；开启时创建新 Source，重新初始化 WASAPI、包时间线和稳定窗口。同一端点的普通选择可以复用现有 Source，所以两种操作的效果并不相同。此健康检查把有证据的异常初始化恢复放到来源线程内，保持正常情况下的持续采集复用。

冷启动时页面开关和桌面的 `capture_enabled` 均为关闭，`Source` 尚未创建。页面先注册事件、读取设置并提交界面状态，再通过两次动画帧回调跨过首次绘制，通知 `ui_ready`，最后用与手动开启相同的 `set_capture_enabled(true)` 创建采集来源。没有有效来源时保持关闭，首次选择有效来源后再自动开启一次；此后手动关闭的状态在切换设备时保持。开启失败时允许手动重试；组件已卸载时取消自动开启。已运行的页面重新挂载会读取桌面真实状态，不会强制关闭现有采集。此顺序用于验证初始化时序假设，尚不能保证消除驱动或 WASAPI 回环异常。

设置操作先看 `App.vue/commitSettings`：它保存独立快照并持有保存锁，成功更新已确认值，失败恢复该值；保持唤醒不再反转当前布尔值。来源切换还持有跨保存/预览的锁，只有保存成功且页面仍存在才启动后续命令。桌面 `routing.rs` 保证候选设置保存成功后才更新运行路由，不能只从页面 IPC 拒绝测试推断桌面没有副作用。冷启动保存失败不自动采集，自动发现成功仍保留初始化错误。

保持唤醒的 Windows 实现见 `awake.rs/activate` 与 `Awake::set`：SystemRequired 后再获取 DisplayRequired，失败撤销，关闭请求对象时释放两类请求。采集时间戳问题从 `capture/timeline.rs/validate_packet_timestamp` 跟到 `capture/diagnostics.rs/Trace::fault` 和 Source 的诊断回调；后者把终止故障提交到有界异步日志，即使未开逐包诊断也能留下出错包数值。

`useDiagnostics.ts` 统一管理快照、300 条日志/120 条摘要、单条文本截断、1 MiB 报告显示限制及路径；结束摘要也计入上限。全部落盘规则见 [日志保留限制](log-retention.md)。

日志路径由初始化 `dataMode` 生成脱敏根目录。便携模式使用 `[程序目录]/data/logs`，安装模式使用 `%APPDATA%/AirPlay Hub/logs`；文件事件只附加文件名，空路径保持空值。实际打开目录仍使用桌面的 `Engine.root/logs`。

端点电平在端点音量调整前测量，因此不能仅凭电平非零触发恢复，需同时查询静音与音量。接口约定见 [微软 IAudioMeterInformation 说明](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudiometerinformation)。

播放回环的实机证据、VAIO 延迟对照和保存设置方法见 [播放回环排查](playback-loopback.md)。本次把 VAIO3 内部延迟从 768 调到 7168 后，独立采集 16 次及项目 Source 3 次均正常；完整 AirPlay 播放仍需实机复测。

## core 串流模块的职责（2026-10-09）

`airplay-core/src/live.rs` 现在只编排会话。GUI / CLI 仍从 `live::run_gui`、`live::run`、`live::run_stereo` 进入，公开的 `GuiContext`、`GuiControl` 和 `GuiEmitter` 通过原位置导出。内部模块只对串流实现可见，二次开发不需要修改调用方的导入路径。

| 文件 | 持有的状态与职责 | 修改时关注 |
|---|---|---|
| `live.rs` | 设备校验、连接参数、启动顺序、采集入口、收尾与最终报告 | 先认证再订阅；错误优先级；报告字段与脱敏 |
| `live/control.rs` | GUI 会话控制、来源引用、CLI Ctrl+C 守卫 | GUI 停止不销毁持续 Source；CLI 处理器随守卫注销 |
| `live/transport.rs` | 原生子进程、唯一 PCM 发送端、写线程、计数与单位常量 | 640 ms 时长预算（含写入中块）及 256 块硬上限；满时立即报错；缓冲回收；EOF 收尾 |
| `live/protocol.rs` | stderr 读线程、就绪通知、QPC 播放计划、成员统计、首个故障 | 原文用于控制，脱敏副本用于展示/落盘；`PCM_READY` 只通知一次 |
| `live/protocol_log.rs` | 协议控制台/文件输出、故障上下文 | 128 条非阻塞队列；丢日志不丢控制状态，日志错误单独报告 |
| `live/report.rs` | 最终错误优先级、脱敏报告落盘及 GUI 快照 | 写盘失败仍发内存报告；`report_write_error` 不覆盖协议/采集/管道首因；没有更早故障才返回 `REPORT_WRITE_FAILED` |
| `live/pipeline.rs` | 本次会话的重采样器、水位控制器、左右路由、遥测与管线诊断 | 滤波器跨包连续；帧/字节/ms 单位；遥测与控制更新时机 |
| `audio_queue.rs` | 两级队列共用的时长策略、预留与 RAII 归还 | 每级各 640 ms；时长按帧数/采样率计算；不能当作实际延迟 |
| `capture/timeline.rs` | 空闲静音、缺口修复与恢复重叠 | 纯时间线计划，不读取或释放 WASAPI；60 秒修复预算可确定性测试 |
| `capture/decode.rs` | PCM/float32 解码、帧裁剪、声道映射与原始包摘要 | 借用原始字节、复用输出；完整包检查不受映射影响；格式验证及 WASAPI 释放仍在 `capture.rs` |
| `capture/clock.rs`、`capture/health.rs` | Windows QPC、端点读取、健康检查和快照 | 原始字节检查在释放前，健康读取在释放后；查询失败不单独判故障 |
| `capture/state.rs`、`capture/diagnostics.rs` | 观察/交付计数，诊断及最终报告 JSON | 首包断续口径保留；sink 成功才提交交付数；日志暂停不推进诊断时间差 |
| `log_store.rs` | 日志文件上限、活动预留和目录回收 | 只删除识别的普通文件；规则见 [日志限制](log-retention.md) |
| `capture/metrics.rs` | 每包电平、信号帧、可选指纹与诊断时间差 | 只读样本；统计结果共用；非有限值/阈值/位指纹规则；时钟由调用者提供 |
| `live/diagnostics.rs` | 有界诊断队列、落盘线程、轮转与丢记录统计 | 容量不决定轮转策略；逐包日志保留 4 段，每段 8 MiB |

阅读顺序是 `live.rs` 的启动 → `pipeline.rs/process` 的音频处理 → `live.rs` 的停止和报告。遇到具体问题再进入协议、发送或诊断模块；不必先通读所有线程实现。

停止顺序有约束：正常采集结束后排出滤波器尾帧，再关闭唯一 PCM 发送端，让写线程发送已接受的队列内容并给后端 EOF。采集或发送已经失败时不再追加尾帧。等待后端退出后再等待读写线程；超过 15 秒先终止并回收后端。报告写入后，依次返回协议首因、采集错误、管道错误或后端退出错误，避免断管掩盖之前的协议故障。

启动日志、漂移日志和管线诊断都通过有界队列交给独立日志线程，打开文件、脱敏、序列化和轮转也不在采集回调执行。漂移写盘错误不再传播为音频错误；CLI 摘要由日志线程输出，GUI 不重复写控制台。协议文件在启动时打开，读线程先解析原始状态和通知就绪，再向独立日志线程提交脱敏副本，文件/控制台写入不在读线程执行；GUI 原始控制事件保持独立投递。

`diagnostics.rs` 的 `LogWorker<T>` 共用非阻塞入队、丢失计数和收尾机制；`DetailLog` 继续表示 JSON 诊断，协议日志使用专门记录类型。Source 日志收尾最多等待 250 ms；会话在回收后端及读写线程后，给协议及各诊断日志共用 250 ms 截止时间。完整串流报告增加 `protocol_log_status`，各日志状态包含 `dropped_records`、`pending_records`、`timed_out` 和 `error`；pending 是尚未完成处理记录的快照，不代表磁盘持久化保证。协议日志溢出/写入失败可能丢展示文本及故障上下文，原始控制状态继续处理。超时不会取消底层 I/O，工作线程可能继续驻留；若恢复则不再处理积压。15 秒只限制后端退出等待，stderr 读取、GUI 回调、协议读线程收尾和最终报告写盘仍不是整个应用停止的硬期限。

`test/core/unit/live/` 中的单元测试覆盖队列满/断开时的计数回滚、已入队 PCM 在 EOF 前排空、断管报错、不同音频包大小下的声道与重采样总时长，以及协议状态与日志脱敏。执行 `cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib`；需要声卡或真实接收设备的测试仍单独忽略，编译和这些单元测试不能代替实机听感验证。

## 页面状态与密码重试

`useStreamSession.ts` 持有会话命令、事件暂存、编号确认、停止和密码重试。`App.vue` 通过回调重置或渲染展示快照，不从展示日志认领会话；页面卸载时 composable 使未完成命令失效并清空密码。

来源选择视图从 `SourcePicker.vue` 阅读：分组/排序和弹窗展示在组件内，`select` 事件交回 `App/selectSource`。页面设置 endpoint、关闭弹窗，随后 `sourceChanged` 重置声道映射、保存设置并启动预览；组件不拥有桌面状态。`open` 是页面控制的状态，因此切页、连接和密码请求仍可统一关闭弹窗，遮罩层级与既有样式保留。

设备卡片从 `DeviceCard.vue` 阅读：组件接收 `DeviceCardData` 及选中/展开/禁用/连接等展示值，选择事件交给 `App/choose`，互换事件交给 `App/swap`。分组、选择与会话仍由页面管理；`set_speaker_order` 成功后才更新 `speakersSwapped`，失败显示错误并保留现有左右名称。单设备展示两个声道但没有互换按钮；立体声卡片保留原有互换功能，不改音频处理。

常规设置从 `GeneralSettings.vue` 阅读：控件先通过 `update:*` 事件更新页面持有的设置/自启/主题，再发送保存、自启、唤醒或映射操作事件。设置字段与二元映射生成新对象/数组，不直接修改 props；`App/startupChanged` 和 `awakeChanged` 保留失败回退，主题 watcher 保留样式与 localStorage 同步。连接期间播放提前量禁用，声道映射仍调用 `set_mapping`，不重新启动会话。

运行统计从 `RuntimeStats.vue` 阅读：组件只读遥测、报告、累计计数与名称，`display.ts/num` 统一数值格式。`App/renderSessionEvent` 把已验证且归属本次会话的事件交给 `useDiagnostics/accept`；后者从原始 `PACKET_STATS` 解析累计值，与每设备会话基线比较后将非负增量加到 `stats`。缺失/非法/负值不覆盖有效基线，设备历史最多 128 项。`resetSessionView` 只清空本次基线及遥测/报告，累计 `stats` 保留；切换标签不改变数据。事件仍由 `useStreamSession` 校验归属，因此旧会话计数不能进入当前统计。

技术详情从 `TechnicalDetails.vue` 阅读：格式和电平使用 `display.ts/formatAudioFormat`、`level`，与底部来源栏/设备卡片共用；Buffer 先发更新事件，保存事件交给 `App/persist`。重检事件交给 `App/resetAuth`，页面把当前 `selection` 传给 `forget_auth_policy`；命令成功后才显示已清除通知，失败进入原有错误提示。没有所选设备或会话忙碌时按钮禁用，组件不修改设备、认证记录或音频。

日志视图从 `LogView.vue` 阅读：开关先发 `update:*` 更新页面设置，再发送保存事件，配置下次连接；当前会话的详细日志/诊断状态由独立快照决定。打开目录事件交给 `App/call('open_logs')`，“清空显示”事件只将 `diagnosticLogs` 置空；日志、路径及报告保留，后续诊断继续接收。完整报告在 `report.device` 存在时显示可展开 JSON。组件不拥有日志数组、IPC 或会话生命周期；页面和 `useStreamSession` 过滤事件及会话归属；`useDiagnostics` 统一截断、汇总和重连快照重置。

| 状态 | 设置条件 | 清除条件 |
|---|---|---|
| `busy` | 用户请求开始连接 | 启动命令失败或 `finished` |
| `connected` | 原生行出现 `PCM_READY` | 新会话开始或 `finished` |
| `playing` | 本次会话收到遥测 | `finished` |
| `stopping` | 用户请求停止 | `finished`；期间忽略遥测 |
| `pending` | `password_required` 提供待认证主机 | 提交完成、停止或收尾；密码失败时可恢复重试目标 |

密码提交后立即清空响应式输入。认证失败后旧会话已经结束，重试必须先创建新会话，等待它再次发出 `password_required`，然后发送密码。桌面管道只接受本次设备地址，`Reply` 销毁时清理其持有的密码字节；这不能保证清除 WebView、IPC 或系统内全部副本。

`test/frontend/dom/session.spec.mjs` 通过 `test:dom` 在真实浏览器挂载生产入口和模板，模拟 Tauri 边界，检查按钮 disabled、点击、Enter/Escape、密码重试与旧会话隔离。`ui/dom.mjs` 只提供命令记录、暂停/完成/拒绝命令和事件注入，不访问组件状态；现有脚本级 10 项会话回归继续覆盖事件积压、卸载等竞态。DOM 测试配置、浏览器和失败产物见 [测试说明](../test/README.md)。

## 停止、关闭采集与退出的区别

- **停止播放**：设置串流停止标志、解除订阅并关闭后端，保留持续采集线程，页面继续显示音量预览。
- **关闭采集 / 切换来源**：桌面端结束来源线程；有活动会话时拒绝关闭采集或更换来源。声道映射变更可以通过独立控制命令处理。
- **隐藏到托盘**：只隐藏窗口，按当前设置继续运行。
- **退出应用**：只进入一次退出流程；停止串流、解除防休眠、结束采集，等待会话清理后退出。

采集线程不等待磁盘或网络。音频订阅队列满会报告故障，诊断队列满允许丢记录并计数。不要为“保证不丢日志”把磁盘写入放回采集回调，否则会引入音频卡顿。

## 修改入口与同步要求

| 修改目标 | 主要入口 | 需要同步检查 |
|---|---|---|
| 新增设置 | `types.ts`、`protocol.ts`、桌面 `settings.rs`、页面控件 | 默认值、单位、范围、旧文件缺字段时的行为 |
| 新增桌面事件 | Rust 事件发送点、`types.ts/StreamEvent`、`protocol.ts`、`useStreamSession/event`、`App.vue/renderSessionEvent` | 会话编号、必填字段、已知事件损坏时的报错、协议测试 |
| 新增报告字段 | `live.rs`、`privacy.rs`；页面显示字段补充类型及校验 | 日志脱敏、扩展 JSON 保留、数值单位 |
| 修改来源预览 | `source.rs`、桌面 `ensure_source`、页面 `source-level` 监听 | 端点过滤、静音就绪、取消、线程复用 |
| 修改采样或水位控制 | `capture.rs`、`convert.rs`、`drift.rs`、`live/pipeline.rs` | 帧与采样区别、包边界、尾帧、缓冲积压及实机听感 |
| 修改日志 | `privacy.rs` 和各落盘入口 | 保留内部控制原值，只展示/保存脱敏副本 |
| 修改原生协议 | `airplay-backend/patches/`、本地入口 `.inc` | 补丁上下文与顺序、固定提交、输入/补丁哈希、生成差异及接收端模拟 |
| 修改 Windows 兼容 | `windows_port.c/.h`、`windows_io.inc`、`windows_audio.inc`、`probe_context.inc` | socket 宽度、错误来源、超时与关闭；不要把源码替换放回 Python |

`protocol.ts` 接收 `unknown`，验证有限数值、整数计数、左右两个声道、设置范围和事件所需字段。报告可保留扩展 JSON，页面消费的字段继续检查；未知事件忽略，已知事件格式错误会显示字段路径，避免打印输入内容。此处负责格式，不替代 Rust 的设备和权限校验。

报告使用 `shallowRef` 并整份替换，不修改内部字段，避免为可扩展 JSON 创建深层响应式代理。新增交互字段需要单独状态，不要直接修改报告。

## TypeScript 版本选择（2026-10-09）

此前 5.9 来自旧锁文件，上一轮维护没有完成升级核查。TypeScript 7.0.2 已发布，但不能仅凭版本号直接替换 Vue 的编译工具链。

在独立目录安装 `typescript@7.0.2` 和当前锁定的 `vue-tsc@3.3.12`，对项目配置执行类型检查，得到 `ERR_PACKAGE_PATH_NOT_EXPORTED`：`typescript/lib/tsc` 不再导出。当前 Vue 检查器依赖编译器接口，而 TypeScript 7 的原生编译器尚未提供对应 API。微软为这类集成推荐 TypeScript 6 兼容包，见 [官方 TypeScript 7 发布说明](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)。

维护分支采用 `typescript: npm:@typescript/typescript6@^6.0.2`，锁文件固定兼容包 6.0.2 及其实际编译器 6.0.3。当前 Vue 类型检查、生产构建和数据边界测试已经通过。后续升级 7 时先验证 `.vue` 模板和脚本检查、编译器 API 使用及生产构建，再更新锁文件；单独执行原生 `tsc` 不能代替 Vue 模板检查。

## 验证

在仓库根目录执行：

```powershell
pnpm --dir airplay-frontend test:protocol
pnpm --dir airplay-frontend build
pnpm --dir airplay-frontend format:check
```

协议测试覆盖现有页面样例、已知事件、异常字段和数值、可空报告、扩展字段、嵌套限制和错误信息不泄露输入值。它不替代实机采集和 AirPlay 播放验证；其他检查见 [贡献指南](../CONTRIBUTING.md)。

## Source 缓冲与状态阅读入口

`source.rs/Subscription::send_audio` 在时长预算接受后调用 `source/buffers.rs/Pool::copy`，仍复制完整输入。`Source::consume` 在 sink 借用结束后归还当前 Vec，解除订阅释放池和剩余块。先读 [Source 测量与限制](source-performance.md)，再看 `test/core/unit/source/buffers.rs` 和 `source.rs` 的跨线程回归；新建/复用/扩容计数不等于全进程分配次数。多个会话布尔值和各层锁的语义见[状态说明](session-state.md)，本轮没有改变生产状态机。
