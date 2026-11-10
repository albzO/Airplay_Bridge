# 构建指南

构建目标为 Windows x64。需要安装：

| 组件 | 用途 |
|---|---|
| Rust 稳定版，MSVC 工具链 | Rust 核心库和桌面应用；需要支持 edition 2024 |
| Visual Studio C++ Build Tools 与 Windows SDK | Rust MSVC 链接、GPUI 的着色器编译和程序资源 |
| MSYS2 UCRT64 | 原生 C/C++ 后端，需 GCC/G++、CMake、Ninja、OpenSSL、winpthreads 和 binutils |
| Python 3 | 生成固定上游源码的编译视图与发行包 Rust 许可清单 |
| Git | 获取固定子模块、校验提交并应用上下文补丁；构建期间也需要可执行文件 |

将 Cargo 和 Python 加入 PATH。后端构建默认使用 `C:\msys64`。旧 Vue/Tauri 对照入口的构建和测试使用 Node.js、pnpm 与 WebView2。

### 获取源码

克隆本仓库，进入项目根目录。第三方源码通过子模块从原作者仓库获取，本仓库仅保存地址与固定提交引用。来源和版本见 [第三方记录](../THIRD_PARTY.md)。

```powershell
# 只初始化本项目需要的三层依赖，保持固定提交
git submodule update --init -- upstream/airplay-cli
git -C upstream/airplay-cli submodule update --init -- libraop
git -C upstream/airplay-cli/libraop submodule update --init -- crosstools
```

无需获取其他上游子模块，不要使用 `--remote` 将依赖升级到最新分支。GitHub 自动生成的源码 ZIP 不包含子模块内容；请通过 Git 克隆后执行上述命令。使用根目录构建脚本，上游原始 Makefile 的完整功能不属于本项目构建范围。

### 上游适配变更

构建使用“固定子模块版本 → 临时源码副本 → 按顺序应用补丁 → 选择编译范围”的流程。源码差异在 [`airplay-backend/patches/`](../airplay-backend/patches/README.md) 中维护，Windows 接口在 `windows_port.c/.h`、`windows_io.inc`、`windows_audio.inc` 中维护；Python 只校验、应用补丁、提取函数/常量和组合本地 C 文件，不通过字符串或正则替换修改源码。

`upstream-manifest.json` 保存三层子模块提交、15 个输入文件的规范化 SHA-256、补丁顺序及哈希、15 个输出快照。输入和补丁按 UTF-8/LF 规范化，允许 CRLF 检出。`git apply --check` 及实际应用均须成功，不启用忽略空白、三方合并或部分接受；补丁失败即丢弃临时副本，已有生成输出保留。子模块源码和 Git 索引不受修改。

维护时按以下顺序操作：

1. 先生成并保留旧输出用于对比。平台实现修改放在本地 Windows 兼容文件；上游函数的修改放在对应补丁；仅增减编译范围时修改 `select_upstream.py`。
2. 在忽略的临时副本中编辑源码并重新生成有上下文的 unified diff，保持 `a/src/...`、`b/src/...` 等相对路径，勿直接在子模块或生成目录长期维护修改。每个补丁以之前补丁已应用的状态为基准，清单中的顺序即应用顺序。
3. 升级依赖时单独更新子模块固定提交，逐项核对原函数、补丁上下文和本地结构声明；更新 `revisions`、相关 `inputs`、补丁哈希及 `THIRD_PARTY.md`。补丁能应用不等于上游 API/协议语义兼容。
4. 运行生成脚本，审查新旧输出差异，再更新受影响的 `outputs` 快照。运行提取测试、原生构建和协议模拟。不得为通过构建自动接受新哈希。清单是维护断言，不是上游可信度认证。

```powershell
python airplay-backend/select_upstream.py upstream/airplay-cli test/.artifacts/upstream-review
python test/backend/test_upstream.py
```

生成目录的 `selection-report.json` 提供提交、输入哈希、按应用顺序排列的补丁及输出哈希，供审查对照。CMake 监视清单、补丁、本地适配、上游输入和子模块 HEAD，变化后重新配置；构建中使用 CMake 找到的 Git。具体测试命令见 [测试说明](../test/README.md)。

