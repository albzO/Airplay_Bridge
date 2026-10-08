# 播放回环排查 / Playback loopback investigation

记录日期：2026-10-09。结论适用于本次实机环境，不是所有 WASAPI 设备的保证。

## 当前结果 / Current result

在 VoiceMeeter VAIO3 / I/O 8 上，普通 WAV 测试音持续播放、Apple Music 已退出，SOFT 同步及 A1/A4 ASIO 路由保持原设置。只将实际 VAIO latency 从 768 改为 7168 后，相同的独立采集对照不再出现持续全零：

| 验证 | VAIO latency 768 | VAIO latency 7168 |
|---|---|---|
| C++ 探针：事件/轮询 × STA/MTA，各重开四次 | 11/16 次原始全零与电平连续矛盾至少 500 ms | 0/16 次；各次 500 ms 后都持续取得振幅约 0.08 的测试音 |
| 项目 Source：连续三次初始化 | 该组三次成功，说明故障有间歇性 | 三次成功，各一次初始化、零重建，采样振幅正确 |

这将当前故障的主要触发条件缩小到 VAIO 内部延迟与回环缓冲的兼容性。正式采集代码和原生后端未因这次参数对照改动。短时原始采集检查仍不代替 AirPlay 完整播放、长时间运行、VoiceMeeter 重启和撕裂声音的听感复测。

Changing only VAIO3 latency eliminated sustained zero-PCM failures in this controlled run: 11/16 at 768 samples versus 0/16 at 7168. Three real Source starts also succeeded without reopening. Full AirPlay playback and long-running stability remain to be checked.

## 保存设置 / Persisting the setting

在 VoiceMeeter 主界面右键正在使用的 VAIO3 虚拟输入标题，选择 `7168 samples (Default)`。仅在独立 VAIO 控制面板修改运行时 latency，可能在音频引擎重启后被 VoiceMeeter 原设置覆盖。不要混淆运行时 `Latency` 与需要重启的 `Max Latency`；这次只改变前者。保存后核对控制面板中 Selected I/O 为 8、Latency 为 7168。

Use the VAIO3 virtual-input caption's latency menu to persist the choice. A runtime-only control-panel change can be overwritten on engine restart; do not confuse current latency with allocated maximum latency.

