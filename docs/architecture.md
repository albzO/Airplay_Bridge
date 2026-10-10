# 架构

## 模块

| 目录 | 职责 |
|---|---|
| `airplay-frontend/src/` | Vue、TypeScript 界面与状态展示 |
| `airplay-frontend/src-tauri/` | Tauri 窗口、设置、认证交互、托盘和会话生命周期 |
| `airplay-core/src/` | Rust 音频核心库与独立诊断 CLI |
| `airplay-backend/` | C/C++ AirPlay 后端入口和 Windows 适配 |
| `upstream/` | 原作者仓库的固定版本子模块，源码不作为普通文件纳入本仓库 |
| `scripts/` | PowerShell 构建入口 |
| `test/` | 前端、后端和核心测试源码、合成样例；生成内容位于忽略的 `.artifacts/` |

界面的 `types.ts` 集中保存与桌面命令共享的数据结构，`commands.ts` 统一命令错误转换；`useStreamSession.ts` 管理启动、停止、密码重试与事件归属，`App.vue` 管理设备视图和诊断快照。桌面端的 `settings.rs` 管理默认值、范围校验、读取和完整写入后的替换；`window.rs` 管理显示、关闭和退出顺序。`main.rs` 保留命令注册、共享状态和会话编排。

`airplay-core/` 包含音频核心代码和独立诊断 CLI；单元测试位于 `test/core/unit/`，采集及转换检查脚本位于 `test/core/checks/`，均随 Git 提交。其 `lib.rs` 导出音频模块供 GUI 直接链接；`main.rs` 生成独立的 `homepod-test.exe`。该 CLI 是产品诊断工具，Cargo 包名暂时保留 `homepod-test`，GUI 不通过启动该 CLI 实现串流。

Rust 单元测试通过原模块的 `#[cfg(test)]` 和 `#[path]` 挂接，保留访问私有实现的能力，原 Cargo 测试入口不变。桌面窗口/单实例模拟检查及后端 nonce 自检实现分别位于 `test/frontend/desktop/`、`test/backend/native/`，原入口仅负责调用。测试目录需随完整源码检出；测试源码不会作为独立文件加入安装包。

## 数据路径

```text
Windows 音频端点
  → WASAPI 共享采集（Playback 使用 loopback）
  → float32 与左右输入映射
  → rubato 重采样、TPDF 抖动和 16-bit 量化
  → 44100 Hz 交错双声道 PCM
  → 有界队列与二进制 stdin
  → airplay-backend.exe
  → ALAC 帧封装、加密 RTP、PTP/NTP 与重传
  → AirPlay 接收端
```

Rust 使用 MSVC 工具链，原生后端使用 MSYS2 UCRT64 GCC/G++。两侧通过进程管道通信，不共享 C++ ABI。

## 采集与会话

`capture.rs` 枚举端点、读取格式和采集包，校验时间戳并报告异常。Playback 使用 Windows 10 1703+ 支持的 WASAPI 音频就绪事件，事件触发后读完所有可用包；事件超时后先检查 GetNextPacketSize；只有确认有待取包才读取真实缓冲，否则推进空闲静音，避免遗漏驱动未通知的音频。Recording 保留既有读取方式。`source.rs` 持有持续采集器，选择来源后即提供预览电平；串流订阅新数据。停止串流解除订阅，切换来源或退出应用才释放采集器。

`live.rs` 管理每次串流的转换器、漂移控制、发送队列、后端进程和诊断。每次会话重建这些状态。`drift.rs` 根据发送时间线与水位微调重采样比例；播放提前量是独立协议参数。PCM 队列有容量限制，不允许无限积压。

`capture/timeline.rs` 独立计算空闲静音、恢复播放时的重叠修剪和缺口修复计划，不访问 WASAPI。调用者提供已交付帧数、设备位置、100 ns QPC 和当前时间，采集循环负责读取、释放、交付和记录诊断。保留 40 ms 空闲交付余量、单次 250 ms 缺口上限，以及 60 秒内最多 5 次/累计 500 ms 的修复限制。

