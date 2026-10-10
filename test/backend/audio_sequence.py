"""Independent wire expectations: wrapping RTP fields and non-wrapping nonce."""
import struct


class AudioSequence:
    def __init__(self, packet=0, timestamp=441000, nonce=0):
        self.packet = packet
        self.timestamp_origin = timestamp
        self.nonce_origin = nonce
        self.index = 0

    def inspect(self, data):
        assert len(data) >= 36
        assert data[:2] == bytes((0x80, 0xe0 if self.index == 0 else 0x60))
        sequence, timestamp, ssrc = struct.unpack('!HII', data[2:12])
        assert sequence == (self.packet + self.index) & 0xffff
        assert timestamp == (self.timestamp_origin + self.index * 352) & 0xffffffff
        assert data[-8:] == (self.nonce_origin + self.index).to_bytes(8, 'little')
        self.index += 1
        return sequence, timestamp, ssrc
