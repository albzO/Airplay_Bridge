# Windows → HomePod 架构决定

> 历史开发记录：保留当时的实现与实机测试过程，部分描述已过时。当前架构见 [架构说明](../architecture.md)，使用命令从仓库根目录执行。

日期：2026-10-04。Rust 测试工具 + Windows C/C++ 后端已实施；真实 HomePod 密码控制会话、固定音频、所选采集端点 录音回放和 30 秒实时流均通过，用户确认无杂音。所选采集端点 原生 48 kHz / 双声道 / float32 经 rubato 转为 44.1 kHz / 双声道 / 16 位 PCM。已加入可调延迟、QPC 时间线水位 PI 控制和独立 64 位音频 nonce：8 项普通 Rust 测试、21 项协议模拟测试、真实 所选采集端点 与按时钟消费模拟器的 30 秒漂移控制通过。两小时漂移验证为控制器模拟，HomePod 长时运行及较低延迟仍待实测。命令和状态见 README.md。

## 决定

采用 music-assistant/airplay-cli 的 native AirPlay 2 路径，保留 C/C++ 协议实现，增加薄 Windows 适配层和一个连接探针入口。先完成单 HomePod、密码开启状态下的认证及 session SETUP 验证。

最终进程边界：

```text
Windows Apps → 音频路由软件 → 所选采集端点 recording endpoint（麦克风使用独立录音端点）
                                    ↓
                       Bridge：WASAPI Capture / 格式转换
                                    ↓ binary PCM stdin
                       cliairplay.exe：AirPlay 2 backend
                                    ↓
                                  HomePod
```

PCM 格式必须显式约定采样率、位深、声道、字节序和交错方式。控制通道与 PCM 分开；日志通过 stderr 输出。后续 Windows 控制通道再选 Named Pipe，第一阶段不实现。

Bridge 采用 Rust，GUI 框架留待音频链路成功后决定。有限回放通过 PCM 文件交接；短时实时桥接已通过有界队列、写入线程和二进制 stdin 传输 PCM。后端握手完成后发出 PCM_READY 才开始采集，EOF 请求尾部静音和 TEARDOWN。Ctrl+C 由 Rust 处理，C 后端完成清理。当前不引入跨语言 DLL API，后端进程退出不会直接拖垮未来的采集或 GUI。

长期漂移：发送端按 QPC 定速消耗 44.1 kHz PCM，Rust 根据产生帧数与同一 QPC 时间线计算全管道水位。慢 PI 控制每秒调整 rubato 比例，预热 5 秒，限制 ±800 ppm 及 20 ppm/s；比例块内平滑变化。接收端按 PTP/NTP 锚点安排播放，播放提前量独立于重采样比例，不通过移动锚点修正采集时钟差。歌词时间线需要播放器配合，当前 音频路由软件／采集端点 路径没有向播放器回报 HomePod 延迟的接口。

音量：实时流中 Rust 公告 DACP HTTP 回传服务，将经过目标 IP/Active-Remote 校验的音量命令经独立 localhost TCP 连接送给 C 后端。C 的反馈线程处理 GET_PARAMETER 查询、SET_PARAMETER 设置和设备报告，不将文字控制命令混入 PCM，也不改 PCM 增益。设备报告不自动回写，百分比请求使用 AirPlay 的 -30..0 dB 曲线及 -144 dB 静音值。用户已确认真实 HomePod 顶部按钮可控制音量。

事件：session SETUP 返回 eventPort 后，C 连接该端口并启动独立事件线程。密码配对在清除完整 64 字节 SRP 会话密钥前派生 Events-Salt 密钥；pair-verify 使用 32 字节 X25519 secret。事件使用独立 HAP 上下文、方向密钥和 nonce 计数；组装 TCP/HAP/RTSP 三层消息，解析 /command 的二进制 plist type/value，在事件连接上加密回应 200 并保持原 CSeq。设备信息和播放命令暂只记录及确认收到，不实施播放器控制；未知事件保留兼容性，帧认证或协议边界错误则终止并记录原因。线程在 TEARDOWN 前停止和清理，统计进入 Rust 流报告，避免正常拆会话时将事件 EOF 误报为异常。

## 已核对的上游

- airplay-cli main 下载快照：`8e79242996b7ef52352ee49d390e6db434bf88a6`。
- 它锁定的 libraop：`81c2182649da8645ac2a58b78e9f370c79a4165b`。
- 源码在 `upstream/airplay-cli`。libraop 和所需 crosstools 已检出固定提交；音频相关嵌套子模块未下载。当前最小会话探针已可构建。
- 上游 Makefile 主要按 Linux/macOS 构建，链接 pthread/dl，并引用相应平台的预编译 codec/mdns/OpenSSL 库；不能直接把这些库用于 Windows。
- 已验证 MSYS2 UCRT64 GCC/G++、CMake/Ninja/OpenSSL 与 Rust MSVC 工具链。构建脚本显式设置工具路径，解决当前进程 PATH 未刷新问题。

## 第一阶段最小执行路径

