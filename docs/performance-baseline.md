# 合成性能基线（2026-10-10）

本记录测量单机 Release 构建的合成处理路径，用于维护前后对照，不是跑分、CPU 使用率或实机播放延迟认证。CI 未纳入本轮。

本轮工具链为 Windows x64 MSVC、`rustc 1.99.0 (b940084d7 2026-09-28)`，使用现有 Cargo 锁定依赖；比较在同一主机和当前电源环境下连续执行，没有固定 CPU 亲和性或隔离系统负载。

## 方法与复现

入口为 `test/core/unit/live/pipeline.rs` 的 `synthetic_performance_baseline`，默认忽略。输入为左右分别 0.25/-0.5 的合成 float32 双声道，每包 10 ms。每种输入采样率执行 1 轮预热、3 轮测量，每轮音频 5 秒。实际调用 `AudioPipeline::process`、跨包 Converter、PCM 队列和写线程，终点为 `io::sink`。每轮验证输出恰好为 220500 帧、882000 字节。

没有 WASAPI、节奏等待、协议时钟、水位控制反馈、日志、GUI 或网络接收端。计时是墙钟时间；每轮首个 process 含转换器初始化，max 因而不能作为稳态实时延迟。Source 的采集复制及左右交换不在这组基线内，另有功能回归覆盖。

```powershell
$env:AIRPLAY_BASELINE_NAME = 'performance-before.json'
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --release --lib synthetic_performance_baseline -- --ignored --nocapture
# 修改后用相同工具链、主机和参数，名称改为 performance-after.json 再运行一次
Remove-Item Env:AIRPLAY_BASELINE_NAME
python test/core/checks/compare_performance.py test/.artifacts/core/performance-before.json test/.artifacts/core/performance-after.json
```

JSON 位于忽略的 `test/.artifacts/core/`；名称只能为该目录内的 `.json` 文件。比较脚本取每种采样率三轮 p95 的中位数；可选 `--max-regression-percent 25` 在任一采样率超过选定阈值时返回 1，默认只报告。尚未建立跨多台主机的稳定阈值，不能将本次短测变化当作确定的性能提升。

## 本轮记录

修改前已保存相同入口的结果，之后加入时长预算和缓冲复用。下表来自本机两份原始产物；数值为三轮中位数，process p95 单位 µs，每轮 wall 单位 ms。

| 输入 Hz | p95 修改前 | p95 修改后 | 变化 | wall 修改前 / 后 |
|---:|---:|---:|---:|---:|
| 44100 | 60.3 | 59.1 | -2.0% | 26.26 / 25.40 |
| 48000 | 51.5 | 53.8 | +4.5% | 25.02 / 25.17 |
| 96000 | 55.0 | 54.4 | -1.1% | 26.49 / 26.34 |
| 192000 | 68.8 | 63.9 | -7.1% | 31.42 / 31.04 |

耗时没有呈现一致的大幅下降。修改后每轮新建 PCM Vec 1–3 个，复用 459–1999 次；PCM 预算峰值约 5–10.9 ms，远低于 640 ms 上限。新建/复用计数只统计 Vec 对象取得方式，不是通用堆分配跟踪器；复用 Vec 扩容仍可能分配。修改前每次发送都新建 PCM 字节 Vec，没有相同计数器，不能将这些计数换算为整机 CPU 降幅。

## 预算与解释边界

Source 和 PCM 队列现在分别限制为 640 ms（包括处理中/写入中的块），并各设 256 块硬上限。预算由帧数和该块采样率计算，ns 向上取整，避免固定 64 块在 44.1/48/96/192 kHz 下对应不同音频时长。极小块仍可能先触及块数上限；预算满立即报错并停止该串流分支，不通过继续扩大积压来恢复。

两级预算不代表实际音频延迟，也不包含原生后端预缓冲、协议播放提前量或接收端渲染。PCM 回收池最多保留 32 个 Vec，写线程只尝试归还。Source 仍复制采集数据；本轮没有实现零拷贝或采集缓冲池。

后续实机测量应另记录来源/驱动/采样格式、CPU 与内存、DPC/调度、队列峰值、丢包/重传、长测时长和听感。32-bit RTP 时间戳全链路回绕、睡眠恢复、磁盘压力及实际 HomePod 声道/时钟兼容性仍需专门验证。
