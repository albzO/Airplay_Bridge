# 技术路线与技术选择

[English](tech-stack.md) · **简体中文**

本文记录桌面界面三次选型的原因、2.0.0 的技术组合、自行实现的部分与已知取舍。模块职责见[架构](architecture.md)，使用说明见[界面与设置](ui.md)。

## 路线

| 版本 | 界面技术 | 说明 |
|---|---|---|
| 1.0.x | Vue 3 + Vite，Tauri 2 WebView | `airplay-frontend/src/`、`src-tauri/`。界面经 IPC 调用 Rust 后台，依赖 WebView2。 |
| 1.1.0（发行候选，未发布） | GPUI `0.2.2` + gpui-component `0.5.1` | `airplay-frontend/src-gpui/`。去掉 WebView，界面与桌面服务同进程直接调用。功能完整，但视觉与交互未达到发布要求。 |
| 2.0.0（发行候选） | GPUI `gpui-pre 0.3.11` + Ely GPUI Components | `airplay-frontend/src-ely/`。在 1.1.0 的服务拆分之上重做界面。 |

三版共用同一套 Rust 桌面服务（`airplay-desktop`）、音频核心（`airplay-core`）和原生后端（`airplay-backend`）。界面层更换不改变音频流水线、协议、配置格式和数据目录。

## 为什么用 GPUI

- **不依赖 WebView2**：GPUI 用 Win32 窗口、DirectX 渲染、DirectWrite 文字，干净的 Windows 上不需要额外运行时。
- **同进程直接调用**：界面直接持有 `Engine`、在后台线程执行 `DesktopJob`，不经过 JSON IPC 和事件名约定，类型由编译器检查。
- **全 Rust**：构建工具链只剩 Rust MSVC 和原生后端的 MSYS2，不再需要 Node、pnpm 和前端构建。

## 为什么从 gpui-component 换到 Ely

1.1.0 用 gpui-component 实现了全部功能，但界面评审认为布局、层次和动效达不到发布要求。2.0.0 改用 [Ely GPUI Components](https://elygpui.com)：

- **设计基础更完整**：主题令牌（颜色、圆角、字号、阴影、密度）、浅色 / 深色、无障碍角色和“减少动画”开关都由组件库统一提供。
- **组件覆盖所需**：开关、滑块、分段控件、数字输入、密码框、标签页、折叠区、骨架屏、转圈、提示、横幅、窗口按钮和标题栏拖动区都有现成实现。
- **动效部件**：可折叠区域的高度动画、淡入 / 上浮过渡，时长统一受主题控制。
- **许可证**：Ely 为 MIT；随包字体为 SIL OFL 1.1，图标为 ISC，见 [THIRD_PARTY.md](../THIRD_PARTY.md)。

## 关键约束：为什么是独立 crate

Ely 依赖 `gpui-pre 0.3.11`（Zed 的 GPUI 预发布包），与 1.1.0 使用的 crates.io `gpui 0.2.2` 是两个不兼容的类型体系，组件不能混用。因此新界面是独立 crate `airplay-frontend/src-ely`，而不是在 `src-gpui` 里逐个替换控件。

不依赖 gpui 的模块两套界面共用同一份源码：`src-ely` 用 `#[path]` 直接引用 `src-gpui/src/` 下的 `session.rs`、`inbox.rs`、`diagnostics.rs`、`volume.rs`、`desktop_job.rs`。删除 `src-gpui` 前要先把这几个文件移入 `src-ely`。

可执行文件沿用 `airplay-bridge-gpui.exe` 的名字，桌面服务据此识别为预览版：开机自启项和单实例信号与已安装版本分开。

## 版本固定

| 依赖 | 固定方式 | 原因 |
|---|---|---|
| Ely GPUI Components | Git 依赖，固定提交 `69a62b1` | 上游标注为 early，尚无稳定 crate，API 仍在变化 |
| gpui-pre / gpui-pre-platform | `=0.3.11` | 与 Ely 固定提交所用版本一致 |
| Rust | 1.98 及以上 | Ely 工作区的最低版本；本机验证使用 1.99 |

只依赖 `ely-core`（主题、基础组件、表单、布局、动效、窗口外壳），不引入 Ely 的编辑器、终端、图表等行业组件，减小编译量和二进制体积。

## 自行实现的部分

Ely 不满足需求或无法定制的地方，在 `src-ely` 内自行实现：

| 模块 | 原因 |
|---|---|
| `panel.rs` 分组卡片 | Ely 的 Card 为 8 px 圆角、20 px 内边距，不满足 8 px 网格与“标题在卡片外、行内容上下居中”的版式 |
| `elastic.rs` 平滑与弹性滚动 | gpui 滚动每格滚轮直接跳到目标，Ely 和 gpui 都没有越界回弹；改为临界阻尼弹簧跟随目标，边界处带阻尼拉伸 |
| `source_picker.rs` 音频来源下拉 | Ely Select 的列表滚动在组件内部，无法换成平滑滚动；外观仍复用 Ely 的选择框、浮层和选项行 |
| `visibility.rs` 窗口可见性 | 托盘、最小化、显示器熄灭时暂停空闲采集；显示器状态来自 Windows 电源设置通知 |
| `lang.rs` 中英双语 | 文案在调用处成对书写（`t("中文", "English")`），不使用 Ely 的消息目录，改文案时不会漏掉一种语言；Ely 组件自带文字随同一语言切换 |
| 标题栏 | 隐藏系统标题栏，用 Ely 的拖动区和 Windows 窗口按钮画在窗口内 |

## 2.0.0 随界面带来的服务层调整

- **保持唤醒只在串流时生效**：以会话的“已建立”阶段为判据，停止、失败、退出或收尾超时都会释放，见[熄屏排查](display-off-capture.md)。
- **发现设备单独登记**：只与开始串流互斥，可与保存设置、切换采集同时进行，界面不再整体锁住。

这两项在 `airplay-desktop` 中实现，旧 GPUI 和 WebView 界面也随之生效。

## 已知取舍与待办

- 自写下拉没有键盘上下选择、回车确认和 Esc 关闭。
- 自管滚动后，Tab 移动焦点到屏幕外的控件时页面不会自动滚动。
- 动效在 debug 与 release 构建下帧间隔均约 6 ms，但只在本机一台显示器上验证。
- 打包脚本已改为构建 `src-ely`，`build-gpui.ps1 -Legacy` 仍可构建 `src-gpui`。Ely 的 MIT 许可证以及 Lucide、Inter、JetBrains Mono、IBM Plex Sans 的许可证随包放在 `licenses/ely/`。对外版本在补丁号为 0 时省略（2.0.0 显示为 2.0），Cargo 和 Windows 资源的数字版本保留完整各段。
- 真实接收端的串流、密码、音量和立体声交换由维护者实机验收。
