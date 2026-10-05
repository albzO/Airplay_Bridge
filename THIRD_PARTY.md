# 第三方源码记录

第三方源码通过 `upstream/airplay-cli` 子模块引用，不作为普通文件纳入本仓库。`libraop` 和 `crosstools` 是其固定版本的嵌套依赖，初始化方式见 [构建指南](docs/building.md)。本地编译仍需下载这些源码，并保留原有版权与许可证声明。

这是一份本机开发测试目录，不是完成许可证审查的发布包。

| 来源 | 固定版本 | 用途 |
|---|---|---|
| music-assistant/airplay-cli | `8e79242996b7ef52352ee49d390e6db434bf88a6` | HAP/SRP、加密 RTSP、session SETUP、PTP/NTP |
| philippe44/libraop | `81c2182649da8645ac2a58b78e9f370c79a4165b` | bplist 实现 |
| libraop/crosstools | `41544c653760a205cbf6cebfdeda4a9394cc6455` | 日志及平台声明 |
| mdns-sd | `0.21.4`，Rust 依赖见 Cargo.lock | Rust mDNS 发现 |
| Microsoft windows-rs | `windows 0.62.2`，依赖见 Cargo.lock | WASAPI 共享采集与 COM API 绑定 |
| rubato | `5.0.1`，依赖见 Cargo.lock | Rust sinc 重采样 |
| audioadapter-buffers | `5.2.0`，依赖见 Cargo.lock | 交错双声道采样缓冲适配 |
| Tauri / WebView2 | Rust 依赖见 `desktop/src-tauri/Cargo.lock` | Windows 窗口、前端资源与 Rust IPC |
| Vue / Vite / TypeScript | 依赖见 `desktop/pnpm-lock.yaml` | 桌面界面与构建工具 |
| OpenSSL、GCC runtime、winpthreads | 本机 MSYS2 UCRT64 安装版本 | 密码学及 Windows 运行时 DLL |

airplay-cli 仓库根 LICENSE 为 GPLv3，所选 AP2 文件的文件头标注 Apache-2.0；保留原文件头，不把整个组合程序概括为 Apache-2.0。libraop 的 bplist 文件引用 LICENSE，但这个检出版本根目录没有对应文件。crosstools 标注 MIT。正式分发前需要核对这些来源及全部依赖的许可证要求。

`probe/select_upstream.py` 从固定源文件生成 Windows 编译视图，保留协议主体；改动包括 socket 类型、Windows I/O、排除 buffered TCP/MRP/共享守护进程、诊断标记及错误响应体脱敏。音频路径选用上游 realtime SETUP、加密 RTP、时钟锚点和重传函数，以及 alac_ext.cpp 中的 16 位双声道 raw ALAC 帧封装；后者增加参数及分配失败检查。生成结果位于 `build/probe-native/generated`，不能在该目录手改。

上游源文件保留在 `upstream/`，本地改动保留在 `probe/`。Rust 库的确切版本及校验值保留在 `tester/Cargo.lock`。构建脚本将现有上游和部分运行时许可证复制到 `dist/licenses/`；该动作不代表正式发布所需材料已经齐全。

事件通道：本地 `events_entry.inc` 参考固定 `ap2_mrp.c` 的反向事件处理流程，采用已有 HAP AEAD 和 bplist 字符串读取器；没有引入完整 MRP data-channel 或播放器控制实现。生成的 HAP 视图新增事件方向密钥，在原配对控制密钥派生处同时派生事件密钥，并创建独立上下文。完整 SRP 密钥和方向交换核对 [pyatv transient pairing](https://github.com/postlund/pyatv/blob/master/pyatv/protocols/airplay/auth/hap_transient.py) 及 [事件通道说明](https://github.com/postlund/pyatv/blob/master/docs/documentation/protocols.md#event-channel)；未复制 pyatv 代码，也未新增运行时依赖。

长时实时发送的本地修正：生成视图将上游绑定 16 位 RTP 序号的 nonce 改为独立 64 位小端计数，序号回绕不再重复 nonce；`audio_nonce.c` 提供计数和自检。AP2 后缀承载完整 8 字节 nonce，独立接收端实现参照 https://github.com/mikebrady/shairport-sync/blob/master/rtp.c 的 decipher_player_put_packet；没有复制其代码。
