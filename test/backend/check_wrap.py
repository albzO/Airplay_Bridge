"""Real paced localhost transport over 65536 packets; no audio hardware required."""
import mock_receiver as m

if __name__ == '__main__':
    # 526 秒合成 PCM 加 EOF 尾部静音，跨越 RTP 回绕并请求 65535/0 两包重传。
    # 526 seconds of synthetic PCM plus EOF silence crosses RTP wrap and requests packets 65535/0 again.
    m.test_live_case('live-wrap-ntp', 0, seconds=526)
    print('PASS full transport RTP wrap, independent nonce, decryption, PCM and cross-wrap retransmission')
