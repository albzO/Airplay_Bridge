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

界面的 `types.ts` 集中保存与桌面命令共享的数据结构，`commands.ts` 统一命令错误转换。桌面端的 `settings.rs` 管理默认值、范围校验、读取和完整写入后的替换；`window.rs` 管理显示、关闭和退出顺序。`main.rs` 保留命令注册、共享状态和会话编排。

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

诊断与音频使用独立队列，诊断写盘阻塞不会阻塞音频。短采集缺口只在时间戳和设备位置一致且未超出限制时修复；持续或严重异常停止并记录原因。

Source 启动日志与会话漂移日志也使用独立工作线程，文件错误只进入日志状态。会话的诊断日志共用 250 ms 收尾期限；Source 的启动日志单独使用 250 ms。超时后返回状态并停止等待，不能保证取消正在阻塞的系统 I/O。采集与会话结束判断不依赖日志保存成功。

## 控制与隐私

Tauri 命令用于发现、设置、连接和音量；事件返回状态、实时电平、统计和日志。密码通过本机命名管道交给后端，不放入进程参数或持久配置。DACP 和设备事件用于音量反馈，不在 PCM 管道中混入控制文字。

GUI 配置与日志存放在 `%APPDATA%\AirPlay Hub`，CLI 诊断产物位于 `%APPDATA%\AirPlay Hub\cli`。源码、生成文件、运行数据分别管理。

程序目录含 `portable.flag` 时，GUI 与 `tools/` 中的 CLI 改用该程序目录下的 `data/`；目录解析以可执行文件为准。后端程序只从可执行文件附近的程序目录加载，调试构建额外允许项目中的 `dist/runtime/`，不从用户数据目录查找。

日志过滤位于 `privacy.rs`：设备清单提供本次编号和已知标识替换，字段规则兜底隐藏密码与凭据，地址规则隐藏陌生 IP、MAC 和 UUID。清单、UI 设备信息和协议控制仍使用真实数据。此过滤不能识别任意未标注或编码后的敏感内容；日志分享前仍需检查。

## 生成文件与第三方子模块

`airplay-backend/select_upstream.py` 从固定版本子模块生成 Windows 编译视图，输出到 `build/airplay-backend/generated/`。修改生成规则或本地适配文件，不手改生成目录。第三方来源和版本见 [第三方记录](../THIRD_PARTY.md)。

## 当前已知问题

虚拟播放端点 Playback 首次初始化偶尔返回穿插全零的音频包，切换来源后可能恢复。日志显示这类异常可在重采样前出现；确切原因仍待确认。已将 Playback 改为事件驱动并补充释放前原始缓冲统计，修正效果需要首次启动实机验证。长测稳定不代表所有首次初始化或睡眠恢复场景都已解决。
