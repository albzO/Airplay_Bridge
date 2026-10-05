# 贡献指南

本项目目前是 Windows x64 的开发版本。开始修改前阅读 [架构](docs/architecture.md) 和 [构建指南](docs/building.md)。

## 开发约定

- `airplay-frontend/` 管理界面和桌面生命周期；`airplay-core/` 包含共用音频库与诊断 CLI；`airplay-backend/` 管理原生协议适配。
- `upstream/` 是固定版本第三方子模块。修改适配优先在 `airplay-backend/` 完成，不直接编辑 `build/` 中的生成文件。更新依赖提交应单独提交，并更新第三方记录；不要把第三方源码作为普通文件提交到本仓库。
- 保留 Rust 和 pnpm 锁文件。依赖升级与功能修改尽量分开。
- 不提交设备清单、密码、录音、日志、构建产物或个人工具路径。
- 界面修改应检查浅色/深色、最小窗口尺寸、键盘交互和托盘状态。

## 验证

根据改动范围选择检查，命令均在仓库根目录执行：

```powershell
# Rust 音频库测试，不连接实际接收端
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib

# 前端类型检查和构建
pnpm --dir airplay-frontend build

# 桌面程序编译检查
cargo check --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 完整本机构建（已有全部依赖）
.\scripts\build-desktop.ps1 -Offline
```

协议相关的模拟检查位于 `airplay-backend/tests/`。这些检查和编译通过都不能替代实机播放验证。音频修改请说明设备、来源、驱动、格式、提前量、Buffer、测试时长及实际听感；不要为了普通布局调整重复长测。

## 提交问题或改动

报告应包含复现步骤、预期行为、实际行为和相关版本。日志只分享相关片段，并删除密码、访问凭据、个人路径和设备标识等敏感信息。

提交改动时说明解决的问题、最终行为、验证方式和未验证的范围。避免把大范围格式化与功能变更混在一起。

## 许可证状态

项目自有代码和文档采用 [Apache License 2.0](LICENSE)。除明确另行约定外，主动提交供本项目合并的贡献按 Apache-2.0 的贡献条款提供；贡献者保留其版权，不要求转让版权。提交时应确保有权提供该贡献，并明确标注第三方内容。归属见 [NOTICE](NOTICE)，授权范围与第三方待确认事项见 [许可证状态](docs/licensing.md)。

引用、复制或改编第三方代码时，应在 [THIRD_PARTY.md](THIRD_PARTY.md) 记录原仓库、固定版本、实际文件、许可证和修改范围，并保留原始版权声明。仅用于协议行为核对的参考资料与编译依赖分别记录，不以致谢代替授权依据。

更新第三方依赖时核对声明及版本适用范围，不根据同作者其他项目的许可证推定授权。不要在未完成相应核对时将整个组合程序统一标为 MIT、Apache-2.0 或 GPL。