保留根目录 `test/`：生产模块通过挂接复用其中的测试及诊断检查实现，完整源码检出已包含这些文件。测试源码和合成样例随 Git 提交，测试生成内容位于忽略的 `test/.artifacts/`；运行方式见 [测试说明](../test/README.md)。

### 构建桌面应用

在项目根目录的 PowerShell 中执行：

```powershell
# 首次构建：下载依赖并构建全部组件
.\scripts\build-desktop.ps1

# GPUI 桌面应用和诊断 CLI 使用 Release 配置
.\scripts\build-desktop.ps1 -Release

# 已下载全部依赖后，离线构建
.\scripts\build-desktop.ps1 -Offline
```

输出位于 `dist/`。运行：

```powershell
.\dist\airplay-bridge.exe
```

保留完整 `dist/` 目录，包括 `runtime/airplay-backend.exe` 和同目录的所需 DLL；仅复制 GUI exe 无法完成音频串流。

### 自定义后端工具路径

`scripts/build-desktop.ps1` 默认调用同目录的 `build-backend.ps1`。若 MSYS2 或 Python 位于其他位置，先单独构建后端，再跳过该步骤构建界面：

```powershell
# 将路径替换为自己的工具安装位置
.\scripts\build-backend.ps1 -MsysRoot 'D:\Tools\msys64' -Python 'D:\Tools\Python\python.exe'
.\scripts\build-desktop.ps1 -SkipBackend
```

`-SkipBackend` 要求 `dist/` 中已有可用的后端及 DLL。

原生协议修改在 `airplay-backend/patches/` 和本地 C 适配中维护，`select_upstream.py` 只选择与组合编译内容。修改后需要重新构建后端；不要直接编辑 `build/airplay-backend/generated/` 或固定上游源码。控制会话与音频流分步建立，音频 `SETUP` 即使发生在控制会话已接受之后，也使用 8 秒启动预算。普通控制请求仍使用 2 秒预算；预算是响应等待上限，不是固定播放延迟。

Maintain native changes in `airplay-backend/patches/` and local C adapters; Python selects and assembles the patched build. Rebuild after changes. Do not edit generated files or pinned upstream sources. Audio `SETUP` retains an 8-second startup budget even after control session acceptance; regular control requests retain their 2-second budget. These are response deadlines, not fixed playback delays.

## 发行目录

`dist/airplay-bridge.exe` 为主入口，原生后端和 DLL 位于 `dist/runtime/`，诊断工具位于 `dist/tools/`，说明位于 `dist/docs/`，许可证位于 `dist/licenses/` 和根目录 LICENSE、NOTICE。构建脚本会持续输出这一布局。

不将设备清单、日志、录音或界面检查输出加入发行包。GUI 数据位于 `%APPDATA%\AirPlay Hub`，CLI 数据位于其 `cli/` 子目录。

## 版本更新与发行 / Version updates and distribution

桌面版本取自 `airplay-frontend/src-gpui/Cargo.toml`，同时用于程序文件资源、安装包和便携包。发行脚本在 `releases/v<版本>/` 输出沿用既有 AppId 的 Inno Setup 安装包、便携 ZIP 和 `SHA256SUMS.txt`。通过 `-Iscc` 可指定 Inno Setup 编译器。

发行版 GUI 和诊断 CLI 静态链接 MSVC C 运行库。原生 AirPlay 后端所需的 DLL 随 `runtime/` 一起分发。

```powershell
.\scripts\package-release.ps1 -Offline
# 已构建同版本 Release 时复用 dist
.\scripts\package-release.ps1 -SkipBuild -Offline
# 另外生成用于全新安装的 NSIS 包
.\scripts\package-release.ps1 -SkipBuild -Offline -IncludeNsis
```

既有 Inno Setup 安装用默认安装包升级。便携版升级前备份 `data/`，退出后替换程序与 `runtime/`。发行脚本从明确清单复制文件，另附 Windows Rust 依赖索引及其源码包中的许可证和通知。文档改动随下一次打包重新生成校验值。
