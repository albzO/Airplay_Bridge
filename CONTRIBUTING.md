# 贡献指南

本项目目前是 Windows x64 的开发版本。开始修改前阅读 [架构](docs/architecture.md) 和 [构建指南](docs/building.md)。

## 开发约定

- `desktop/` 管理界面和桌面生命周期；`tester/` 包含共用音频库与诊断 CLI；`probe/` 管理原生协议适配。
- `upstream/` 是固定版本第三方子模块。修改适配优先在 `probe/` 完成，不直接编辑 `build/` 中的生成文件。更新依赖提交应单独提交，并更新第三方记录；不要把第三方源码作为普通文件提交到本仓库。
- 保留 Rust 和 pnpm 锁文件。依赖升级与功能修改尽量分开。
- 不提交设备清单、密码、录音、日志、构建产物或个人工具路径。
- 界面修改应检查浅色/深色、最小窗口尺寸、键盘交互和托盘状态。

## 验证

根据改动范围选择检查，命令均在仓库根目录执行：

```powershell
# Rust 音频库测试，不连接实际接收端
cargo test --manifest-path tester/Cargo.toml --locked --offline --lib

# 前端类型检查和构建
pnpm --dir desktop build

# 桌面程序编译检查
cargo check --manifest-path desktop/src-tauri/Cargo.toml --locked --offline

# 完整本机构建（已有全部依赖）
.\scripts\build-desktop.ps1 -Offline
```

协议相关的模拟检查位于 `probe/tests/`。这些检查和编译通过都不能替代实机播放验证。音频修改请说明设备、来源、驱动、格式、提前量、Buffer、测试时长及实际听感；不要为了普通布局调整重复长测。

## 提交问题或改动

报告应包含复现步骤、预期行为、实际行为和相关版本。日志只分享相关片段，并删除密码、访问凭据、个人路径和设备标识等敏感信息。

提交改动时说明解决的问题、最终行为、验证方式和未验证的范围。避免把大范围格式化与功能变更混在一起。

## 许可证状态

项目尚未选定自有代码的统一开源许可证，第三方声明保留在 [THIRD_PARTY.md](THIRD_PARTY.md) 及各来源目录。贡献指南不构成许可证授权，也不要求转让版权。
