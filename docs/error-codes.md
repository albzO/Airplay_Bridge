# 错误代码

本表对应 `docs/error-codes.json`，供 Rust 和界面共用。符号代码用于定位原因；进程退出码仅表示结果类别。同一个退出码 1 可以对应多个符号代码，不能把退出码当成具体协议错误。

## 进程退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | 操作成功；用户停止由 `cancelled` / `stopped_by_user` 标志说明 |
| 1 | 运行失败；必须结合符号代码、phase 和原始系统/协议状态判断 |
| 2 | 命令参数或输入无效 |
| 10–19 | 经过协议字段确认的认证／配对结果，见下表 |

HTTP 状态码、Windows HRESULT、HAP TLV error 是三个独立命名空间，不与软件退出码混用。例如 HTTP 403 不是软件错误 403，退出码 12 也不是 HTTP 12。

## 代码目录

| 符号代码 | CLI 退出码 | 含义 | 处理方式 |
| --- | --- | --- | --- |
| `BACKEND_FAILED` | 1 | 后端操作失败。 | 查看 phase、HTTP/HRESULT 和故障上下文；不能据此断定密码错误。 |
| `BACKEND_USAGE_ERROR` | 2 | 后端命令参数无效。 | 核对参数、文件格式和命令帮助。 |
| `PASSWORD_REJECTED` | 10 | 设备拒绝了输入的 AirPlay 密码。 | 核对密码后重试；只有明确的 M4 secret 拒绝才使用此码。 |
| `PASSWORD_REQUIRED` | 11 | 设备要求输入 AirPlay 密码。 | 提示输入密码；这也是认证流程中的正常挑战。 |
| `PAIRING_BACKOFF` | 12 | 设备要求延迟配对重试。 | 停止连续尝试，按设备的等待要求稍后重试。 |
| `AUTH_REJECTED` | 13 | 设备认证失败，无法确认是密码错误。 | 核对访问权限、认证方式及证明校验结果。 |
| `PAIRING_REQUIRED` | 14 | 设备要求完整配对或有效配对凭据。 | 仅提供 AirPlay 访问密码可能不足以完成授权。 |
| `ACCESS_DENIED` | 15 | 设备拒绝了访问权限。 | 核对设备访问设置；不自动提示更换密码。 |
| `PAIRING_MAX_TRIES` | 16 | 设备已达到认证尝试次数上限。 | 停止重试，按设备的配对恢复流程处理；不保证等待即可恢复。 |
| `PAIRING_MAX_PEERS` | 17 | 设备已达到配对数量上限。 | 检查并清理不再使用的设备配对记录。 |
| `PAIRING_UNAVAILABLE` | 18 | 设备当前不提供此配对方式。 | 核对设备配对状态与兼容性。 |
| `PAIRING_BUSY` | 19 | 设备正在处理其他配对请求。 | 结束其他配对过程后再试。 |
| `INVALID_ARGUMENT` | 2 | 参数无效或超出支持范围。 | 检查必填参数、缓冲、声道映射、音量及输入文件。 |
| `BACKEND_MISSING` | 1 | 缺少音频传输后端。 | 从完整发布目录启动。 |
| `DEVICE_NOT_FOUND` | 1 | 接收端未发现或名称不唯一。 | 刷新设备，使用完整名称。 |
| `DEVICE_SELECTION_INVALID` | 2 | 请选择接收端。 | 选择一台设备或一个立体声对。 |
| `DEVICE_ADDRESS_MISSING` | 1 | 接收端没有可用 IPv4 地址。 | 检查网络及设备发现。 |
| `STEREO_PAIR_INVALID` | 1 | 所选设备不属于同一立体声对。 | 检查设备组信息并重新发现。 |
| `OPERATION_BUSY` | 1 | 当前操作与已有会话冲突。 | 停止串流或等待当前发现操作完成。 |
| `SOURCE_NOT_SELECTED` | 1 | 未选择音频来源。 | 选择有效来源。 |
| `SOURCE_UNAVAILABLE` | 1 | 所选音频来源不可用。 | 检查设备是否存在、启用并支持共享采集。 |
| `CAPTURE_QUEUE_FULL` | 1 | 采集分支队列已满。 | 检查处理速度、设备时钟及调度；不等同于内存泄漏。 |
| `PCM_QUEUE_FULL` | 1 | PCM 队列已满。 | 检查发送阻塞、处理速度和时钟漂移。 |
| `CAPTURE_UNSTABLE` | 1 | 采集管线未能在期限内稳定。 | 检查来源后重试；正常静音本身不是故障。 |
| `CAPTURE_LOOPBACK_STALLED` | 1 | 播放端点有输出，但回环原始音频持续全零。 | 连接前最多自动重建采集三次；播放中先停止串流，再重新开启采集。 |
| `CAPTURE_TIMESTAMP_INVALID` | 1 | 采集时间戳无效或倒退。 | 检查驱动及设备时钟，保留诊断日志。 |
| `CAPTURE_POSITION_INVALID` | 1 | 采集位置倒退或重复。 | 重新开启采集并检查驱动。 |
| `CAPTURE_GAP_EXCESSIVE` | 1 | 采集缺口超过恢复上限。 | 检查睡眠恢复、驱动或系统调度。 |
| `CAPTURE_ENDED` | 1 | 采集器已结束。 | 重新开启采集或查看具体初始化错误。 |
| `STREAM_START_TIMEOUT` | 1 | 串流启动或密码等待超时。 | 检查设备响应和密码输入。 |
| `STREAM_STOP_TIMEOUT` | 1 | 后端未在停止期限内退出。 | 查看停止阶段日志。 |
| `STREAM_INTERRUPTED` | 1 | 后端或发送管道中断。 | 查看最早的故障，而非随后出现的队列错误。 |
| `SESSION_ENDED` | 1 | 会话已经结束。 | 重新开始连接。 |
| `VOLUME_UNAVAILABLE` | 1 | 音量控制通道尚未就绪或已关闭。 | 等待连接完成或重连。 |
| `AWAKE_FAILED` | 1 | 系统唤醒请求失败。 | 检查 Windows 返回错误。 |
| `AUTH_CACHE_FAILED` | 1 | 认证方式记录读写失败。 | 检查应用数据目录权限。 |
| `APP_EXITING` | 1 | 应用正在退出。 | 等待退出完成。 |
| `CANCELLED` | 1 | 操作已取消。 | 用户取消不应显示为串流故障。 |
| `CAPTURE_INIT_FAILED` | 1 | 音频采集初始化失败。 | 检查来源、驱动以及 Windows 错误详情。 |
| `DISCOVERY_FAILED` | 1 | 设备发现失败。 | 检查本地网络与发现服务。 |
| `SETTINGS_SAVE_FAILED` | 1 | 设置保存失败。 | 检查应用数据目录和磁盘。 |
| `AUTOSTART_FAILED` | 1 | 开机自启设置失败。 | 检查当前用户的启动项权限。 |
| `WINDOW_ACTION_FAILED` | 1 | 窗口操作失败。 | 查看窗口状态与 Windows 错误。 |
| `LOG_DIRECTORY_FAILED` | 1 | 无法打开日志目录。 | 检查目录是否存在。 |
| `PASSWORD_SUBMIT_FAILED` | 1 | 密码请求通道失败。 | 检查会话是否仍有效。 |
| `STREAM_FAILED` | 1 | 串流操作失败。 | 查看具体错误详情和故障日志。 |
| `APP_INITIALIZE_FAILED` | 1 | 应用初始化失败。 | 检查配置和音频设备枚举。 |
| `INTERNAL_ERROR` | 1 | 未分类的内部故障。 | 保留详细错误；不要猜测密码或权限原因。 |
| `AUDIO_FORMAT_UNSUPPORTED` | 1 | 音频格式不受支持。 | 检查采样率、位深、声道及格式头。 |
| `INPUT_FILE_INVALID` | 2 | 输入音频文件无效。 | 使用有效、支持的 WAV／PCM 文件。 |
| `RESAMPLE_FAILED` | 1 | 重采样过程异常。 | 检查转换输入、时长及漂移控制。 |
| `CAPTURE_TIMEOUT` | 1 | 采集未在期限内收到完整音频。 | 检查来源与驱动。 |
| `CAPTURE_BUFFER_INVALID` | 1 | 采集缓冲区无效。 | 检查驱动及当前采集格式。 |
| `PCM_PIPE_FAILED` | 1 | PCM 写入或管道异常。 | 检查后端进程和最早的故障日志。 |
| `LOG_WRITE_FAILED` | 1 | 诊断日志写入异常。 | 检查磁盘和目录权限，音频故障需另行判断。 |