```text
IP/port + TXT 或 mDNS discovery
 → TCP connect
 → GET /info
 → HAP pairing（实际密码，记录所用认证分支）
 → SRP proof verified / control keys derived
 → 加密请求及响应验证
 → 进程内 timing service
 → encrypted session SETUP，要求返回 200
 → 若返回 eventPort，建立相应连接
 → 短暂保持控制会话并验证一次加密控制往返
 → TEARDOWN / 释放 socket、线程和密钥
```

session SETUP 与后面的 audio stream SETUP 是两个不同阶段。探针应停在前者之后，不调用 RECORD 或建立音频流，也不输出表示播放就绪的状态。

`ap2_native_connect()` 目前把认证、timing、session SETUP、event connection、RECORD、stream SETUP 等写在同一流程里；现有 connect API 不是纯认证 API。因此需加一个窄范围的 probe 选项/入口，在 session 接受后返回，并保留专用清理路径，避免复制一份协议实现。

最小运行路径不等于最小编译文件集合：`ap2_client.c` 同时引用 RAOP、ALAC、MRP 等组件。优先用独立构建目标和少量条件编译隔离探针不使用的功能；根据实际符号依赖决定是否仍需编译某些辅助对象。不能只凭几个文件名声称已获得独立可链接后端。

主要复用模块：

| 模块 | 用途 |
|---|---|
| `ap2_client.c` | native 连接顺序、认证分支、RTSP、session SETUP 与清理 |
| `ap2_hap.c` | SRP、pair-verify、密钥派生、HAP 加密 framing |
| `ap2_io.c` | 有截止时间的网络 I/O、状态与响应诊断 |
| `ap2_plist.c` / `ap2_bplist.cpp` / libraop `bplist.cpp` | SETUP binary plist 构造与响应解析 |
| `ap2_ptp.c` | 进程内 PTP/NTP timing，保留上游 HomePod 兼容选择 |
| 必需的 cross/platform/log 工具 | 平台能力、网络辅助和日志；逐项确认嵌套子模块 |

## 密码不能直接等同于兼容成功

当前 `ap2_native_pair()` 会先把用户密码作为 transient SRP secret。被拒绝后，在新 TCP 连接上重试：有保存凭据则 pair-verify，没有则尝试固定 transient PIN。

必须明确记录 password / stored credentials / fixed PIN 三种路径。固定 PIN 成功不能被报告成“用户密码接受”。错误密码测试也不能只看 pairing 返回值，而要看最终 session 是否接受及所用路径。

上游 DESIGN 明确记录实际 HomePod 可能在握手成功后返回 session SETUP 401。因此 `--password` 的存在只证明有密码输入与认证尝试，不证明兼容这台 HomePod。遇到 401/403 时应保存状态行、脱敏响应头、限长响应体和 TLV 错误，判断是否存在额外 challenge 或需要持久配对；不能把所有拒绝都推断成密码拼错。

如果需要完整 HAP pair-setup/保存凭据，应复用上游现有实现，并先核实该 HomePod 可接受的授权方式。AirPlay 访问密码和 HomeKit 配对 PIN 不能任意互换。

## Windows 工具与适配

建议初始目标为 Windows x64，使用 MSYS2 UCRT64 的 MinGW-w64 GCC/G++、CMake、Ninja、OpenSSL 和 winpthreads。若目标是 ARM64，需要调整工具链。

- GCC/G++：与上游 GNU C/C++ 写法更接近，减少编译器迁移噪声。
- CMake/Ninja：新增单独的 Windows probe 构建目标，显式控制源文件与依赖，不迁移全部 Makefile 目标。
- OpenSSL：复用当前密码学调用；使用同一架构和工具链构建的 Windows 库。
- winpthreads：先保留线程/锁接口，减少协议移植改动；不急于重写为 Win32 threads。
- Winsock：原生 TCP/UDP；后端运行时不依赖 WSL 或 MSYS POSIX runtime。可能需要随 exe 分发编译器、OpenSSL 或 winpthreads DLL。

确认的适配点：

| 当前依赖 | Windows 处理 | 原因 |
|---|---|---|
| POSIX socket headers / socket fd | Winsock headers、WSAStartup、SOCKET / INVALID_SOCKET | Windows socket 不是普通文件 fd，64 位句柄不可随意截为 int |
| socket `close/read/write` | `closesocket/recv/send` | socket 和文件 I/O 的语义不同 |
| `fcntl(O_NONBLOCK)` | `ioctlsocket(FIONBIO)` | Windows 不支持用 POSIX fcntl 管 socket |
| `poll` | 经验证的 WSAPoll 或 select 适配 | 要保留超时、取消、部分读写和重试语义 |
| `errno` 网络错误 | WSAGetLastError + 错误映射 | 不能用 POSIX errno 推断 Winsock 状态 |
| `SO_RCVTIMEO/SO_SNDTIMEO` timeval | Windows DWORD 毫秒值 | 不能直接复用 POSIX 参数布局 |
| `clock_gettime`、休眠 | 单调时钟与系统时间分别适配 | 截止时间与 AirPlay/PTP 时间线不能混用；PTP 不能简单改为低精度毫秒 Sleep |
| pthread | winpthreads | 避免第一阶段修改线程模型 |
| `mkfifo` / cmdpipe | 第一阶段排除 | 探针没有外部播放控制需求，无须实现 Named Pipe |
| `shm_open/mmap` / PTP daemon | 第一阶段排除共享模式，保留进程内 timing | 单设备不需要多进程共享时钟；相关引用仍需构建隔离 |
| `SIGPIPE` / Unix 信号 | Windows console Ctrl+C handler 等必要适配 | 要保证退出能够清理会话与线程 |
| 中文名称/参数 | UTF-8 内部格式，Windows 边界转换 | 验证 console、mDNS TXT 和 plist 各层，不能只改终端 code page |

