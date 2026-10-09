# 更新日志

## 1.0.2 — 2026-10-10

- 修复暂停播放后误报回环无音频并重建采集的问题：回环一致性检查仅在启动阶段运行，后续暂停、恢复或停止串流不重新触发。
  Fix false loopback recovery during playback pauses by limiting consistency checks to capture startup.
- 修复设备已有播放会话时音频流建立过早超时的问题：音频 `SETUP` 使用 8 秒启动预算，收到回应立即继续，不增加正常播放延迟。
  Fix premature audio stream setup timeouts by retaining the 8-second startup budget without increasing normal playback latency.
- 桌面应用、前端包、Rust 核心、锁文件和安装包版本统一更新为 `1.0.2`，保留原安装身份和历史发行记录。
  Update application, frontend, core, lockfiles and installers to `1.0.2`, preserving the existing installation identity and release history.
- 测试源码集中到根目录 `test/frontend/`、`test/backend/`、`test/core/`，恢复 Git 提交并更新运行入口、项目结构与贡献说明；日志、录音和生成内容继续忽略。
  Centralize tests under root `test/frontend/`, `test/backend/` and `test/core/`, restore Git tracking and update runners and project guides while excluding runtime artifacts.

验证范围：采集健康状态回归及本地延迟回应的单设备/立体声协议测试已通过；真实设备的暂停/恢复及已有播放会话切换仍需确认。
Validation scope: capture health regressions and local single/stereo receiver checks with delayed responses pass; real-device pause/resume and takeover checks remain pending.

## 1.0.1 — 历史候选 / Previous candidate

本阶段维护重点是播放设备的采集启动、回环异常诊断与设备连接。2026-10-10 发现暂停播放误触发自动重建及已占用设备首次连接超时，随后候选包加入以下修复。后续修复版本统一为 `1.0.2`，此处保留原维护记录。

This previous candidate includes capture startup and audio stream setup fixes. Further fixed builds use version `1.0.2`; the earlier maintenance record is retained below.

### 采集启动与恢复

- 软件启动时采集开关先关闭，窗口首次显示并完成配置初始化后自动启用；没有有效来源时等待用户选择设备。
- Playback 回环在 Windows 端点有电平、未静音且音量大于零时，检查转换前的原始音频是否持续全零，避免仅凭数据包到达就判断采集正常。
- 自动重建检查仅用于首次采集稳定之前；连续 500 ms 无端点/原始音频矛盾后永久结束检查。后续暂停、恢复或停止串流不重新开启该检查，不因静音撤销就绪或重建实例。
- 初始化阶段发现持续回环异常时，释放旧采集实例并进行有次数限制的重建；正常安静来源可完成初始化，串流期间不自动重置采集时钟。
- 来源启动日志无需先开始串流即可记录初始化、重建次数和采集进度，便于排查首次打开软件时的无声问题。

### 设备连接 / Receiver connection

- 修复控制会话已建立后，音频流 `SETUP` 误用普通控制请求 2 秒超时的问题；音频流建立始终使用现有的 8 秒启动预算，给接收端切换原有播放留出时间。
  Audio stream `SETUP` now retains the existing 8-second startup budget after the control session is accepted, allowing more time for the receiver to switch playback.
- 响应到达立即继续，不额外等待 8 秒，不改变播放提前量；密码错误、访问拒绝及正常控制请求的处理保持原有行为。
  The connection continues immediately on response without changing playback latency; password errors, access denials and regular control requests retain their existing behavior.
- 本地加密协议模拟验证：延迟 3 秒回应的单设备（NTP/PTP）及立体声均一次成功，旧后端在约 2 秒超时；超过预算仍有限退出，密码错误和 403 拒绝不会进入音频发送。
  Local encrypted-protocol checks pass for single receivers (NTP/PTP) and stereo with a 3-second response delay; the previous backend times out at about 2 seconds. Over-budget responses remain bounded, and password errors or 403 denials do not start audio transmission.

### VoiceMeeter 排查与验证

- 补充 VAIO3 回环排查、内部延迟设置，以及从 VoiceMeeter 主界面保存延迟的方法。
- 本机在 VAIO latency 为 768 时，独立采集对照有 11/16 次出现持续全零；改为 7168 后该组对照全部通过。
- 按低延迟需求改为 1536 后，30 次独立采集启动、6 次项目来源启动及一次 60 秒连续采集均通过；项目来源各一次初始化成功，未触发重建。
- 上述结果适用于本次 Windows 11 / VoiceMeeter 环境。1536 不是所有设备的通用设置；完整 AirPlay 串流的撕裂听感、更长时间运行及音频引擎重启后仍需验证。

### 版本与发行材料

- 桌面应用、前端包、Rust 核心与安装脚本统一为 `1.0.1`，同步更新锁文件、项目说明和发行说明。
- 便携包与安装包附带播放回环排查文档，保留 `1.0.0` 的历史更新记录。

## 1.0（1.0.0）正式版

AirPlay Hub 首个正式版本，面向 Windows x64，提供从 Windows 音频设备到 AirPlay 接收端的桌面串流。

### 音频与设备

- WASAPI 录音设备和 Playback loopback 采集，来源选单分组并按设备名称排序。
- 实时来源电平、手动停止采集；重新开始串流前等待采集管线稳定。
- 左右输入声道映射、44100 Hz 重采样、16-bit PCM 量化及 ALAC 加密发送。
- 局域网设备发现、按需密码认证、单设备与已有 HomePod 立体声对。
- 立体声扬声器位置交换、发送端音量调节及 HomePod 顶部按钮音量同步。
- 可调播放提前量、采集 Buffer、队列监测与长期时钟漂移校正。

### 界面与运行

- 底部固定音频来源及串流操作区，设置内提供常规、运行统计、技术详情和日志。
- 首次启动或保存的来源已失效时显示“请选择音频流来源”，选择有效设备后启动采集。
- 录音与 Playback 来源使用不同图标、分组说明及类型标记，便于区分。
- 设置返回按钮与标签同排，标签栏滚动时固定在窗口顶部。
- 缩小窗口时保持设备标题、图标和 Stereo 标记尺寸；更新立体声图标及透明圆形应用图标。
- 明暗及跟随系统外观、窗口居中、托盘、单实例和可选开机自启。
- 保持系统唤醒默认开启，开机自启默认关闭，关闭窗口动作可配置。
- 详细日志与额外采集诊断开关；日志默认脱敏，保留 UUID 与技术数据。
- 统一错误代码及处理说明，区分密码错误、访问限制和配对状态。
- 核对并登记 55 项应用错误码，新增密码通信管道错误 `AUTH_PIPE_FAILED`，不将其判为密码错误。
- 系统错误提示保留原始编号，并注明 Windows Win32、Winsock 或 HRESULT 来源；密码通信和 PCM 管道错误注明具体操作环节。
- Windows 系统错误码通过官方文档查询，不逐项复制到应用错误码目录。

### 兼容范围与限制

- 已主要验证 HomePod mini 单设备与立体声对；其他 AirPlay 接收端的兼容性尚未完整验证。
- 发送格式固定为 44100 Hz、16-bit、双声道，尚未实现通用多房间组网。
- 部分虚拟 Playback 端点首次采集曾出现静音或撕裂，相关初始化和诊断改进已加入；不同驱动环境仍需验证。
- 延迟估计不包含应用和扬声器实际发声耗时，不会同步外部播放器的歌词或视频。

使用方法、构建步骤和第三方材料见发行包附带说明或仓库文档；正式版本标签不改变已记录的第三方授权核对状态。
