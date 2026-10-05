# AirPlay Hub

Windows 上的 AirPlay 2 音频发送工具。通过桌面界面选择录音或播放设备，将音频发送到 HomePod 等 AirPlay 接收端。

当前处于开发与实机验证阶段，主要验证 HomePod mini。其他接收端的兼容性尚未完整验证。

## 功能

- WASAPI 录音与 Playback loopback 采集，实时来源电平。
- 局域网设备发现、按需密码认证、单设备及 HomePod 立体声对。
- 左右输入映射、扬声器位置交换、音量与 HomePod 按钮同步。
- 44100 Hz 重采样、16-bit PCM 量化、ALAC 帧封装与加密传输。
- 可调播放提前量、Buffer、长期时钟漂移校正。
- 托盘、单实例、深浅色模式、系统唤醒、统计与诊断日志。

## 快速开始

开发环境需要 Windows x64、Rust MSVC、Visual Studio C++ Build Tools / Windows SDK、MSYS2 UCRT64、Python、Node.js 和 pnpm；运行桌面界面需要 WebView2。具体依赖见 [构建指南](docs/building.md)。

在仓库根目录运行：

```powershell
# 首次构建全部组件
# 首次克隆后先按构建指南初始化第三方子模块
.\scripts\build-desktop.ps1

# 已下载全部依赖后离线构建
.\scripts\build-desktop.ps1 -Offline

# 启动；保留整个 dist 文件夹中的后端及 DLL
.\dist\airplay-bridge.exe
```

选择音频来源、刷新并选择设备，点击开始串流。需要密码时在设备卡片中输入。立体声对需先在 Apple 家庭 App 中组成。

底部固定音频来源与串流操作；右上角齿轮打开设置。默认关闭窗口收起到托盘，托盘右键“退出”关闭应用。保持系统唤醒默认开启，允许屏幕熄灭。

根目录的 `Build.ps1` 和 `Build-UI.ps1` 作为兼容入口保留。

## 仓库结构

```text
├── desktop/          Vue 界面与 Tauri 桌面应用
├── tester/           共用 Rust 音频库及诊断 CLI
├── probe/            C/C++ AirPlay 后端与 Windows 适配
├── upstream/         第三方子模块（仅提交地址与固定版本引用）
├── scripts/          构建脚本
├── docs/             使用、构建、架构与历史记录
├── .github/          问题与 PR 模板
├── CONTRIBUTING.md   开发与贡献约定
└── THIRD_PARTY.md    第三方来源和许可证记录
```

`build/`、`dist/`、`target/`、`node_modules/` 和 `.local/` 是本地生成或个人资料目录，不提交 Git。旧环境练习和备份保存在 `.local/`。

第三方源码由原作者仓库提供，本仓库通过 `.gitmodules` 和固定提交引用依赖。首次获取所需子模块的命令见 [构建指南](docs/building.md)。GitHub 自动生成的源码 ZIP 不包含子模块内容。

## 文档

- [构建指南](docs/building.md)
- [界面与设置](docs/ui.md)
- [CLI 诊断](docs/cli.md)
- [架构](docs/architecture.md)
- [文档导航与历史记录](docs/README.md)
- [贡献指南](CONTRIBUTING.md)

## 运行数据

GUI 配置与日志位于 `%APPDATA%\com.airplaywin.bridge`；CLI 设备清单、日志和录音诊断位于 exe 所在目录。密码不持久保存，也不作为进程参数传递。

日志可能包含设备名称、网络地址、端点标识与本机路径，分享前请脱敏。额外逐包诊断不保存原始音频；四段滚动限制只适用于该诊断文件，不适用于所有日志。

## 当前限制

- 仅支持 Windows；发送格式固定为 44100 Hz、16-bit、双声道。
- 立体声面向已有的 HomePod 立体声对，尚未实现通用多房间组网。
- VAIO3 Playback 首次打开偶尔出现全零片段与撕裂，原因仍在排查。
- 编译、模拟检查及发送成功不能代替实际接收端播放验证。
- 延迟估计不包含应用和扬声器实际发声耗时，不会同步外部播放器的歌词或视频。

## 许可证状态

第三方源码的原始版权及许可证声明保留在各来源目录，固定版本和适配说明见 [THIRD_PARTY.md](THIRD_PARTY.md)。自有代码尚未指定统一开源许可证，第三方核对也未完成；本次仓库整理不改变这些声明。
