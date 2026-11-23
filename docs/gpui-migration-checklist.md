# AirPlay Bridge GPUI 迁移任务清单

以 `main` 的 `79f1295`（v1.0.3）为对照。首轮目标是在 Windows x64 上用 GPUI 替换 Vue/Tauri 桌面层，复用 `airplay-core`、原生 `airplay-backend.exe` 和 CLI。现有串流、配置和退出行为作为验收依据。

## P0 · 固定基线，验证 Windows 原型

- [ ] **保存旧版对照。** 保留 v1.0.3 安装包、便携包和校验值，用脱敏数据记录主要页面、设置默认值、首启和恢复流程。在同一台机器上记录启动到可交互时间、空闲及托盘 CPU/内存、播放时的音频丢包或队列异常，供新旧版对比。按 [测试说明](../test/README.md) 记录现有测试结果。
- [ ] **固定构建输入。** 在隔离的 Windows x64/MSVC 小项目验证 GPUI，记录 Rust、Windows SDK、GPUI 来源和 feature，提交 `Cargo.lock`。如果采用 `gpui-component`，验证它与 GPUI 共用兼容版本，确认密码框、下拉和滑块可用。选 crates.io 时锁定验证过的版本，选 Git 时固定到提交。用 `cargo tree --locked --target x86_64-pc-windows-msvc` 核对解析结果和新增依赖的许可与 NOTICE。缓存依赖后验证离线 Release 构建。参见 [GPUI 官方 README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)、[GPUI Component 安装说明](https://longbridge.github.io/gpui-component/#install-gpui-component)。
- [ ] **打通托盘和单实例。** 原型验证关闭窗口后事件循环继续运行，托盘点击能恢复、取消最小化并尝试激活主窗，托盘菜单可退出。连续双击启动时只初始化一次，第二个进程通知已有实例恢复窗口。优先验证 GPUI 窗口接口和 Windows 托盘库组合，必要时通过小范围 Win32 适配处理 HWND、消息投递及窗口显示。正式迁移前确定实例作用域和启动竞态处理方式。对照 [窗口与托盘](../airplay-frontend/src-ely/src/shell.rs) 和 [GPUI Windows 窗口示例](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/window.rs)。
- [ ] **验证发布环境和输入。** 在计划支持的最低 Windows 版本和 Windows 11 上启动 Release 原型，至少覆盖常见缩放比例、跨屏移动、最小尺寸、中文字体与 IME、Tab 焦点、密码掩码、Enter 提交和错误后全选。检查无 WebView2 的干净环境能显示窗口，确认实际 GPU/驱动和运行库要求。GPUI 上游使用 Win32 窗口和 DirectWrite 文字渲染，具体验收以锁定版本为准。参见 [GPUI Windows 平台说明](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md#gpui_platform)。

v1.0.3 两份发行文件通过 SHA256 核对。预研用 GPUI `0.2.2`、GPUI Component `0.5.1` 和 tray-icon `0.25.1` 在 Windows x64/MSVC 上通过离线 Release 依赖编译。按维护者要求跳过独立原型的手动验证，直接进入桌面服务拆分。待新入口接入后验证它的 Windows 行为。

## P1 · 把桌面逻辑从 Tauri 接口中抽出来

- [x] **提取可直接调用的 Rust 桌面服务。** `airplay-desktop` 持有 `Engine`、配置、发现、Source、串流和认证逻辑。界面直接调用这套服务，配置作业在交给后台前预留占用。参见 [桌面服务](../airplay-desktop/src/lib.rs) 和 [配置](../airplay-desktop/src/configuration.rs)。
- [x] **建立后台到 UI 的事件边界。** `Initialization` 提供 Rust 初始化数据，`EventSink` 交付电平和带会话编号的事件。Tauri 适配层保留既有 JSON 字段。核心的通知队列继续处理电平合并、控制顺序和最终报告。参见 [架构和通知契约](architecture.md)。
- [x] **分离退出收尾和窗口退出。** `Engine::begin_shutdown` 复用统一停止期限、唤醒释放和 Source 收尾。窗口事件循环与进程退出留给 UI 入口。继续使用现有 Windows 自启注册表项、安装及便携数据路径、密码管道。参见 [窗口退出实现](../airplay-frontend/src-ely/src/shell.rs) 和 [数据目录实现](../airplay-core/src/data_dir.rs)。

## P2 · 按用户流程迁移界面

- [x] **接入 GPUI 窗口和事件。** Windows x64 的独立入口接上 `Engine`、后台作业、托盘、单实例唤醒和退出收尾。电平与遥测合并，控制 FIFO 有上限并为最终事件留独立位置。参见 [事件桥接](../airplay-frontend/src-ely/src/inbox.rs) 和 [窗口入口](../airplay-frontend/src-ely/src/shell.rs)。
- [x] **实现最小播放闭环。** 按播放和录音分组选择来源，配置提交后启动预览；可刷新和选择 AirPlay 接收端或立体声对，开始、停止以及输入和重试密码。原生 Rust 会话同步取得可信编号，区分 `PCM_READY` 与实际播放，并过滤迟到事件。参见 [播放页](../airplay-frontend/src-ely/src/player.rs) 和 [会话状态](../airplay-frontend/src-ely/src/session.rs)。
- [x] **补齐播放控件和常规设置。** 音量显示使用接收端回报，静音可恢复最近一次确认的非零音量。播放页提供立体声左右互换，常规设置提供输入声道映射、播放提前量、关闭动作、外观、开机自启和保持唤醒。播放与设置共用底部的来源、采集和串流控件。设置提交失败时沿用已确认值。参见 [播放与设置控件](../airplay-frontend/src-ely/src/view.rs)。
- [x] **迁移诊断和窗口交互。** 设置页已接入运行统计、技术详情、日志和会话报告。会话日志使用脱敏副本，清空显示只清除逐包摘要。窗口采用 Windows 原生标题栏，关闭交由现有托盘和退出逻辑处理。参见 [诊断状态](../airplay-frontend/src-ely/src/diagnostics.rs)、[诊断页面](../airplay-frontend/src-ely/src/settings_view.rs) 和 [日志保留规则](log-retention.md)。

从仓库根目录运行 `.\scripts\build-gpui.ps1 -Offline` 构建预览版。脚本从 `dist/runtime` 取现有后端，输出 `build/gpui-preview/airplay-bridge-gpui.exe`。预览版使用自己的 `portable.flag`、`data/` 和 `AirPlay Bridge GPUI Preview` 开机自启项。真机串流、托盘和输入体验的验收仍按后续清单执行。

## P2.5 · 2.0 界面（Ely）

1.1 候选界面经评审未达发布要求，2.0 在同一服务层上用 `gpui-pre 0.3.11` + Ely GPUI Components 重做，原因见[技术路线](tech-stack.zh-CN.md)。

- [x] **独立 crate 与共用模块。** `airplay-frontend/src-ely` 接入 `Engine`、托盘、单实例与退出收尾，会话、事件、诊断、音量和作业模块从 `src-gpui` 移入。
- [x] **界面与交互。** 窗口内标题栏、8 px 网格分组卡片、底部固定栏、设备卡片展开、设置四个子页、过渡动画、平滑与弹性滚动、自写来源下拉。
- [x] **双语与安装语言。** 中英文案、跟随系统或手动选择；NSIS 与 Inno Setup 写入安装语言。
- [x] **服务层调整。** 保持唤醒只在串流中生效；发现设备只与开始串流互斥；空闲时窗口不可见即暂停采集。桌面服务 25 项、2.0 界面 15 项、1.1 界面 9 项、旧前端逻辑 29 项与 DOM 29 项通过。
- [x] **切换打包入口。** `build-desktop.ps1`、`package-release.ps1` 改为构建 `src-ely`，依赖通知包含 Ely、随包字体与图标，版本取自 `src-ely/Cargo.toml`，对外显示为 2.0。
- [ ] **实机验收。** 真实接收端的串流、密码、音量、立体声交换、熄屏后恢复采集，以及安装语言传递。

## P3 · 回归通过后切换发行入口

- [x] **完成本机自动回归。** 核心 119 项、桌面服务 21 项、GPUI 9 项、旧前端逻辑 29 项和 DOM 29 项通过。模拟接收端覆盖认证、密码管道及重试、立体声和慢启动。GPUI 增加 COM 作业线程回归。
- [x] **切换构建与打包。** `build-desktop.ps1` 默认构建 GPUI 版 `airplay-bridge.exe`。Windows x64 上已跑通完整的 `-Release -Offline` 构建，并用产出的后端重跑模拟回归。Cargo 版本写入 Windows 资源和两个安装器，发行包包含后端、CLI 和 Rust 依赖通知。Inno Setup 沿用 v1.0.3 的 AppId，NSIS 可选。候选 1.1.0 的三个包已生成并通过 SHA256 校验。参见 [构建及发行说明](building.md) 和 [打包脚本](../scripts/package-release.ps1)。
- [x] **验证便携升级和回退。** 在隔离目录用 1.1.0 覆盖 v1.0.3，验证设置、设备缓存、认证策略和测试日志的文件哈希。候选程序在本机完成设备枚举，读取旧版提前量和关闭动作，并从窗口退出。恢复 v1.0.3 程序与备份数据后核对版本及哈希。
- [ ] **完成安装与实机验收。** 在隔离 Windows 验证 Inno Setup 原位升级、卸载与回退，检查自启和数据路径。用真实接收端验证 Playback 与录音来源、托盘后台播放和多次启停，并检查键盘、窗口拖动及不同 DPI 下的布局。
- [ ] **结束回退期。** 保留 v1.0.3 发行包，实机验收通过后再发布 2.0。旧 Vue/Tauri 入口、1.1 GPUI 界面、浏览器对照测试和相关构建依赖已删除，可从 Git 历史查阅。
