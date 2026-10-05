# CLI 诊断

CLI 位于 `dist/homepod-test.exe`。示例中的设备名称均为占位名称，请替换为 `discover` 发现的实际显示名称：

```powershell
# 扫描接收端及 Windows 音频端点
.\dist\homepod-test.exe discover
.\dist\homepod-test.exe audio-devices

# 验证认证与控制会话，不发送音频
.\dist\homepod-test.exe test --device 'Living Room'

# 发送固定测试音
.\dist\homepod-test.exe tone --device 'Living Room'
```

CLI 中的 `capture-b3`、`replay-b3`、`stream-b3` 和 `stream-stereo` 属于开发阶段的 B3 采集诊断路径，需要匹配的录音端点。它们不是通用音频来源选择接口；其他采集设备请使用桌面界面。


命令从仓库根目录执行。
