# 代码阅读与修改指南

维护分支沿用现有命名和排版，代码注释使用中英双语，中文在前、英文在后，在同一注释块解释数据来源、参数单位、状态条件和设计原因。两种语言保持字段名、数值及单位一致，修改逻辑时同步更新。本指南串联关键入口；准确行为仍以入口实现和对应测试为准。

测试实现统一位于根目录 `test/`，按 `frontend/`、`backend/`、`core/` 分类。Rust 测试通过原模块的 `#[path]` 挂接，生产文件只保留挂接声明；运行入口和合成样例说明见 [测试说明](../test/README.md)。

## 建议阅读顺序

| 顺序 | 文件                                                                            | 阅读时要解决的问题                                                             |
| ---- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| 1    | `airplay-frontend/src-ely/src/player.rs`                                     | 界面持有哪些状态？后台作业、设备发现和采集怎样调度？                           |
| 2    | `session.rs`、`inbox.rs`、`diagnostics.rs`（同目录）                            | 会话编号怎样隔离迟到事件？后台事件怎样合并、限量后进入界面？                   |
| 3    | `view.rs`、`settings_view.rs`、`panel.rs`（同目录）                             | 播放页和设置页怎样由状态绘制？控件事件交回 `Player` 的哪个方法？               |
| 4    | 桌面 `main.rs`、`session.rs`、`stream_start.rs`、`configuration.rs`             | 怎样预留会话、后台准备、取消与发布？配置如何互斥？                             |
| 5    | `airplay-core/src/source.rs`                                                    | 谁持有持续采集线程？预览与串流如何共用它？                                     |
| 6    | `airplay-core/src/live.rs`                                                      | 采集、转换、管道、协议日志和最终报告怎样连接？                                 |
| 7    | `capture.rs`、`convert.rs`、`drift.rs`                                          | 音频格式、资源释放、重采样状态和时钟校正为什么这样实现？                       |
| 8    | 桌面的 `auth.rs`、`settings.rs`、`routing.rs`，界面的 `shell.rs`，核心的 `privacy.rs` | 密码、配置、运行路由、退出和日志的边界在哪里？                                 |
| 9    | `airplay-backend/patches/README.md`、`upstream_guard.py`、`select_upstream.py`  | 固定源码怎样校验、在临时副本应用补丁并选择编译？平台实现与协议修改分别在哪里？ |

## 从点击播放到实际串流

