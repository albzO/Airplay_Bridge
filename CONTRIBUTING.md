# 贡献指南

本项目目前是 Windows x64 的开发版本。开始修改前阅读 [代码阅读指南](docs/code-reading-guide.md)、[架构](docs/architecture.md) 和 [构建指南](docs/building.md)。

## 开发约定

- `airplay-frontend/` 管理界面和桌面生命周期；`airplay-core/` 包含共用音频库与诊断 CLI；`airplay-backend/` 管理原生协议适配。
- 测试源码统一放在根目录 `test/`，按 `frontend/`、`backend/`、`core/` 分组并随 Git 提交。生产目录仅保留模块挂接及诊断入口，生成内容放在忽略的 `test/.artifacts/`。
- `upstream/` 是固定版本第三方子模块。修改适配优先在 `airplay-backend/` 完成，不直接编辑 `build/` 中的生成文件。更新依赖提交应单独提交，并更新第三方记录；不要把第三方源码作为普通文件提交到本仓库。
- 上游源码修改维护为 `airplay-backend/patches/` 中有上下文的补丁，Windows 实现维护在本地 C 兼容层；Python 只校验、应用补丁和选择编译内容，不添加源码字符串替换。变更须审查 `upstream-manifest.json` 的固定提交、输入/补丁哈希及应用顺序，审查生成差异后更新输出快照；运行 `python test/backend/test_upstream.py`。步骤见 [上游适配变更](docs/building.md#上游适配变更)，不要自动接受新清单。
- 保留 Rust 和 pnpm 锁文件。依赖升级与功能修改尽量分开。
- 不提交设备清单、密码、录音、日志、构建产物或个人工具路径。
- 沿用现有命名和排版，代码注释使用中英双语，中文在前、英文在后，在同一注释块内对应说明。关键入口说明输入来源、参数单位、状态条件、资源生命周期和设计原因；复杂流程先说明整体步骤，再补充局部原因，避免只写职责标签或逐行重复代码。
- 双语注释保留一致的字段名、协议术语、数值范围和单位；修改逻辑时同步更新两种语言。短注释相邻排列，长说明按语言分段，不逐词交错，也不翻译代码标识或第三方版权声明。
- 文件按外部导入、本地导入、常量与类型、状态、辅助方法、入口与生命周期排列；Rust 测试通过模块尾部的挂接引用 `test/`，依赖初始化顺序优先于机械排序。
- 前端使用 2 空格、单引号、分号和约 100 列换行；Rust 使用 rustfmt。不要将第三方或生成文件纳入统一格式化。
- `.editorconfig` 与 `.gitattributes` 统一文本编码、缩进和 LF 换行；图片保持二进制内容。
- 通用数据类型放在 `src/types.ts`，桌面数据格式校验放在 `src/protocol.ts`，命令错误转换放在 `src/commands.ts`；设置校验与持久化放在 `src-tauri/src/settings.rs`，窗口行为放在 `window.rs`。外部 JSON 接收为 `unknown`，验证后再使用；不要用 `any` 或直接断言掩盖字段不一致。
- 新日志路径应通过 `log_store.rs` 限制单文件并纳入保留命名；队列和页面历史也须有上限，规则见 [日志保留限制](docs/log-retention.md)。
- 日志在落盘和展示日志前脱敏；真实设备数据只用于发现、选择和控制。凭据不要加入日志、进程参数、配置文件或发布包。
- 界面修改应检查浅色/深色、最小窗口尺寸、键盘交互和托盘状态。

## 验证

根据改动范围选择检查，命令均在仓库根目录执行：

```powershell
# 当前源码及待新增文件的敏感数据规则检查；输出仅包含位置与规则名
python scripts/check-sensitive-data.py

# 排版检查；首次安装工具依赖时使用锁文件
pnpm --dir airplay-frontend format:check
cargo fmt --manifest-path airplay-core/Cargo.toml --check
cargo fmt --manifest-path airplay-frontend/src-tauri/Cargo.toml --check

# Rust 音频库测试，不连接实际接收端
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib
cargo test --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 前端类型检查和构建
pnpm --dir airplay-frontend test:protocol
pnpm --dir airplay-frontend test:session
pnpm --dir airplay-frontend test:diagnostics
pnpm --dir airplay-frontend test:dom
pnpm --dir airplay-frontend build

# 桌面程序编译检查
cargo check --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 完整本机构建（已有全部依赖）
.\scripts\build-desktop.ps1 -Offline
```

测试源码与合成素材现在统一在 `test/` 维护，恢复 Git 提交。前端样例只使用文档保留地址和合成标识；真实设备清单、配置、录音、截图输出和日志仍不提交。目录和完整运行说明见 [测试说明](test/README.md)。

后端协议模拟入口在 `test/backend/`，前端协议入口为 `pnpm --dir airplay-frontend test:protocol`，真实模板自动交互为 `test:dom`，人工模拟界面为 `test:ui`。DOM 测试在 Windows 默认使用已安装 Edge，无需编译后端；浏览器选择、产物及运行边界见 [测试说明](test/README.md)。这些检查和编译通过都不能替代实机播放验证。音频修改请说明设备、来源、驱动、格式、提前量、Buffer、测试时长及实际听感；不要为了普通布局调整重复长测。

队列、缓冲或转换性能修改先保留 Release 合成基线，再运行相同入口比较；方法见 [性能基线](docs/performance-baseline.md)。慢速回绕检查为 `python test/backend/check_wrap.py`，约 9 分钟，不需要声卡；PTP 脚本串行执行。CI 暂未纳入本轮修改。

模拟检查需要 `test/backend/requirements.txt` 中的 Python 依赖。如需检查新构建的后端而保留现有发行包，可设置 `$env:AIRPLAY_TEST_BACKEND` 为后端的绝对路径，再运行 `test/backend/check_auth.py` 和 `test/backend/check_gui_pipe.py`；检查完成后清除该变量。不设置时仍使用 `dist/runtime/airplay-backend.exe`。

敏感数据检查使用少量明确规则，不是完整凭据扫描器。`--history` 只读检查所有本地历史提交；封存快照或第三方历史示例命中规则时需人工判断，不因此改写历史。当前源码和历史结果应分别记录。

## 提交问题或改动

报告应包含复现步骤、预期行为、实际行为和相关版本。日志只分享相关片段，并删除密码、访问凭据、个人路径和设备标识等敏感信息。

提交改动时说明解决的问题、最终行为、验证方式和未验证的范围。避免把大范围格式化与功能变更混在一起。

## 许可证状态

项目自有代码和文档采用 [Apache License 2.0](LICENSE)。除明确另行约定外，主动提交供本项目合并的贡献按 Apache-2.0 的贡献条款提供；贡献者保留其版权，不要求转让版权。提交时应确保有权提供该贡献，并明确标注第三方内容。归属见 [NOTICE](NOTICE)，授权范围与第三方待确认事项见 [许可证状态](docs/licensing.md)。

引用、复制或改编第三方代码时，应在 [THIRD_PARTY.md](THIRD_PARTY.md) 记录原仓库、固定版本、实际文件、许可证和修改范围，并保留原始版权声明。仅用于协议行为核对的参考资料与编译依赖分别记录，不以致谢代替授权依据。

更新第三方依赖时核对声明及版本适用范围，不根据同作者其他项目的许可证推定授权。不要在未完成相应核对时将整个组合程序统一标为 MIT、Apache-2.0 或 GPL。
