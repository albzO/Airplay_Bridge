"""Check converter artifacts independently using Python's PCM WAV reader."""
import json
import pathlib
import sys
import wave

pcm = pathlib.Path(sys.argv[1])
report = json.loads(pcm.with_suffix('.pcm.json').read_text(encoding='utf-8'))
with wave.open(str(pcm.with_suffix('.pcm.wav')), 'rb') as wav:
    assert (wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) == (44100, 2, 2)
    frames = wav.getnframes()
    assert wav.readframes(frames) == pcm.read_bytes()
assert frames == report['stats']['output_frames']
assert frames == (report['stats']['input_frames'] * 44100 + report['input_rate'] // 2) // report['input_rate']
assert pcm.stat().st_size == frames * 4
print(f'PASS PCM/WAV: {frames} frames, stereo 44100 Hz int16; raw bytes match')
