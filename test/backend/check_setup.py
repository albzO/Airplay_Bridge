"""Local regression evidence; never contacts a real receiver."""
import json
import pathlib
import plistlib
import runpy
import socket
import subprocess
import sys
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
import mock_receiver as m

original_receive = m.Channel.receive
delay = 3.0
setups = 0

def delayed_receive(self, conn):
    global setups
    header, body = original_receive(self, conn)
    if header.startswith(b'SETUP ') and 'streams' in plistlib.loads(body):
        setups += 1
        time.sleep(delay)
    return header, body

m.Channel.receive = delayed_receive

def failure_case(binary, budget):
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0))
    listener.listen(2)
    listener.settimeout(15)
    errors = []
    worker = threading.Thread(target=m.receiver, args=(listener, 'live-ptp', errors), daemon=True)
    worker.start()
    began = time.monotonic()
    result = subprocess.run([str(binary), '--host', '127.0.0.1', '--port',
        str(listener.getsockname()[1]), '--password', m.SECRET, '--bind-ip', '127.0.0.1',
        '--timing', 'ptp', '--pcm-stdin', '--hold-seconds', '0'], input=b'',
        capture_output=True, timeout=16)
    elapsed = time.monotonic() - began
    worker.join(4)
    log = (result.stdout + result.stderr).decode('utf-8', errors='replace')
    assert result.returncode == 1 and 'PCM_READY' not in log, log
    assert 'RTSP channel failed during SETUP' in log and 'timed out' in log, log
    assert budget <= elapsed < budget + 3, elapsed
    assert not worker.is_alive()
    assert m.SECRET not in log
    print(f'PASS bounded SETUP failure: budget={budget}s elapsed={elapsed:.2f}s; no PCM_READY')
    return {'budget_seconds': budget, 'elapsed_seconds': round(elapsed, 2), 'passed': True}

results = []
if len(sys.argv) == 3 and sys.argv[1] == '--baseline':
    results.append(failure_case(pathlib.Path(sys.argv[2]).resolve(), 2))
else:
    for mode in ('live-ntp', 'live-ptp'):
        setups = 0
        results.append(m.test_live_case(mode, 0))
        assert setups == 1, setups
    setups = 0
    runpy.run_path(str(ROOT / 'test/backend/check_stereo.py'), run_name='__main__')
    assert setups == 2, setups
    results.append({'case': 'delayed-stereo-setup', 'passed': True, 'setups': setups})
    delay = 9
    results.append(failure_case(m.BACKEND, 8))
    delay = 0
    results.append(m.test_case('pair403', m.SECRET, 15, ['FAILED phase=pairing http=403'], ['SESSION_ACCEPTED']))
    results.append(m.test_case('wrong-password', 'wrong-local-password', 10,
        ['ERROR code=PASSWORD_REJECTED exit=10'], ['SESSION_ACCEPTED']))
    results.append(m.test_case('tone-stream403', m.SECRET, 15,
        ['FAILED phase=stream-setup http=403', 'TEARDOWN http=200'], ['AUDIO_BEGIN']))
    setups = 0
    results.append(m.test_live_case('live-ptp', 0))
    assert setups == 1, setups

(m.ARTIFACTS / 'setup-check.json').write_text(
    json.dumps(results, indent=2), encoding='utf-8')
