# AirPlay Bridge

Windows 上的 AirPlay 2 音频发送工具。将录音设备或播放设备的音频发送到 AirPlay 接收端，提供桌面界面、HomePod 立体声串流、音量同步、可调播放提前量和时钟漂移校正。

当前版本处于开发与实机验证阶段，验证主要面向 HomePod mini。其他 AirPlay 接收端的认证、时钟及音量行为可能不同，尚未完成通用兼容性验证。

## 功能

- **音频来源**：选择 Windows 录音设备，或通过 WASAPI loopback 采集播放设备的输出；选择来源后显示实时电平。
- **设备发现**：通过局域网 mDNS 发现 AirPlay 设备，选择单设备或已有的 HomePod 立体声对。
- **认证**：支持自动认证及按需密码输入，区分密码错误、配对限流和访问权限问题。
- **音频处理**：输入声道映射、44100 Hz 重采样、16-bit PCM 量化与 ALAC 帧封装。
- **播放控制**：播放提前量、发送预读 Buffer、音量、静音及立体声扬声器位置交换。
- **运行管理**：托盘、单实例、深色模式、可选保持系统唤醒、运行统计和诊断日志。

实际可用的音频来源取决于 Windows 音频端点和驱动；Playback 采集目标应是正在接收音频输出的设备。

## 软件架构

```text
Vue / TypeScript 桌面界面
        │ Tauri 命令与事件
        ▼
Tauri / Rust 应用管理
  设备发现、设置、认证、托盘、会话生命周期
        │ 直接调用 Rust 库
        ▼
Rust 音频处理库（tester）
  WASAPI 采集 → 声道映射 → rubato 重采样 → 16-bit PCM
        │ 有界队列与二进制 stdin 管道
        ▼
C/C++ 原生后端（cliairplay-probe.exe）
  配对与加密控制 → ALAC 帧封装 → 加密 RTP / 时钟 / 重传
        │ 局域网
        ▼
AirPlay 接收端
```

### 界面与应用管理

前端使用 Vue 3、TypeScript 和 Vite，Tauri 2 负责 Windows 窗口与 WebView2。Rust 管理设备发现、配置、密码交互、托盘和串流会话，通过事件向界面发送状态及统计。

GUI 直接链接 `tester` 中的 Rust 库，**不会启动 `homepod-test.exe` 来实现串流**。`homepod-test.exe` 是使用同一套核心模块的独立诊断工具。

### 采集与音频处理

录音端点使用 WASAPI 共享模式；播放端点使用 WASAPI loopback。GUI 选择来源后建立持续采集器，串流订阅当前采集数据；停止串流解除订阅，来源电平仍可更新。切换来源或退出程序时释放采集器。

采集数据转换为 float32，经声道映射、有状态的 rubato sinc 重采样、TPDF 抖动和量化，生成 44100 Hz、16-bit、小端交错双声道 PCM。音频沿有界队列送到独立写入线程，再写入原生后端。实时串流无需先保存音频文件。

漂移控制器根据发送时间线和流水线水位微调重采样比例，补偿采集与发送时钟的速度差。控制器不通过跳过整包音频校正漂移；队列和时间线异常会报告故障。

### AirPlay 协议后端

`probe/` 是本项目的 Windows 适配及协议入口，基于固定版本的 `airplay-cli` 源码生成编译视图。后端负责认证、加密 RTSP 控制、音频会话、ALAC 帧封装、加密 RTP、时钟同步、事件响应和重传。

Rust 使用 MSVC 工具链，原生后端使用 MSYS2 UCRT64 的 GCC/G++；二者通过进程管道通信。密码通过本机命名管道提交，音量反馈经 DACP 服务和设备事件回传到应用。

## 目录结构

