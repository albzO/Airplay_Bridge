# AirPlay Hub 1.0

正式版 1.0，软件版本号 `1.0.0`。版本说明见同目录 `release-notes.md`。

启动上一级目录中的 `airplay-bridge.exe`。保留整个发行目录；`runtime/` 中的后端和 DLL 是串流必需文件。

```text
airplay-bridge.exe        桌面应用
runtime/airplay-backend.exe  原生 AirPlay 后端（同目录包含依赖 DLL）
tools/homepod-test.exe   可选的诊断 CLI
docs/                    使用与第三方说明
licenses/                第三方许可证
LICENSE                  自有代码许可证
NOTICE                   项目版权及第三方通知
```

## 使用

Windows x64，需要 Microsoft Edge WebView2 Runtime。确保电脑与接收端在可互相访问的局域网中，选择音频来源，刷新并选择设备，开始串流。

若设备要求密码，在底部来源区上方输入。立体声对需先在 Apple 家庭 App 中组成。默认关闭窗口收起到托盘；托盘右键“退出”关闭程序。

设置中提供外观、保持系统唤醒、播放提前量、声道映射、统计、技术详情和日志。Playback 采集的是向所选播放设备输出的声音。

## 配置与诊断

GUI 配置及日志：`%APPDATA%\AirPlay Hub`。

CLI 缓存、日志和采集诊断：`%APPDATA%\AirPlay Hub\cli`。

```powershell
.\tools\homepod-test.exe discover
.\tools\homepod-test.exe audio-devices
```

运行数据不写入发行目录。1.0 版日志默认隐藏设备名、IP 和用户目录，保留 UUID 与技术数据；分享历史日志前仍需检查。

## 第三方声明

自有代码声明见根目录 LICENSE、NOTICE；第三方声明见 THIRD_PARTY.md 和 licenses/。完整发行材料的许可证核对状态见同目录 licensing.md。本目录整理不改变现有许可证状态。

错误代码与处理方式见 `error-codes.md`。日志默认隐藏设备名、IP、用户目录等信息，仅保留 UUID 与技术数据。
