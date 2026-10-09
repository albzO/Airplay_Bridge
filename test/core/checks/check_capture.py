"""Independently check the real float32 capture WAV against its JSON report."""
import json
import math
import pathlib
import struct
import sys

path = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else max(
    (item for item in (pathlib.Path(__file__).resolve().parents[3] / 'dist/captures').glob('capture-*.wav')
     if not item.stem.endswith('.pcm')),
    key=lambda item: item.stat().st_mtime_ns)
wave = path.read_bytes()
report = json.loads(path.with_suffix('.json').read_text(encoding='utf-8'))
assert wave[:4] == b'RIFF' and wave[8:12] == b'WAVE'
assert struct.unpack_from('<I', wave, 4)[0] == len(wave)-8
chunks = {}
offset = 12
while offset < len(wave):
    tag, size = struct.unpack_from('<4sI', wave, offset)
    chunks[tag] = wave[offset+8:offset+8+size]
    assert len(chunks[tag]) == size
    offset += 8+size+(size % 2)
assert offset == len(wave)
fmt = chunks[b'fmt ']
tag, channels, rate, byte_rate, block, bits = struct.unpack_from('<HHIIHH', fmt)
assert tag == 0xfffe and bits == 32
assert fmt[24:40] == bytes.fromhex('0300000000001000800000aa00389b71')
assert channels == report['format']['channels'] and rate == report['format']['rate']
assert byte_rate == rate*block and block == channels*4
assert struct.unpack('<I', chunks[b'fact'])[0] == report['frames']
assert len(chunks[b'data']) == report['frames']*block
peaks = [0.0]*channels
signal = 0
for frame in struct.iter_unpack('<'+'f'*channels, chunks[b'data']):
    assert all(math.isfinite(value) for value in frame)
    signal += any(abs(value) > 1e-6 for value in frame)
    for index, value in enumerate(frame):
        peaks[index] = max(peaks[index], abs(value))
assert signal == report['signal_frames']
assert peaks == report['channel_peaks']
assert report['mode'] == 'shared' and report['stream_complete']
print(f'PASS real capture WAV: {rate} Hz, {channels} channels, {report["frames"]} frames, signal={signal}')
print(path)
