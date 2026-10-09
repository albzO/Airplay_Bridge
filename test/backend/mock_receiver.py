"""Independent localhost receiver exercises SRP and encrypted SETUP.

Requires Python cryptography. This is a transport/protocol regression test,
not evidence of compatibility with a real HomePod firmware.
"""
import hashlib
import hmac
import json
import os
import pathlib
import plistlib
import re
import socket
import subprocess
import threading
import time
import math
import select
import struct
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

ROOT = pathlib.Path(__file__).resolve().parents[2]
ARTIFACTS = ROOT / 'test/.artifacts/backend'
ARTIFACTS.mkdir(parents=True, exist_ok=True)
# 可验证新编译的后端，避免覆盖已有发行包。
# Test a newly built backend without overwriting the existing distribution.
BACKEND = pathlib.Path(os.environ.get('AIRPLAY_TEST_BACKEND', ROOT / 'dist/runtime/airplay-backend.exe'))
source = (ROOT / 'upstream/airplay-cli/src/ap2_hap.c').read_text(encoding='utf-8')
group = re.search(r'static const char srp_n_hex_3072\[\] =\s*(.*?);', source, re.S).group(1)
N = int(''.join(re.findall(r'"([0-9A-F]+)"', group)), 16)
G = 5
SECRET = 'local-mock-password'
PCM_FIXTURE = [(i * 31 - 16000, 16000 - i * 29) for i in range(1001)]
LIVE_FIXTURE = [((i*31) % 32000 - 16000, 16000 - (i*29) % 32000) for i in range(44137)]

