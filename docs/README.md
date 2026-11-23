# 文档导航

本文档对应 AirPlay Hub **2.0.0 发行候选**，v1.0.3 为对照版本。这里只保留当前版本的使用、开发、排查和交付说明；历史版本与已完成的优化、审查过程从 Git 历史查阅。

## 使用与排查

- [界面与使用](ui.md)：来源、设备、设置、托盘和日志。
- [CLI 诊断](cli.md)：独立诊断工具入口。
- [错误代码](error-codes.md)：错误含义、来源和处理方式。
- [日志保留限制](log-retention.md)：页面历史、文件和目录上限。
- [播放回环排查](playback-loopback.md)：Playback 来源、VoiceMeeter 及无声问题。
- [自动熄屏采集故障](display-off-capture.md)：时间戳错误、保持唤醒及适用边界。

## 开发与验证

- [技术路线](tech-stack.zh-CN.md)：三版界面的选型原因、2.0 的技术组合、自行实现部分与取舍。
- [架构](architecture.md)：模块、会话状态、启动取消、通知、认证保存和停止期限。
- [代码阅读指南](code-reading-guide.md)：代码入口、数据流和修改位置。
- [构建指南](building.md)：环境、上游补丁、版本同步及安装包。
- [测试说明](../test/README.md)：测试分类、命令和生成内容。
- [性能基线](performance-baseline.md)：合成测量方法、产物限制与解释范围。
- [贡献指南](../CONTRIBUTING.md)：开发、验证和提交约定。

## 当前发行与授权

- [更新日志](../CHANGELOG.md)：当前及历史版本的功能变更。
- [v1.0.3 交付简报](release-v1.0.3.md)：本次更改、验证范围和产物位置。
- [第三方依赖与来源](../THIRD_PARTY.md)：固定版本、修改和授权来源。
- [许可证状态](licensing.md)：授权边界和发行材料。

`error-codes.json` 是 Rust/TypeScript 共用的运行时错误契约，随源码保留。

## 文档维护规则

每次发行直接更新现行文档，功能变化写入 CHANGELOG；当前交付简报随版本替换。已完成的优化、审查、逐轮测试计数和操作流水不继续积存在 docs，也不另建历史目录。

删除过程记录前，先将仍有效的规则或限制合并到相应指南，并确认原文已入 Git。查阅旧文档时使用 `git log -- docs`，再用 `git show <commit>:docs/<文件名>` 查看。
