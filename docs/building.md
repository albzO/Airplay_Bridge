# 构建指南

构建目标为 Windows x64。需要安装：

| 组件 | 用途 |
|---|---|
| Rust 稳定版，MSVC 工具链 | Rust 核心库和桌面应用；需要支持 edition 2024 |
| Visual Studio C++ Build Tools 与 Windows SDK | Rust MSVC 链接及 Windows 开发环境 |
| MSYS2 UCRT64 | 原生 C/C++ 后端，需 GCC/G++、CMake、Ninja、OpenSSL、winpthreads 和 binutils |
| Python 3 | 固定上游源码的编译视图生成 |
| Git | 获取固定子模块、校验提交并应用上下文补丁；构建期间也需要可执行文件 |
| Node.js 20.19+ 或 22.12+，以及 pnpm | Vue/Vite 前端构建 |
| Microsoft Edge WebView2 Runtime | 运行桌面界面 |

将 Python、Node.js 和 pnpm 加入 PATH。构建脚本默认从 `%USERPROFILE%\.cargo\bin` 使用 Cargo，从 `C:\msys64` 使用 MSYS2。

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

# Rust 桌面应用使用 release 配置；原生后端仍按 build-backend.ps1 的配置构建
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

维护者提出更新到新版本时，默认同步桌面应用、前端包、Rust 核心、锁文件及安装脚本版本，并更新 CHANGELOG、README、文档导航、发行包说明和受影响的使用文档；保留历史版本记录。发行文档记录功能改动、使用方式与串流验证范围，不保存安装测试结果。

Whenever the maintainer requests a new version, synchronize application, frontend, core, lockfile and installer versions, and update the changelog, README, documentation index, distribution guide and affected usage documents. Preserve release history, document behavior and streaming validation scope, and omit installation test results.

先构建并检查安装包，再同步发行包内的 Markdown。更改说明文件后重新打包、更新 SHA256 校验值；确认包内说明与源码一致，且没有配置、设备清单或日志。更改程序或安装逻辑时需要重新运行相关安装测试，测试后清理临时记录。

Build and check the installer before synchronizing packaged Markdown. After documentation changes, repack and refresh SHA256 checksums; ensure packaged documents match source and exclude runtime data. Rerun relevant installation checks when application or installer behavior changes, and clean up temporary records afterward.

现有正式安装使用 `scripts/installpackcompiler.iss` 的 Inno Setup 身份。`scripts/package-release.ps1` 当前生成便携 ZIP 和 NSIS 安装包；面向原安装版升级时，需用 Inno Setup 编译器重新生成同名安装包，并重新计算校验值。仅修改版本号不能让 NSIS 接管 Inno Setup 的卸载项和安装目录。

Existing installations use the Inno Setup identity in `scripts/installpackcompiler.iss`. The current packaging script emits a portable ZIP and an NSIS installer; compile the Inno Setup installer for upgrades of existing installations and refresh checksums afterward. A version change alone does not bridge installer families.
