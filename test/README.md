# 测试 / Tests

项目测试源码统一放在根目录 `test/`，随 Git 提交。生产目录只保留测试模块挂接及诊断入口；已有测试的私有模块访问关系和忽略标记保持不变。

Test sources are committed under root `test/`. Production modules retain attachment/diagnostic entry points; existing private-module access and ignored hardware checks are preserved.

| 目录 | 内容 |
|---|---|
| `frontend/` | JSON 协议边界测试、模拟 UI、合成设备样例和桌面 Rust 测试 |
| `backend/` | 本机模拟接收端、认证、密码管道、立体声、延迟 SETUP 和原生 nonce 检查 |
| `core/unit/` | 采集健康状态、转换、漂移、来源生命周期、串流管线与脱敏单元测试 |
| `core/checks/` | 已采集 WAV 与转换 PCM 的独立校验脚本 |
| `.artifacts/` | 截图、模拟 PCM 和检查输出；不提交 Git |

## 运行 / Run

以下命令在项目根目录执行。先按 [构建指南](../docs/building.md) 安装生产依赖并构建后端；Python 模拟检查另需 `backend/requirements.txt` 中的依赖。

Run commands from the project root after installing/building production dependencies. Python receiver checks additionally need `backend/requirements.txt`.

```powershell
# Rust 单元测试 / Rust unit tests
cargo test --manifest-path airplay-core/Cargo.toml --locked --offline --lib
cargo test --manifest-path airplay-frontend/src-tauri/Cargo.toml --locked --offline

# 前端协议及模拟页面 / Frontend protocol and mocked UI
pnpm --dir airplay-frontend test:protocol
pnpm --dir airplay-frontend test:session
pnpm --dir airplay-frontend test:ui
pnpm --dir airplay-frontend test:ui:build

# 本机模拟接收端 / Local mock receivers
python -m pip install -r test/backend/requirements.txt
python test/backend/check_auth.py
python test/backend/check_gui_pipe.py
python test/backend/check_gui_pipe.py --direct
python test/backend/check_stereo.py
python test/backend/check_setup.py
python test/backend/mock_receiver.py

# 已有采集/转换文件的校验 / Validate existing capture/conversion artifacts
python test/core/checks/check_capture.py "path/to/capture.wav"
python test/core/checks/check_conversion.py "path/to/capture.pcm"
```

`test:session` 自动执行真实页面脚本，覆盖命令返回前的事件、旧会话事件、密码重试、停止、启动失败、事件积压和卸载；替换桌面 IPC 与浏览器接口，不覆盖 DOM 点击、布局或真实 WebView。

`test:session` executes the actual page script for early/stale events, password retries, stopping, startup failure, overflow and unmount. Desktop IPC/browser APIs are mocked; DOM clicks, layout and the real WebView are not covered.

后端默认读取 `dist/runtime/airplay-backend.exe`。设置 `AIRPLAY_TEST_BACKEND` 可检查另一个构建；`check_setup.py --baseline <旧后端路径>` 可重现旧的 2 秒 SETUP 超时。模拟检查只连接 localhost，不连接真实接收端。

Backend checks default to `dist/runtime/airplay-backend.exe`; `AIRPLAY_TEST_BACKEND` selects another build. `check_setup.py --baseline <old-backend-path>` reproduces the earlier 2-second timeout. Mock checks connect only to localhost.

需要真实声卡、局域网监听或接收端的 Rust 检查保留 `#[ignore]`，默认不会运行。明确选择检查名称后才使用 `--ignored`；这些检查可能采集或连接真实设备。测试和编译不能替代实机听感验证。

Rust checks needing real audio hardware, network listeners or receivers remain ignored by default. Select an individual check before using `--ignored`; these checks can capture or connect to real devices. Automated checks do not establish real-device playback quality.

设备样例只使用文档保留地址和合成标识。测试源码、模拟样例可以提交；真实配置、设备清单、录音、截图输出、日志和缓存仍忽略，不加入发行包。

Fixtures use documentation addresses and synthetic identities. Commit test sources and synthetic fixtures; exclude real configuration, discovery data, recordings, screenshot output, logs and caches from Git and release packages.
