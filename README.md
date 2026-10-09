# AirPlay Hub

Windows 上的 AirPlay 2 音频发送工具。通过桌面界面选择录音或播放设备，将音频发送到 HomePod 等 AirPlay 接收端。

当前维护版本为 **1.0.2（待发布）**，暂停播放误触发重建及设备接管超时的修复仍需实机回归确认。主要验证 HomePod mini 单设备及立体声对，其他接收端的兼容性尚未完整验证。版本说明见 [更新日志](CHANGELOG.md)，项目授权状态见 [许可证说明](docs/licensing.md)。

## AI 辅助开发声明

本项目在 OpenAI Codex 的协助下开发。AI 参与了架构方案讨论、代码生成与修改、问题分析和文档编写；项目维护者负责需求与功能方向、开发决策、真实设备测试和发布管理。本项目不声称全部代码由维护者独立手写。

AirPlay 协议基础及部分组件来自第三方开源项目，其归属和许可证见 [THIRD_PARTY.md](THIRD_PARTY.md)。AI 辅助开发不改变这些第三方声明。生成的代码仍可能存在缺陷，实际验证范围与当前限制在下文说明。

## 功能

- WASAPI 录音与 Playback loopback 采集，实时来源电平。
- 局域网设备发现、按需密码认证、单设备及 HomePod 立体声对。
- 左右输入映射、扬声器位置交换、音量与 HomePod 按钮同步。
- 44100 Hz 重采样、16-bit PCM 量化、ALAC 帧封装与加密传输。
- 可调播放提前量、Buffer、长期时钟漂移校正。
- 托盘、单实例、开机自启、跟随系统的深浅色模式、系统唤醒、统计与诊断日志。

## 快速开始

使用发行包：将完整目录解压到本机，启动 `airplay-bridge.exe`。需要 Windows x64 和 Microsoft Edge WebView2 Runtime；保留 `runtime/` 中的后端和 DLL。

从源码构建需要 Rust MSVC、Visual Studio C++ Build Tools / Windows SDK、MSYS2 UCRT64、Python、Node.js 和 pnpm。具体依赖见 [构建指南](docs/building.md)。

在仓库根目录运行：

```powershell
# 获取固定版本的第三方源码；只初始化本项目所需依赖
git submodule update --init -- upstream/airplay-cli
git -C upstream/airplay-cli submodule update --init -- libraop
git -C upstream/airplay-cli/libraop submodule update --init -- crosstools

# 构建全部组件；桌面应用使用 release 配置
.\scripts\build-desktop.ps1 -Release

# 已下载全部依赖后离线构建
.\scripts\build-desktop.ps1 -Release -Offline

# 启动；保留整个 dist 文件夹中的后端及 DLL
.\dist\airplay-bridge.exe
```

选择音频来源、刷新并选择设备，点击开始串流。需要密码时在底部音频来源区上方的密码浮层中输入。立体声对需先在 Apple 家庭 App 中组成。

底部固定音频来源与串流操作；右上角齿轮打开设置。默认关闭窗口收起到托盘，托盘右键“退出”关闭应用。保持系统唤醒默认开启，允许屏幕熄灭。

构建入口位于 `scripts/`，后端、依赖获取和自定义工具路径见 [构建指南](docs/building.md)。

## 软件架构

```text
Vue / TypeScript 界面
        ↓ Tauri 命令与事件
Rust 桌面管理与音频核心
  WASAPI 采集 → 声道映射 → rubato 重采样 → 16-bit PCM
        ↓ 有界队列和进程管道
C/C++ 原生后端
  认证与控制会话 → ALAC 帧封装 → 加密 RTP、时钟与重传
        ↓ 局域网
AirPlay 接收端
```

GUI 直接链接 `airplay-core/` 中的 Rust 库，协议后端为独立的 `airplay-backend.exe`；`homepod-test.exe` 是使用同一核心模块的诊断工具。Python 仅在构建阶段生成上游适配源码，不参与运行时音频处理。模块和生命周期说明见 [架构文档](docs/architecture.md)。

## 仓库结构

```text
├── airplay-frontend/  Vue 界面与 Tauri 桌面应用
├── airplay-core/       共用 Rust 音频库及诊断 CLI
├── airplay-backend/  C/C++ AirPlay 后端与 Windows 适配
├── upstream/        第三方子模块（仅提交地址与固定版本引用）
├── scripts/         构建脚本
├── docs/            使用、构建、架构与历史记录
├── .github/         问题与 PR 模板
├── CONTRIBUTING.md  开发与贡献约定
├── LICENSE          Apache-2.0 完整许可文本
├── NOTICE           项目版权及第三方归属说明
└── THIRD_PARTY.md   第三方来源和许可证记录
```

