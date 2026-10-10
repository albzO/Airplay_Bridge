# 代码结构复查与仓库整理（2026-10-11）

基于已提交的 `d5b5b25` 检查第一方代码、调用入口、编译器诊断、测试挂接、文档链接和本地生成目录。本次整理不更改协议、音频包、Source/Session 生命周期或安装身份；修改留待手动提交，未执行 commit 或 push。

## 结构结论

| 层 | 当前边界 | 结论 |
|---|---|---|
| Vue 页面与组件 | App 编排设置/命令/设备选择；视图组件只展示及发事件；useStreamSession 管会话，useDiagnostics 管诊断 | 分工已有实现和回归，不按行数继续拆分 |
| Tauri 桌面 | main 注册命令与共享状态；session/stream_start/configuration 管理预留、后台准备及配置互斥，settings/routing/auth/awake/window 管持久化、控制及资源 | 保留单一 Engine；状态和锁含义见[会话状态](session-state.md) |
| Rust 核心 | Source 持续采集及单订阅，capture 子模块负责时间线/解码/统计；live 子模块负责转换、管道、协议与日志 | 继续保持持续来源与每次播放会话的区别 |
| C 后端 | 本地 Windows 兼容层、上下文补丁、固定上游版本和校验/选择脚本 | 文件通过 CMake、include 或生成选择器使用，不能只按文件名判断为废弃 |
| 测试 | frontend/backend/core 分类，Rust 私有模块通过 path 挂接 | 将零散子模块测试归位，保留模块名与运行过滤器 |
| 构建和文档 | scripts 管构建/打包；docs 分使用、开发验证、维护授权和历史 | 补齐根目录树，调整导航与清理说明 |

NSIS 与 Inno 两套脚本分别由当前打包脚本和已有安装版升级流程使用，不属于无用重复。`homepod-test.exe` 是现用诊断 CLI；桌面启动检查和原生 nonce 自检也有明确入口，继续保留。Rust 的 public API、C 导出/宏适配符号不能仅凭一次文本搜索删除。

## 代码清理

- `vue-tsc --noUnusedLocals --noUnusedParameters` 确认 App 中解构出的 session 未使用，已移除。脚本测试改为直接观察生产 useStreamSession 返回的会话编号，不为测试保留无用页面绑定。
- `backend::executable` 的 `_root` 参数从未参与定位，已移除并更新两个调用方。程序仍只从允许的程序目录定位后端，不改为从数据目录加载；原定位回归通过。
- tsconfig 正式启用未使用局部变量/参数检查，后续生产 build 自动执行。Rust 全目标检查无未使用代码警告；这不等于证明所有公开 API 都被外部使用。
- capture、health、Source 的测试挂接统一放到模块尾部，避免测试声明打断生产实现。

## 测试归位

所有文件内容和模块名保留，仅调整路径及文档引用。

| 原文件（相对 test/core/unit） | 新文件 |
|---|---|
| backend_executable_tests.rs | backend.rs |
| capture_gap_recovery_tests.rs | capture/gap_recovery.rs |
| capture_loopback_checks.rs | capture/loopback.rs |
| source_baseline.rs | source/baseline.rs |
| source_buffers.rs | source/buffers.rs |

现有 Cargo 测试命令、Source 性能过滤器、默认忽略标记、私有成员访问和覆盖范围保持不变。没有删除正式测试或合成素材。

## 本地生成残留

删除了 `test/.artifacts/` 下 upstream-before / after / baseline / guarded 四份旧生成快照，以及一次上游迁移使用的七个临时脚本/数据文件；这些不是正式 manifest、patch、选择器或测试依赖。同时删除第一方三个 Python 字节码缓存目录。

总计 **14 个目标、77 个文件、2084456 字节（约 1.99 MiB）**。删除前逐项确认绝对路径在工作区内、没有 Git 跟踪文件、路径及子树没有重解析点；没有递归清空整个仓库或 Git 元数据。

性能 before/after 原始结果、review-audit 故障证据、配置与快照备份、编译缓存、依赖、工具以及 dist/releases 继续保留。新增 formatter 忽略项覆盖 releases、test/.artifacts 和 Tauri gen，避免把生成内容混入排版。后续缓存可重建；运行测试时可设置 `PYTHONDONTWRITEBYTECODE=1` 避免重新产生 Python 字节码。

## 验证

- 核心 97 通过 / 9 默认忽略；桌面 9、前端脚本 23、真实模板 DOM 28 通过。
- 上游固定输入/补丁/输出校验 7 项、模拟测试清理工具 3 项通过；删除旧快照后仍能从正式源码运行。
- Vue 类型与未使用检查、生产和模拟 UI 构建通过。Rust 格式、前端格式、敏感数据规则和差异检查随本次整理执行。
- 测试挂接目标和文档本地链接核对；发行说明里的相对链接按其复制到 dist/docs 后的路径解释。

未重跑原生协议矩阵、长测、性能测量、安装验证或 HomePod 播放。CI 按已有约定暂缓。没有依据本次静态检查重新给可维护性/性能打分。

## 仍存在的结构边界

1. stderr 独立读取状态已在后续任务补齐，保留原始 OS 错误、EOF/取消区别与协议首因。
2. 后续任务增加共享 15 秒收尾期限和应用退出兜底；任意回调及底层 I/O 仍不能安全强杀，未退出资源阻止重连。实现和验证边界见 [stderr 与停止期限](stream-shutdown.md)。
3. 后续任务已将启动准备和配置命令迁出 main，增加短锁预留、取消核对与准备期限，见[启动准备](stream-startup.md)。配置/初始化驱动调用仍无统一响应期限，未完成操作持有占用阻止重复累积。
4. App、桌面编排模块和 capture 仍负责必要的跨模块编排。后续拆分应围绕实际变更或可测试的职责，不继续以减少行数为目标。

前两项由后续独立变更处理，本次结构清理自身没有包含这些行为修改。仍未宣称完成驱动或并发模型证明。