def integer(value, padded=False):
    return value.to_bytes(384 if padded else max(1, (value.bit_length() + 7) // 8), 'big')

def sha(*parts):
    return hashlib.sha512(b''.join(parts)).digest()

def tlv(items):
    result = bytearray()
    for tag, value in items:
        for pos in range(0, len(value), 255):
            chunk = value[pos:pos+255]
            result += bytes([tag, len(chunk)]) + chunk
    return bytes(result)

def parse_tlv(data):
    result = {}
    pos = 0
    while pos < len(data):
        tag, size = data[pos:pos+2]
        result[tag] = result.get(tag, b'') + data[pos+2:pos+2+size]
        pos += size + 2
    return result

def exact(conn, size):
    data = b''
    while len(data) < size:
        part = conn.recv(size - len(data))
        if not part: raise EOFError('peer closed')
        data += part
    return data

def plain_message(conn):
    data = bytearray()
    while not data.endswith(b'\r\n\r\n'):
        data += exact(conn, 1)
        if len(data) > 16384: raise ValueError('header too large')
    header = bytes(data)
    size = re.search(rb'Content-Length:\s*(\d+)', header, re.I)
    return header, exact(conn, int(size.group(1))) if size else b''

def response(request, status=200, body=b'', content_type='application/x-apple-binary-plist'):
    cseq = re.search(rb'CSeq:\s*(\d+)', request, re.I).group(1)
    extra = b'WWW-Authenticate: Digest realm="mock", nonce="test"\r\n' if status == 401 else b''
    return (b'RTSP/1.0 ' + str(status).encode() + b' Result\r\nCSeq: ' + cseq + b'\r\n' + extra +
            b'Content-Type: ' + content_type.encode() + b'\r\nContent-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body)

def fragmented(conn, data):
    for pos in range(0, len(data), 37):
        conn.sendall(data[pos:pos+37])
        time.sleep(0.001)

class Channel:
    def __init__(self, session, events=False):
        salt = b'Events-Salt' if events else b'Control-Salt'
        read_info = b'Events-Read-Encryption-Key' if events else b'Control-Write-Encryption-Key'
        write_info = b'Events-Write-Encryption-Key' if events else b'Control-Read-Encryption-Key'
        self.read = ChaCha20Poly1305(HKDF(algorithm=hashes.SHA512(), length=32, salt=salt,
                                       info=read_info).derive(session))
        self.write = ChaCha20Poly1305(HKDF(algorithm=hashes.SHA512(), length=32, salt=salt,
                                        info=write_info).derive(session))
        self.rx = self.tx = 0
        self.carry = b''

    def receive(self, conn):
        while True:
            end = self.carry.find(b'\r\n\r\n')
            if end >= 0:
                header = self.carry[:end+4]
                size = int(re.search(rb'Content-Length:\s*(\d+)', header, re.I).group(1))
                if len(self.carry) >= end+4+size:
                    body = self.carry[end+4:end+4+size]
                    self.carry = self.carry[end+4+size:]
                    return header, body
            aad = exact(conn, 2)
            size = int.from_bytes(aad, 'little')
            cipher = exact(conn, size+16)
            self.carry += self.read.decrypt(b'\0'*4 + self.rx.to_bytes(8, 'little'), cipher, aad)
            self.rx += 1

    def send(self, conn, data, corrupt=False):
        wire = b''
        for pos in range(0, len(data), 1024):
            block = data[pos:pos+1024]
            aad = len(block).to_bytes(2, 'little')
            wire += aad + self.write.encrypt(b'\0'*4 + self.tx.to_bytes(8, 'little'), block, aad)
            self.tx += 1
        if corrupt:
            wire = wire[:-1] + bytes([wire[-1] ^ 1])
        fragmented(conn, wire)

def decode_raw_stereo(frame):
    """Independent ALAC escape-frame reader (16-bit stereo only)."""
    bits = ''.join(f'{byte:08b}' for byte in frame)
    offset = 0
    def read(count):
        nonlocal offset
        value = int(bits[offset:offset+count], 2)
        offset += count
        return value
    assert read(3) == 1  # stereo channel element
    assert read(4) == 0 and read(12) == 0
    assert read(1) == 1 and read(2) == 0 and read(1) == 1
    assert read(32) == 352
    samples = []
    for _ in range(352):
        left, right = read(16), read(16)
        samples.append((left - 65536 if left >= 32768 else left,
                        right - 65536 if right >= 32768 else right))
    assert read(3) == 7  # end element
    assert set(bits[offset:]) <= {'0'}
    return samples

def receive_audio(data, control, key, stream, ptp, stop, errors, stats, pcm=False, live=False, allow_short=False):
    cipher = ChaCha20Poly1305(key[:32])
    wire_packets = {}
    next_seq = 0
    samples_seen = sync_seen = 0
    retransmitted = False
    try:
        while not stop.is_set():
            ready, _, _ = select.select([data, control], [], [], 0.1)
            for endpoint in ready:
                packet, _ = endpoint.recvfrom(4096)
                if endpoint is control:
                    if packet[1] == 0xd6:
                        assert packet[:4] == b'\x80\xd6\x00\x42'
                        assert packet[4:] == wire_packets[9]
                        retransmitted = True
                    else:
                        assert len(packet) == (28 if ptp else 20)
                        assert packet[1] == (0xd7 if ptp else 0xd4)
                        if ptp:
                            frame1, frame2 = struct.unpack('!I', packet[4:8])[0], struct.unpack('!I', packet[16:20])[0]
                            assert (frame2-frame1) & 0xffffffff == 77175
                            assert int.from_bytes(packet[20:28], 'big') != 0
                            if 'first_anchor' not in stats:
                                stats['first_anchor']={'wall_ns':int.from_bytes(packet[8:16],'big'),
                                    'play_pos':(frame1-11035)&0xffffffff,
                                    'clock_id':int.from_bytes(packet[20:28],'big')}
                        sync_seen += 1
                    continue
                assert packet[0] == 0x80 and packet[1] == (0xe0 if next_seq == 0 else 0x60)
                seq, timestamp, ssrc = struct.unpack('!HII', packet[2:12])
                assert seq == next_seq
                assert timestamp == 441000 + seq*352
                assert ssrc == (0 if ptp else stream['streamConnectionID'])
                assert packet[-8:] == seq.to_bytes(2, 'little') + b'\0'*6
                assert len(packet) <= 1472  # fits a standard Ethernet IPv4 MTU
                plain = cipher.decrypt(b'\0'*4 + packet[-8:], packet[12:-8], packet[4:12])
                samples = decode_raw_stereo(plain)
                for i, value in enumerate(samples):
                    if pcm or live:
                        fixture = LIVE_FIXTURE if live else PCM_FIXTURE
                        index = samples_seen + i - (0 if live else 44100)
                        expected_pair = fixture[index] if 0 <= index < len(fixture) else (0, 0)
                        assert value == expected_pair, (seq, i, value, expected_pair)
                        continue
                    t = (samples_seen+i)/44100
                    expected = 0
                    if 1 <= t < 5:
                        local = t-1
                        fade = min(1, local/.05, (4-local)/.05)
                        expected = round(1036*fade*math.sin(2*math.pi*440*local))
                    assert value[0] == value[1] and abs(value[0]-expected) <= 1, (seq, i, value, expected)
                wire_packets[seq] = packet
                if seq == 10:
                    control.sendto(b'\x80\xd5\x00\x42\x00\x09\x00\x01', ('127.0.0.1', stream['controlPort']))
                samples_seen += 352
                next_seq += 1
        if not allow_short:
            assert samples_seen >= (3*44100 + len(LIVE_FIXTURE) if live else 4*44100 + len(PCM_FIXTURE) if pcm else 8*44100), samples_seen
            assert sync_seen > 0 and retransmitted
        stats.update(packets=next_seq, frames=samples_seen, sync=sync_seen, retransmit_verified=True)
    except Exception as error:
        errors.append('audio: '+repr(error))

def send_events(conn, key, mode, errors, ready, stats):
    try:
        conn.settimeout(5)
        assert ready.wait(5), 'events never became ready'
        channel = Channel(key, events=True)
        if 'badtag' in mode:
            channel.send(conn,b'POST /command RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 0\r\n\r\n',corrupt=True)
        elif 'badframe' in mode:
            fragmented(conn,b'\x01\x04') # 1025-byte HAP frame is invalid.
        elif 'badlength' in mode:
            channel.send(conn,b'POST /command RTSP/1.0\r\nCSeq: 1\r\nContent-Length: -1\r\n\r\n')
        elif 'largeheader' in mode:
            channel.send(conn,b'POST /command RTSP/1.0\r\nX-Padding: '+b'x'*8200)
        else:
            bodies = [plistlib.dumps({'type':'updateInfo','value':{'name':'mock HomePod','padding':'x'*3200}},fmt=plistlib.FMT_BINARY),
                      plistlib.dumps({'type':'sendMediaRemoteCommand','value':'paus'},fmt=plistlib.FMT_BINARY),
                      plistlib.dumps({'type':'futureEvent','value':'unknown'},fmt=plistlib.FMT_BINARY),b'']
            # One fragmented encrypted stream contains large/multi-frame messages
            # and several requests within a frame; counters persist across rounds.
            for round_id in range(2):
                requests = b''
                for i, body in enumerate(bodies):
                    seq=round_id*4+i
                    requests += (f'POST /command RTSP/1.0\r\nCSeq: {seq}\r\nContent-Type: application/x-apple-binary-plist\r\nContent-Length: {len(body)}\r\n\r\n'.encode()+body)
                channel.send(conn,requests)
                for i in range(4):
                    header,body=channel.receive(conn)
                    assert header.startswith(b'RTSP/1.0 200 OK\r\n'),header
                    assert re.search(rb'CSeq:\s*(\d+)',header)[1]==str(round_id*4+i).encode(),header
                    assert b'Audio-Latency: 0\r\n' in header and not body
            # Optional CSeq, HTTP version and an empty heartbeat also work.
            channel.send(conn,b'POST /heartbeat HTTP/1.1\r\nContent-Length: 0\r\n\r\n')
            header,body=channel.receive(conn)
            assert header.startswith(b'HTTP/1.1 200 OK\r\n') and b'CSeq:' not in header and not body
            stats['replies']=9
            return
        conn.settimeout(0.3)
        try:
            byte=conn.recv(1)
            assert not byte, 'invalid event was acknowledged'
        except (ConnectionResetError,socket.timeout):
            pass # No acknowledgement may be sent for unauthenticated input.
        stats['rejected']=True
    except Exception as error:
        errors.append('events: '+repr(error))

def receiver(listener, mode, errors, summary=None):
    audio_stop = threading.Event()
    audio_worker = None
    audio_sockets = []
    audio_stats = {}
    volume_updates = []
    event_listener = event_connection = event_worker = None
    event_ready = threading.Event()
    event_stats = {}
    rejected_event = socket.socket()
    rejected_event.bind(('127.0.0.1', 0))
    try:
        for attempt in range(2):
            conn, _ = listener.accept()
            with conn:
                conn.settimeout(15)
                header, _ = plain_message(conn)
                assert header.startswith(b'GET /info ')
                fragmented(conn, response(header, body=plistlib.dumps({'name': 'mock HomePod'}, fmt=plistlib.FMT_BINARY)))
                header, body = plain_message(conn)
                assert header.startswith(b'POST /pair-setup ')
                assert b'X-Apple-HKP: 4' in header
                if mode in ('pair403','pair470'):
                    fragmented(conn, response(header, 403 if mode=='pair403' else 470))
                    return
                if mode=='pair-policy':
                    fragmented(conn,response(header,body=tlv([(6,b'\x02'),(7,b'\x02')]),content_type='application/octet-stream'))
                    return
                if mode in ('pair-backoff','pair-max-tries','pair-max-peers','pair-unavailable','pair-busy'):
                    error={'pair-backoff':3,'pair-max-tries':5,'pair-max-peers':4,'pair-unavailable':6,'pair-busy':7}[mode]
                    fragmented(conn,response(header,body=tlv([(6,b'\x02'),(7,bytes([error]))]),content_type='application/octet-stream'))
                    return
                salt = os.urandom(16)
                b = int.from_bytes(os.urandom(32), 'big')
                secret='3939' if mode=='auto-open' else SECRET
                x = int.from_bytes(sha(salt, sha(b'Pair-Setup:' + secret.encode())), 'big')
                v = pow(G, x, N)
                k = int.from_bytes(sha(integer(N, True), integer(G, True)), 'big')
                B = (k*v + pow(G, b, N)) % N
                challenge = tlv([(6, b'\x02'), (2, salt), (3, integer(B))])
                fragmented(conn, response(header, body=challenge, content_type='application/octet-stream'))
                header, body = plain_message(conn)
                proof = parse_tlv(body)
                Araw = proof[3]
                A = int.from_bytes(Araw, 'big')
                assert A % N != 0
                u = int.from_bytes(sha(integer(A, True), integer(B, True)), 'big')
                shared = pow((A * pow(v, u, N)) % N, b, N)
                key = sha(integer(shared))
                mixed = bytes(a ^ b for a, b in zip(sha(integer(N)), sha(integer(G))))
                expected = sha(mixed, sha(b'Pair-Setup'), salt, Araw, integer(B), key)
                if not hmac.compare_digest(expected, proof[4]):
                    fragmented(conn, response(header, body=tlv([(6, b'\x04'), (7, b'\x02')]), content_type='application/octet-stream'))
                    if mode=='wrong-password' or mode=='auto-wrong-password' and attempt==1: return
                    continue
                server_proof = sha(Araw, expected, key)
                fragmented(conn, response(header, body=tlv([(6, b'\x04'), (4, server_proof)]), content_type='application/octet-stream'))
                channel = Channel(key)
                header, body = channel.receive(conn)
                assert header.startswith(b'SETUP ')
                setup = plistlib.loads(body)
                if summary is not None: summary['group_uuid']=setup.get('groupUUID')
                ptp = mode.endswith('-ptp')
                assert setup['timingProtocol'] == ('PTP' if ptp else 'NTP')
                if ptp:
                    assert setup['timingPeerList'] and setup['timingPeerInfo']
                else:
                    assert setup['timingPort'] > 0
                assert 'streams' not in setup
                if mode == 'setup401':
                    channel.send(conn, response(header, 401))
                    return
                reply = {'eventPort': 0, 'diagnosticPadding': '测'*1200}
                if mode.startswith('events-') and mode!='events-fail' or mode=='tone-events-ptp':
                    event_listener=socket.socket();event_listener.bind(('127.0.0.1',0));event_listener.listen(1);event_listener.settimeout(5)
                    reply['eventPort']=event_listener.getsockname()[1]
                if mode == 'events-fail': reply['eventPort'] = rejected_event.getsockname()[1]
                channel.send(conn, response(header, body=plistlib.dumps(reply, fmt=plistlib.FMT_BINARY)), corrupt=mode == 'bad-tag')
                if mode == 'bad-tag': return
                if event_listener:
                    event_connection,_=event_listener.accept()
                    if not mode.startswith('tone'): event_ready.set()
                    event_worker=threading.Thread(target=send_events,args=(event_connection,key,mode,errors,event_ready,event_stats),daemon=True)
                    event_worker.start()
                while True:
                    header, body = channel.receive(conn)
                    if header.startswith(b'TEARDOWN '):
                        if event_worker:
                            event_worker.join(6)
                            assert not event_worker.is_alive() and event_stats,event_stats
                        if mode.startswith('volume'):
                            assert volume_updates == [-27.0, -144.0, -4.5], volume_updates
                        channel.send(conn, response(header))
                        audio_stop.set()
                        if audio_worker:
                            audio_worker.join(2)
                            assert not audio_worker.is_alive()
                            assert audio_stats, errors
                        return
                    if mode.startswith(('tone', 'pcm', 'live', 'volume')):
                        if mode.startswith('volume') and header.startswith(b'GET_PARAMETER '):
                            assert body == b'volume\r\n' and b'text/parameters' in header
                            channel.send(conn,response(header,501 if mode=='volume-queryunsupported' else 200,
                                b'' if mode=='volume-queryunsupported' else b'volume: -12.000000\r\n', 'text/parameters'))
                            continue
                        if mode.startswith('volume') and header.startswith(b'SET_PARAMETER '):
                            assert b'text/parameters' in header
                            volume_updates.append(float(body.decode().split(':')[1]))
                            assert len(volume_updates)<=3, 'volume report echo loop'
                            channel.send(conn,response(header));continue
                        if header.startswith(b'RECORD '):
                            assert not audio_sockets
                            channel.send(conn, response(header, 403 if mode == 'tone-record403' else 200))
                            event_ready.set()
                            continue
                        if header.startswith(b'SETUP '):
                            stream = plistlib.loads(body)['streams'][0]
                            assert stream['type'] == 96 and stream['ct'] == 2
                            assert stream['sr'] == 44100 and stream['spf'] == 352
                            assert stream['audioFormat'] == 1 << 18
                            assert stream['shk'] == key[:32]
                            assert stream['dataPort'] > 0 and stream['controlPort'] > 0
                            if mode == 'tone-stream403':
                                channel.send(conn, response(header, 403))
                                continue
                            if mode == 'tone-invalid-ports':
                                channel.send(conn, response(header, body=plistlib.dumps({'streams': [{'dataPort': 0, 'controlPort': 0}]}, fmt=plistlib.FMT_BINARY)))
                                continue
                            for _ in range(2):
                                udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                                udp.bind(('127.0.0.1', 0)); audio_sockets.append(udp)
                            reply = {'streams': [{'dataPort': audio_sockets[0].getsockname()[1], 'controlPort': audio_sockets[1].getsockname()[1]}]}
                            if mode == 'live-clamp-ptp':
                                reply['streams'][0].update(latencyMin=44100, latencyMax=88200)
                            audio_worker = threading.Thread(target=receive_audio, args=(*audio_sockets, key, stream, ptp, audio_stop, errors, audio_stats, mode.startswith('pcm'), mode.startswith('live'), mode in ('live-truncated', 'live-stall')), daemon=True)
                            audio_worker.start()
                            channel.send(conn, response(header, body=plistlib.dumps(reply, fmt=plistlib.FMT_BINARY)))
                            continue
                        if header.startswith(b'SETPEERS '):
                            peers=plistlib.loads(body)
                            assert ptp and peers in (["127.0.0.1"]*2,["127.0.0.1"]*3)
                            channel.send(conn, response(header)); continue
                        if header.startswith(b'POST /feedback '):
                            channel.send(conn, response(header)); continue
                    assert header.startswith(b'GET /info '), header
                    channel.send(conn, response(header, 403 if mode == 'hold403' else 200, plistlib.dumps({'name': 'mock'}, fmt=plistlib.FMT_BINARY)))
                    if mode == 'hold403':
                        # Probe still tears down the accepted session.
                        continue
    except Exception as error:
        errors.append(repr(error))
    finally:
        audio_stop.set()
        if audio_worker: audio_worker.join(2)
        if summary is not None: summary['audio']=audio_stats
        for endpoint in audio_sockets: endpoint.close()
        rejected_event.close()
        if event_connection: event_connection.close()
        if event_listener: event_listener.close()
        if event_worker: event_worker.join(2)
        listener.close()

def test_case(mode, password, expected_code, required, forbidden=(), automatic=False):
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0)); listener.listen(2); listener.settimeout(15)
    port = listener.getsockname()[1]
    errors = []
    worker = threading.Thread(target=receiver, args=(listener, mode, errors), daemon=True)
    worker.start()
    command = [str(BACKEND), '--host', '127.0.0.1', '--port', str(port),
               '--password-stdin', '--bind-ip', '127.0.0.1', '--timing', 'ptp' if mode.endswith('-ptp') else 'ntp',
               '--hold-seconds', '2' if mode in ('success', 'success-ptp', 'hold403') or mode.startswith('events-') and mode!='events-fail' else '0']
    if automatic: command.append('--password-auto')
    if mode.startswith(('tone', 'volume')): command.append('--tone')
    controls = None
    controls_worker = None
    if mode.startswith('volume'):
        controls=socket.socket(); controls.bind(('127.0.0.1',0));controls.listen(1);controls.settimeout(15)
        command.extend(['--volume-control-port',str(controls.getsockname()[1])])
        def forward():
            conn,_=controls.accept()
            with conn:
                payload=b'SET 10\nSET 0\nREPORT -6\nSTEP 5\nSET NaN\n'
                for index in range(0,len(payload),3): conn.sendall(payload[index:index+3]);time.sleep(.002)
                time.sleep(9)
            controls.close()
        controls_worker=threading.Thread(target=forward,daemon=True);controls_worker.start()
    if mode.startswith('pcm'):
        fixture = ARTIFACTS / 'mock-stereo.pcm'
        fixture.write_bytes(b''.join(struct.pack('<hh', *frame) for frame in PCM_FIXTURE))
        command.extend(['--pcm-file', str(fixture)])
    result = subprocess.run(command, input=password+'\n', text=True, encoding='utf-8', capture_output=True, timeout=35)
    worker.join(3)
    log = result.stdout + result.stderr
    assert result.returncode == expected_code, (mode, result.returncode, log)
    for marker in required: assert marker in log, (mode, marker, log)
    for marker in forbidden: assert marker not in log, (mode, marker, log)
    assert password not in log, 'password leaked into log'
    assert not errors, errors
    assert not worker.is_alive(), 'receiver did not finish'
    if controls_worker: controls_worker.join(3);assert not controls_worker.is_alive()
    print(f'PASS {mode}: exit={result.returncode}, required markers verified')
    return {'case': mode, 'exit': result.returncode, 'passed': True}

def test_live_case(mode, expected_code):
    listener = socket.socket()
    listener.bind(('127.0.0.1', 0)); listener.listen(2); listener.settimeout(15)
    errors = []
    worker = threading.Thread(target=receiver, args=(listener, mode, errors), daemon=True)
    worker.start()
    command = [str(BACKEND), '--host', '127.0.0.1',
               '--port', str(listener.getsockname()[1]), '--password', SECRET,
               '--bind-ip', '127.0.0.1', '--timing', 'ptp' if mode.endswith('-ptp') else 'ntp',
               '--hold-seconds', '0', '--pcm-stdin']
    if mode in ('live-lowlatency-ptp', 'live-clamp-ptp'):
        command.extend(['--latency-ms', '500', '--buffer-ms', '64'])
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    ready = threading.Event()
    logs = []
    def read_log():
        for line in child.stderr:
            text = line.decode('utf-8', errors='replace')
            logs.append(text)
            if 'PCM_READY' in text: ready.set()
    reader = threading.Thread(target=read_log, daemon=True); reader.start()
    assert ready.wait(10), logs
    if mode == 'live-truncated':
        child.stdin.write(b'abc'); child.stdin.close()
    elif mode == 'live-stall':
        # Hold the pipe open without producing audio: bounded startup timeout.
        child.wait(timeout=12); child.stdin.close()
    else:
        payload = b''.join(struct.pack('<hh', *frame) for frame in LIVE_FIXTURE)
        began = time.monotonic()
        for pos in range(0, len(payload), 1764):
            block = payload[pos:pos+1764]
            # Partial writes split even stereo sample boundaries.
            for index in range(0, len(block), 37):
                child.stdin.write(block[index:index+37]); child.stdin.flush()
            delay = began + (pos+len(block))/176400 - time.monotonic()
            if delay > 0: time.sleep(delay)
        child.stdin.close()
    child.wait(timeout=15); reader.join(2); worker.join(3)
    log = ''.join(logs)
    assert child.returncode == expected_code, (mode, child.returncode, log)
    assert 'TEARDOWN http=200' in log and SECRET not in log
    clock = re.search(r'PCM_CLOCK start_qpc=(\d+) frequency=(\d+) prebuffer_frames=(\d+) lead_ms=(\d+)', log)
    assert clock and int(clock[1]) > 0 and int(clock[2]) > 0, log
    if mode == 'live-lowlatency-ptp': assert clock[4] == '500', log
    if mode == 'live-clamp-ptp': assert clock[4] == '1000', log
    assert ('AUDIO_TRANSPORT_OK' in log) == (expected_code == 0), log
    if mode == 'live-stall': assert 'PCM_STALL' in log
    assert not errors and not worker.is_alive(), errors
    print(f'PASS {mode}: streaming pipe, partial writes/EOF or failure cleanup verified')
    return {'case': mode, 'exit': child.returncode, 'passed': True}

if __name__ == '__main__':
    reports = [
        test_case('success', SECRET, 0, ['PASSWORD_ACCEPTED', 'SRP_VERIFIED', 'ENCRYPTED_RESPONSE_OK', 'SESSION_ACCEPTED', 'TEARDOWN http=200', 'RESULT value=session-ok']),
        test_case('wrong-password', 'wrong-local-password', 10, ['FAILED phase=pairing', 'ERROR code=PASSWORD_REJECTED exit=10', 'RESULT value=failed'], ['PASSWORD_ACCEPTED', 'SESSION_ACCEPTED','Pairing leg 2/2']),
        test_case('setup401', SECRET, 15, ['PASSWORD_ACCEPTED', 'FAILED phase=session-setup http=401', 'WWW-Authenticate'], ['SESSION_ACCEPTED']),
        test_case('pair403', SECRET, 15, ['FAILED phase=pairing http=403'], ['SESSION_ACCEPTED']),
        test_case('hold403', SECRET, 15, ['SESSION_ACCEPTED', 'HOLD_FAILED http=403'], ['CONTROL_HOLD_OK', 'RESULT value=session-ok']),
        test_case('events-fail', SECRET, 1, ['EVENTS_FAILED', 'TEARDOWN http=200'], ['RESULT value=session-ok']),
        test_case('bad-tag', SECRET, 1, ['FAILED phase=session-setup'], ['ENCRYPTED_RESPONSE_OK', 'SESSION_ACCEPTED']),
        test_case('success-ptp', SECRET, 0, ['TIMING value=ptp', 'ENCRYPTED_RESPONSE_OK', 'CONTROL_HOLD_OK', 'RESULT value=session-ok']),
        test_case('tone-ntp', SECRET, 0, ['RECORD_OK', 'STREAM_SETUP_OK', 'AUDIO_TRANSPORT_OK', 'retransmitted=1', 'RESULT value=audio-transport-ok']),
        test_case('tone-ptp', SECRET, 0, ['SETPEERS_OK', 'AUDIO_TRANSPORT_OK', 'retransmitted=1', 'RESULT value=audio-transport-ok']),
        test_case('pcm-ntp', SECRET, 0, ['AUDIO_TRANSPORT_OK', 'retransmitted=1', 'RESULT value=audio-transport-ok']),
        test_case('pcm-ptp', SECRET, 0, ['SETPEERS_OK', 'AUDIO_TRANSPORT_OK', 'retransmitted=1', 'RESULT value=audio-transport-ok']),
        test_case('tone-record403', SECRET, 15, ['FAILED phase=record http=403', 'TEARDOWN http=200'], ['STREAM_SETUP_OK', 'AUDIO_BEGIN']),
        test_case('tone-stream403', SECRET, 15, ['FAILED phase=stream-setup http=403', 'TEARDOWN http=200'], ['AUDIO_BEGIN']),
        test_case('tone-invalid-ports', SECRET, 1, ['missing/invalid remote ports', 'TEARDOWN http=200'], ['AUDIO_BEGIN']),
        test_live_case('live-ntp', 0),
        test_live_case('live-ptp', 0),
        test_live_case('live-truncated', 1),
        test_live_case('live-stall', 1),
        test_live_case('live-lowlatency-ptp', 0),
        test_live_case('live-clamp-ptp', 0),
        test_case('volume-ptp', SECRET, 0, ['VOLUME_CURRENT source=query db=-12.000','VOLUME_CURRENT source=device db=-6.000',
            'percent=10.00 db=-27.000 http=200','percent=0.00 db=-144.000 http=200','percent=85.00 db=-4.500 http=200',
            'VOLUME_REJECT reason=invalid-number','PACKET_STATS','rtx_requested=1 rtx_resent=1 rtx_expired=0']),
        test_case('volume-queryunsupported', SECRET, 0, ['VOLUME_QUERY_UNAVAILABLE http=501','VOLUME_CURRENT source=device db=-6.000',
            'percent=85.00 db=-4.500 http=200','AUDIO_TRANSPORT_OK']),
        test_case('events-fragmented', SECRET, 0, ['EVENTS_READY','type=updateInfo','type=sendMediaRemoteCommand value=paus',
            'type=futureEvent','EVENTS_STATS requests=9 replies=9 failed=false','TEARDOWN http=200']),
        test_case('tone-events-ptp', SECRET, 0, ['EVENTS_STATS requests=9 replies=9 failed=false','AUDIO_TRANSPORT_OK','TEARDOWN http=200']),
        test_case('events-badtag', SECRET, 1, ['EVENTS_FAILED reason=authentication','EVENTS_STATS requests=0 replies=0 failed=true','TEARDOWN http=200'], ['[PROBE] EVENT method=']),
        test_case('events-badframe', SECRET, 1, ['EVENTS_FAILED reason=invalid-frame-length','TEARDOWN http=200'], ['[PROBE] EVENT method=']),
        test_case('events-badlength', SECRET, 1, ['EVENTS_FAILED reason=invalid-content-length','TEARDOWN http=200'], ['[PROBE] EVENT method=']),
        test_case('events-largeheader', SECRET, 1, ['EVENTS_FAILED reason=header-too-large','TEARDOWN http=200'], ['[PROBE] EVENT method=']),
    ]
    (ARTIFACTS / 'mock-test-results.json').write_text(json.dumps(reports, indent=2), encoding='utf-8')
