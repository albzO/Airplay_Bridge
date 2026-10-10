"""One focused dual-receiver check; real HomePod stereo remains a listening test."""
import json
import re
import socket
import struct
import subprocess
from harness import managed_case
import threading
import time
import mock_receiver as m


@managed_case
def check_stereo(*, resources):
    listeners=[]; workers=[]; summaries=[{},{}]; errors=[]
    for summary in summaries:
        listener=resources.socket();listener.bind(('127.0.0.1',0));listener.listen(2);listener.settimeout(15)
        listeners.append(listener)
        worker=resources.thread(m.receiver,(listener,'live-group-ptp',errors,summary,resources))
        workers.append(worker)
    command=[str(m.BACKEND),'--host','127.0.0.1','--port',str(listeners[0].getsockname()[1]),
        '--peer-host','127.0.0.1','--peer-port',str(listeners[1].getsockname()[1]),
        '--peer-identity','A1B2C3D4E5F60719','--peer-active-remote','2',
        '--password',m.SECRET,'--bind-ip','127.0.0.1','--timing','ptp','--hold-seconds','0',
        '--pcm-stdin','--latency-ms','300']
    child=resources.popen(command,stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    ready=threading.Event();lines=[]
    def read_log():
        for line in child.stderr:
            text=line.decode('utf-8',errors='replace');lines.append(text)
            if '[PROBE] PCM_READY' in text: ready.set()
    reader=resources.thread(read_log)
    assert ready.wait(12),''.join(lines)
    raw=b''.join(struct.pack('<hh',*frame) for frame in m.LIVE_FIXTURE)
    began=time.monotonic()
    for pos in range(0,len(raw),1764):
        block=raw[pos:pos+1764];child.stdin.write(block);child.stdin.flush()
        delay=began+(pos+len(block))/176400-time.monotonic()
        if delay>0:time.sleep(delay)
    child.stdin.close();child.wait(timeout=15);reader.join(2)
    for worker in workers:worker.join(3)
    log=''.join(lines)
    assert child.returncode==0,log
    assert not errors and all(not w.is_alive() for w in workers),errors
    assert log.count('Engine started on UDP 319/320')==1,log
    assert 'GROUP_READY members=2 shared_ptp=true lead_ms=300' in log,log
    assert 'PASSWORD_REUSED host=127.0.0.1 source=authenticated-peer' in log,log
    peer_log=log.split('[PROBE] GROUP_CONNECT member=1',1)[1]
    assert 'AUTH_ATTEMPT host=127.0.0.1 mode=password' in peer_log,peer_log
    assert 'mode=automatic' not in peer_log and 'fixed PIN' not in peer_log,peer_log
    assert log.count('GROUP_TEARDOWN')==2 and 'http=0' not in log,log
    assert summaries[0]['group_uuid']==summaries[1]['group_uuid']
    assert summaries[0]['audio']['frames']==summaries[1]['audio']['frames']
    frozen=int(re.search(r'GROUP_ANCHOR wall_ns=(\d+)',log)[1])
    for summary in summaries:
        anchor=summary['audio']['first_anchor']
        delta=(anchor['play_pos']-441000+2**31)%2**32-2**31
        reconstructed=anchor['wall_ns']-delta*1000000000//44100-300000000
        assert abs(reconstructed-frozen)<=23000,(reconstructed,frozen)
    assert summaries[0]['audio']['first_anchor']['clock_id']==summaries[1]['audio']['first_anchor']['clock_id']
    (m.ARTIFACTS/'stereo-check.json').write_text(json.dumps(summaries,indent=2))
    print('PASS two receivers: same stereo samples/RTP progression, one PTP engine, same clock/anchor/group UUID, retransmission and both TEARDOWNs')


if __name__ == '__main__':
    check_stereo()
