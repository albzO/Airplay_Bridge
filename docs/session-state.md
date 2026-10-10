# 会话状态与锁边界

本次检查基于 `f4c3bb6` 及 Source 缓冲池更改。没有发现必须重写会话状态机的具体问题，保留现有行为，记录不同状态的含义，避免为了减少布尔值而合并不同事件。

## 前端状态

入口为 `airplay-frontend/src/useStreamSession.ts`。

| 状态 | 含义与有效组合 |
|---|---|
| `busy` | 本次连接占用界面，覆盖准备、密码验证、播放及停止收尾 |
| `starting` | `start_stream` 命令尚未返回，允许 `busy=true, session=null`；暂存事件直到得到可信会话编号 |
| `connected` | 收到本次 `PCM_READY`，不等于已收到遥测或接收端已经发声 |
| `playing` | 收到本次遥测；它是观察结果，不是设备实际出声的认证 |
| `stopping` | 已请求停止，仍等待 `finished`；此时 `busy` 保持 true，已有 connected/playing 可以暂时保留 |
| `pending / sending` | 密码目标与当前提交；迟到提交按请求代次和会话编号隔离 |
| `retry / retrySending` | 上次密码拒绝后的重连及自动重提阶段，与首次连接不同 |
| `disposed / request` | 页面卸载与异步请求代次，阻止迟到命令更改新状态 |

`finished` 统一撤销 busy、playing、connected、stopping 和发送状态；旧编号或已结束会话的事件不再打开会话。命令返回前的 `finished` 必须保留，不能只把早到事件全部丢弃。当前没有把“遥测先于 PCM_READY”直接认定为非法组合，也没有增加新的事件顺序要求。

未来若改成显式阶段，适合表示 idle / starting / active / stopping，但 PCM_READY、遥测、密码目标和请求归属仍需要各自表达。不要用界面中文 `phase` 字符串充当内部状态枚举。现有早到事件、停止、密码重试、旧会话及卸载回归应作为行为约束。

## 桌面、Source 与协议

| 对象/状态 | 含义 |
|---|---|
| `Engine.session: Option<Session>` | 独占连接/播放占用，从登记到工作线程和密码服务收尾后才清除 |
| `GuiControl.stop` | 停止请求，不表示工作线程已经结束 |
| `Engine.source` | 独立持续采集资源；会话停止后可继续预览 |
| `Source.stop / done` | 分别表示停止请求和采集完成；stop=true、done=false 是正常收尾阶段 |
| `Source.subscriber` | 最多一个串流分支；解除订阅释放队列和回收池，不重置 WASAPI |
| `Source.diagnostic_enabled` | 是否需要逐包记录，不表示 Source 或会话是否运行 |
| `ProtocolState.transport` | 后端已通知传输就绪；首个协议错误及最新时钟等快照另行保留 |

`Engine` 的 `discovering`、`quitting` 和 `capture_enabled` 分别服务发现互斥、一次退出通知和采集开关表达，不应与会话播放阶段混成一个布尔状态。

## 当前锁边界

- `Engine.session` 保护登记与收尾。`start_stream` 登记后释放它才启动工作线程，停止命令仅设置原子标志；工作线程释放占用后再通知 finished。准备阶段仍有枚举、保存和资源初始化，不能据此宣称所有桌面命令都有严格时限。
- `set_mapping` 在枚举后按 session → source → settings 取得对象，重新核对端点，再保存候选并发布路由。不要新增反向嵌套锁；读取设置的前置快照在取得后续锁之前释放。
- Source 的 subscriber 锁把登记、重建判断和每包投递串行化。回调读取 progress、更新 warmup 时分别释放短锁，再取得 subscriber；登记/恢复在 subscriber 内访问 progress/warmup。不要让新的统计路径同时持有 progress/warmup 再反向等待 subscriber。
- 新缓冲池的接收端由 Subscription 独占，在已有 subscriber 锁内尝试取回；不增加另一把池 Mutex。消费者在 sink 借用结束后尝试归还，不持有 subscriber 锁调用 sink，也不等待回收空间。
- 协议、音量和路由锁保护不同的快照/成组值；不能只为减少锁数量而扩大音频路径的临界区。整体退出和线程 join 仍有原先的阻塞边界。

本轮重跑前端脚本 23 项及核心 97 项，覆盖原有会话归属、Source 失败/取消/重连与新增缓冲复用。没有新增生产状态机或声称完成并发模型证明，CI 继续暂缓。
