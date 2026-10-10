# 文档导航

本文档对应 AirPlay Hub `1.0.2`，版本改动和验证范围见更新日志。

- [更新日志](../CHANGELOG.md)：本版改进、历史版本与兼容范围。

- [构建指南](building.md)：开发环境、依赖、构建、安装包与版本更新约定。
- [测试说明](../test/README.md)：根目录测试分类、DOM 自动交互、运行入口、合成样例与生成内容。
- [界面与使用](ui.md)：来源、设备、设置、托盘和日志。
- [日志保留限制](log-retention.md)：页面历史、单文件大小、目录清理、活动文件保护及适用边界。
- [CLI 诊断](cli.md)：独立诊断工具入口。
- [架构](architecture.md)：模块职责、音频与控制通道。
- [代码阅读与修改指南](code-reading-guide.md)：关键入口、状态变化、数据流、参数单位和 TypeScript 兼容说明。
- [播放回环排查](playback-loopback.md)：VoiceMeeter VAIO 延迟、原始全零音频的实机对照与设置保存方法。
- [第三方依赖与来源](../THIRD_PARTY.md)：固定版本、使用范围、本地修改和参考资料。
- [许可证状态](licensing.md)：已确认授权、具体待确认事项和发行材料。
- [贡献指南](../CONTRIBUTING.md)：开发约定与验证方式。
- [维护审查记录](security-review-20261008.md)：分支封存、安全修复、历史数据边界与验证结果。
- [维护进度](maintenance-progress.md)：会话、采集解码/统计、日志、协议回绕、上游补丁和音频队列维护记录。
- [原审查复查](review-followup.md)：逐项完成度、剩余问题、优先级和本轮验证范围。
- [性能基线](performance-baseline.md)：无声卡合成测量、复现与解释范围。
- [Source 性能与缓冲池](source-performance.md)：复制/入队对比、存储复用限制及完整性回归。
- [会话状态与锁](session-state.md)：各层状态含义、正常组合及并发边界。
- [自动熄屏采集故障](display-off-capture.md)：时间戳错误证据、系统/屏幕唤醒措施、故障记录与复测边界。

## 历史记录

- [早期架构决定](history/architecture-notes.md)
- [立体声实机验证过程](history/stereo-validation.md)

历史记录用于追溯决定，当前行为以使用与架构文档为准。

- [错误代码与处理方式](error-codes.md)：统一代码目录、协议状态区别与排查建议。