mDNS 用于获得真实端口及 TXT，不能假定所有设备都使用固定端口。先允许明确 IP/port 输入来隔离发现问题；正常路径再接入 `_airplay._tcp` 及必要的 `_raop._tcp` 发现。不急于另写一套发现库，先审核上游 libmdns/crosstools 的 Windows 路径。

## 执行顺序与验收

1. **冻结源码与依赖版本**：完成嵌套子模块/依赖清单、许可证检查、Windows 工具链确认，保存 HomePod 型号、固件、端口、TXT 和网络接口信息。目的是让错误可复现。
2. **建立最小 probe 构建目标**：添加探针入口及停在 session SETUP 后的分支。可以尝试原 Makefile 来收集障碍，但不把完整二进制移植作为前提。
3. **完成平台 I/O 与密码学链接**：先能 TCP connect、GET /info，再推进 SRP；逐个修复阻塞问题，不混入采集或 GUI。
4. **验证密码认证与加密往返**：真实密码、错误密码、未提供密码，记录认证分支与 HTTP/TLV 结果。密钥派生不等于加密通信已经有效。
5. **验证真实 session SETUP**：按设备能力选择 timing，HomePod 路径优先保留 PTP；NTP 只作明确标记的诊断/上游兼容选择，不预设它必然适用。200 后检查响应、event connection，保持短暂会话并清理。分别报告 pairing、session 接受和稳定性结果。
6. **测试固定 PCM**：只有第一阶段完成后才加 RECORD、stream SETUP、ALAC、RTP/timing 播放验证。验收是 HomePod 实际出声，而非发送包成功。
7. **接入 音频路由软件**：枚举 recording endpoints，通过持久 endpoint ID 选择 录音端点，WASAPI capture，按后端格式转换，使用有界缓冲和二进制 stdin。音频路由软件 管路由/混音，Bridge 管采集与传输。验收含长时间稳定性、断线、采样率差异和缓冲漂移。
8. **GUI**：设备、音源、密码、连接状态和脱敏日志；框架此时再决定。

第一阶段通过条件：记录设备身份和实际密码要求；SRP/所选 HAP 路径成功；至少一次有效加密响应；session SETUP 200；事件连接如适用；短暂控制会话保持和退出结果明确。输出 `SESSION_ACCEPTED`，不声称已经能够播放音频。

当前已实现的命令接口：

```powershell
.\dist\homepod-test.exe discover
.\dist\homepod-test.exe test --device "Receiver A"
```

保留 `--password <password>` 兼容入口；交互测试推荐隐藏输入。日志不输出密码、长期凭据、共享 secret 或派生密钥，认证响应体做敏感字段脱敏。构建成功后依据真实 `--help` 提供可执行命令。

## 所需实机信息

开发环境需要编译工具链与 Windows 依赖。实机验证需要 HomePod IP/发现到的端口、型号与固件版本、网络可达性、Require Password 保持开启、用户在本机输入实际密码，以及 AirPlay 访问控制配置。错误密码或授权配置问题不能凭静态源码代替实机判断。

## 参考

- https://github.com/music-assistant/airplay-cli
- https://github.com/music-assistant/airplay-cli/blob/8e79242996b7ef52352ee49d390e6db434bf88a6/src/ap2_client.c
- https://github.com/music-assistant/airplay-cli/blob/8e79242996b7ef52352ee49d390e6db434bf88a6/src/ap2_hap.c
- https://github.com/music-assistant/airplay-cli/blob/8e79242996b7ef52352ee49d390e6db434bf88a6/Makefile
- https://github.com/music-assistant/airplay-cli/blob/8e79242996b7ef52352ee49d390e6db434bf88a6/DESIGN.md
- https://www.msys2.org/docs/environments/


## GUI 持续采集（2026-10-05）

桌面界面由 Vue + Tauri 承载。`airplay-core/src/source.rs` 独占一个持续 WASAPI 采集器，同一来源的电平预览和串流共享数据；GUI 开始/停止串流只订阅/解除发送分支。CLI 仍可独立采集。订阅后新建转采样、漂移控制、有界 PCM 队列和原生发送会话，避免把认证期间的旧音频送出。短且可信的录音缺口补静音并计数；严重异常明确终止。输入映射与扬声器位置分开保存，日志和窗口关闭行为集中在设置页。逐包诊断滚动保留最近四个片段，故障时保留最新现场。