| 路径 | 职责 |
|---|---|
| `desktop/src/` | Vue 界面、状态展示和样式 |
| `desktop/src-tauri/src/` | Rust 桌面应用、设置、认证与会话管理 |
| `tester/src/capture.rs` | WASAPI 端点枚举、采集、时间戳和格式信息 |
| `tester/src/source.rs` | 持续采集器、预览和串流订阅 |
| `tester/src/convert.rs` | 重采样、抖动和 PCM 量化 |
| `tester/src/drift.rs` | 水位与时钟漂移控制 |
| `tester/src/live.rs` | 实时流水线、后端进程和诊断 |
| `tester/src/discovery.rs` | mDNS 设备发现 |
| `tester/src/volume.rs` | DACP 音量反馈 |
| `tester/src/main.rs` | 独立 CLI 诊断入口 |
| `probe/` | C/C++ 后端入口、Windows 适配和源码生成脚本 |
| `upstream/` | 固定版本的第三方源码快照 |
| `environment-check/` | 开发环境验证示例，不参与应用构建 |
| `Build.ps1` | 构建后端、CLI 并整理运行时 DLL |
| `Build-UI.ps1` | 构建前端与桌面应用 |

`build/`、`dist/`、Rust `target/` 和 `node_modules/` 为本地构建产物，不纳入源码仓库。

## 构建要求

构建目标为 Windows x64。需要安装：

| 组件 | 用途 |
|---|---|
| Rust 稳定版，MSVC 工具链 | Rust 核心库和桌面应用；需要支持 edition 2024 |
| Visual Studio C++ Build Tools 与 Windows SDK | Rust MSVC 链接及 Windows 开发环境 |
| MSYS2 UCRT64 | 原生 C/C++ 后端，需 GCC/G++、CMake、Ninja、OpenSSL、winpthreads 和 binutils |
| Python 3 | 固定上游源码的编译视图生成 |
| Node.js 20.19+ 或 22.12+，以及 pnpm | Vue/Vite 前端构建 |
| Microsoft Edge WebView2 Runtime | 运行桌面界面 |

将 Python、Node.js 和 pnpm 加入 PATH。构建脚本默认从 `%USERPROFILE%\.cargo\bin` 使用 Cargo，从 `C:\msys64` 使用 MSYS2。

### 获取源码

克隆本仓库，进入项目根目录。`upstream/` 已作为普通源码纳入，**无需初始化子模块**。来源和固定提交见 [THIRD_PARTY.md](THIRD_PARTY.md)。

上游目录中的 `.gitmodules` 保留自原始项目；其未检出的其他依赖不用于本项目的构建入口。请使用根目录构建脚本，不将本快照当作完整的上游 Makefile 构建环境。

### 构建桌面应用

在项目根目录的 PowerShell 中执行：

```powershell
# 首次构建：下载依赖并构建全部组件
.\Build-UI.ps1

# Rust 桌面应用使用 release 配置；原生后端仍按 Build.ps1 的配置构建
.\Build-UI.ps1 -Release

# 已下载全部依赖后，离线构建
.\Build-UI.ps1 -Offline
```

输出位于 `dist/`。运行：

```powershell
.\dist\airplay-bridge.exe
```

保留完整 `dist/` 目录，包括 `cliairplay-probe.exe` 和所需 DLL；仅复制 GUI exe 无法完成音频串流。

### 自定义后端工具路径

`Build-UI.ps1` 默认调用 `Build.ps1`。若 MSYS2 或 Python 位于其他位置，先单独构建后端，再跳过该步骤构建界面：

```powershell
# 将路径替换为自己的工具安装位置
.\Build.ps1 -MsysRoot 'D:\Tools\msys64' -Python 'D:\Tools\Python\python.exe'
.\Build-UI.ps1 -SkipBackend
```

`-SkipBackend` 要求 `dist/` 中已有可用的后端及 DLL。

## 使用方法