| `AUTH_PIPE_FAILED` | 1 | 密码通信管道异常。 | 检查会话和最早的后端故障；不表示密码错误。 |

## 系统错误来源

Windows 系统错误码不在应用代码目录中逐一登记。报错保留原始编号，并注明来源和操作环节，例如：

```text
[AUTH_PIPE_FAILED] 密码通信管道异常。来源：Windows / Win32 232；详情：密码通信 / 发送响应：… (os error 232)
```

`os error 232` 是 Windows 的 `ERROR_NO_DATA`（管道正在关闭），不是本软件的退出码，也不能据此判断密码错误。`0x800700E8` 是包装同一 Win32 232 的 HRESULT；其他 HRESULT 和 Winsock 错误分别注明各自命名空间。密码管道记录读取请求、发送响应或等待读取完成的具体阶段；PCM 管道注明音频发送环节。

原始定义见 [Microsoft 系统错误索引](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes)、[Win32 0–499](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-) 和 [HRESULT 结构](https://learn.microsoft.com/en-us/windows/win32/com/structure-of-com-error-codes)。

## 认证判定与旧码调整

- `PASSWORD_REJECTED` 仅在实际用户 secret 的 M4 认证明确被拒绝时使用。HTTP 401/403、证明不匹配、M2 策略拒绝不归入密码错误。
- `PASSWORD_REQUIRED` 表示设备要求密码。旧 `AUTH_REQUIRED` 是此码的读取兼容别名，新日志不再生成它。
- 旧版将 HAP TLV 3 和 5 都归为退出码 12；现在 12 仅表示 Backoff（TLV 3），5 单独归为 `PAIRING_MAX_TRIES` / 16。TLV 4、6、7 分别使用 17、18、19。依据 [Apple HomeKit ADK 的 HAPPairingError 定义](https://github.com/apple/HomeKitADK/blob/master/HAP/HAPPairing.h)。
- HTTP 470 对应 `PAIRING_REQUIRED`；HTTP 401/403 对应 `ACCESS_DENIED`，保留阶段和状态以便诊断。
- 普通后端失败统一输出 `BACKEND_FAILED` / 1。内部资源创建失败不得再使用参数错误退出码 2。
- 队列满、时间戳错误、采集器结束、启动／停止超时现在有独立符号标识。未匹配的 Windows 错误按当前操作归类，原始详情作为附加信息保留，不虚构根因。
- `CANCELLED` 可作为 CLI 非成功结束的 1；GUI 用户停止仍由取消标志表示，不显示红色故障。

## 输出与隐私

用户提示格式为 `[符号代码] 简要原因。来源：错误来源；详情：操作环节与原始错误`。认证后端格式为 `[PROBE] ERROR code=... exit=... host=... phase=...`，日志落盘前隐藏真实设备名、IP、用户名路径和设备公钥／MAC，保留 UUID、采样格式、包计数、时序及错误码。

界面设备列表保留真实名称。内部连接、认证缓存和音频来源选择保留真实值；脱敏只作用于日志和故障报告，不能改变协议路由。历史日志不自动覆盖或删除，旧日志可能仍含个人信息。当前可执行文件也不会随源码编辑自动变化，必须在下次编译更新后生效。第三方源码与 LICENSE 作者署名不做匿名化。

## 维护

新增故障时先登记 JSON 中的代码、退出码、提示和处理方式，再为明确的检测点添加匹配。前端按 Tauri 命令提供兜底类别；Rust 会话和 CLI 使用同一目录。修改此 JSON 后同步此 Markdown；原始 HRESULT / HTTP / TLV 要保留，禁止全部折叠成密码错误。
