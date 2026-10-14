# 合成性能基线

合成 Release 测量用于同一环境下的维护前后对照。历史测量值从 Git 查阅，这里只维护当前入口、方法及解释边界；CI 暂缓。

## 转换和 PCM 发送

入口为 `test/core/unit/live/pipeline.rs::synthetic_performance_baseline`，默认忽略。输入为左右 0.25/-0.5 的 float32 双声道，每包 10 ms；每采样率一轮预热、三轮测量，每轮 5 秒音频量。

实际调用 AudioPipeline::process、跨包 Converter、PCM 队列和写线程，写到 io::sink；每轮核对 220500 输出帧、882000 字节。不包含 WASAPI、Source 复制、左右互换分支、协议时钟、水位反馈、日志、GUI 或接收端。

```powershell
$env:AIRPLAY_BASELINE_NAME = 'performance-before.json'
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --release --lib synthetic_performance_baseline -- --ignored --nocapture
Remove-Item Env:AIRPLAY_BASELINE_NAME
# 修改后以相同输入和参数生成 performance-after.json，再比较。
python test/core/checks/compare_performance.py test/.artifacts/core/performance-before.json test/.artifacts/core/performance-after.json
```

每轮首个 process 包含初始化，max 不能直接解释为稳态实时延迟。

## Source 复制与缓冲池

入口为 `test/core/unit/source/baseline.rs::synthetic_source_performance_baseline`，默认忽略。使用真实 Subscription::send_audio 路径，包含订阅锁、复制、时长预算和入队；消费者逐样本核对位值、帧数、采样率和进度，借用结束后归还缓冲。

每种采样率一轮预热、三轮测量，每轮 5000 个 10 ms 包，每 8 包确认消费以限制积压；不按现实时间播放。它不包含 Source 完整采集回调、电平/健康检查、DSP、协议、GUI 或磁盘，也不测量左右互换性能。

```powershell
$env:AIRPLAY_SOURCE_BASELINE_NAME = 'source-before.json'
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --release --lib synthetic_source_performance_baseline -- --ignored --nocapture
Remove-Item Env:AIRPLAY_SOURCE_BASELINE_NAME
# 修改后生成 source-after.json，再使用同一比较脚本。
python test/core/checks/compare_performance.py test/.artifacts/core/source-before.json test/.artifacts/core/source-after.json
```

source_buffers 的 created/reused/growths 统计缓冲取得方式；复用后扩容仍可能分配，不能换算为全进程分配或 CPU 降幅。队列、回收池和音频完整性规则见[架构](architecture.md#音频完整性与缓冲)。

## 产物与比较

JSON 写入忽略的 `test/.artifacts/core/`，名称仅允许该目录下的 .json 文件；默认固定名覆盖，Source 每份小于 64 KiB。前后数据分别在两次代码状态下生成，不修改生产实现来伪造旧基线，不混用两种夹具。

比较脚本取各采样率三轮 p95 的中位数，默认只报告；可选 `--max-regression-percent 25` 作为人工选定阈值。保持同一工具链、锁定依赖、主机、电源状态和输入，记录系统负载；短测受调度、缓存和计时分辨率影响，尚无跨机器稳定阈值。

两级 640 ms 队列预算是积压上限，不能相加解释为真实播放延迟。实机验证应另记录来源/驱动/格式、CPU/内存、DPC/调度、队列峰值、丢包/重传、时长和听感。32-bit RTP 时间戳完整回绕、睡眠恢复、磁盘压力及实际 HomePod 声道/时钟兼容性需要专门验证。
