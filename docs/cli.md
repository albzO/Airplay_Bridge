# CLI 诊断

CLI 位于 `dist/tools/homepod-test.exe`。示例中的设备名称均为占位名称，请替换为 `discover` 发现的实际显示名称：

```powershell
# 扫描接收端及 Windows 音频端点
.\dist\tools\homepod-test.exe discover
.\dist\tools\homepod-test.exe audio-devices

# 验证认证与控制会话，不发送音频
.\dist\tools\homepod-test.exe test --device 'Living Room'

# 发送固定测试音
.\dist\tools\homepod-test.exe tone --device 'Living Room'
```

音频诊断命令为 `capture`、`replay`、`stream` 和 `stream-stereo`。默认使用 Windows 默认 Playback 端点；通过 `--endpoint` 显式选择 `audio-devices` 列出的任意受支持 Recording / Playback 端点 ID。

```powershell
.\dist\tools\homepod-test.exe capture --endpoint '<端点 ID>' --seconds 10 --convert
.\dist\tools\homepod-test.exe stream --device 'Receiver A' --endpoint '<端点 ID>'
.\dist\tools\homepod-test.exe stream-stereo --left 'Receiver A' --right 'Receiver B' --endpoint '<端点 ID>'
```

立体声必须显式指定两个设备，没有写死的个人设备名称。桌面端优先恢复上次的有效端点；找不到时使用 Windows 默认 Playback 端点。立体声卡片按照组信息和稳定标识排列，不按特定名称排列；组主标识不代表物理左声道。

错误定义与处理方式见 [错误代码](error-codes.md)。命令从仓库根目录执行。