1. `Player::start` 重置本次诊断快照，在后台作业中调用桌面 `start_stream`。设置中的 `latency` 是播放提前量，`buffer` 是后端预缓冲时长；两者单位都是 ms。`mapping` 是来源声道下标，从 0 开始。
2. 桌面 `start_stream` 在短锁内预留会话编号并返回；后台检查设备名称、立体声关系、端点及声道，再保存偏好、创建密码服务。Preparing 使用 15 秒预算；慢步骤前后核对取消，超时保留占用，见[启动取消与提交](architecture.md#启动取消与设置提交)。
3. 工作线程先发 `preparing_source`。`ensure_source` 复用同端点的健康采集线程；来源改变时先结束旧线程。新建 Source 先由本次准备持有，调用 `wait_ready` 后核对取消，再发布 Source/密码通道和 Active 阶段。连续采集至少稳定 500 ms、至少收到 3 包、最近一包不超过 250 ms 才算就绪；故障或长间隔会重置窗口，最长等待 8 秒。静音也可就绪。
4. `live::run_gui` 启动原生后端进行协议连接。`PCM_READY` 表示握手完成，会话进入 `Ready`；遥测到达后才进入 `Playing` 并显示串流中。
5. `Source::consume` 挂接一个订阅者，交付交错 float32 立体声。`Converter` 跨音频包保存滤波器历史，转换为 44.1 kHz 的 i16。写管道线程将 PCM 发送给后端。
6. 遥测约每 250 ms 更新一次。控制器以实际启动水位为基准，按水位偏差调整转换比例。`correction_ppm` 是采样率微调量：水位偏高时减慢输出，不能用它表示音量。
7. 工作线程收尾生成脱敏报告、关闭密码管道、释放会话占用，最后发 `finished`；界面据此结束忙碌状态，并保留本次诊断数据。

界面与桌面服务同进程，`start_stream` 同步返回本次会话编号，`Session::start` 先记下编号再处理事件。每条串流事件带 `session_id`，`Session::accept` 只接受当前编号；`finished` 后迟到的同编号事件也被丢弃，停止期间不再打开密码框或更新播放状态。`inbox.rs` 合并高频电平与遥测，只保留最新快照；控制事件队列上限为 32 条，并为收尾事件保留独立位置。

播放端点另有原始音频启动检查：仅在首次稳定之前，端点电平大于 0.01、确认未静音且音量非零，但全部原始声道持续全零时，暂停就绪判定；矛盾持续 500 ms 后释放旧 WASAPI 客户端，再创建新客户端。尚未挂接串流时最多重建三次，每次重置采集计数与稳定窗口，并在 `source-startup.jsonl` 记录 `loopback_reopen`。连续 500 ms 无矛盾后，当前客户端永久结束这项启动检查，后续暂停/恢复不重新开启；电平和原始包不一致不能单独证明运行中的回环失败。正常安静来源也可完成检查。挂接订阅前仍核对就绪；已有串流不自动重建或重置时钟。

左侧采集开关关闭时会停止并等待旧 Source 退出，释放音频客户端、采集服务和事件句柄；开启时创建新 Source，重新初始化 WASAPI、包时间线和稳定窗口。同一端点的普通选择可以复用现有 Source，所以两种操作的效果并不相同。此健康检查把有证据的异常初始化恢复放到来源线程内，保持正常情况下的持续采集复用。

冷启动时 `Player::initialize` 读取设置后同时开始设备发现和来源预览。发现期间只锁定设备卡片和开始串流按钮，其他控件照常可用。没有串流时，窗口隐藏到托盘、最小化或显示器熄灭会停止采集，窗口重新可见后由 `resume_capture` 重建，最多重试 3 次、间隔 2 秒；串流中不受影响。判断逻辑见 `visibility.rs/next_step`。

设置保存在后台作业中执行，成功后才更新界面持有的已确认值，失败时恢复。桌面 `routing.rs` 保证候选设置保存成功后才更新运行路由。

保持唤醒的 Windows 实现见 `awake.rs/activate` 与 `Awake::set`：SystemRequired 后再获取 DisplayRequired，失败撤销，关闭请求对象时释放两类请求。采集时间戳问题从 `capture/timeline.rs/validate_packet_timestamp` 跟到 `capture/diagnostics.rs/Trace::fault` 和 Source 的诊断回调；后者把终止故障提交到有界异步日志，即使未开逐包诊断也能留下出错包数值。

`diagnostics.rs` 统一管理快照、300 条日志/120 条摘要、单条文本 4096 字符截断、1 MiB 报告显示限制及路径；结束摘要也计入上限，设备累计统计最多 128 项。全部落盘规则见 [日志保留限制](log-retention.md)。

日志路径由初始化 `dataMode` 生成脱敏根目录。便携模式使用 `[程序目录]/data/logs`，安装模式使用 `%APPDATA%/AirPlay Hub/logs`；文件事件只附加文件名，空路径保持空值。实际打开目录仍使用桌面的 `Engine.root/logs`。

端点电平在端点音量调整前测量，因此不能仅凭电平非零触发恢复，需同时查询静音与音量。接口约定见 [微软 IAudioMeterInformation 说明](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudiometerinformation)。

播放回环的实机证据、VAIO 延迟对照和保存设置方法见 [播放回环排查](playback-loopback.md)。本次把 VAIO3 内部延迟从 768 调到 7168 后，独立采集 16 次及项目 Source 3 次均正常；完整 AirPlay 播放仍需实机复测。

## core 串流模块的职责（2026-10-09）

`airplay-core/src/live.rs` 现在只编排会话。GUI / CLI 仍从 `live::run_gui`、`live::run`、`live::run_stereo` 进入，公开的 `GuiContext`、`GuiControl` 和 `GuiEmitter` 通过原位置导出。内部模块只对串流实现可见，二次开发不需要修改调用方的导入路径。

| 文件                                         | 持有的状态与职责                                          | 修改时关注                                                                                                    |
| -------------------------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `live.rs`                                    | 设备校验、连接参数、启动顺序、采集入口、收尾与最终报告    | 先认证再订阅；错误优先级；报告字段与脱敏                                                                      |
| `live/control.rs`                            | GUI 会话控制、来源引用、CLI Ctrl+C 守卫                   | GUI 停止不销毁持续 Source；CLI 处理器随守卫注销                                                               |
| `live/transport.rs`                          | 原生子进程、唯一 PCM 发送端、写线程、计数与单位常量       | 640 ms 时长预算（含写入中块）及 256 块硬上限；满时立即报错；缓冲回收；EOF 收尾                                |
| `live/protocol.rs`                           | stderr 读线程、就绪通知、QPC 播放计划、成员统计、首个故障 | 原文用于控制，脱敏副本用于展示/落盘；`PCM_READY` 只通知一次                                                   |
| `live/protocol_log.rs`                       | 协议控制台/文件输出、故障上下文                           | 128 条非阻塞队列；丢日志不丢控制状态，日志错误单独报告                                                        |
| `live/report.rs`                             | 最终错误优先级、脱敏报告落盘及 GUI 快照                   | 写盘失败仍发内存报告；`report_write_error` 不覆盖协议/采集/管道首因；没有更早故障才返回 `REPORT_WRITE_FAILED` |
| `live/pipeline.rs`                           | 本次会话的重采样器、水位控制器、左右路由、遥测与管线诊断  | 滤波器跨包连续；帧/字节/ms 单位；遥测与控制更新时机                                                           |
| `audio_queue.rs`                             | 两级队列共用的时长策略、预留与 RAII 归还                  | 每级各 640 ms；时长按帧数/采样率计算；不能当作实际延迟                                                        |
| `capture/timeline.rs`                        | 空闲静音、缺口修复与恢复重叠                              | 纯时间线计划，不读取或释放 WASAPI；60 秒修复预算可确定性测试                                                  |
| `capture/decode.rs`                          | PCM/float32 解码、帧裁剪、声道映射与原始包摘要            | 借用原始字节、复用输出；完整包检查不受映射影响；格式验证及 WASAPI 释放仍在 `capture.rs`                       |
| `capture/clock.rs`、`capture/health.rs`      | Windows QPC、端点读取、健康检查和快照                     | 原始字节检查在释放前，健康读取在释放后；查询失败不单独判故障                                                  |
| `capture/state.rs`、`capture/diagnostics.rs` | 观察/交付计数，诊断及最终报告 JSON                        | 首包断续口径保留；sink 成功才提交交付数；日志暂停不推进诊断时间差                                             |
| `log_store.rs`                               | 日志文件上限、活动预留和目录回收                          | 只删除识别的普通文件；规则见 [日志限制](log-retention.md)                                                     |
| `capture/metrics.rs`                         | 每包电平、信号帧、可选指纹与诊断时间差                    | 只读样本；统计结果共用；非有限值/阈值/位指纹规则；时钟由调用者提供                                            |
| `live/diagnostics.rs`                        | 有界诊断队列、落盘线程、轮转与丢记录统计                  | 容量不决定轮转策略；逐包日志保留 4 段，每段 8 MiB                                                             |

阅读顺序是 `live.rs` 的启动 → `pipeline.rs/process` 的音频处理 → `live.rs` 的停止和报告。遇到具体问题再进入协议、发送或诊断模块；不必先通读所有线程实现。

停止顺序有约束：正常采集结束后排出滤波器尾帧，再关闭唯一 PCM 发送端，让写线程发送已接受的队列内容并给后端 EOF。采集或发送已经失败时不再追加尾帧。等待后端退出后再等待读写线程；超过 15 秒先终止并回收后端。报告写入后，依次返回协议首因、采集错误、管道错误或后端退出错误，避免断管掩盖之前的协议故障。

启动日志、漂移日志和管线诊断都通过有界队列交给独立日志线程，打开文件、脱敏、序列化和轮转也不在采集回调执行。漂移写盘错误不再传播为音频错误；CLI 摘要由日志线程输出，GUI 不重复写控制台。协议文件在启动时打开，读线程先解析原始状态和通知就绪，再向独立日志线程提交脱敏副本，文件/控制台写入不在读线程执行；GUI 原始控制事件保持独立投递。

`diagnostics.rs` 的 `LogWorker<T>` 共用非阻塞入队、丢失计数和收尾机制；`DetailLog` 表示 JSON 诊断，协议日志使用专门记录类型。Source 日志最多等待 250 ms；会话各日志共用 250 ms 且受总停止期限约束。`protocol_log_status` 等状态保留 `dropped_records`、`pending_records`、`timed_out` 和 `error`；pending 不代表磁盘持久化保证。日志溢出可能丢展示文本，原始协议状态继续处理；超时不能取消底层 I/O。统一预算、通知/认证策略线程和超时占用见[停止错误与日志](architecture.md#停止错误与日志)。

`test/core/unit/live/` 中的单元测试覆盖队列满/断开时的计数回滚、已入队 PCM 在 EOF 前排空、断管报错、不同音频包大小下的声道与重采样总时长，以及协议状态与日志脱敏。执行 `cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib`；需要声卡或真实接收设备的测试仍单独忽略，编译和这些单元测试不能代替实机听感验证。

## 界面状态与密码重试

界面只有一个状态持有者 `Player`（`airplay-frontend/src-ely/src/player.rs`）。`view.rs` 和 `settings_view.rs` 是只读状态的绘制函数，控件回调调用 `Player` 的方法，例如 `choose_receiver`、`select_source`、`swap_speakers`、`save_settings`、`set_mapping`、`change_autostart`、`reset_auth`。耗时操作都通过 `run_job` 交给后台作业线程，完成后在界面线程更新状态。

| 状态（`session::Phase`） | 进入条件                           | 离开条件                                       |
| ------------------------ | ---------------------------------- | ---------------------------------------------- |
| `Connecting`、`Preparing` | 用户开始串流                       | 握手完成、需要密码、失败或停止                 |
| `Password`、`Verifying`  | `password_required` / 提交密码     | 验证完成、停止或收尾；密码失败时可重试         |
| `Ready`                  | 原生行出现 `PCM_READY`             | 收到遥测或收尾                                 |
| `Playing`                | 本次会话收到遥测                   | 停止或 `finished`                              |
| `Stopping`               | 用户请求停止                       | `finished`；期间忽略遥测                       |

密码提交后立即清空输入。认证失败后旧会话已经结束，重试先创建新会话，等待它再次发出 `password_required` 后发送密码。桌面管道只接受本次设备地址，`Reply` 销毁时清理其持有的密码字节；这不能保证清除系统内全部副本。

`test/frontend/desktop/gpui_session.rs`、`gpui_inbox.rs`、`gpui_diagnostics.rs` 直接调用上述模块，覆盖迟到事件、事件合并与上限、诊断截断和累计统计。

## 停止、关闭采集与退出的区别

- **停止播放**：设置串流停止标志、解除订阅并关闭后端，保留持续采集线程，界面继续显示音量预览。
- **关闭采集 / 切换来源**：桌面端结束来源线程；有活动会话时拒绝关闭采集或更换来源。声道映射变更可以通过独立控制命令处理。
- **隐藏到托盘**：只隐藏窗口，按当前设置继续运行。
- **退出应用**：只进入一次退出流程；停止串流、解除防休眠、结束采集，等待会话清理后退出。

采集线程不等待磁盘或网络。音频订阅队列满会报告故障，诊断队列满允许丢记录并计数。不要为“保证不丢日志”把磁盘写入放回采集回调，否则会引入音频卡顿。

## 修改入口与同步要求

| 修改目标           | 主要入口                                                                                                       | 需要同步检查                                                    |
| ------------------ | -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| 新增设置           | 桌面 `settings.rs`、`Player` 的保存方法、`settings_view.rs` 控件                                               | 默认值、单位、范围、旧文件缺字段时的行为                        |
| 新增桌面事件       | Rust 事件发送点、`inbox.rs` 的事件解析、`Player::accept_stream`                                                | 会话编号、必填字段、已知事件损坏时的报错、收件箱测试            |
| 新增报告字段       | `live.rs`、`privacy.rs`；界面展示字段同步 `diagnostics.rs`                                                            | 日志脱敏、扩展 JSON 保留、数值单位                              |
| 修改来源预览       | `source.rs`、桌面 `ensure_source`、`Player::preview` 与电平快照                                                | 端点过滤、静音就绪、取消、线程复用                              |
| 修改采样或水位控制 | `capture.rs`、`convert.rs`、`drift.rs`、`live/pipeline.rs`                                                     | 帧与采样区别、包边界、尾帧、缓冲积压及实机听感                  |
| 修改日志           | `privacy.rs` 和各落盘入口                                                                                      | 保留内部控制原值，只展示/保存脱敏副本                           |
| 修改原生协议       | `airplay-backend/patches/`、本地入口 `.inc`                                                                    | 补丁上下文与顺序、固定提交、输入/补丁哈希、生成差异及接收端模拟 |
| 修改 Windows 兼容  | `windows_port.c/.h`、`windows_io.inc`、`windows_audio.inc`、`probe_context.inc`                                | socket 宽度、错误来源、超时与关闭；不要把源码替换放回 Python    |

`inbox.rs/StreamUpdate::decode` 只取界面需要的字段；未知事件或缺少必填字段的事件不进入会话状态，诊断内容另由 `diagnostics::decode` 提取。控制事件超过 32 条时记下溢出会话，由界面停止该会话；`finished` 单独保存，不会因队列满而丢失。此处负责格式，不替代 Rust 的设备和权限校验。

## 验证

在仓库根目录执行：

```powershell
cargo test --manifest-path airplay-frontend/src-ely/Cargo.toml --locked --offline
cargo fmt --manifest-path airplay-frontend/src-ely/Cargo.toml --check
```

这些测试覆盖会话、事件、诊断、音量、滚动和窗口可见性逻辑，不替代实机采集和 AirPlay 播放验证；其他检查见 [贡献指南](../CONTRIBUTING.md)。

## Source 缓冲与状态阅读入口

`source.rs/Subscription::send_audio` 在时长预算接受后调用 `source/buffers.rs/Pool::copy`，仍复制完整输入。`Source::consume` 在 sink 借用结束后归还当前 Vec，解除订阅释放池和剩余块。先读[性能基线](performance-baseline.md)与[音频完整性限制](architecture.md#音频完整性与缓冲)，再看缓冲池及 Source 跨线程测试；新建/复用/扩容计数不等于全进程分配次数。各层状态和锁的语义见[状态与所有权](architecture.md#状态与所有权)，准备/配置互斥见[启动取消与提交](architecture.md#启动取消与设置提交)。
