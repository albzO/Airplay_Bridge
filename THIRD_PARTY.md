# 第三方依赖与来源

本文件记录 AirPlay Hub 使用的第三方源码、依赖包、参考资料和本地适配。它是来源清单，不是对第三方代码的重新授权，也不是完整的传递依赖许可证清单。

## 依赖获取与归属

第三方源码通过 `upstream/airplay-cli` Git 子模块引用，由原作者仓库提供。主仓库保存地址和固定提交，不把第三方源码作为普通文件维护。`libraop` 和 `crosstools` 是固定的嵌套子模块，获取步骤见 [构建指南](docs/building.md)。

保留上游的版权文件头、许可证和声明文件。子模块引用方式不改变第三方代码的授权条件；本地生成的适配版本也应保留原始归属。历史提交曾包含源码快照，转换为子模块不会自动清除旧提交。

## 原生协议源码

| 来源 | 固定提交 | 本项目实际用途 | 可核实的许可证信息 |
|---|---|---|---|
| [music-assistant/airplay-cli](https://github.com/music-assistant/airplay-cli) | `8e79242996b7ef52352ee49d390e6db434bf88a6` | HAP/SRP、加密 RTSP、realtime 会话、PTP/NTP、事件相关协议与 raw ALAC 帧封装 | 所选 AP2 文件及 `alac_ext.cpp` 文件头为 Apache-2.0；上游根 LICENSE 为 GPLv3，上游完整发行版包含其他许可证组件 |
| [philippe44/libraop](https://github.com/philippe44/libraop) | `81c2182649da8645ac2a58b78e9f370c79a4165b` | `src/bplist.cpp`、`src/bplist.h` 二进制 plist 读写 | 固定提交缺少被文件头引用的许可证；当前公开分支已有 MIT 声明，历史版本适用范围待确认，见下文 |
| [philippe44/crosstools](https://github.com/philippe44/crosstools) | `41544c653760a205cbf6cebfdeda4a9394cc6455` | 平台与日志接口声明；本地提供日志实现 | 固定提交的 `LICENSE` 为 MIT |

这些提交对应当前构建输入；版本更新需同时修改子模块引用、源码哈希检查及此表。本项目不构建上游完整 CLI，也不使用其全部 RAOP、MRP、编解码器及其他嵌套组件。具体编译范围以 `airplay-backend/CMakeLists.txt` 和 `airplay-backend/select_upstream.py` 为准。

### 哪一部分的授权尚未确认

待确认对象是 **libraop 固定提交中的 `src/bplist.cpp` 和 `src/bplist.h`**，不是整个 C/C++ 后端，也不是所有第三方依赖。

证据与范围：

1. 两个文件头保留 Philippe 的版权信息，并写有 `See LICENSE`。
2. 当前固定提交 `81c2182649da8645ac2a58b78e9f370c79a4165b` 的根目录没有该许可证文件。
3. 固定版本 airplay-cli 的 [第三方记录](https://github.com/music-assistant/airplay-cli/blob/8e79242996b7ef52352ee49d390e6db434bf88a6/THIRD_PARTY_NOTICES.md) 也将其 bplist 授权列为未明确。
4. 2026-10-05 查阅时，libraop 当前公开分支的 [LICENSE.txt](https://github.com/philippe44/libraop/blob/master/LICENSE.txt) 已声明其自身代码采用 MIT，并区分其他第三方组件的授权条件。该链接指向可变化的分支，不是本项目已固定版本的许可证文件。

因此，不应继续描述为“libraop 完全没有许可证”，也不能仅凭其他同作者库采用 MIT 就推定历史版本的授权。后续可核实新声明对所用旧版文件的适用范围，或审查并升级至带有明确声明的固定版本。本次文档整理不升级依赖，也不替上游作者补发许可证。

### 本地适配与修改

`airplay-backend/` 是本项目的入口和适配代码，协议基础仍来自上游：

- `patches/` 保存三组有上下文的源码补丁：Windows 平台、HAP 事件/认证诊断、会话与 realtime 音频适配。`upstream_guard.py` / `upstream-manifest.json` 校验三层固定提交、15 个输入和补丁哈希，在临时副本应用；`select_upstream.py` 选择并组合编译内容，不再用字符串替换修改源码。输出快照由独立测试核对，固定子模块提交保持不变。
- `windows_port.c/.h`、`windows_io.inc`、`windows_audio.inc` 适配 Winsock、超时、部分读写及平台接口；`probe_context.inc` 保存本地编译上下文声明。
- `auth_auto.inc` 增加按需密码、认证错误分类与配对限流处理。
- `probe_entry.inc`、`audio_entry.inc` 组织会话、音频发送和清理流程，调用上游协议函数。
- `events_entry.inc` 增加独立事件通道、密钥上下文、消息组装、解析和回应。
- `volume_entry.inc` 增加设备音量查询、设置和反馈处理。
- `pcm_source.c` 增加 PCM 文件/管道输入及预读缓冲。
- `audio_nonce.c` 将音频 nonce 与 16 位 RTP 序号分离；重传仍复用原密文。
- `raw_codec.c` 包装上游 raw ALAC 实现。生成的 `raw_alac.c` 选自上游 `alac_ext.cpp`，增加参数和分配失败检查，不作为本项目原创编码算法声明。

生成源码位于 `build/airplay-backend/generated/`，不提交仓库，不应直接编辑。修改记录由生成脚本、本地适配文件及 Git 历史共同维护。

## Rust 与前端依赖

| 组件 | 版本记录 | 用途 | 已核实范围 |
|---|---|---|---|
| [mdns-sd](https://github.com/keepsimple1/mdns-sd) | `0.21.4` | mDNS 发现 | 该版本 Cargo 元数据：Apache-2.0 OR MIT |
| [windows-rs](https://github.com/microsoft/windows-rs) | `windows 0.62.2` | WASAPI、COM 和 Windows API 绑定 | 该版本 Cargo 元数据：MIT OR Apache-2.0 |
| [rubato](https://github.com/HEnquist/rubato) | `5.0.1` | 有状态 sinc 重采样 | 该版本 Cargo 元数据：MIT OR Apache-2.0 |
| [audioadapter-buffers](https://github.com/HEnquist/audioadapter-buffers-rs) | `5.2.0` | 音频缓冲适配 | 该版本 Cargo 元数据：MIT OR Apache-2.0 |
| Tauri、单实例插件、Serde 等 | Rust manifests 与 Cargo.lock | 桌面应用、序列化与会话管理 | 以各具体版本的许可证文件及元数据为准 |
| Vue、Vite、TypeScript、Tauri JS API 等 | `airplay-frontend/package.json` 与 `airplay-frontend/pnpm-lock.yaml` | 界面与前端构建 | 以各具体版本的许可证文件及元数据为准 |
| [Playwright Test](https://github.com/microsoft/playwright) | `@playwright/test 1.64.0` 与对应锁文件 | 仅开发时的真实模板 DOM 回归，不在生产入口导入 | 该版本 npm 包元数据及所附 LICENSE：Apache-2.0 |

Rust 精确版本与校验值位于 `airplay-core/Cargo.lock` 和 `airplay-frontend/src-tauri/Cargo.lock`；前端依赖位于 `airplay-frontend/pnpm-lock.yaml`。锁文件提供版本记录，不替代许可证文本、版权归属或传递依赖清单。

## 原生运行时与构建工具

原生后端链接 MSYS2 UCRT64 的 OpenSSL Crypto 和 winpthreads，并依赖所需 GCC 运行时 DLL。当前构建脚本递归检查 DLL 导入并复制运行时；依赖集合随工具链安装版本变化，不使用上游自带 OpenSSL 的版本描述代替本机实际版本。

已在当前构建环境核实：OpenSSL 所附 LICENSE 为 Apache-2.0 文本；winpthreads 附带自己的 COPYING。其他 DLL、GCC 运行时的许可证及例外条款应按实际打包版本核实。Python、CMake、Ninja、GCC/G++、Rust、Node.js 和 pnpm 是构建工具，不代表全部会被打包进应用。

`scripts/build-backend.ps1` 目前复制部分许可证到 `dist/licenses/`。这一步尚未生成完整的 Rust、前端及原生运行时第三方声明集合；不能据此认定发布包已经完成许可证材料整理。

## 实现参考资料

以下项目用于核对协议行为，未作为运行时依赖引入；已有记录表明未复制其实现代码：

- [pyatv transient pairing](https://github.com/postlund/pyatv/blob/master/pyatv/protocols/airplay/auth/hap_transient.py) 与 [事件通道说明](https://github.com/postlund/pyatv/blob/master/docs/documentation/protocols.md#event-channel)：核对 SRP 密钥长度、方向密钥及事件通道。
- [Shairport Sync RTP 实现](https://github.com/mikebrady/shairport-sync/blob/master/rtp.c)：核对音频 nonce 的传输格式与接收端处理。

这些分支链接用于阅读参考，不是本项目固定依赖版本。后续若复制或改编参考代码，应另行记录来源、版本、授权及修改。

## 授权与发布状态

本项目原创贡献及可授权的本地修改采用 [Apache License 2.0](LICENSE)，归属说明见 [NOTICE](NOTICE)。第三方代码保持各自原有授权，不因项目根目录 LICENSE 改为 Apache-2.0；子模块引用或版本标签不会替代授权要求。

详细的已确认事项、未完成事项和发布材料清单见 [许可证状态](docs/licensing.md)。本文件不宣称完成整个组合程序的许可证兼容性审查，也不把上游完整发行版的 GPL 结论直接套用于本项目选取的全部文件。
