# 上游源码补丁

这些补丁作用于 `upstream/airplay-cli` 的固定版本源码副本，按 `../upstream-manifest.json` 中的顺序应用。路径以 airplay-cli 根目录为基准，包含其 `libraop/src/` 输入。提交与来源见 [第三方记录](../../THIRD_PARTY.md)。构建不修改子模块、不修改 Git 索引，不需要 `patch.exe`；需要 Git 可执行文件。

| 补丁 | 修改范围 |
|---|---|
| `0001-windows-platform.patch` | Windows 头文件、socket 宽度、Winsock I/O 引用、PTP 线程标志/关闭、bplist 字节序及 include 路径 |
| `0002-hap-events-and-errors.patch` | 保留实际 M2/M4 状态、事件通道密钥及独立上下文、认证阶段标记、禁止记录原始认证响应体 |
| `0003-session-and-realtime.patch` | 会话/音频建立分步、认证分类及本地密码适配入口、SETUP 预算、错误/端口检查、64-bit 音频 nonce、raw ALAC 参数/分配检查 |

`windows_port.c/.h`、`windows_io.inc` 和 `windows_audio.inc` 实现平台接口；`probe_context.inc` 保存所选代码的本地结构声明；其余 `.inc` 管理本地入口、密码重试、事件及音量。修改上游函数使用补丁，修改平台实现使用本地 C 文件，不再在 Python 添加 `replace` / `re.sub`。

生成器保留函数和常量选择、PTP daemon 截断及本地 C 文件组合，用于排除未构建的完整 CLI 功能。补丁后的完整上游文件是选择编译内容的中间输入，并非可独立构建的通用 Windows 分支；升级时仍须审查函数依赖和 `probe_context.inc` 的结构声明。

补丁使用标准 unified diff 和上下文行，应用前验证输入/补丁哈希与三层提交。逐组执行 `git apply --check --whitespace=error-all`，检查成功后应用；不忽略空白，不执行三方合并或保留部分失败结果。源码与补丁按 LF 规范化，支持 Git CRLF 检出。

维护和升级步骤见 [构建指南](../../docs/building.md#上游适配变更)。不得只为了让构建通过而更新清单或输出快照。补丁迁移保持既有协议行为，不代表无需实机验证或上游升级可以自动完成。
