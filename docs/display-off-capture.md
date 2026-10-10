# 自动熄屏与采集时间戳故障

## 排查结论

用户确认触发场景为 Windows 自动超时熄屏。错误来自 `capture/timeline.rs` 对 WASAPI 包时间戳的校验：当前包时间戳小于或等于上一包时，返回 `CAPTURE_TIMESTAMP_INVALID` 并停止采集。

本次安装版日志中，Source 运行约 **21 分 20.85 秒**后失败，串流报告只有采集错误，协议与 PCM 管道错误为空。最后一个正常进度包为 480 帧、48 kHz；保存的是上一包快照，没有出错包的 QPC 数值，不能进一步判定是重复还是倒退，也不能确定驱动内部原因。

快照中的 `timestamp_errors=0` 只统计 WASAPI 显式时间戳错误标志，而且快照来自已接受的包；软件检测到非递增时间戳仍可能终止采集。附近有 Kernel-Power 566 / SessionUnlock 事件，未查到相邻的睡眠 42 / 恢复 107 事件；这些仅是时间相关证据，不证明显示器、驱动或 Modern Standby 的具体因果关系。

已确认的代码缺口是：原有保持唤醒只请求 `PowerRequestSystemRequired`。它阻止自动睡眠，却不阻止自动熄屏；本次安装版设置中 `keepAwake` 已开启。[Microsoft 电源请求说明](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest)区分系统与显示器请求，需要同时请求两者来保持系统和屏幕唤醒。

## 更改行为

- 复用原 `keepAwake` 设置，默认开启。设置页显示“避免自动睡眠和屏幕熄灭”，在应用运行期间生效，包括托盘、未串流和来源预览状态。
- 同一电源请求对象依次获取 SystemRequired、DisplayRequired。第二项失败撤销第一项，不发布成功状态；错误保留 Windows 信息并归类为 `AWAKE_FAILED`。
- 关闭开关时销毁请求对象，退出时释放请求并关闭句柄；不修改全局电源计划或用户设置。[Microsoft PowerCreateRequest 文档](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powercreaterequest)说明 CloseHandle 会清理对象。
- 保留时间戳校验、Source/Session 生命周期和音频帧顺序。此次采用避免自动熄屏的缓解措施，没有宣称修复底层驱动。

手动睡眠、合盖、强制策略等不在此措施的保证范围内。Windows 对电源请求有策略和用户操作方面的限制，不能将本次改动理解为睡眠/恢复完整支持。

## 故障记录与空间限制

终止性的 `timeline_fault` 即使未开启逐包诊断、没有串流订阅，也会提交到现有 Source 异步日志。记录出错包编号、可用帧数、设备位置、flags、当前及上一包 QPC，以及读取故障时的本机 QPC。重复/倒退错误文本也包含两个包的时间戳，供界面及报告排查。

`packet_qpc_100ns` 与 `previous_packet_qpc_100ns` 是 WASAPI 已换算的 **100 ns 单位**；`read_qpc_ticks` 是本机原始计数，需结合 `capture_start.qpc_frequency` 换算，不能直接相减。[GetBuffer 文档](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)定义了包时间戳单位。

没有新增逐包常驻写盘。记录沿用 Source **32 条非阻塞队列**，`source-startup.jsonl` 当前及上一段各 **2 MiB**，并受目录 **256 MiB / 256 文件**保留规则约束。队列满、磁盘错误等仍可能丢记录，状态会报告丢失/写入错误；错误处理不依赖落盘成功。完整规则见[日志限制](log-retention.md)。

## 验证与复测

- 核心：93 通过、8 项硬件检查默认忽略；加强倒退数值及错误类别断言，检查未启用逐包诊断时的故障 JSON 字段。
- 桌面：9 通过；覆盖两类请求顺序、系统失败、显示器失败后的撤销、撤销失败信息、重复开关与真实 Windows 句柄释放。
- 前端：脚本 23、DOM 28 通过；验证新说明、保存等待、错误类别、回退和重试；类型检查、生产及模拟 UI 构建通过。

首次桌面编译与前端生产构建并行，Tauri 嵌入资源恰好被替换而失败；待构建完成后重跑桌面测试通过。桌面测试实际调用 Windows 电源 API，但没有等待真实显示器超时；DOM 使用模拟 IPC。未执行 HomePod 播放或自动熄屏实机复现。

使用重新构建的程序复测：确认设置页开关开启，持续播放并等待超过 Windows 显示器空闲超时，检查屏幕保持亮起且采集正常。关闭开关或退出后，Windows 应恢复原有超时行为。若仍出现时间戳异常，保留对应报告及 Source 日志中的 `timeline_fault`，继续比较包时间戳与本机计数。