1. 确保 Windows 电脑和接收端位于可互相访问的局域网，网络允许 mDNS 发现及 AirPlay 通信。
2. 启动桌面应用，选择音频来源。Playback 来源需要有应用向所选设备输出声音。
3. 刷新并选择 AirPlay 设备；立体声模式需要设备已在 Apple 家庭 App 中组成一对。
4. 开始串流。设备要求密码时，在界面输入 AirPlay 访问密码；无需通过命令行传入密码。
5. 在设备区域调节音量，按需设置播放提前量、输入映射及 Buffer。
6. 停止串流后等待会话关闭。默认关闭窗口会收起到托盘；托盘菜单“退出”关闭应用。关闭动作可在设置中修改。

再次运行 exe 会唤回已有实例。更多界面与设置说明见 [desktop/README.md](desktop/README.md)。

### 播放提前量与 Buffer

- **播放提前量**：向接收端请求的播放时间偏移。设备能力可能限制实际生效值；较低设置需要在目标网络和设备上验证。
- **Buffer**：发送端 PCM 预读深度，与播放提前量是不同参数。
- **漂移校正**：微调重采样比例，维持长期流水线水位；不替代网络故障处理。

界面的延迟估计不包含应用和扬声器实际发声耗时，也不会自动调整其他播放器的歌词或视频时间线。

## CLI 诊断

CLI 位于 `dist/homepod-test.exe`。示例中的设备名称均为占位名称，请替换为 `discover` 发现的实际显示名称：

```powershell
# 扫描接收端及 Windows 音频端点
.\dist\homepod-test.exe discover
.\dist\homepod-test.exe audio-devices

# 验证认证与控制会话，不发送音频
.\dist\homepod-test.exe test --device 'Living Room'

# 发送固定测试音
.\dist\homepod-test.exe tone --device 'Living Room'
```

CLI 中的 `capture-b3`、`replay-b3`、`stream-b3` 和 `stream-stereo` 属于开发阶段的 B3 采集诊断路径，需要匹配的录音端点。它们不是通用音频来源选择接口；其他采集设备请使用桌面界面。

## 配置、隐私与日志

- GUI 配置、设备缓存和日志位于 `%APPDATA%\com.airplaywin.bridge`。
- CLI 输出位于 exe 所在目录下，例如 `dist/devices.json`、`dist/logs/`，录音诊断文件位于 `dist/captures/`。
- 密码不持久保存到配置，也不作为进程参数传递；认证方式记录不包含密码。
- 默认日志以关键故障及必要上下文为主。设置中可分别启用详细会话日志和逐包采集诊断。
- 逐包采集诊断记录时间戳、帧数、电平和处理耗时等元数据，不保存原始音频；单会话按约 8 MiB 轮转，保留四段。其他日志的保留策略不同，不能据此推断全部日志总量有同样上限。

设备清单、配置和诊断日志可能包含设备名称、局域网地址、端点 ID 与本机路径。分享日志或提交问题前，应检查并脱敏。`.gitignore` 排除常见本地运行数据，但不能代替人工检查。

## 当前限制

- 当前实现和构建脚本面向 Windows，尚未支持其他平台。
- 当前发送格式固定为 44100 Hz、16-bit、双声道；这不是所有 AirPlay 设备的格式上限声明。
- 立体声实现面向已配对的 HomePod，不是通用多房间任意设备组网。
- 虚拟音频驱动的 Playback 回采行为和长期稳定性仍需验证。
- UDP 提交成功、无重传请求或编译通过，均不能单独证明接收端播放效果。

## 第三方来源与许可证

本项目使用固定版本的 `music-assistant/airplay-cli`、`libraop`、`crosstools` 以及 Rust/前端依赖。保留第三方源码中的许可证与版权声明，版本及适配说明见 [THIRD_PARTY.md](THIRD_PARTY.md)。

第三方许可证核对尚未完成。本仓库当前用于开发快照保存，未声明整个组合项目采用统一的 MIT 或 Apache-2.0 许可证；公开分发源码或二进制前需要完成相应核对。
