# Source 性能测量与有界缓冲池（2026-10-11）

## 更改与音频完整性

此前 `Subscription::send_audio` 每包 `samples.to_vec()`。现在仍复制完整输入来脱离 WASAPI/解码缓冲生命周期，只把消费者已经用完的 Vec 存储归还给当前订阅。没有合并、拆分或重排音频包，不改变 float32 位值、帧顺序、采样率、进度快照和现有左右互换方式。

`source/buffers.rs` 管理每次订阅独立的池，采集端只 `try_recv`，消费者在 sink 借用结束后 `try_send`。没有可用存储时分配；容量不足时扩容。关闭、消费者失败或采集异常时解除订阅，释放池和积压音频；下一次订阅重新建池，不带入上次数据。

| 边界 | 行为 |
|---|---|
| 回收数量 | 最多 32 个 Vec |
| 单个回收容量 | 最多 16384 个 f32，即 64 KiB，以实际 capacity 检查 |
| 额外空闲存储 | 至多 2 MiB，不包括音频队列内和消费者正在使用的块 |
| 超大包 | 保持完整交付，用完释放，不为入池而拆包 |
| 池满或已断开 | 释放该缓冲，不等待 |
| 音频积压 | 继续遵守原 640 ms 时长预算及 256 块上限，包含处理中块 |
| 非完整立体声帧 | 返回 InvalidFormat，避免静默接受奇数样本 |

`source_buffers.created / reused / growths` 在最终 capture 报告及 stream_detach 诊断中汇总，分别表示新建 Vec、取得旧 Vec、旧 Vec 容量不足。它们不统计全进程堆分配，复用后扩容仍可能分配。本轮不增加逐包计时或日志，现有[日志限制](log-retention.md)继续生效。

## 测量方法

入口为 `test/core/unit/source/baseline.rs::synthetic_source_performance_baseline`，显式 Release 执行，默认忽略。每种采样率 1 轮预热、3 轮测量，每轮 5000 个 10 ms 双声道包，即 50 秒音频量；不按现实时间等待。每 8 包确认消费完成以限制积压，消费者逐样本核对位值、包长度、采样率及合成进度标记。

实际调用生产 send_audio，计时包含订阅锁、复制、时长预算及入队。消费者运行在另一个线程，wall 包含确认等待与内容验证。没有 WASAPI、Source 电平/健康检查、重采样、协议、网络、GUI 或磁盘日志；不是整个采集回调耗时、CPU 使用率或实际播放延迟。

本机 Windows x64 MSVC，沿用锁定依赖和同一工具链。修改前基于 `f4c3bb6` 加入测量入口执行，生产复制路径尚未改动；修改后使用同一输入、轮数、验证与计时边界，仅增加消费者缓冲归还。没有固定亲和性或隔离系统负载。

## 本次结果

三轮中位数，单包复制/入队 p95 单位 µs；wall 是每轮毫秒。

| 输入 Hz | p95 前 / 后 | 变化 | wall 前 / 后 ms |
|---:|---:|---:|---:|
| 44100 | 1.8 / 0.5 | -72.2% | 6.13 / 5.24 |
| 48000 | 1.6 / 0.5 | -68.8% | 6.48 / 5.23 |
| 96000 | 2.1 / 2.0 | -4.8% | 11.86 / 11.72 |
| 192000 | 2.6 / 2.4 | -7.7% | 16.48 / 16.18 |

修改前每轮新建 5000 个 Vec；修改后四种采样率的三轮中位数均为新建 **8**、复用 **4992**、扩容 **0**。队列峰值均为 **80 ms**，由批量 8 包的夹具决定，不代表实机积压。稳态逐包新建已大幅减少；耗时收益主要出现在本次较低采样率样例，高采样率变化小。微秒级短测受调度、缓存和计时分辨率影响，不能将上述百分比解释为整机 CPU 或实际延迟下降。

## 复现与产物限制

```powershell
$env:AIRPLAY_SOURCE_BASELINE_NAME = 'source-after.json'
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --release --lib synthetic_source_performance_baseline -- --ignored --nocapture
Remove-Item Env:AIRPLAY_SOURCE_BASELINE_NAME
python test/core/checks/compare_performance.py test/.artifacts/core/source-before.json test/.artifacts/core/source-after.json
```

当前源码执行的是池版本；before 是本次修改生产代码前采集的本地产物，不随 Git 提交。以后比较更改前后时，分别在两个版本使用同一测量入口运行。每份 JSON 小于 64 KiB，只允许 `.json` 文件名，写入忽略的 `test/.artifacts/core/`；默认固定文件名并覆盖，不自动生成无限编号。比较脚本默认只报告，可显式指定阈值，不将本次波动设为统一门槛。

## 回归与剩余边界

新增 4 项默认回归：样本位值/独立复制/缩短及扩容；池数量/容量/断开/新订阅；非法或超预算包不复制及超大包完整交付；通过实际 consume 跨线程处理变长包/变采样率/进度，再验证回收统计。原有消费者失败、停止、解除订阅后 Source 存活、下一会话无旧数据和时间线/转换回归继续通过。

核心 **97 通过 / 9 默认忽略**，其中 7 项设备/本机环境检查、2 项显式性能基线。Release Source 基线修改前后各 1 次通过。前端脚本 **23 通过**，未更改生产会话状态机，状态含义与锁边界见[会话状态](session-state.md)。完整 WASAPI/电平诊断路径、系统 CPU/DPC、HomePod 长测和压力场景仍待专门测量；CI 暂缓。
