# 架构

## 模块

| 目录 | 职责 |
|---|---|
| `desktop/src/` | Vue、TypeScript 界面与状态展示 |
| `desktop/src-tauri/` | Tauri 窗口、设置、认证交互、托盘和会话生命周期 |
| `tester/src/` | Rust 音频核心库与独立诊断 CLI |
| `probe/` | C/C++ AirPlay 后端入口和 Windows 适配 |
| `upstream/` | 原作者仓库的固定版本子模块，源码不作为普通文件纳入本仓库 |
| `scripts/` | PowerShell 构建入口 |

`tester` 是历史目录名，不只是测试代码。其 `lib.rs` 导出音频模块供 GUI 直接链接；`main.rs` 生成独立的 `homepod-test.exe`。GUI 不通过启动该 CLI 实现串流。

## 数据路径

```text
Windows 音频端点
  → WASAPI 共享采集（Playback 使用 loopback）
  → float32 与左右输入映射
  → rubato 重采样、TPDF 抖动和 16-bit 量化
  → 44100 Hz 交错双声道 PCM
  → 有界队列与二进制 stdin
  → cliairplay-probe.exe
  → ALAC 帧封装、加密 RTP、PTP/NTP 与重传
  → AirPlay 接收端
```

Rust 使用 MSVC 工具链，原生后端使用 MSYS2 UCRT64 GCC/G++。两侧通过进程管道通信，不共享 C++ ABI。

## 采集与会话

`capture.rs` 枚举端点、读取格式和采集包，校验时间戳并报告异常。Playback 使用 Windows 10 1703+ 支持的 WASAPI 音频就绪事件，事件触发后读完所有可用包；事件超时仅推进空闲静音，不据此读取真实缓冲。Recording 保留既有读取方式。`source.rs` 持有持续采集器，选择来源后即提供预览电平；串流订阅新数据。停止串流解除订阅，切换来源或退出应用才释放采集器。

`live.rs` 管理每次串流的转换器、漂移控制、发送队列、后端进程和诊断。每次会话重建这些状态。`drift.rs` 根据发送时间线与水位微调重采样比例；播放提前量是独立协议参数。PCM 队列有容量限制，不允许无限积压。

诊断与音频使用独立队列，诊断写盘阻塞不会阻塞音频。短采集缺口只在时间戳和设备位置一致且未超出限制时修复；持续或严重异常停止并记录原因。

## 控制与隐私

Tauri 命令用于发现、设置、连接和音量；事件返回状态、实时电平、统计和日志。密码通过本机命名管道交给后端，不放入进程参数或持久配置。DACP 和设备事件用于音量反馈，不在 PCM 管道中混入控制文字。

GUI 配置与日志存放在 `%APPDATA%\com.airplaywin.bridge`，CLI 诊断产物位于运行目录。源码、生成文件、运行数据分别管理。

## 生成文件与第三方子模块

`probe/select_upstream.py` 从固定版本子模块生成 Windows 编译视图，输出到 `build/probe-native/generated/`。修改生成规则或本地适配文件，不手改生成目录。第三方来源和版本见 [第三方记录](../THIRD_PARTY.md)。

## 当前已知问题

VAIO3 Playback 首次初始化偶尔返回穿插全零的音频包，切换来源后可能恢复。日志显示这类异常可在重采样前出现；确切原因仍待确认。已将 Playback 改为事件驱动并补充释放前原始缓冲统计，修正效果需要首次启动实机验证。长测稳定不代表所有首次初始化或睡眠恢复场景都已解决。
