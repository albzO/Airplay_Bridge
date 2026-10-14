# 代码职责、复杂度与模块化审阅

这份文档用于回答三个问题：每个文件做什么、项目难维护在哪里、下一步应如何划分模块。先看整体结论和拆分顺序，再按目录查逐文件记录。所有分析集中在本文，尚未执行代码重构。

**阅读导航**：[项目全貌](#overview) · [复杂度重点](#hotspots) · [解耦顺序](#roadmap) · [范围与方法](#method) · [逐文件记录](#files) · [核对与验证边界](#verification)

<a id="overview"></a>

## 项目全貌

**整体维护复杂度：高。主要成本来自跨线程、跨进程、跨语言的状态与资源生命周期。** 音频解码、时间线、漂移控制、队列等局部模块已经有较清晰的边界；复杂度集中在把它们组织起来的 Source、Live、桌面启动和 App 页面控制逻辑。继续拆文件有价值，但必须同时明确状态所有者、调用方向和停止协议。

本次逐文件记录 **166 个自维护代码、测试、脚本、补丁及配置文件，共 24,534 物理行**。这不是纯业务代码行数：包含空行、注释、补丁上下文、测试和 JSON。第三方上游源文件不计入该数字，未逐文件审阅。

| 当前目录 | 文件数 | 物理行数 | 实际职责 | 维护重点 |
|---|---:|---:|---|---|
| 根目录配置 | 1 | 7 | 格式化规范 | 保持统一即可 |
| airplay-backend/ | 25 | 2,773 | 上游提取/补丁、Windows 移植、认证/RTSP、音频发送 | 共享 C 上下文、片段式集成与补丁升级成本 |
| airplay-core/ | 33 | 6,848 | WASAPI、持续采集、DSP、后端进程、日志及 CLI | 采集与会话的两种生命周期、资源关闭顺序 |
| airplay-frontend/（不含 src-tauri） | 21 | 3,818 | Vue 界面、IPC 校验、会话/诊断状态 | App 编排、异步回滚、全局样式层叠 |
| airplay-frontend/src-tauri/ | 15 | 1,969 | 桌面服务、命令、配置、认证管道、窗口/托盘 | Engine 中心耦合、准备阶段取消与提交 |
| scripts/ | 6 | 539 | 构建、敏感数据检查、发布和两套安装器 | 发布清单与模式规则重复 |
| test/ | 64 | 7,998 | 单元、模拟协议、浏览器、实机与性能检查 | 测试基础设施和场景代码也需要分层 |
| docs/error-codes.json | 1 | 582 | Rust/TS 共享错误契约 | 可执行资源位置、文本匹配规则 |
| **合计** | **166** | **24,534** | | |

当前主要关系可以按三条链理解：

| 链路 | 调用或数据流 | 必须保持的边界 |
|---|---|---|
| 桌面控制 | Vue → Tauri commands → 配置/会话准备 → core::live → 原生后端进程 | 命令确认、准备完成、会话终态是不同事件；旧会话不能提交到新会话 |
| 实时音频 | WASAPI → 持续 Source → 订阅队列 → AudioPipeline/重采样 → PCM writer → 原生后端 → 接收端 | Source 可跨串流存活；收到 PCM_READY 后才挂接；队列预算包含在途工作 |
| 状态与诊断 | 原生日志标记/采集状态 → Rust 解析与汇总 → Tauri 事件 → TS 校验 → 页面状态 | 控制事件不能被慢日志阻塞；电平可合并，终态不可随意丢弃；展示前脱敏 |

认证成功、加密响应成功、会话 SETUP 成功、音频传输成功应继续分别报告。模拟器和本机测试覆盖其中部分机制，不应合并成“真实设备播放已验证”。

<a id="hotspots"></a>

## 复杂度重点

逐文件人工评级中有 **10 个“很高”条目：9 个生产/补丁文件和 1 个模拟测试文件**。不对这些评级求平均分：一个低行数的共享上下文也可能比长 JSON 更难修改。

| 优先关注对象 | 复杂度来源 | 建议模块边界 | 不宜采用的拆法 |
|---|---|---|---|
| core/src/source.rs | 预热、恢复、长期采集、单订阅、通知和停止共享状态 | Warmup、RecoveryPolicy、Subscription、SourceSupervisor | 按函数数量平均切成多个依赖同一大对象的文件 |
| core/src/live.rs | 目标校验、资源获取、启动、DSP、停止、报告集中编排 | TargetPlan、SessionResources、ShutdownCoordinator、ReportSnapshot | 将关闭逻辑散落到各调用者且分别重置超时 |
| core/src/capture.rs | 900 行中混合设备枚举、格式、COM、采集循环、录音落盘 | endpoint、format、WASAPI session、capture runner、recording | 重新揉合已经独立的 timeline/health/decode |
| src-tauri/src/main.rs 与 stream_start.rs | Engine 中心状态、命令适配、准备事务、watchdog 和终态发布 | AppState、SourceManager、SessionService、thin commands | 每个命令继续直接锁整个 Engine 并执行慢操作 |
| src/App.vue | 设置确认/回滚、预览、设备选择、首屏生命周期交叉 | useSettingsTransaction、useSourcePreview、useDeviceSelection、useDesktopLifecycle | 只拆模板而继续由 App 控制所有状态 |
| backend 的 probe_context/probe_entry/audio_entry | 共享字段、上游私有结构、双端/PTP/线程清理和发送节奏 | 字段所有权表、分组 context、PCM input、scheduler、feedback | 仅把 .inc 改名为 .c 而没有接口与所有权变化 |
| backend/patches/0003-session-and-realtime.patch | 多种协议行为和实时逻辑在同一补丁演进 | 按认证/会话/时钟/音频主题审查补丁 | 升级时直接覆盖生成代码 |
| src/style.css | 1,349 行含重复层叠和历史布局规则，影响面隐蔽 | 先去重，再分 tokens/base/layout/component styles | 原样切成多个文件，继续依赖引入顺序覆盖 |
| mock_receiver.py 与大型前端测试 | 协议模拟、场景配置、驱动、断言和资源管理集中 | 独立协议判定器、fixture、按行为分组的用例 | 让 mock 直接复用生产算法得出预期结果 |

表内 core/backend/src 等缩写用于阅读，完整路径与具体依据见逐文件条目。

已经值得保留的设计包括：audio_queue 的时间配额与 RAII、capture 下的纯规则模块、独立 drift 控制器、统一 shutdown 期限、日志与协议控制隔离、TS 运行时解码、会话 ID 隔离，以及本工作区新增的 Source 通知线程。这些模块存在复杂逻辑，但边界本身合理。

需要优先消除的结构性耦合有三处：

1. **生产构建反向依赖 test/**：桌面 main 非 cfg(test) 引入 startup.rs，后端也编入 nonce_selfcheck.c。先把运行时探针/自检归入明确生产模块，再让测试调用它们。
2. **通用能力挂在高层业务目录**：Source 依赖 live::diagnostics。将日志后台写入能力放独立 diagnostics 模块，Source/Live 只提供各自事件，避免目录依赖方向倒置。
3. **跨层契约依赖字符串与重复解释**：原生标记、Rust 解析、TS nativeFields、共享错误表和测试桩必须同步。建立清晰的 NativeEvent/桌面 DTO 边界；先集中解析与版本约束，再考虑生成类型，不要求一次更换整套 IPC。

<a id="roadmap"></a>

## 建议的解耦顺序

以下是后续开发路线，不代表本次已经修改实现。先在现有 crate/前端工程内形成模块，再根据依赖稳定性决定是否拆独立 crate；过早增加工程数量会增加构建和公共 API 维护成本。

| 阶段 | 具体改动 | 完成判据 | 重点回归 |
|---|---|---|---|
| 1：整理边界 | 迁移生产自检/启动探针，抽出通用 diagnostics；明确共享错误表归属 | 生产构建无需从 test 取实现；Source 不再依赖 live 的日志实现 | 后端 self-check、桌面 smoke、日志限时/轮转/脱敏 |
| 2：拆纯规则与平台适配 | capture 的 endpoint/format/COM session/runner 分离；Source 的 warmup/recovery 先独立 | 每个状态对象有明确所有者；纯规则可无设备验证 | decode/timeline/health/clock、预热和恢复用例 |
| 3：收拢会话生命周期 | live 启动资源守卫、停止协调器、报告快照；Tauri SessionService/SourceManager | 每阶段获取/提交/回滚明确；重复停止只缩短期限；未完成清理不能启动新资源 | session、shutdown、transport、report、旧结果/取消测试 |
| 4：拆页面控制逻辑 | 抽设置事务/来源预览/设备选择/页面生命周期，再拆模板和样式 | App 只组合页面；持久化成功前后语义不变；每状态单一写入口 | Node session/protocol/diagnostics + Playwright DOM |
| 5：降低原生与测试升级成本 | 定义 C 子上下文所有权，分离发送/反馈；按主题整理补丁和模拟器；共享发布清单 | 上游固定输入/输出可复现；场景配置不靠字符串隐式组合；安装器规则明确 | upstream 哈希、认证/SETUP/双端/回绕；安装与卸载单独验收 |

每阶段先保持行为一致，再单独优化性能或调整策略。例如重构 Source 时不要同时修改预热时长、恢复次数和队列预算，否则回归难以归因。

建议模块之间只交换必要信息：采集层输出带格式/位置的音频块；订阅层负责配额与取消；DSP 层只处理样本和校正量；传输层负责进程/管道；会话层拥有资源并统筹关闭；诊断层接收快照；桌面层负责系统交互；Vue 通过经过校验的 DTO 更新状态。避免下层回调直接操作 UI、保存设置或推断业务错误类别。

判断一次拆分是否有效，可以记录四项变化：一次功能修改需触及的模块数、跨模块可变状态数量、关键路径持锁范围、同一行为的重复实现数量。文件更短只是辅助指标；不能以删除测试或丢失错误信息来换取行数下降。

<a id="method"></a>

## 范围与方法

- 审阅日期：2026-10-13 至 2026-10-14；主仓库基线：`a6d0cb1bafb985af12199d339f4f828ca6468004`。分析对象是当前工作区，包含已有未提交的 source/notifications 改动；不等同于该提交的纯净快照。
- 逐文件执行 **读取 → 分析 → 写入本文 → 读取下一文件**。按目录顺序记录，最后汇总跨模块结论。本文为静态维护性审阅，不是运行性能测量或测试通过证明。
- 覆盖自维护生产代码、测试代码、构建/安装脚本、代码补丁及关键配置。排除生成目录、依赖缓存、发布二进制、锁文件、图标和普通说明文档。第三方 `upstream/airplay-cli` 单独作为依赖边界，不能视作已逐文件审阅。
- **规模**：每个条目的行数为物理行数（含空行与注释），只衡量阅读量。
- **复杂度**：低/中/高/很高是人工维护性评级，不是计算得出的圈复杂度。综合分支与协议逻辑、可变状态、线程/异步/资源生命周期、跨层依赖及修改影响范围；测试代码的高复杂度也可能来自场景数量而非产品设计缺陷。
- 每个文件记录职责、复杂度依据、依赖/边界和建议；算法复杂度仅在可清楚确定且有意义时补充。建议不是已经证实的缺陷。

<a id="files"></a>

## 逐文件记录

### 根目录配置

#### `.prettierrc.json` — 7 行

- **实现**：统一前端格式化的引号、分号、缩进、行宽和尾逗号。
- **复杂度：低**。规范文件本身没有运行逻辑；自动格式化能减少无意义 diff，但不能替代职责拆分。

### airplay-backend/

#### `airplay-backend/CMakeLists.txt` — 63 行

- **实现**：限定 Windows MinGW/C17/C++17 环境；读取上游输入和补丁清单，追踪配置依赖；运行 Python 生成器，再编译协议、音频及 Windows 适配代码，链接 OpenSSL、pthread 和系统网络库。
- **复杂度：中**。主要风险是构建依赖图：配置阶段生成代码、三级子模块 HEAD、清单与补丁顺序共同决定产物。
- **依赖与解耦**：构建同时引入生产源和 nonce 自检源；建议将“上游生成”“平台适配”“协议程序”整理为明确的 CMake 目标，保持输入变化必重生成的约束。

#### `airplay-backend/audio_entry.inc` — 210 行

- **实现**：反馈保活线程、音量轮询、管道分段读取、PCM 预缓冲、352 帧节拍发送、双设备同步锚点，以及文件/测试音播放和结束静音。
- **复杂度：很高**。同一片段处理线程停止、EOF/超时、实时调度、双接收端健康状态与资源释放；音频处理随帧数线性增长，但维护难点是时间和生命周期。
- **依赖与解耦**：直接访问上游 `ap2cl_s`，依赖 Win32、pthread、RTSP/RTP 和音量模块。优先拆出 PCM 输入、发送调度、反馈生命周期；保留迟到失败和双端共同失败语义。

#### `airplay-backend/audio_nonce.c` — 12 行

- **实现**：把独立 64 位计数器编码为 12 字节 AEAD nonce，计数耗尽时拒绝继续。
- **复杂度：低；正确性敏感度高**。固定循环 O(1)，但不能与 16 位 RTP 序号混用。
- **依赖与解耦**：仅依赖接口头和内存清零；已是适合独立测试的纯逻辑边界。

#### `airplay-backend/auth_auto.inc` — 46 行

- **实现**：按 HAP/HTTP 错误分类自动配对失败；仅可信的 M4 密钥拒绝触发密码请求，重置连接后重试一次。
- **复杂度：中**。逻辑短，但认证策略、数字错误码和连接重置相互绑定。
- **依赖与解耦**：依赖上游配对结构、密码回调及日志脱敏；可把错误分类提取为纯映射，用命名枚举替代裸数字。

#### `airplay-backend/events_entry.inc` — 175 行

- **实现**：独立反向事件通道的加密帧解包、RTSP/HTTP 请求解析、长度/CSeq 校验、bplist 命令摘要、加密 ACK，以及线程启停与统计。
- **复杂度：高**。流式半包、多请求缓存、认证失败和 socket 生命周期交织；命令只记录不执行。
- **依赖与解耦**：依赖 HAP、bplist、socket 和上游会话状态。优先将有界帧解析/请求解析提取成无 socket 的解析器，工作线程只负责输入输出和会话健康上报。

#### `airplay-backend/logging.c` — 40 行

- **实现**：提供上游日志符号，持有密码独立副本，用互斥锁保护脱敏输出，清零临时缓冲区。
- **复杂度：中**。字符串替换简单，但跨线程秘密生命周期和全局日志级别增加耦合。
- **依赖与解耦**：依赖 cross_log、pthread、Win32 清零；保持统一输出入口，后续可将日志配置和秘密存储封装为明确上下文。

#### `airplay-backend/main.c` — 233 行

- **实现**：Windows Unicode 命令行转换、参数/组合校验、自检入口、隐藏密码输入与命名管道密码交换、Winsock 初始化及退出清理。
- **复杂度：高**。大量选项组合、两套隐藏密码读取代码、秘密缓冲区和多个 goto 清理路径。
- **依赖与解耦**：入口依赖 Win32 控制台/管道及 probe 接口。建议拆分 options 解析校验、password provider 和进程入口；保持密码与 PCM 不争用 stdin 的约束。

#### `airplay-backend/patches/0001-windows-platform.patch` — 425 行

- **实现**：将上游 HAP/I/O/PTP/plist 移植到 Windows：统一平台头、socket 宽度、替换 POSIX I/O、PTP 停止原子标志和字节序适配。
- **复杂度：高**。运行逻辑散布多个上游文件，升级时需要维护签名一致性、线程退出和 Windows socket 语义。
- **依赖与解耦**：与固定上游版本及 windows_port/windows_io 紧密绑定；保持平台补丁单独分组，通过清单和生成器验证，不应直接维护生成结果。

#### `airplay-backend/patches/0002-hap-events-and-errors.patch` — 128 行

- **实现**：HAP 派生独立事件通道读写密钥与计数器，记录 M2/M4 阶段，补充配对进度事件并关闭失败响应体转储。
- **复杂度：高**。改动量有限，但涉及方向相反的事件密钥与 SRP/X25519 两条认证路径。
- **依赖与解耦**：与认证错误分类、事件通道和上游 HAP 内部结构绑定；维持独立密码学上下文，升级时针对两种密钥来源验证。

#### `airplay-backend/patches/0003-session-and-realtime.patch` — 447 行

- **实现**：分开会话 SETUP 与音频 SETUP，接入自动认证和阶段日志、共享 PTP、RECORD/SETPEERS 严格成功检查，裁剪 buffered/MRP 路径，接入实时 ALAC 与独立 nonce。
- **复杂度：很高**。同时改变认证、会话、时序、数据面和编码路径，是上游升级影响面最大的补丁。
- **依赖与解耦**：与生成器提取区段、多个 .inc 片段和上游私有字段共同演化。建议按认证/会话建立/实时数据面拆分补丁，并保留顺序及完整生成验证；拆文件不应改变协议顺序。

#### `airplay-backend/pcm_source.c` — 18 行

- **实现**：以 UTF-8 路径打开 Windows PCM 文件，验证非空、4 字节帧对齐及不超过 60 秒，返回帧数。
- **复杂度：低**。固定格式、明确资源失败清理，读取文件元数据 O(1)。
- **依赖与解耦**：Win32 路径转换及 stdio；已经独立，格式常量可与调用端统一声明。

#### `airplay-backend/probe.h` — 24 行

- **实现**：定义运行选项、密码回调和运行/日志/PCM/nonce 对外接口。
- **复杂度：低（接口耦合中）**。无算法，但扁平选项同时承载设备、认证、音频和双设备参数。
- **依赖与解耦**：可按 device/auth/audio/group 拆成选项子结构，降低新增功能对所有调用者的影响。

#### `airplay-backend/probe_context.inc` — 83 行

- **实现**：为抽取的上游实现重建类型、时钟工具、常量及完整会话上下文。
- **复杂度：高（结构性）**。一个结构拥有认证、RTSP、PTP、音频、重传、事件、反馈、音量和双端引用；多个线程共享部分字段。
- **依赖与解耦**：这是 .inc 集成的核心耦合点。建议先明确字段所有者/线程读写规则，再引入子上下文；独立字段分组比机械拆文件更有效。

#### `airplay-backend/probe_entry.inc` — 219 行

- **实现**：初始化/销毁会话资源，运行单设备或双设备连接，认证复用、音频建立、共同延迟选择、SETPEERS、hold/TEARDOWN 与退出码映射。
- **复杂度：很高**。部分初始化失败、多个线程、主从 PTP 借用关系、双设备回滚及错误优先级都由此协调。
- **依赖与解耦**：直接操作整个会话结构。建议提取会话资源拥有者和 group coordinator；明确借用者先释放、错误不可被清理结果覆盖等契约。

#### `airplay-backend/raw_alac.h` — 8 行

- **实现**：声明 opaque ALAC 编码器及 PCM 编码接口。
- **复杂度：低**。边界清晰；建议补充输出缓冲区释放责任和固定格式契约，降低调用误用。

#### `airplay-backend/raw_codec.c` — 15 行

- **实现**：仅接受 352 帧/44.1kHz/16-bit/双声道的编码器包装，调用生成的 raw ALAC 编码函数。
- **复杂度：低**。格式校验与小对象生命周期，编码成本由下层承担。
- **依赖与解耦**：已隔离编码接口；将来支持更多格式时扩展此适配层，避免发送器直接感知编码实现。

#### `airplay-backend/select_upstream.py` — 138 行

- **实现**：从已补丁化副本中按函数名抽取 C 实现，识别字符串/注释中的括号，提取常量并拼接本地片段，输出最小 Windows 源集和 raw ALAC。
- **复杂度：高**。自制轻量 C 函数扫描器、字符串边界和拼接顺序形成隐式构建接口；对每个指定函数重复扫描源码。
- **依赖与解耦**：依赖 UpstreamGuard、固定符号和 PTP 注释标记。优先维护提取清单/生成结果契约测试；长期可推动上游公开库边界，减少按源码文本裁剪。

#### `airplay-backend/shm_disabled.c` — 9 行

- **实现**：为未支持的共享 PTP daemon 提供明确返回失败的链接桩。
- **复杂度：低**。无状态；应保持“能力不可用”可见，不能把桩函数存在误当成多进程共享时钟已实现。

#### `airplay-backend/upstream-manifest.json` — 55 行

- **实现**：固定三级上游 revision、输入摘要、补丁顺序/摘要和生成输出摘要。
- **复杂度：低（变更协调成本高）**。是可复现构建契约，无运行算法。
- **依赖与解耦**：生成器、构建系统和升级测试共享此清单；任何补丁/片段变动必须同步输出摘要，避免维护第二份 pin 来源。

#### `airplay-backend/upstream_guard.py` — 72 行

- **实现**：检查源/补丁摘要及 Git revision，在临时副本依次检查并应用补丁，生成 selection-report。
- **复杂度：中**。重点是临时目录、Git 环境隔离、换行归一化和失败路径；摘要工作随输入字节数线性增长。
- **依赖与解耦**：依赖 Git CLI 和 manifest；finish 输出实际摘要，本文件本身不对照 manifest.outputs 拒绝差异，输出一致性需由外部验证承担。

#### `airplay-backend/volume_entry.inc` — 91 行

- **实现**：连接本地音量桥，查询当前音量，处理 REPORT/SET/STEP，换算百分比与 dB，并发送 RTSP 参数。
- **复杂度：高**。行协议缓存、非阻塞 socket、未知音量、避免回报回环和远端确认状态混合。
- **依赖与解耦**：由反馈线程驱动，直接更新会话字段；建议提取纯音量命令/换算层，传输层只执行命令并反馈结果。

#### `airplay-backend/windows_audio.inc` — 7 行

- **实现**：设置 socket 非阻塞并转换/记录 Windows 错误。
- **复杂度：低**。平台辅助函数；更适合归入统一 socket 适配接口，避免按音频命名却被音量等路径复用。

#### `airplay-backend/windows_io.inc` — 29 行

- **实现**：有 deadline 的 socket 完整写入和单次读取，处理短写、可重试错误及 EOF。
- **复杂度：中**。小循环承担网络超时契约；设置非阻塞后不恢复原模式，需要调用者遵循统一语义。
- **依赖与解耦**：依赖 ap2_io_poll_fd 和 Windows errno 映射；可独立为平台 I/O 单元，便于模拟短写/超时。

#### `airplay-backend/windows_port.c` — 145 行

- **实现**：Winsock 到 POSIX 风格的适配、错误映射、8 秒连接/写入 deadline、超时选项转换、QPC/墙钟、睡眠、字符串搜索和网卡 MAC 获取。
- **复杂度：高**。跨操作系统语义转换和阻塞模式切换比包装函数数量更重要。
- **依赖与解耦**：平台功能集中但职责较多；可按 socket、clock、network identity 分组，先统一两个完整写入实现的阻塞模式契约。

#### `airplay-backend/windows_port.h` — 81 行

- **实现**：统一平台 include、指针宽度 socket 类型、C ABI 声明和 POSIX 名称宏替换。
- **复杂度：中（影响面高）**。大量宏隐式改变调用含义，include 顺序和实现保护宏影响编译。
- **依赖与解耦**：限制在上游适配翻译单元内部使用，避免继续向新模块传播 read/write/close 等通用宏。

### airplay-core/

#### `airplay-core/Cargo.toml` — 13 行

- **实现**：定义 homepod-test 包与 mDNS、serde、重采样和 Windows API 依赖。
- **复杂度：低**。平台能力、音频算法和序列化同属一个 crate；后续模块化先通过内部接口隔离，无需立即拆多个 crate。

#### `airplay-core/src/audio_queue.rs` — 108 行

- **实现**：有界音频通道、按帧时长预留预算、峰值统计、RAII 自动归还额度，先预留再创建数据。
- **复杂度：中**。原子更新与 Drop 共同保证失败/消费中数据预算；每个 channel 拥有自己的统计对象，两级使用同一 640ms 策略，并非一个全局共享额度。
- **依赖与解耦**：仅标准库；已有良好泛型边界，保留非阻塞、在途块计入预算的契约。

#### `airplay-core/src/backend.rs` — 359 行

- **实现**：定位后端程序、校验转换报告/PCM、按发现结果选择设备、启动有限测试子进程、收集脱敏日志与阶段标记、输出认证/会话/传输结果。
- **复杂度：高**。主 test 函数横跨输入验证、进程生命周期、日志消费和 CLI 展示，长流程的错误退出需同步考虑 reader 和 child。
- **依赖与解耦**：discovery/privacy/log_store/failure + 文件系统/子进程。建议提取 BackendLocator、测试执行器和结构化结果报告，CLI 只负责打印。

#### `airplay-core/src/capture.rs` — 900 行

- **实现**：COM/HANDLE/音频包 RAII、端点枚举与默认设备、WAVE 格式解析、WASAPI 初始化、实时采集调度、静音补偿/恢复衔接，以及 WAV 录音和报告。
- **复杂度：很高**。Windows unsafe 边界、事件与轮询回退、音频时间线、健康监测、诊断回调和文件输出集中；实时函数参数逐层累加，record 路径还重复一套采集统计。
- **依赖与解耦**：依赖 capture 七个子模块及外部 sink。优先分离 endpoint/format、WASAPI session/packet 守卫、实时 runner、WAV recorder；以 CaptureOptions/Observers 减少位置参数。每包解码/测量 O(帧数×声道数)，不得让 sink 阻塞网络。

#### `airplay-core/src/capture/clock.rs` — 38 行

- **实现**：QPC 频率/计数读取和 100ns 单位换算，区分时间线失败与诊断降级。
- **复杂度：低**。O(1)，边界有效性明确；可用具名时间单位类型避免 ticks 与 100ns 混用。

#### `airplay-core/src/capture/decode.rs` — 79 行

- **实现**：PCM8/16/24/32 与 float32 解码，帧窗口/声道选择到立体声，原始包峰值/非有限值摘要。
- **复杂度：中**。字节宽度、符号扩展和范围检查重要；映射解码 O(选中帧数)，全通道诊断 O(帧数×声道数)。
- **依赖与解耦**：纯字节逻辑但依赖父模块 Format 的验证前提；适合与 format 放在平台无关模块。

#### `airplay-core/src/capture/diagnostics.rs` — 157 行

- **实现**：采集阶段/包/补静音/时间线故障事件、时间差、端点快照及最终 JSON 报告。
- **复杂度：中**。字段多、回调和动态启用策略需一致；部分重要事件只检查 callback 是否存在，包级详情才使用 active。
- **依赖与解耦**：依赖采集内部类型和 IAudioClient；可让上层传入启动快照，移除诊断层对 COM 的直接依赖，并逐步用 typed event 固定字段契约。

#### `airplay-core/src/capture/health.rs` — 169 行

- **实现**：缓存端点电平/静音/音量读数；启动期间比较端点可听输出与原始包是否全零；500ms 持续矛盾产生可恢复 LoopbackStalled，稳定后永久结束检查。
- **复杂度：高**。时序状态机、COM 查询缺失、正常静音和启动误判需区分。
- **依赖与解耦**：纯 LoopbackHealth 已与监测器部分分离；可进一步让监测/进度写入成为适配器，保留策略输入可注入时间以便确定性测试。

#### `airplay-core/src/capture/metrics.rs` — 73 行

- **实现**：立体声峰值/有信号帧计数、可选样本指纹，记录诊断包间接收/设备/QPC 差值。
- **复杂度：低**。样本统计 O(n)、时差 O(1)，不修改音频，无 WASAPI 依赖；已是可复用纯逻辑模块。

#### `airplay-core/src/capture/state.rs` — 101 行

- **实现**：音频包标志、观察统计、成功交付计数和互斥保护的实时进度更新。
- **复杂度：中**。两套统计具有不同语义：报告排除首包 discontinuity，实时进度保留；观察成功不等于 sink 交付成功。
- **依赖与解耦**：对 Windows 的依赖限于标志常量；维持 observe 与 delivered 的明确分界，避免后续合并统计时丢失语义。

#### `airplay-core/src/capture/timeline.rs` — 168 行

- **实现**：制定采集包跳过/使用/前置静音计划，验证设备位置与 QPC，一分钟内有限缺口修复，空闲补静音和恢复后的重叠修剪。
- **复杂度：高（领域复杂度）**。多时钟对齐和修复预算不可随意简化；历史最多数个修复记录，单包计算有界。
- **依赖与解耦**：不访问 WASAPI，时间可注入，已是理想的独立状态机；优先保持并扩充边界测试，不建议继续碎拆。

#### `airplay-core/src/convert.rs` — 321 行

- **实现**：有状态 sinc 重采样、TPDF 抖动/i16 量化、±800ppm 平滑比例调整、尾帧精确冲刷，以及 WAV→PCM/WAV 转换 CLI 功能。
- **复杂度：高**。滤波器延迟、跨包缓存、自适应积分时长和量化统计需要共同保持；固定滤波参数下成本随样本数增长。
- **依赖与解耦**：rubato/audioadapter + capture 格式解析 + 文件 I/O。建议把 wave 文件适配和报告移出 Converter，保持 DSP 连续状态不可按输入包重建。

#### `airplay-core/src/data_dir.rs` — 87 行

- **实现**：按可执行文件旁 portable.flag 选择便携 data，否则用 AppData/AirPlay Hub；迁移旧目录时仅复制缺失文件，失败清理半成品。
- **复杂度：中**。目录模式、迁移标记、递归复制和不覆盖已有数据的约束；成本随迁移文件量增长。
- **依赖与解耦**：仅标准库，GUI/CLI 共用；职责集中，建议保留独立存储策略边界。

#### `airplay-core/src/discovery.rs` — 140 行

- **实现**：浏览 AirPlay/RAOP 服务，按地址合并能力和密码要求，输出设备列表到 devices.json。
- **复杂度：中**。网络事件、两个服务来源合并、超时和持久化在一条流程；设备与 RAOP 双重遍历约 O(D×R×地址比较)。
- **依赖与解耦**：mdns-sd、文件系统和 CLI 打印。建议 discovery 返回 Vec<Device>，把存储/展示放适配层，便于 GUI 无需文件往返。

#### `airplay-core/src/drift.rs` — 71 行

- **实现**：以总 PCM 水位为输入的 PI 漂移控制，启动五次学习基准、低通滤波、抗积分饱和和 ppm 变化限速。
- **复杂度：中（算法理解成本高）**。每次 O(1)，状态少但参数含时钟领域意义。
- **依赖与解耦**：只依赖 serde，无设备/线程；保持此纯控制器，通过外部提供水位与 dt。

#### `airplay-core/src/failure.rs` — 139 行

- **实现**：编译时嵌入错误目录，分类应用错误、解析 Win32/Winsock/HRESULT 来源，生成解释和退出码，校验后端 ERROR 行的代码/退出码。
- **复杂度：中**。依赖字符串模式优先级和多层错误命名空间；匹配成本随目录条数和字符串长度增长。
- **依赖与解耦**：docs/error-codes.json 实际是运行时契约来源，不能当普通文档。可保留兼容文本解析，同时让新内部调用传 typed error，减少再次猜测字符串。

#### `airplay-core/src/lib.rs` — 14 行

- **实现**：crate 模块导出，audio_queue 私有，其余主要模块公开。
- **复杂度：低（公共 API 面较宽）**。可逐步形成少量 facade/re-export，减少桌面层依赖内部实现模块，但无需一次隐藏全部现有 API。

#### `airplay-core/src/live.rs` — 550 行

- **实现**：GUI/CLI 实时串流总编排：目标校验、日志和 DACP、后端启动/就绪、Source 或直接采集、转换管线、统一停止预算及完整报告。
- **复杂度：很高**。run_targets 贯穿资源创建到销毁，GUI/CLI 与单/双设备组合，加上大量诊断字段和失败优先级，使修改影响面很大。
- **依赖与解耦**：连接核心几乎全部服务。优先提取 TargetPlan、SessionResources/启动事务、ShutdownCoordinator、ReportSnapshot；保留 PCM_READY 后才订阅和同一 deadline 收尾的约束。

#### `airplay-core/src/live/control.rs` — 96 行

- **实现**：GUI 控制面/会话依赖结构、停止预算、音量通道、映射/交换标志，以及 CLI Ctrl+C 注册守卫。
- **复杂度：中**。Arc/Mutex/原子状态混合，CLI 使用全局停止状态；cleanup_complete 是桌面会话保留资源的契约。
- **依赖与解耦**：控制契约包含 Source 和 JSON emitter；可分离平台无关 StreamControl 与 Windows console adapter，降低核心对 GUI 命名的依赖。

#### `airplay-core/src/live/diagnostics.rs` — 286 行

- **实现**：泛型非阻塞日志 worker、有界队列/丢弃计数、Source/包/漂移多种落盘策略、轮转、统一 deadline 收尾。
- **复杂度：高**。慢磁盘隔离、发送端克隆、pending 统计、脱离等待后放弃积压，以及不同保留策略相互影响。
- **依赖与解耦**：source 也使用 live 下的 DetailLog，目录归属与复用关系不一致；建议提升为独立 diagnostics 服务，策略与线程机制分离。

#### `airplay-core/src/live/pipeline.rs` — 288 行

- **实现**：会话级左右交换、重采样、PCM 入队、水位异常判断、PI 更新、UI 遥测和诊断，正常结束冲刷滤波尾帧。
- **复杂度：高**。音频路径与三个更新周期、QPC/协议时钟、GUI emitter 和诊断序列化耦合。
- **依赖与解耦**：Converter/Controller 已独立；建议抽出 PipelineSnapshot/遥测发布器，让 process 保留路由、转换、控制与队列操作。采样处理 O(n)，避免同步 UI 发布拖延实时回调。

#### `airplay-core/src/live/protocol.rs` — 324 行

- **实现**：解析后端文本标记/播放时钟，跨线程存储首因和统计，stderr reader、PCM_READY 等待、取消及日志收尾。
- **复杂度：高**。协议解析、多个锁、I/O 错误来源、进程退出与管道仍被持有的情形，均影响就绪判断。
- **依赖与解耦**：后端日志实际兼作进程间控制协议，GUI 还接收原始/脱敏双文本。建议将 NativeEvent 解析从 reader 拆出，后续升级为有版本的结构化 IPC。

#### `airplay-core/src/live/protocol_log.rs` — 92 行

- **实现**：独立 worker 向控制台/文件输出脱敏协议日志；精简模式保留最近 24 行并在首个故障时写上下文，限制体量并补充 I/O 操作信息。
- **复杂度：中**。有损队列只承载日志、不能承载控制标记，这是关键隔离边界。
- **依赖与解耦**：复用泛型 LogWorker，Write 可注入，测试性良好；可随 diagnostics 服务迁移。

#### `airplay-core/src/live/report.rs` — 97 行

- **实现**：按协议首因→采集→stderr→PCM→传输判定结果，限时写报告并独立向 GUI/CLI 发布，保存写入故障而不覆盖主错误。
- **复杂度：中**。失败优先级和两个异步截止时间是重点。
- **依赖与解耦**：shutdown/log_store/failure 和 emitter；已明确把结果与持久化分开，可接收 typed snapshot 取代任意 JSON。

#### `airplay-core/src/live/transport.rs` — 349 行

- **实现**：后端参数/进程启动、Windows Job Object 子树管理、有期限终止；PCM S16LE 序列化、有界写队列、缓冲复用与写线程取消。
- **复杂度：高**。进程树和线程 I/O 生命周期、EOF 语义、原子计数回滚、缓冲池相互协调。
- **依赖与解耦**：目前合并进程管理和音频传输两类职责；建议拆 BackendProcess 与 PcmTransport，保留 Write 注入和非阻塞入队测试。序列化 O(样本数)，池容量有限。

#### `airplay-core/src/log_store.rs` — 241 行

- **实现**：日志文本截断、单文件/目录预算、受管文件识别、按时间清理旧日志、活动文件登记、禁止并发替换和有限报告写入。
- **复杂度：高**。文件保留规则、进程内锁与 Windows 文件共享语义共同保护正在写的文件；清理排序 O(F log F)。
- **依赖与解耦**：标准库及 Windows OpenOptions；可把纯文件名/保留计划与实际删除分离，避免诊断业务继续扩展此低层存储模块。

#### `airplay-core/src/main.rs` — 165 行

- **实现**：CLI 子命令路由、参数范围校验、共享数据根目录和错误退出码；串联发现、采集、转换、回放/实时串流。
- **复杂度：中**。业务算法委托核心，但手工参数搜索与命令组合持续增长。
- **依赖与解耦**：作为应用适配层合理；可提取 typed CommandOptions，统一未知/缺失参数验证，避免把 CLI 文案继续下沉到库。

#### `airplay-core/src/privacy.rs` — 306 行

- **实现**：按设备别名、敏感字段、IP/MAC/UUID、系统路径和响应体脱敏文本，递归脱敏 JSON 且保留统计数值。
- **复杂度：高**。手工文本扫描、引号转义、UTF-8 边界、替换顺序和字段例外均可能影响保密性与诊断可用性。
- **依赖与解耦**：Device 与环境变量读取；可将策略、设备字典构造、文本扫描分开，并在构造时固定环境字典。文本多轮替换约随字典数×文本长度增长，不适合无节制用于音频热路径。

#### `airplay-core/src/shutdown.rs` — 106 行

- **实现**：15 秒共享停止预算（只能缩短）、限时 join、CancelSynchronousIo，以及最多 8 个限时尽力清理任务。
- **复杂度：高**。取消请求不等于线程完成，超时后资源可能仍被持有，线程额度通过 RAII 回收。
- **依赖与解耦**：通用生命周期基础设施混有 Windows I/O 取消；保留单一预算模型，可将平台 cancel 包装放子模块。

#### `airplay-core/src/source.rs` — 530 行

- **实现**：持续 WASAPI Source、预览电平、启动预热、有限 loopback 重建、唯一串流订阅、音频/诊断队列与报告基线，统一停止与通知收尾。
- **复杂度：很高**。Source 生命周期长于串流；预热、重建、挂接必须用一致锁序防竞态；生产/消费/通知三种线程语义并存。
- **依赖与解耦**：capture、audio_queue、live::diagnostics、shutdown、buffers、notifications。优先提取 Warmup/RecoveryPolicy、Subscription/消费守卫、SourceSupervisor；工作区已有通知隔离改动，不能再按旧版本认定 GUI 回调直接运行在采集线程。

#### `airplay-core/src/source/buffers.rs` — 73 行

- **实现**：每订阅独立 f32 缓冲池、复制/复用/扩容统计，最多保留 32 个且每个不超过 16K 样本。
- **复杂度：低**。复制 O(n)，回收非阻塞且有空间上限；已是清楚的性能优化边界，不改变音频内容。

#### `airplay-core/src/source/notifications.rs` — 115 行（工作区新增）

- **实现**：采集通知独立线程，电平只保存最新快照；控制事件最多 64 条，终态独立槽位；释放队列锁后才调用 GUI emitter。
- **复杂度：中**。控制优先、终态不能被覆盖、关闭后不再发非零电平，以及慢回调下的有界等待。
- **依赖与解耦**：已降低 Source 对 GUI 速度的耦合；建议用显式 imports 代替 super::*，进一步用 SourceEvent 类型明确三类消息的区别。

#### `airplay-core/src/volume.rs` — 293 行

- **实现**：DACP mDNS 广播、目标设备与 Active-Remote 校验、HTTP 音量命令解析、localhost 转发、UI 命令通道和停止清理。
- **复杂度：高**。两个监听 socket/线程、广播资源、有界队列、同步日志与协议响应交织。
- **依赖与解耦**：mdns-sd、TCP、privacy/log_store/shutdown。建议分离纯 DacpRequestParser、广告生命周期、VolumeBridge；字符串命令可在内部使用枚举，只在 C 边界编码。

### airplay-frontend/：入口与桌面适配

#### `airplay-frontend/index.html` — 12 行

- **实现**：中文页面元数据、Vue 挂载容器和 main.ts 入口。
- **复杂度：低**。静态壳，保持品牌标题与桌面配置一致即可。

#### `airplay-frontend/package.json` — 30 行

- **实现**：Vue/Tauri API 依赖与 Vite 构建、类型检查、格式检查、协议/会话/诊断/DOM 测试命令。
- **复杂度：低**。测试入口跨目录指向 test/frontend；保留命令作为稳定开发入口，版本需与 Rust/Tauri/安装器协调。

#### `airplay-frontend/pnpm-workspace.yaml` — 2 行

- **实现**：允许 esbuild 安装构建脚本。
- **复杂度：低**。工具链权限配置，无业务逻辑。

#### `airplay-frontend/src-tauri/Cargo.toml` — 19 行

- **实现**：桌面程序依赖 Tauri/单实例插件、本地核心 crate 和 Windows 管道/电源/注册表能力，release 使用 thin LTO。
- **复杂度：低**。清楚的桌面→核心依赖方向；平台接口仍较多，适合保留在桌面基础设施模块。

#### `airplay-frontend/src-tauri/build.rs` — 5 行

- **实现**：追踪图标变更并执行 Tauri 构建。
- **复杂度：低**。标准构建胶水，无需拆分。

#### `airplay-frontend/src-tauri/capabilities/default.json` — 1 行

- **实现**：为 main 窗口声明 core:default 权限。
- **复杂度：低**。权限边界配置；新增插件能力时在此明确声明。

#### `airplay-frontend/src-tauri/src/auth.rs` — 226 行

- **实现**：仅当前用户/SYSTEM 可访问的本地命名管道、主机白名单、密码请求/应答、内存清零和阻塞 I/O 取消。
- **复杂度：高**。Windows SID/ACL/句柄 unsafe 操作、线程、密码生命周期与 GUI 回调共同组成敏感边界。
- **依赖与解耦**：GuiEmitter、shutdown、Windows Security/Pipes。建议分离 SecurePipe 创建、请求帧编解码、认证交互协调；保留非远程/首实例及密码不入命令行的约束。

#### `airplay-frontend/src-tauri/src/auth_memory.rs` — 116 行

- **实现**：缓存设备是否需要密码的策略（不存密码），按身份+公钥生成 key，从有限历史日志迁移，原子式 pending 文件保存和忘记策略。
- **复杂度：中**。旧日志迁移、设备重置失效与文本认证标记相连。
- **依赖与解耦**：Device/JSON/文件 I/O；建议把策略事件识别并入统一 NativeEvent 解析器，持久化与集合操作分开。

#### `airplay-frontend/src-tauri/src/awake.rs` — 97 行

- **实现**：应用级系统/显示器电源请求，双请求成功后才发布 active，部分失败回滚，关闭和 Drop 释放。
- **复杂度：中**。Win32 资源生命周期和原子式状态转换重要；activate 可注入，边界测试方便。
- **依赖与解耦**：已是独立平台服务，可进一步用 OwnedHandle 明确句柄所有权。

#### `airplay-frontend/src-tauri/src/configuration.rs` — 257 行

- **实现**：配置/发现/采集预览操作互斥登记，异步命令转阻塞任务，校验会话代次，配置保存与电源回滚，声道映射和交换提交。
- **复杂度：高**。慢操作期间会话可能停止/退出；磁盘 rename 是提交点，内存/设备状态必须保持一致。
- **依赖与解耦**：直接访问 Engine 多个锁和原子字段。建议把 Operation 作为应用服务入口契约，进一步将 source 管理、设置事务从 Tauri command 包装中分离。

#### `airplay-frontend/src-tauri/src/main.rs` — 359 行

- **实现**：Engine 共享状态、Source 复用/替换、初始化/密码/音量等命令、数据模式、托盘/单实例、Tauri 注册及窗口关闭策略。
- **复杂度：很高**。入口仍承担应用服务与资源管理；多个子模块通过 crate 根访问 Engine，形成中心耦合。
- **依赖与解耦**：特别注意 startup_checks 从 test/frontend/desktop/startup.rs **非 cfg(test)** 引入，实际随生产程序编译。建议把 Engine/SourceManager/command adapters 拆到生产目录，测试目录仅保留测试与独立诊断 harness。

#### `airplay-frontend/src-tauri/src/routing.rs` — 54 行

- **实现**：先持久化映射/左右交换，再发布到内存设置和实时控制面；保存失败不改变播放路由。
- **复杂度：中**。实现短，关键是事务顺序；已可通过依赖注入测试，无需进一步碎拆。

#### `airplay-frontend/src-tauri/src/session.rs` — 180 行

- **实现**：会话唯一预留、单调 id、准备阶段状态机/15 秒期限、看门狗过期、终态一次发布和有条件释放。
- **复杂度：高**。超时/取消/激活竞态、残留资源必须保留占用；这是应用生命周期核心。
- **依赖与解耦**：不在状态锁内执行慢工作，设计边界较好；保持作为独立领域状态机，减少对具体 auth reply 类型的依赖可改善复用。

#### `airplay-frontend/src-tauri/src/settings.rs` — 103 行

- **实现**：camelCase 设置结构、默认值、范围验证、旧配置兼容，保存前后检查取消并通过 pending/rename 提交。
- **复杂度：中**。主要在持久化提交语义及前后端字段一致性；映射范围留给实际设备验证。
- **依赖与解耦**：serde/文件系统；结构与存储可在规模增长时分开，目前集中合理。

#### `airplay-frontend/src-tauri/src/startup.rs` — 69 行

- **实现**：查询/设置当前用户 Run 注册表启动项，Unicode 引号路径、幂等删除和句柄关闭。
- **复杂度：低**。独立平台适配，注意 enabled 仅检查值存在而非校验路径内容；可用注册表句柄守卫简化释放。

#### `airplay-frontend/src-tauri/src/stream_start.rs` — 368 行

- **实现**：快速预留会话、看门狗/工作线程，设备/声道校验、认证策略、密码服务、Source 准备和原子发布，运行核心串流与失败回滚/终态通知。
- **复杂度：很高**。准备结果迟到、取消、panic、未激活 Source 的所有权和清理失败占用等情况交叉。
- **依赖与解耦**：Engine、session、auth、Source、live 和 Tauri。建议拆 PreparedSession 启动事务、TypedEventBridge 和 worker runner，避免日志/认证策略文件保存运行在通用 emitter 回调内。

#### `airplay-frontend/src-tauri/src/window.rs` — 90 行

- **实现**：显示/聚焦/拖动/最大化等窗口命令，托盘关闭策略，以及独立退出看门狗和后台资源清理。
- **复杂度：高**。正常清理与强制进程退出共用原始 deadline；重复退出需幂等，主事件循环卡住也要兜底。
- **依赖与解耦**：Tauri、Engine、Source 和 GuiControl；建议窗口操作与 AppShutdownCoordinator 分开，退出顺序作为应用级契约保留。

#### `airplay-frontend/src-tauri/tauri.conf.json` — 25 行

- **实现**：应用身份/版本、无系统边框窗口尺寸、前端产物路径、CSP 和图标；关闭 Tauri bundle。
- **复杂度：低**。发布实际依赖外部打包脚本；无边框意味着窗口行为由 Vue/Tauri 自维护。

### airplay-frontend/src/：界面与状态

#### `airplay-frontend/src/App.vue` — 785 行

- **实现**：页面壳、设备分组/选择、设置事务回滚、采集开关/首次绘制后启动、音量、密码视图、事件监听、主题/窗口操作与子组件组合。
- **复杂度：很高**。不是模板长这么简单：初始化、持久化、异步事件与多种 busy 标志交叉；保存成功后的 confirmedSettings 和迟到回调需一致。
- **依赖与解耦**：已有 session/diagnostics composable，但根组件仍是应用控制器。优先抽 useSettingsTransaction、useSourcePreview、useDeviceSelection、useDesktopLifecycle；模板再分 PlaybackPage/SettingsPage/AudioDock，避免仅搬模板。

#### `airplay-frontend/src/DeviceCard.vue` — 81 行

- **实现**：单设备/立体声卡片、展开详情、声道电平/映射与交换事件。
- **复杂度：低**。纯 props/emit 展示；依赖 App 定义的 SVG mask 和全局 CSS，视觉边界可局部化。

#### `airplay-frontend/src/GeneralSettings.vue` — 156 行

- **实现**：自启、唤醒、窗口行为、主题、延迟与声道映射控件，通过 computed setter 发更新事件，不直接改 props。
- **复杂度：低到中**。表单项目多但副作用上交 App；可保持组件边界，统一单次提交事件能减少“update 后再 persist”双事件契约。

#### `airplay-frontend/src/LogView.vue` — 88 行

- **实现**：详细/逐包诊断开关、日志路径/最近记录、完整报告、打开目录和清空显示事件。
- **复杂度：低**。无 I/O 与会话状态管理，依赖 props 契约；有界显示由 useDiagnostics 保证。

#### `airplay-frontend/src/RuntimeStats.vue` — 89 行

- **实现**：运行时长、水位、漂移、采集/转换报告和按设备累计传输统计展示。
- **复杂度：低**。O(设备数) 渲染，数据聚合不在组件内；明确发送端延迟估计范围，边界合理。

#### `airplay-frontend/src/SourcePicker.vue` — 111 行

- **实现**：按播放/录音分组排序来源，选择弹层、电平条、禁用与键盘 Escape。
- **复杂度：低**。排序 O(n log n)，不直接切换设备；若将来复用多个实例，固定 DOM id 需要实例化。

#### `airplay-frontend/src/TechnicalDetails.vue` — 78 行

- **实现**：设备/WASAPI/发送格式、电平、Buffer 设置、认证策略重置和发现能力展示。
- **复杂度：低**。受控表单与只读详情；保持副作用外置，术语与实际预缓冲阶段一致。

#### `airplay-frontend/src/commands.ts` — 12 行

- **实现**：统一 Tauri invoke 和错误描述。
- **复杂度：低**。目前命令名/参数/返回值仍由调用者自由指定；可建立 CommandMap 提升编译期前后端接口检查。

#### `airplay-frontend/src/display.ts` — 21 行

- **实现**：有限数值占位、dBFS 电平和音频格式文案。
- **复杂度：低**。纯展示函数 O(1)，已适合共用，不应承载业务状态推断。

#### `airplay-frontend/src/errors.ts` — 48 行

- **实现**：命令到默认错误码映射，共享 JSON 目录和 OS 来源识别，保留已格式化错误。
- **复杂度：中**。与 Rust failure.rs 重复来源/文本分类逻辑，可能漂移。
- **依赖与解耦**：建议前后端传结构化 ErrorInfo，共享目录仅承担代码/文案，文本解析保留为兼容层。

#### `airplay-frontend/src/main.ts` — 4 行

- **实现**：加载全局 CSS，创建并挂载 Vue App。
- **复杂度：低**。保持简单入口。

#### `airplay-frontend/src/protocol.ts` — 303 行

- **实现**：unknown 输入的运行时校验；设备/来源/设置/初始化/事件解码，安全整数/有限数值/两元素数组校验，报告 JSON 深度限制和未知事件兼容。
- **复杂度：中到高**。大量字段/分支主要来自边界契约，保留有价值；不能仅用 TypeScript 泛型替代。
- **依赖与解耦**：types.ts 与 Rust JSON 生产者需同步；可按基础 readers、device/settings、events 分区或生成 schema，保留只输出字段路径的错误语义。

#### `airplay-frontend/src/style.css` — 1349 行

- **实现**：全局基础控件、浅/深主题、窗口壳、设备卡片、设置/统计/日志、来源弹层、密码框和响应式音频 dock。
- **复杂度：高（层叠与修改影响面）**。多次重定义 .shell/main/.unit/.audio-dock 等规则，保留 aside/nav/titlebar 等旧布局样式；最终外观依赖文件顺序。
- **依赖与解耦**：所有组件共用全局选择器。优先去重并核对无用规则，再按 tokens/base/layout/component 划分；组件内 scoped 样式适合局部布局，共享主题变量继续统一。CSS 行数不能等同算法复杂度。

#### `airplay-frontend/src/types.ts` — 186 行

- **实现**：设备/来源/格式/设置/遥测/初始化、Source 预览及按 kind 区分的 StreamEvent 类型。
- **复杂度：中（契约规模）**。类型本身无运行开销，但字段命名混合 camelCase/snake_case，需与 Rust 输出一致。
- **依赖与解耦**：UI-only DeviceCardData 与 IPC DTO 已注释区分；将来可生成共享契约，减少 Rust/TS/decoder 三份同步。

#### `airplay-frontend/src/useDiagnostics.ts` — 196 行

- **实现**：有界日志/诊断/报告快照、路径展示、原生标记字段提取、每会话基线和跨会话设备统计增量。
- **复杂度：中到高**。重连保留累计值、设备淘汰、计数回退和大小限制需统一；报告大小检查会序列化整个报告。
- **依赖与解耦**：职责明确，不发命令；可抽 PacketStatsAccumulator 纯模型，把 nativeFields 迁至协议适配层，避免 App 通过诊断模块解析控制音量。

#### `airplay-frontend/src/useStreamSession.ts` — 244 行

- **实现**：启动/停止/密码提交与重试，命令确认前最多缓存 512 事件，按可信 session_id 过滤、取消迟到启动、组件卸载清理。
- **复杂度：高**。命令响应与事件可乱序，多个布尔状态和 request ticket 联合表达生命周期。
- **依赖与解耦**：session 所有权已集中；建议显式 SessionPhase 联合类型/纯 reducer + effects，保留现有早到 finished、溢出、旧会话和重试竞态测试。

#### `airplay-frontend/tsconfig.json` — 16 行

- **实现**：配置 ES2022、Bundler 模块解析、Vue/TS 严格类型检查及 JSON 导入；只检查源码和 Vite 配置。
- **复杂度：低**。这是编译期约束，不提供运行时数据校验；测试脚本另有执行入口。

#### `airplay-frontend/vite.config.ts` — 7 行

- **实现**：启用 Vue 插件，保留终端输出，开发监听排除 Rust 桌面目录。
- **复杂度：低**。职责单一，适合维持独立配置。

### scripts/：构建、发布与仓库检查

#### `scripts/build-backend.ps1` — 76 行

- **实现**：检查 UCRT64/Python/Cargo，配置编译原生后端及 Rust CLI，递归收集工具链 DLL，复制许可证和说明，执行打包后自检。
- **复杂度：中**。DLL 收集用队列和去重集合遍历依赖图；部署布局、开发工具定位、编译和分发打包集中在一起。
- **解耦建议**：提取工具链探测、原生构建、运行时依赖收集三个可复用步骤，供桌面及发布脚本调用；保留每一步退出码检查。

#### `scripts/build-desktop.ps1` — 49 行

- **实现**：可选先编译后端，定位 pnpm/Node，安装锁定依赖，构建前端与 Tauri 程序并汇总到 dist。
- **复杂度：中**。离线/发布/跳过后端组合以及环境 PATH 恢复增加分支；Release 参数仅控制桌面 Rust 构建，后端脚本仍使用自己的 Debug 配置。
- **解耦建议**：统一构建配置对象和分发文件清单，减少与后端脚本重复复制文档；将个人机器的工具路径回退封装为开发环境适配。

#### `scripts/check-sensitive-data.py` — 100 行

- **实现**：按文件路径和正则检查 Git 当前源码/未跟踪文件，或遍历全部历史；输出规则与位置，不回显命中值，排除子模块递归。
- **复杂度：中**。当前扫描大致与总文本量成正比；历史模式按“提交 × 文件”重复读取 blob，同一内容可被反复检查。
- **解耦建议**：保留纯 findings 规则函数，把 Git 文件枚举和结果呈现分离；历史扫描可按 blob 缓存内容规则结果，路径规则仍逐路径判断。

#### `scripts/installer.nsi` — 95 行

- **实现**：NSIS 当前用户 x64 安装器；拒绝覆盖便携目录，检查/安装 WebView2，写快捷方式及卸载注册项，按生成清单卸载，并仅删除指向本安装路径的自启动项。
- **复杂度：中**。注册表视图、外部安装进程、便携/安装模式边界是主要复杂点。
- **解耦建议**：与 Inno Setup 实现共享发布参数及行为验收清单，明确唯一默认安装流程，避免两套规则长期漂移。本次未实际安装或卸载。

#### `scripts/installpackcompiler.iss` — 135 行

- **实现**：Inno Setup 安装包定义，显式文件清单、桌面快捷方式；安装前查询 WebView2，征求安装器用户同意后运行引导程序并复查注册表。
- **复杂度：中**。与 NSIS 不同：默认 Program Files/管理员安装，版本写死，当前代码没有对应的 portable.flag 目录防护和自启动清理逻辑。
- **解耦建议**：先决定两种安装器的支持范围，再统一版本、负载清单及模式约束；维护复杂度来自行为分叉，不在 Pascal 函数本身。

#### `scripts/package-release.ps1` — 84 行

- **实现**：从 Tauri 配置取版本，检查 WebView2 引导程序签名，按白名单建立独立暂存负载，生成逐文件卸载清单，编译 NSIS；随后添加便携标记打 ZIP 并计算 SHA256。
- **复杂度：中**。关键约束是先生成安装器再添加 portable.flag，以及显式清单不能夹带本地数据。
- **解耦建议**：将负载清单作为构建、NSIS、Inno 共用的数据源；暂存目录生命周期和发布产物校验可独立封装，保持当前两种运行模式边界。

### test/backend/：原生协议与集成测试

#### `test/backend/audio_sequence.py` — 20 行

- **实现**：独立验证 RTP 首包标志、16 位序号、32 位时间戳回绕，以及不随 RTP 序号回绕的 64 位 nonce。
- **复杂度：低**。每包常数成本；适合保留为独立协议判定器，避免从生产实现导入期望结果。

#### `test/backend/check_auth.py` — 24 行

- **实现**：通过模拟接收端验证自动认证、固定 PIN/密码路由、错误密码与配对策略错误；校验过长/多行密码在参数阶段拒绝且不泄露。
- **复杂度：低**。表驱动用例清晰；证明协议分支行为，不代表真实设备播放能力。

#### `test/backend/check_gui_pipe.py` — 31 行

- **实现**：用 ctypes 建立真实 Windows 命名管道，连接模拟认证服务器，验证原生后端 GUI 密码请求、直接重试、日志脱敏和线程清理。
- **复杂度：中**。代码虽短，却混合 Win32 FFI、管道帧、socket、线程及子进程；压缩成单行增加理解成本。
- **解耦建议**：提取上下文管理的管道 fixture 和明确的读写函数；这覆盖原生管道协议，不等于验证 Tauri ACL 配置。

#### `test/backend/check_setup.py` — 79 行

- **实现**：给模拟接收端 SETUP 注入延迟，测试 NTP/PTP/双声道握手与超时预算、错误阶段、失败时不发 PCM_READY，并写结果 JSON。
- **复杂度：中**。全局替换 Channel.receive、共享 delay/setups、runpy 执行另一个脚本，使测试顺序影响环境。
- **解耦建议**：把延迟和计数放进每个接收端实例，用可恢复 fixture 管理注入；将立体声场景改成可调用用例。

#### `test/backend/check_stereo.py` — 66 行

- **实现**：同时驱动两个模拟接收端，喂实时 PCM；断言共享 PTP 引擎、认证密码复用、组 UUID、音频帧、时间锚和双方 TEARDOWN。
- **复杂度：中**。包含双端并发及时间锚逆算，时序依赖高于行数体现的程度。
- **解耦建议**：提取 PCM 实时馈送和锚校验辅助函数；保留跨接收端断言。模拟器结果不能代替 HomePod 实际同步听测。

#### `test/backend/check_wrap.py` — 8 行

- **实现**：运行 526 秒本机实时传输，让 65536 包序号回绕并测试跨界重传、nonce 和 PCM 解密。
- **复杂度：低；运行成本高**。入口简单，耗时由真实发送节奏决定；适合独立长时回归组，不塞进每次快速单元检查。

#### `test/backend/harness.py` — 107 行

- **实现**：集中登记 socket、线程和子进程，以 context manager/decorator 在成功或异常时限时清理；先终止子进程再关管道，避免 flush 阻塞，保留原异常和清理故障。
- **复杂度：中偏高**。难点是失败路径的资源顺序与有界等待；这是应集中维护的测试基础设施。
- **解耦建议**：保持与具体协议无关，新增模拟场景统一走此资源边界，避免各脚本自己拼接清理逻辑。

#### `test/backend/mock_receiver.py` — 585 行

- **实现**：模拟 SRP 服务端、HAP 加密控制/事件通道、RTSP 会话、UDP 音频解密与 ALAC 校验、重传、音量和错误注入；也负责启动后端、节奏化写 PCM 和批量运行场景。
- **复杂度：很高**。一个 mode 字符串同时驱动认证、时钟、事件、音频及故障分支；receiver 是大型嵌套状态机，协议判定与测试调度交织。音频逐样本验证成本与帧数成正比，历史包限定 512。
- **解耦建议**：按 crypto/channel、RTSP 会话、audio verifier、events、scenario runner 拆分；用显式 Scenario 配置替换 startswith/endswith 隐式组合。保持密码学和包期望的独立实现，避免测试与被测端共享同一错误。

#### `test/backend/native/nonce_selfcheck.c` — 17 行

- **实现**：检查 65538 次 nonce 编码及 UINT64_MAX 拒绝溢出，通过后端 --self-check 调用。
- **复杂度：低**。固定循环验证 RTP 序号边界不会让加密计数归零。
- **结构注意**：该 test 路径文件直接编入生产自检入口；若保留运行时自检，建议把自检实现放生产侧，测试侧调用同一入口，明确发布依赖。

#### `test/backend/requirements.txt` — 1 行

- **实现**：固定模拟接收端使用的 cryptography 版本。
- **复杂度：低**。属于测试环境依赖，不应进入应用运行依赖。

#### `test/backend/test_harness.py` — 47 行

- **实现**：验证包判定器能接受合法回绕、拒绝 nonce/时间戳错误；验证测试中途异常后子进程、socket、线程及管道全部清理。
- **复杂度：低**。测试基础设施本身的回归保护；比单纯重复生产函数的计算更有独立价值。

#### `test/backend/test_upstream.py` — 120 行

- **实现**：验证提取产物与审查哈希一致、CRLF 兼容、输入/补丁/版本变化拒绝、无效补丁不污染原目录或已有产物，以及函数提取器处理注释/字符串/重名。
- **复杂度：中**。临时目录和伪造 manifest 是必要的隔离；比仅测试正常构建更能保护升级边界。
- **解耦建议**：保留独立运行能力；输出哈希匹配目前在此测试断言，升级流程应明确把它列为必跑检查。

### test/core/checks/：离线分析与性能对比

#### `test/core/checks/check_capture.py` — 43 行

- **实现**：独立解析采集 WAV 的 RIFF/chunk/float32 格式，逐帧检查有限值、信号帧数和各通道峰值，与 JSON 报告对照。
- **复杂度：低至中**。时间 O(帧数 × 通道数)，一次读取整个 WAV，空间随文件体积增长。
- **边界**：依赖已经采集的真实文件，不负责发起 WASAPI 采集；默认最新文件可能为空，长期可改显式输入参数。

#### `test/core/checks/check_conversion.py` — 16 行

- **实现**：以 Python 标准 WAV reader 验证 44.1kHz/双声道/int16、PCM 与 WAV 数据相等及按采样率换算后的输出帧数。
- **复杂度：低**。时间和空间随输出文件大小线性增长；适合离线产物检查，不能单独证明重采样音质。

#### `test/core/checks/compare_performance.py` — 28 行

- **实现**：比较相同 release/fixture/采样率的合成基准，要求至少三轮，取各轮 p95 与 wall time 的中位数，可按退化阈值失败退出。
- **复杂度：低**。适合基准结果呈现；跨机器、WASAPI、WebView 或真实播放延迟不属于其证明范围。

### test/core/unit/：Rust 核心行为测试

#### `test/core/unit/audio_queue.rs` — 47 行

- **实现**：跨采样率验证 640ms 时间预算含正在处理的块；验证满队列不分配、硬块数上限、断连和非法格式释放配额。
- **复杂度：低**。覆盖关键资源不变量，队列拆分时应保留这些行为而非依赖内部容器形式。

#### `test/core/unit/backend.rs` — 53 行

- **实现**：验证摘要去重且不积累统计行、后端可执行文件只从预定安装位置解析而不使用数据目录/工作目录。
- **复杂度：低**。临时文件清理较冗长，可用统一 fixture；保留路径信任边界的外部行为断言。

#### `test/core/unit/capture/clock.rs` — 13 行

- **实现**：验证 QPC 宽整数换算、极大值、负计数/非法频率拒绝，以及关闭诊断时的零时间源。
- **复杂度：低**。纯边界用例，保护单位转换和溢出处理。

#### `test/core/unit/capture/decode.rs` — 182 行

- **实现**：覆盖 8/16/24/32 位 PCM、有效位对齐、mono 复制、多声道映射、帧窗口和缓冲复用；校验错误无部分输出，float 原值保留及非有限值统计。
- **复杂度：中**。用固定字节作为独立期望，格式组合多但主题集中。适合保持 decode 模块的契约测试，不必因行数拆散。

#### `test/core/unit/capture/diagnostics.rs` — 117 行

- **实现**：验证逐包 JSON 字段、时间/设备位置增量、端点状态、sink 错误和最终报告；关闭逐包诊断仍保留生命周期事件。
- **复杂度：中**。无需硬件，主要保护诊断协议稳定性；字段数量断言偏结构约束，改 schema 时应与消费端一起更新。

#### `test/core/unit/capture/gap_recovery.rs` — 42 行

- **实现**：验证 Windows 自动复位事件、短间隙的时钟一致性、超限/回退拒绝，以及时间戳倒退映射到 CAPTURE_TIMESTAMP_INVALID。
- **复杂度：低至中**。混合一个 Win32 资源测试和纯时间线规则；后续可随平台层/策略层拆分归属。

#### `test/core/unit/capture/health.rs` — 115 行

- **实现**：覆盖启动期端点有电平而 PCM 全零的 500ms 判定、静音/音量零/缺失控制不误恢复、连续失配计时，以及成功启动后暂停不重新开启检测。
- **复杂度：中**。显式注入 Instant，避免真实等待；保护“只在启动期判断”的产品语义，拆模块时应原样保留。

#### `test/core/unit/capture/loopback.rs` — 60 行

- **实现**：默认忽略的真实 Windows 播放端点三秒采集；比对帧数、采样率、有限样本，并打印系统电平与采集电平。
- **复杂度：中**。有 COM/设备环境依赖；覆盖本机 loopback，不连接 HomePod，也不直接断言听感或端到端延迟。

#### `test/core/unit/capture/metrics.rs` — 99 行

- **实现**：验证峰值/信号阈值、非有限值、按原始 float 位计算的指纹、残帧规则，以及诊断暂停后的有符号时间/设备增量。
- **复杂度：低至中**。纯数据测试，固定向量保护既有统计语义；不含设备或线程调度依赖。

#### `test/core/unit/capture/state.rs` — 93 行

- **实现**：区分“已观察到的包/故障”和“sink 成功交付的帧”；覆盖首包 discontinuity、静音补偿、重叠跳过及可选进度快照。
- **复杂度：低至中**。用例保护计数含义，避免模块拆分后前端数字与最终报告口径悄然变化。

#### `test/core/unit/capture/timeline.rs` — 130 行

- **实现**：验证空闲静音的交付余量/块上限、恢复后的重叠裁剪和补齐、时间回退拒绝、60 秒修复次数窗口、累计修复时长及最终帧目标。
- **复杂度：中**。纯时间线状态机测试，输入时钟可控，是采集模块重构时的重要行为基线。

#### `test/core/unit/convert.rs` — 98 行

- **实现**：验证分包不影响输出、左右隔离、通带 RMS、带外抑制、量化饱和和抖动均值、±800ppm 尾帧时长，以及小块精确结束。
- **复杂度：中**。数值契约明确，工作量随生成样本数线性增长；是 DSP 模块拆分的核心回归保护，不等于完整主观音质评测。

#### `test/core/unit/data_dir.rs` — 87 行

- **实现**：验证便携标记只对 GUI/CLI 规定位置生效、旧设置和 CLI 数据一起迁移、新值优先、迁移幂等及删除的数据不复活。
- **复杂度：低至中**。文件场景明确，手动清理占较多篇幅；可共用临时目录 fixture，保留模式和迁移断言。

#### `test/core/unit/drift.rs` — 42 行

- **实现**：用离散仿真模拟两小时偏差/抖动、三小时速率阶跃，验证水位、±800ppm 限幅、20ppm/s 变化率及饱和恢复。
- **复杂度：中**。计算本身线性且无真实等待；稳定性结论仅覆盖该队列模型和扰动范围，不代表真实链路长时测试。

#### `test/core/unit/failure.rs` — 71 行

- **实现**：验证应用错误类别与 Win32/Winsock/HRESULT 来源并存、格式化幂等、唤醒错误归类，以及认证代码/退出码对应关系。
- **复杂度：低至中**。保护跨层错误契约；未来采用结构化错误时应继续验证展示语义，减少对中文文本匹配的依赖。

#### `test/core/unit/live/diagnostics.rs` — 241 行

- **实现**：验证诊断文件上限、完整 JSON 行、轮转保留最近故障、脱敏；模拟写入阻塞/失败，检查丢弃数、待写数和共享清理期限。
- **复杂度：中偏高**。文件 fixture、线程握手和时间预算交织；对超时线程显式释放，避免测试自身泄漏。适合随通用日志模块迁移，保留音频与日志失败隔离的契约。

#### `test/core/unit/live/hardware.rs` — 128 行

- **实现**：两个默认忽略的真实设备测试：30 秒采集/重采样/节奏消费者/漂移控制，以及三秒采集转换和有界交付。
- **复杂度：中**。依赖实际默认输入、系统计时和调度；断言水位、输出帧数及采集故障计数。这里的消费者是模拟发送节奏，不证明 HomePod 端到端播放。

#### `test/core/unit/live/pipeline.rs` — 232 行

- **实现**：验证声道互换复用缓冲、变长输入帧数/尾部滤波、日志失败不打断音频，以及 stderr 故障后停止排队；另有显式运行的合成性能基准。
- **复杂度：中**。将功能测试与基准放在一起；基准每采样率预热加三轮，以 io::sink 测转换/交付，排除设备、时钟、日志和真实发送节奏。
- **解耦建议**：把基准移入清晰的 benchmark 分组；用输出字节/帧数保护 AudioPipeline 和 PcmWriter 之间的契约。

#### `test/core/unit/live/protocol.rs` — 591 行

- **实现**：覆盖后端文本协议解析、ready/时钟/双端统计、原始状态与脱敏展示分离；用阻塞/失败 writer 和 reader 验证日志不会阻塞控制、取消及时、stderr 失败/正常 EOF 区别与错误优先级。
- **复杂度：高**。主要复杂度来自可控并发 fixture、两类日志输出及多种失败时点。测试本身使用握手和显式释放，降低调度随机性。
- **解耦建议**：拆成 marker parsing、reader lifecycle、log isolation 三组，共享 StallWriter/GatedReader fixture；避免把控制流测试误归入纯日志测试。

#### `test/core/unit/live/report.rs` — 166 行

- **实现**：验证报告超限保留旧文件、写入失败不掩盖协议/采集/stderr/管道主错误，保存内容与 GUI 快照一致，以及通知阻塞遵守期限。
- **复杂度：中**。错误优先级组合和文件 fixture 为核心；应随 ReportSink 边界保留，不能把报告故障错误地升级为音频根因。

#### `test/core/unit/live/transport.rs` — 280 行

- **实现**：验证 PCM 编码、队列配额回滚、EOF 排空、断管和停滞；以测试可执行文件作子进程验证优雅退出、强制终止、JobObject 析构及真实管道释放。
- **复杂度：高**。纯 writer 测试与 Windows 进程集成混合，包含多条资源生命周期。
- **解耦建议**：分别归入 PCM writer、backend process 和跨边界集成测试；保留“超时后不再排空积压”“在途写入仍占预算”的断言。

#### `test/core/unit/log_store.rs` — 119 行

- **实现**：验证日志整条写入、超限后不恢复、保留策略预留活跃增长、只清理认可文件名、保护未知文件/目录/被占用文件，以及 UTF-8 截断。
- **复杂度：中**。依赖文件系统和 Windows 共享模式；这里保留策略的保护范围应随日志模块独立迁移。

#### `test/core/unit/privacy.rs` — 60 行

- **实现**：验证无设备列表也能隐藏凭据、嵌套敏感字段、地址/MAC/UUID，以及已发现设备身份；同时保留帧数、时间和速率统计。
- **复杂度：低**。数据驱动的脱敏契约测试；新增文本格式需要持续补充样例，有限样例不能证明任意日志均无遗漏。

#### `test/core/unit/shutdown.rs` — 57 行

- **实现**：验证多次停止不延长期限、多线程共享一份预算、超时后保留句柄以便释放后再 join、阻塞回调限时返回及期限已过不启动新工作。
- **复杂度：中**。通过 channel 控制阻塞，可区分“等待返回”与“工作真正结束”；是全项目生命周期抽象的关键测试。

#### `test/core/unit/source.rs` — 562 行

- **实现**：验证消费者失败释放队列、池化块/采样率/位置保持、预热连续性、启动 loopback 有界重开、活动订阅禁止重置、解绑保持源存活且不带旧音频、停止超时保留工作线程，以及通知溢出/慢 GUI 的终止行为；包含默认忽略的实机用例。
- **复杂度：高**。同时测试预热、恢复、订阅、资源关闭和硬件，依赖 Source 私有状态，结构性改动会牵动大量 fixture。
- **解耦建议**：随生产职责拆为 warmup/recovery/subscription/lifecycle 四组，硬件独立分组；当前新增通知隔离测试应保留，不能依据旧版本假设采集线程直接调用 GUI。

#### `test/core/unit/source/baseline.rs` — 108 行

- **实现**：显式运行的 release 合成基准，测生产 send_audio 与订阅锁、池化传递路径；5000 个 10ms 块，以每八包确认限制积压，输出有界 JSON。
- **复杂度：中**。同步 fixture 和基准统计集中；排除 WASAPI、重采样、网络及日志，不能把耗时改善直接换算成实际播放 CPU 或延迟。

#### `test/core/unit/source/buffers.rs` — 53 行

- **实现**：验证池复用不改变 float 位、样本顺序和长度，复制后所有权独立；覆盖池数量/容量上限、断连和新订阅隔离。
- **复杂度：低**。纯内存资源契约；适合保持小模块测试，避免把缓冲池升级成跨全局的隐式共享状态。

#### `test/core/unit/source/notifications.rs` — 69 行

- **实现**：模拟慢 GUI，验证通知在独立线程执行、电平合并为最新值、控制事件保持顺序；控制队列满仍保留独立终止槽并清除过期电平。
- **复杂度：中**。并发握手明确，覆盖通知通道的不同可靠性要求；这些队列语义应作为独立模块契约。

#### `test/core/unit/volume.rs` — 77 行

- **实现**：区分设备报告 dB 与用户百分比请求，拒绝非有限/越界输入；验证分片 HTTP 回调的 Active-Remote 标识；另有默认忽略的 DACP 广播/转发集成用例。
- **复杂度：中**。解析测试和联网测试混合；可随 volume parser、callback server、advertisement 分层归属。

### test/frontend/desktop/：桌面 Rust 测试及启动探针

#### `test/frontend/desktop/auth.rs` — 77 行

- **实现**：真实本地命名管道密码往返/取消，确认请求事件不携带密码；模拟密码提示回调阻塞，验证关闭沿用现有期限。
- **复杂度：中**。Win32 管道、线程和事件协作；适合在密码帧协议与 AuthServer 生命周期拆分后保留集成覆盖。

#### `test/frontend/desktop/auth_memory.rs` — 48 行

- **实现**：验证认证策略跨重启及地址变化保留、公钥变化失效、只从明确密码证据学习。
- **复杂度：低**。保护“记住是否需要密码”的身份策略，不是持久化密码本身；适合保持独立仓储测试。

#### `test/frontend/desktop/awake.rs` — 115 行

- **实现**：验证系统/显示唤醒请求幂等、真实句柄关闭、获取两种请求的顺序、显示请求失败回滚，以及保留回滚失败信息。
- **复杂度：中**。真实句柄测试隔离到子进程，避免并行复用数值句柄的误判；平台生命周期测试应与纯事务测试区分。

#### `test/frontend/desktop/routing.rs` — 77 行

- **实现**：通过阻塞暂存文件写入制造保存失败，确认磁盘设置、内存、Source/Session 映射及扬声器互换标志均不提前改变；随后重试成功。
- **复杂度：低至中**。保护先持久化再发布的事务语义，适合提取设置服务时作为验收条件。

#### `test/frontend/desktop/session.rs` — 210 行

- **实现**：验证准备阶段阻塞不占会话锁、取消后拒绝迟到结果、超时仍占资源且终态仅一次、旧 session 不影响新预留；覆盖配置/发现/预览共享准入和暂存设置后取消不提交。
- **复杂度：高**。直接构造 Engine，联动 Session、Operation 和 Settings，属于应用编排集成测试。
- **解耦建议**：保留跨模块取消场景，同时将纯预留状态转移移入独立 SessionRegistry 测试，减少每个用例对整个 Engine 布局的依赖。

#### `test/frontend/desktop/settings.rs` — 25 行

- **实现**：验证非法已保存设置回退、非法写入保留旧值、合法保存成功且不残留 pending 文件。
- **复杂度：低**。集中保护文件存储和验证规则；可与配置事务测试分别维护。

#### `test/frontend/desktop/shutdown.rs` — 27 行

- **实现**：验证用户停止不会吞掉真实读取/超时/认证故障，未确认清理时提示重启；仅正常且清理完成的主动取消可不显示失败。
- **复杂度：低**。纯终态展示政策，适合从 stream_start 编排中提取为独立判定函数。

#### `test/frontend/desktop/startup.rs` — 92 行

- **实现**：被桌面程序直接引入的运行时 smoke 探针；指定参数时记录页面就绪、托盘/窗口显隐、采集启用前后状态，修改关闭动作并触发退出；第二实例写复用结果后退出。
- **复杂度：中，目录耦合高**。没有普通单元测试断言，而是运行时探针和产物采集。正常启动不执行这些测试操作，但生产编译依赖 test 目录中的实现。
- **解耦建议**：迁入桌面侧明确的 smoke/diagnostics 模块并用 feature 管控；test 目录保留外部驱动与结果断言，消除“删测试目录就无法构建应用”的边界混淆。

### test/frontend/：前端协议、状态与界面测试

#### `test/frontend/diagnostics.test.mjs` — 106 行

- **实现**：验证日志条数/文本长度、报告体积、清理与重置语义、按设备/会话累加统计，处理缺失/重复/递减计数和 __proto__ 等特殊主机名。
- **复杂度：中**。纯 composable 测试，不依赖浏览器；保护长时运行的内存边界及显示统计口径。

#### `test/frontend/dom/session.spec.mjs` — 1081 行

- **实现**：通过真实 Vue 模板操作验证来源选择、设置保存回滚、声道与设备选择、自动启动/唤醒、日志与统计、控制禁用、密码重试、迟到事件隔离、启动超时等；Tauri IPC 由 fixture 替代。
- **复杂度：高**。覆盖面大，文件名 session 已不足以表达职责；大量选择器、命令桩和产品文案相互绑定。
- **解耦建议**：按 source-settings、device-routing、diagnostics、session-auth 拆 spec，共享少量页面操作 helper。保留用户点击/键盘路径，避免退化成直接改组件内部状态；这些用例不能代替真实 Tauri/设备联调。

#### `test/frontend/fixtures/devices.json` — 26 行

- **实现**：两个虚构接收端共享 tsid、各有 psi 和主从标记，用于立体声分组及单设备选择场景。
- **复杂度：低**。使用文档示例地址；数据含协议语义，改字段需同步分组测试，不能当任意 UI 占位数据。

#### `test/frontend/fixtures/inputs.json` — 56 行

- **实现**：提供播放/录音两个虚构端点，区分设备 24 有效位 PCM 与共享混音 float32 格式。
- **复杂度：低**。支持来源分组及技术信息展示；应保留 device_format/mix_format 的差别，避免测试把两种格式混为一谈。

#### `test/frontend/load-diagnostics.mjs` — 25 行

- **实现**：用 TypeScript transpileModule 将真实 composable 转 CommonJS，在受控 require 映射下加载真实 Vue 和 display 函数。
- **复杂度：中**。少量代码承担自定义模块加载；只转译、不做类型检查，依赖新增必须手工维护映射。
- **解耦建议**：测试入口增多后可统一为标准 TS 测试运行环境，避免每组用例重复自建 loader。

#### `test/frontend/playwright.config.mjs` — 37 行

- **实现**：单 worker/headless Chromium（Windows 默认 Edge）、每例 15 秒/总计 120 秒、无重试、失败留 trace/截图；自管 Vite 服务，不复用已有进程。
- **复杂度：低**。环境约束明确；扩充 spec 后需评估总超时，不能用无限重试隐藏不稳定性。

#### `test/frontend/protocol.test.mjs` — 183 行

- **实现**：以真实 TS 校验器测试初始化默认值/模式、坏数据不部分写入、所有已知事件、遥测数值/会话 ID、来源部分更新、报告扩展 JSON/深度上限及错误不暴露原值。
- **复杂度：中**。跨 Rust→TS 的运行时协议保护；自定义转译只用于执行，类型检查由构建负责。建议将 schema 演进与此组用例绑定。

#### `test/frontend/session.test.mjs` — 376 行

- **实现**：执行真实 App script 与会话 composable，替换 IPC/浏览器接口；验证命令返回与事件乱序、早到终态、停止/卸载/密码重试、512 事件积压保护、旧会话隔离及设置迟到完成。
- **复杂度：高**。除场景本身复杂，还用正则提取 Vue script、AST 移除 imports、new Function 注入依赖和暴露 App 内部名称，测试与页面实现形状紧密绑定。
- **解耦建议**：先提取设置/生命周期 composable，使单元测试直接导入模块；保留 DOM spec 验证模板连接，逐步撤除 App 脚本手工执行器。

#### `test/frontend/ui/dom.html` — 10 行

- **实现**：DOM 自动化测试专用挂载页，加载 dom.mjs。
- **复杂度：低**。仅是测试入口，职责清晰。

#### `test/frontend/ui/dom.mjs` — 116 行

- **实现**：挂载真实前端前建立 Tauri mockIPC，记录命令，允许逐次 hold/resolve/reject 和主动发事件；保持设置只在成功时应用、session ID 提前预留。
- **复杂度：中**。模拟的是桌面边界而非组件内部状态，测试价值较高；命令 switch 需随真实 IPC 契约同步，避免 mock 漂移掩盖问题。

#### `test/frontend/ui/index.html` — 92 行

- **实现**：人工 UI 演示页，内嵌 mockIPC，定时模拟密码提示、错误密码、ready、遥测与停止；回调绑定原 session ID。
- **复杂度：中**。HTML 内嵌场景脚本且另有 dom.mjs mock，容易形成两套命令行为。
- **解耦建议**：共用基础 IPC fixture，演示页只保留定时场景；未知命令当前可静默返回，不能当严格协议验证。

#### `test/frontend/vite.config.mjs` — 35 行

- **实现**：复用生产前端依赖编译人工/DOM 两个测试入口，产物集中在 test/.artifacts；显式 Vue runtime/Tauri API 别名，开发服务仅绑定本机。
- **复杂度：低至中**。目录迁移后模块解析依赖显式别名；统一测试运行环境时需保留同一份 Vue 实例和实际生产源码。

### docs/：被程序导入的共享契约

#### `docs/error-codes.json` — 582 行

- **实现**：Rust/TS 共同读取的应用错误目录，包含 code、退出码、中文解释/操作提示与兼容文本匹配规则；包括通知、停止、采集、认证、日志等分类。
- **复杂度：中，跨层影响高**。数据量不等于算法复杂；主要维护风险是 patterns 顺序/重叠、中文错误文本变化，以及应用错误类别与 OS 原因混淆。
- **解耦建议**：将可执行契约移到 contracts/ 或明确的共享资源目录，由它生成说明；新增代码唯一性/字段形状/退出码契约检查。长期由结构化错误携带 code，patterns 仅保留兼容用途。

<a id="verification"></a>

## 核对与验证边界

- **清单核对**：以 Git 已跟踪及未被忽略的未跟踪文件为基准，检查 rs/c/h/cpp/inc/py/ps1/nsi/iss/ts/vue/css/html/mjs/js/patch 与 CMakeLists.txt；在该自维护范围内没有缺失条目。另列出 Cargo/前端/桌面配置、安装与提取清单、测试 fixture 和共享错误表。166 个文件路径无重复，物理行数已逐项核对。
- **工作区改动**：本次只新建并持续补充本文；原有 source、通知模块、测试及其他文档的未提交改动被作为分析输入保留。未重构源码，未提交或推送。
- **验证性质**：完成静态职责/依赖/复杂度审阅及文档清单核对。本次没有运行构建、单元/浏览器测试、实机采集、HomePod 播放、性能基准或安装卸载；“覆盖某场景”描述测试代码已有的断言，不表示本次已通过。
- **上游边界**：第三方子模块 upstream/airplay-cli 的观察版本为 `8e79242996b7ef52352ee49d390e6db434bf88a6`。本次审阅其自维护提取器、manifest、补丁与兼容性测试，未逐个审阅子模块内部文件，也未计算第三方库的整体复杂度。
- **指标局限**：人工评级不是圈复杂度、缺陷数或性能分数。静态看到的线程/锁/错误路径能够说明维护成本，但未测得锁等待时间、分配频率、真实端到端延迟和设备兼容性。
- **后续维护方式**：模块拆分时同步更新本文的旧路径/新职责、对应测试路径和项目结构说明；每次仅推进一个有验收条件的阶段，避免“文档结构已更新、代码依赖仍未改变”。


