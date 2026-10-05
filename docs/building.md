# 构建指南

构建目标为 Windows x64。需要安装：

| 组件 | 用途 |
|---|---|
| Rust 稳定版，MSVC 工具链 | Rust 核心库和桌面应用；需要支持 edition 2024 |
| Visual Studio C++ Build Tools 与 Windows SDK | Rust MSVC 链接及 Windows 开发环境 |
| MSYS2 UCRT64 | 原生 C/C++ 后端，需 GCC/G++、CMake、Ninja、OpenSSL、winpthreads 和 binutils |
| Python 3 | 固定上游源码的编译视图生成 |
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

保留完整 `dist/` 目录，包括 `cliairplay-probe.exe` 和所需 DLL；仅复制 GUI exe 无法完成音频串流。

### 自定义后端工具路径

`scripts/build-desktop.ps1` 默认调用同目录的 `build-backend.ps1`。若 MSYS2 或 Python 位于其他位置，先单独构建后端，再跳过该步骤构建界面：

```powershell
# 将路径替换为自己的工具安装位置
.\scripts\build-backend.ps1 -MsysRoot 'D:\Tools\msys64' -Python 'D:\Tools\Python\python.exe'
.\scripts\build-desktop.ps1 -SkipBackend
```

`-SkipBackend` 要求 `dist/` 中已有可用的后端及 DLL。
