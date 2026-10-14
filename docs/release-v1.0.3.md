# v1.0.3 更改与交付简报

日期：2026-10-14。完整功能清单见 [CHANGELOG](../CHANGELOG.md)，线程与容量边界见[架构](architecture.md#通知与认证策略)。

## 本轮更改

- Source 电平、采集告警和最终错误交由独立线程发送。电平保留最新快照，控制 FIFO 最多 64 条，最终事件独立保存；采集回调不执行 GUI 通知。
- 会话入口统一包装协议/管线通知。控制 FIFO 最多 128 条，普通展示日志最多 64 条，遥测和各设备统计合计最多 16 个最新快照，报告使用专用位置。控制溢出明确停止，未确认退出的通知线程保留会话占用。
- 认证策略独立后台保存，每会话最多两台设备、每目标一次尝试；通知回调只登记请求/取错误，不写盘或取得策略锁。磁盘提交成功后更新内存，失败不会阻止下一会话重试。忘记策略改为互斥后台命令，超时保存线程保留在旧会话中。
- 统一版本到 1.0.3：前端、核心、桌面 Cargo 包与自有锁文件条目、Tauri、Inno Setup 及当前说明。保留第三方依赖版本和历史版本记录；Rust edition 继续为 2024。
- 包含本次发行前已完成的启动取消、统一停止预算、日志保留、缓冲池、界面拆分和上游补丁迁移等维护内容。音频包、样本顺序及左右互换算法保持原样，CI 暂缓。

## 验证

- 核心：119 通过、9 项设备/性能测试按原规则忽略。
- 桌面：21 通过，包含认证策略、会话互斥、停止及本地密码管道回归。
- 前端脚本：28 通过；类型检查和生产构建通过。
- 桌面及诊断 CLI 均成功执行离线 Release 构建。原生后端及运行库未改源码，复用现有 dist 文件。
- 敏感信息扫描、Rust 格式及 diff 检查通过。没有重复 DOM、性能基线或完整原生协议矩阵；未进行真实 HomePod/WASAPI 长测或安装/卸载操作。

测试、编译和包检查不代表真实设备播放、长时间熄屏或完整升级流程已验证。

## 产物

输出目录为 `releases/v1.0.3/`：

| 文件                                          |  字节数 | 用途                                                          |
| --------------------------------------------- | ------: | ------------------------------------------------------------- |
| AirPlay-Bridge-1.0.3-windows-x64-portable.zip | 8443554 | 便携版，包含 portable.flag，数据在程序目录下 data/            |
| AirPlay-Bridge-1.0.3-windows-x64-setup.exe    | 9632549 | Inno Setup 安装包，沿用原 AppId，数据在 %APPDATA%/AirPlay Hub |
| SHA256SUMS.txt                                |       — | 两份发行文件的 SHA256 校验值                                  |

本次使用现有 Inno Setup 7 编译 `scripts/installpackcompiler.iss`，未使用 NSIS 替代安装身份。构建命令为：

```powershell
.\scripts\build-desktop.ps1 -Release -Offline -SkipBackend
cargo build --manifest-path airplay-core/Cargo.toml --locked --offline --release
# 将核心 Release CLI 复制到 dist/tools，再按明确清单生成便携包。
& 'C:\Program Files\Inno Setup 7\ISCC.exe' /Q /Oreleases\v1.0.3 scripts\installpackcompiler.iss
```

GUI 文件版本及安装包产品版本为 1.0.3。便携 ZIP 的 21 个文件逐个与明确来源清单核对 SHA256，GUI/后端/CLI 的 PE 架构均为 x64；包内 Markdown 与构建时的源码一致，没有配置、设备清单、录音、运行日志或 data/。安装脚本的明确文件清单没有 portable.flag。WebView2 引导程序具有有效 Microsoft 签名。

产物由发行时的工作区构建，未上传发布。发行文件和构建产物继续按仓库规则忽略。
