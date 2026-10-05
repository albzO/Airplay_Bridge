# HomePod 立体声实机验证

2026-10-04：用户确认黑球、黄球已在 Apple 家庭 App 配成一个立体声对；当前向黄球发送实时流时，仅黄球发声。

保存的设备清单中两台 tsid/gid 相同，黑球 igl=1，黄球 igl=0。这里将黑球作为优先验证的组主目标；这些字段不能单独证明组主会自动向成员转发本工具的 realtime 音频。

## 第一轮：向组主发送声道分离内容

现有采集、转换和 ALAC 路径已经传送双声道。需要确认的是两台物理 HomePod 的接收和声道分配行为。

已准备 `dist/audio/stereo-check.pcm` 及格式报告，使用现有 convert-pcm 从 48 kHz 双声道 float32 WAV 转换。14 秒内容为：1 秒静音、3 秒仅左声道 440 Hz、1 秒静音、3 秒仅右声道 880 Hz、1 秒静音、3 秒双声道、2 秒静音。峰值 -30 dBFS，首尾有渐变。后端还会添加起始静音、播放提前量和尾部静音，听到的开始时刻并非命令启动时刻。

先结束当前串流，再在项目目录运行，密码只在本机隐藏输入：

```powershell
.\dist\homepod-test.exe play-pcm --device 黑球 --input .\dist\audio\stereo-check.pcm
```

记录低音段、高音段各是哪台发声，最后双声道段是否两台同时发声。家庭 App 里的左右设置用于对照，不根据外壳颜色假定左右。

若只有黑球发声，再向黄球播放同一文件，确认两台分别接收双声道时的声道行为：

```powershell
.\dist\homepod-test.exe play-pcm --device 黄球 --input .\dist\audio\stereo-check.pcm
```

## 按实机结果决定实现

- 组主能驱动两台且声道正确：保留单会话，增加按立体声对发现/选择组主，再核对实时采集、两台音量操作与事件来源。
- 组主仍只驱动自己：需要实现两台的独立认证/音频会话，使用一个共享 PTP 时钟和相同播放起点；Rust 只采集和重采样一次，将同一帧序列分发给两台。完整双声道或单独声道的分配依据第一轮听到的行为决定。
- 不直接同时启动两个现有 stream-b3：目前每个后端都有独立的 PTP 引擎/起点，无法据此保证两台同步。

双会话时还需统一停止/失败处理，并允许已确认的组成员回传音量；现有 DACP 服务只接受当前目标 IP。立体声对不等于任意两台同播，多房间同步另行处理。

优先实机验证；本轮仅检查测试素材的帧数、格式和声道分离，没有重新跑完整协议模拟测试，也没有修改现有协议行为。

## 已确认结果与双会话实现

用户实测：黑球只在第一、第三段响，第二段安静；黄球只在第二、第三段响。确认黑球取左声道、黄球取右声道，并且分别连接任何一台都未自动带动另一台。因此采用双会话，向两台发送相同的完整双声道帧，让设备自己的左右配置取音。

已加入 `stream-stereo`，默认左黑球、右黄球，要求设备清单 tsid 相同且非空。B3 采集、rubato 和漂移控制各运行一次，只启动一个 C 后端进程。后端依次认证两台，共用同一进程内的 PTP 引擎及 groupUUID；两台准备好后才发 PCM_READY 启动采集。发送前冻结共同墙钟/RTP 锚点，每个 352 帧包依次发送给两台。每台独立维护 HAP、音频 key/nonce、反馈、事件和重传。两台统一 lead，窗口不兼容时退出。两台各有一个 DACP 公告和本机控制连接，保留自己的音量回传。任何一台失败时停止整组，清理已接受的会话；PTP 引擎最后清理。

先停止其他流，再运行：

```powershell
.\dist\homepod-test.exe stream-stereo --left 黑球 --right 黄球 --seconds 180 --latency-ms 300
```

两台使用同一家庭 AirPlay 密码，本机隐藏输入一次。`--left/--right` 指定设备，不修改家庭 App 的左右配置或重新混音。需要刷新地址或组身份时先 discover。

日志新增 GROUP_CONNECT、GROUP_READY、GROUP_ANCHOR、GROUP_TEARDOWN；包统计/事件/反馈标明 host。JSON 保存 peer_device、shared_ptp、member_packet_stats，两个音量回传日志分别为 volume.log、peer.volume.log。

已编译，只执行一项针对双接收端的检查：两台逐样本收到相同内容及 RTP 进度、一个 PTP 引擎、相同 groupUUID/clockID/冻结锚点、重传和两边 TEARDOWN 均通过。脚本 `probe/tests/check_stereo.py`，报告 `build/stereo-check.json`，未重跑完整模拟测试。真实 HomePod 双设备播放、同步听感和音量交互尚待本轮实机确认。