`audio_queue.rs` 给 Source→转换和转换→管道两级队列分别设置 640 ms 音频时长预算，计入消费者处理中/管道写入中的块；不是两级合计 640 ms，也不是播放延迟。按每块帧数与采样率向上取整为 ns 预留，满时立即报错；RAII 在处理结束、失败和丢弃时归还。另设 256 块硬上限，防止极小包的元数据积压。PCM 字节缓冲由写线程非阻塞归还，池最多保留 32 个；左右互换使用会话内复用数组，Source 仍复制采集数据以脱离 WASAPI 缓冲生命周期。

诊断与音频使用独立队列，诊断写盘阻塞不会阻塞音频。短采集缺口只在时间戳和设备位置一致且未超出限制时修复；持续或严重异常停止并记录原因。

Source 启动日志与会话漂移日志也使用独立工作线程，文件错误只进入日志状态。会话的诊断日志共用 250 ms 收尾期限；Source 的启动日志单独使用 250 ms。超时后返回状态并停止等待，不能保证取消正在阻塞的系统 I/O。采集与会话结束判断不依赖日志保存成功。

协议 stderr 读取线程先解析原始状态和通知 `PCM_READY`，再把脱敏文本交给 `protocol_log.rs` 的 128 条有界队列，控制台与文件写入均在日志线程。原始 GUI 控制事件不经过这条可丢记录的队列。故障模式保留最近 24 条上下文与原有约 256 KiB 写入限制；溢出可能使上下文不完整，但首个协议故障仍保存在状态及后续完整串流报告中。`protocol_log_status` 与其他会话日志共用 250 ms 收尾期限，记录丢失、未完成、超时和写入错误。stderr 读取、GUI 事件回调、读线程 join 和最终报告写盘仍须另行管理，不能由日志期限推导整个应用的停止上限。

## 控制与隐私

Tauri 命令用于发现、设置、连接和音量；事件返回状态、实时电平、统计和日志。密码通过本机命名管道交给后端，不放入进程参数或持久配置。DACP 和设备事件用于音量反馈，不在 PCM 管道中混入控制文字。

GUI 配置与日志存放在 `%APPDATA%\AirPlay Hub`，CLI 诊断产物位于 `%APPDATA%\AirPlay Hub\cli`。源码、生成文件、运行数据分别管理。

程序目录含 `portable.flag` 时，GUI 与 `tools/` 中的 CLI 改用该程序目录下的 `data/`；目录解析以可执行文件为准。后端程序只从可执行文件附近的程序目录加载，调试构建额外允许项目中的 `dist/runtime/`，不从用户数据目录查找。

日志过滤位于 `privacy.rs`：设备清单提供本次编号和已知标识替换，字段规则兜底隐藏密码与凭据，地址规则隐藏陌生 IP、MAC 和 UUID。清单、UI 设备信息和协议控制仍使用真实数据。此过滤不能识别任意未标注或编码后的敏感内容；日志分享前仍需检查。

## 生成文件与第三方子模块

构建从固定版本子模块复制所需源码到临时目录，按清单顺序应用 `airplay-backend/patches/` 的上下文补丁，再由 `select_upstream.py` 选择函数/常量并组合本地 C 适配，输出到 `build/airplay-backend/generated/`。上游函数修改维护在补丁，Winsock/I/O 实现维护在本地兼容层，声明集中于 `probe_context.inc`；Python 不做源码字符串替换。子模块只读，不手改生成目录。第三方来源和版本见 [第三方记录](../THIRD_PARTY.md)。

`upstream-manifest.json` 保存三层子模块提交、15 个生成输入的规范化 SHA-256、补丁顺序/哈希和输出快照。`upstream_guard.py` 验证提交及内容，对临时副本逐组执行 `git apply --check` 和实际应用，失败即丢弃副本；报告保存实际使用的清单与输出哈希。生成快照由 `test/backend/test_upstream.py` 检查；生产构建不自动接受新清单。仍只选择所需 realtime 协议代码，排除 buffered TCP、MRP 和共享 PTP daemon；这不是上游完整 CLI 的 Windows 移植。升级步骤见 [构建指南](building.md#上游适配变更)。

## 当前已知问题

虚拟播放端点 Playback 首次初始化偶尔返回穿插全零的音频包，切换来源后可能恢复。日志显示这类异常可在重采样前出现；确切原因仍待确认。已将 Playback 改为事件驱动并补充释放前原始缓冲统计，修正效果需要首次启动实机验证。长测稳定不代表所有首次初始化或睡眠恢复场景都已解决。