`build/`、`dist/`、`target/`、`node_modules/` 和 `.local/` 是本地生成或个人资料目录，不提交 Git。旧环境练习和备份保存在 `.local/`。

第三方源码由原作者仓库提供，本仓库通过 `.gitmodules` 和固定提交引用依赖。首次获取所需子模块的命令见 [构建指南](docs/building.md)。GitHub 自动生成的源码 ZIP 不包含子模块内容。

## 文档

- [1.0 更新日志](CHANGELOG.md)
- [构建指南](docs/building.md)
- [界面与设置](docs/ui.md)
- [CLI 诊断](docs/cli.md)
- [架构](docs/architecture.md)
- [文档导航与历史记录](docs/README.md)
- [贡献指南](CONTRIBUTING.md)
- [第三方依赖与来源](THIRD_PARTY.md)
- [许可证状态](docs/licensing.md)

## 运行数据

GUI 配置与日志位于 `%APPDATA%\AirPlay Hub`；CLI 设备清单、日志和录音诊断位于 `%APPDATA%\AirPlay Hub\cli`。密码不持久保存，也不作为进程参数传递。

日志默认隐藏设备名称、IP、用户目录、公钥、MAC 和 UUID，并过滤密码、令牌及凭据字段，保留格式、时序与统计数据。设备使用本次清单内的编号关联，不使用永久 UUID。历史日志不会自动改写，分享旧日志前仍需检查。额外逐包诊断不保存原始音频；四段滚动限制只适用于该诊断文件，不适用于所有日志。

维护分支的安全审查、历史数据边界和验证记录见 [维护审查记录](docs/security-review-20261008.md)。

## 当前限制

- 仅支持 Windows；发送格式固定为 44100 Hz、16-bit、双声道。
- 立体声面向已有的 HomePod 立体声对，尚未实现通用多房间组网。
- 虚拟播放端点 Playback 首次打开偶尔出现全零片段与撕裂，原因仍在排查。
- 编译、模拟检查及发送成功不能代替实际接收端播放验证。
- 延迟估计不包含应用和扬声器实际发声耗时，不会同步外部播放器的歌词或视频。

## TODO

- [ ] 添加软件捕获：采集指定应用的音频。
- [ ] 多设备编组：多个 AirPlay 接收端同步播放。

## 第三方代码与致谢

协议后端基于 [Music Assistant airplay-cli](https://github.com/music-assistant/airplay-cli)，使用 [libraop](https://github.com/philippe44/libraop) 的 bplist 读写及 [crosstools](https://github.com/philippe44/crosstools) 的接口声明。Rust 音频处理使用 rubato，界面使用 Vue 和 Tauri。感谢这些项目的作者和维护者。

第三方代码属于原作者；本项目提供 Windows 适配、桌面应用和音频流水线。固定版本、使用文件、本地修改、包依赖及协议参考资料分别记录在 [THIRD_PARTY.md](THIRD_PARTY.md)。子模块引用不改变第三方授权条件。

## 许可证

本项目自有代码及文档采用 **Apache License 2.0**，完整条款见 [LICENSE](LICENSE)，归属说明见 [NOTICE](NOTICE)。除另有声明的内容外，该许可适用于本项目的原创贡献和可授权的本地修改，不覆盖或替换第三方的原有许可。

Apache-2.0 允许商业使用、修改和分发，无需单独取得本项目的商业授权，但需遵守保留许可与相关声明、标明修改等条件；软件按原样提供，不附带保证。第三方组件仍须遵守各自许可。

目前明确待确认的是固定版本 libraop 的 `bplist.cpp` / `bplist.h` 授权依据：该历史提交缺少引用的许可证，当前上游已提供 MIT 声明，尚需确认对所用版本的适用范围。实际发行包的依赖通知材料也尚未完整汇总。详细依据见 [许可证状态](docs/licensing.md)，不把这些事项笼统描述为所有依赖许可证不明。

## 贡献与问题反馈

问题和改动请说明复现步骤、预期与实际行为、相关版本及验证范围。分享日志前移除个人设备、地址、端点与路径信息。开发约定和检查方式见 [贡献指南](CONTRIBUTING.md)。