官方手册第 48–49 页说明 VAIO latency 包含 loopback 路径，需覆盖连接程序最大缓冲的三倍，并应从 VoiceMeeter 的延迟菜单保存。见 [官方 Potato 手册](https://vb-audio.com/Voicemeeter/VoicemeeterPotato_UserManual.pdf)。

## 验证过程 / Evidence and limits

2026-10-09 实机排查：异常会话的原始包为全零，端点电平约 0.39，发送端无丢包记录；这将排查范围缩小到采集路径，但不能单凭日志判断是 WASAPI、驱动还是初始化时序。音乐播放时，对两个活跃播放端点（包含 VoiceMeeter）分别进行了事件驱动和轮询采集，每种方式重开三次，共 12 次；项目 Source 线程另重开六次。均持续取得非零音频，未复现半秒后无声。因此保留原初始化模式，加入上述有界恢复；这些短时采集检查不能代替首次登录、AirPlay 播放和撕裂声音的复测。

后续排查已在独立程序中复现，不能再把上述短时成功当成问题已消除：

- 直接调用 WASAPI 的 Rust 探针共 12 次启动，3 次在前少量非零包之后持续全零。请求缓冲时长为 0 或 100 ms、是否事先枚举全部端点，都出现过失败。
- 失败客户端再保持运行 2 秒仍为全零；同客户端 Stop/Reset/Start、COM 消息派发、额外无声播放流，在对应失败样本中均未恢复。会话音量为 1 且未静音。
- 不依赖 Rust 或项目代码的 C++ 探针，在同一次测试中固定一个活跃 VoiceMeeter 播放端点，交错启动事件/轮询与 STA/MTA 四种组合，各 4 次。11/16 次出现原始全零与即时端点电平大于 0.01 连续矛盾至少 500 ms；四种组合都发生失败。数据包连续到达，不能据此改线程模式或等待方式并宣称修复。

These independent probes reproduce the raw WASAPI failure without Vue, Source, resampling, or native AirPlay transport. All four event/polling and STA/MTA combinations failed in this run; changing those settings is not a demonstrated fix.

用户确认 VoiceMeeter 一直运行，只重开本软件，因此 VoiceMeeter 自身刚启动不是该次现象的前提。随后改播自行生成的普通 WAV 测试音，端点电平约 0.08。Apple Music 进程仍在时，四种组合共 16 次中 11 次复现；用户完全退出 Apple Music 后，测试前后均确认进程不存在，相同组合 16 次仍有 11 次复现。因此本轮证据不支持把 Apple Music 或受保护音频作为根因。微软确实说明受保护音频可能不允许回环采集，但不能仅凭全零就作此判断，见 [微软 Loopback Recording 说明](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)。

VoiceMeeter remained running while only this app was reopened. Ordinary generated WAV playback reproduced the failure with Apple Music fully exited; this run does not support attributing the fault to Apple Music or protected content.

PCM16 自动转换也不是已验证的修复：12 次转换采集中，所有包都有非零字节，但 3 次在 500 ms 后只有约 0.0000305 的量化噪声，没有振幅约 0.08 的测试音；另有间歇丢失。因此不能把转换后的非零字节直接当成恢复证据，也不能据此加入格式降级。项目 Source 的连续 3 次实机初始化取得测试音、未重建，这仅验证这 3 个样本，不能证明之后稳定。

PCM16 conversion can dither silent input into nonzero bytes. Check decoded signal amplitude before claiming recovery: three of twelve converted captures contained only about one PCM16 least-significant bit after warmup. Do not add a format fallback based on byte presence alone.

环境线索：Windows 11 build 26200、VoiceMeeter 3.1.2.2、实际 VAIO 驱动 3.4.1.7；保存的 VAIO latency 数值为 768、LoopBack 为 1。用户截图随后确认运行模式为 SOFT，I/O 8 控制面板实际 latency 为 768、internal SR 为 48 kHz，输入缓冲统计呈红色。官方 2026 年 9 月更新列出的 3.4.1.8 STRICT 同步修复不能直接解释 SOFT 模式的该次故障，见 [官方 VoiceMeeter 更新说明](https://voicemeeter.com/voicemeeter-updates-september-2026/)。

Screenshots confirm SOFT synchronization and live VAIO I/O 8 latency of 768 samples at 48 kHz, with red input-buffer statistics. The vendor's newer STRICT fix is not direct evidence for this SOFT-mode fault.

官方 Potato 手册第 48 页要求 VAIO 内部延迟至少为音频引擎及连接程序最大缓冲的三倍，且延迟也用于 loopback 流。随后仅把正在测试的 VAIO3/I/O 8 latency 调为默认 7168；SOFT、A1/A4 的 ASIO 路由及缓冲保持原设置。用户确认 A1/A4 是 VM 内部/ASIO 路由，不应把其缓冲直接当成此次 WASAPI 回环故障的依据。读取到 WASAPI 包长 480 并不能单独代表驱动全部输入流的最大缓冲；以控制面板实时统计及复测结果核实。此项对照已完成，结果见文首；独立驱动面板中的临时值仍需从主界面保存。见 [官方手册](https://vb-audio.com/Voicemeeter/VoicemeeterPotato_UserManual.pdf)。

Only VAIO3/I/O 8 latency changed in the successful controlled retest. SOFT and the user's A1/A4 ASIO routing were unchanged. The vendor documents a three-buffer latency requirement including loopback; persist the runtime value through VoiceMeeter's main interface.

探针统计只保存包数、电平、等待模式等诊断数值，没有录音和端点标识。正式启动日志中的设备标识已脱敏，不能用相同的脱敏占位符证明两次采集选择了同一个端点。

