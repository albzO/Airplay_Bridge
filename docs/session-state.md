# 会话状态与锁边界

最初检查基于 `f4c3bb6` 及 Source 缓冲池更改，保留不同观察状态的含义。2026-10-11 在 `374f3e8` 后针对启动长时间持锁增加准备阶段与后台占用；没有把 PCM_READY、遥测和密码状态合并，完整流程见[启动准备与取消](stream-startup.md)。

## 前端状态

入口为 `airplay-frontend/src/useStreamSession.ts`。

| 状态 | 含义与有效组合 |
|---|---|
| `busy` | 本次连接占用界面，覆盖准备、密码验证、播放及停止收尾 |
| `starting` | `start_stream` 命令尚未返回，允许 `busy=true, session=null`；暂存事件直到得到可信会话编号 |
| `connected` | 收到本次 `PCM_READY`，不等于已收到遥测或接收端已经发声 |
| `playing` | 收到本次遥测；它是观察结果，不是设备实际出声的认证 |
| `stopping` | 已请求停止，仍等待 `finished`；编号返回前先保留意图，返回后按编号补发，busy 保持 true |
| `pending / sending` | 密码目标与当前提交；迟到提交按请求代次和会话编号隔离 |
| `retry / retrySending` | 上次密码拒绝后的重连及自动重提阶段，与首次连接不同 |
| `disposed / request` | 页面卸载与异步请求代次，阻止迟到命令更改新状态 |

`finished` 统一撤销 busy、playing、connected、stopping 和发送状态；旧编号或已结束会话的事件不再打开会话。命令返回前的 `finished` 必须保留，不能只把早到事件全部丢弃。当前没有把“遥测先于 PCM_READY”直接认定为非法组合，也没有增加新的事件顺序要求。

未来若改成显式阶段，适合表示 idle / starting / active / stopping，但 PCM_READY、遥测、密码目标和请求归属仍需要各自表达。不要用界面中文 `phase` 字符串充当内部状态枚举。现有早到事件、停止、密码重试、旧会话及卸载回归应作为行为约束。

## 桌面、Source 与协议

| 对象/状态 | 含义 |
|---|---|
| `Engine.session: Option<Session>` | 慢准备之前预留编号；Preparing → Active，收尾确认后按匹配编号清除，过期占用保留到退出 |
| `Preparation` | Preparing/Active/Expired/Done；准备 15 秒预算、发布核对及单次结束通知 |
| `Engine.configuration_busy` | 配置/发现/预览/采集开关最多一个后台操作；与会话预留在同一短锁下仲裁 |
| `GuiControl.stop / shutdown` | 停止请求及首次请求的 15 秒期限；不表示工作线程已经结束 |
| `GuiControl.cleanup_complete` | 未确认退出的资源阻止重连，旧会话占用保留到应用退出 |
| `Engine.source` | 独立持续采集资源；会话停止后可继续预览 |
| `Source.stop / done` | 分别表示停止请求和采集完成；stop=true、done=false 是正常收尾阶段 |
| `Source.subscriber` | 最多一个串流分支；解除订阅释放队列和回收池，不重置 WASAPI |
| `Source.diagnostic_enabled` | 是否需要逐包记录，不表示 Source 或会话是否运行 |
| `ProtocolState.reader` | reading/eof/failed/cancelled；与协议故障及日志写入状态分开 |
| `ProtocolState.transport` | 后端已通知传输就绪；首个协议错误及最新时钟等快照另行保留 |

`Engine` 的 `configuration_busy`、`quitting` 和 `capture_enabled` 分别服务后台变更互斥、一次退出通知和采集开关表达，不应与播放观察状态混成一个布尔状态。

## 当前锁边界

- `Engine.session` 只保护预留、归属、发布和释放。启动工作线程在锁外读取文件、枚举、保存及等待；停止按编号设置原子标志。独立准备监控不等待 Source/文件锁，过期后保留占用。配置占用也在同一短锁内仲裁，释放时不等待后台操作。
- `set_mapping` 登记配置占用，后台枚举后核对会话，克隆控制对象、保存候选再发布；不持有会话/Source/设置快照锁写盘。配置占用防止端点变化及旧写覆盖新写。Source 切换可持有 Source 锁等待旧采集退出；内存发布使用 Source → session → Preparation 阶段锁，其他路径不得持有 session 再等待 Source。
- Source 的 subscriber 锁把登记、重建判断和每包投递串行化。回调读取 progress、更新 warmup 时分别释放短锁，再取得 subscriber；登记/恢复在 subscriber 内访问 progress/warmup。不要让新的统计路径同时持有 progress/warmup 再反向等待 subscriber。
- 新缓冲池的接收端由 Subscription 独占，在已有 subscriber 锁内尝试取回；不增加另一把池 Mutex。消费者在 sink 借用结束后尝试归还，不持有 subscriber 锁调用 sink，也不等待回收空间。
- 协议、音量和路由锁保护不同的快照/成组值；不能只为减少锁数量而扩大音频路径的临界区。收尾等待改用统一期限；Source 超时保留句柄、旧会话未确认退出时阻止重连，完整边界见 [stderr 与停止期限](stream-shutdown.md)。

启动准备阶段重跑桌面 17、前端脚本 28、DOM 29 项和核心 112 项（9 项默认忽略）。Preparing 专用阶段服务取消、超时及发布互斥，未据此宣称完成并发模型或驱动证明，CI 继续暂缓。
