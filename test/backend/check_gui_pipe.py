"""One native GUI password-pipe check with the authenticated mock receiver."""
import ctypes as c
from ctypes import wintypes as w
import socket, threading, subprocess, struct, sys
import mock_receiver as m
k=c.WinDLL('kernel32',use_last_error=True)
k.CreateNamedPipeW.argtypes=[w.LPCWSTR,w.DWORD,w.DWORD,w.DWORD,w.DWORD,w.DWORD,w.DWORD,c.c_void_p];k.CreateNamedPipeW.restype=w.HANDLE
k.ConnectNamedPipe.argtypes=[w.HANDLE,c.c_void_p];k.ReadFile.argtypes=[w.HANDLE,c.c_void_p,w.DWORD,c.POINTER(w.DWORD),c.c_void_p];k.WriteFile.argtypes=k.ReadFile.argtypes;k.CloseHandle.argtypes=[w.HANDLE]
direct='--direct' in sys.argv
name=r'\\.\pipe\airplay-bridge-native-check'
h=k.CreateNamedPipeW(name,3,8,1,2048,2048,5000,None);assert h!=w.HANDLE(-1).value
errors=[]
def pipe_server():
 try:
  assert k.ConnectNamedPipe(h,None) or c.get_last_error()==535
  def read(n):
   buf=c.create_string_buffer(n);used=w.DWORD();assert k.ReadFile(h,buf,n,c.byref(used),None) and used.value==n;return buf.raw
  length=struct.unpack('<I',read(4))[0];assert read(length)==b'127.0.0.1'
  secret=m.SECRET.encode();buf=c.create_string_buffer(struct.pack('<I',len(secret))+secret);used=w.DWORD();assert k.WriteFile(h,buf,len(buf)-1,c.byref(used),None)
 except BaseException as e:errors.append(str(e))
 finally:k.CloseHandle(h)
p=threading.Thread(target=pipe_server,daemon=True);p.start()
s=socket.socket();s.bind(('127.0.0.1',0));s.listen(2);s.settimeout(15)
r=threading.Thread(target=m.receiver,args=(s,'success',errors) if direct else (s,'auto-password',errors),daemon=True);r.start()
result=subprocess.run([str(m.BACKEND),'--host','127.0.0.1','--port',str(s.getsockname()[1]),'--password-auto','--password-pipe',name,'--bind-ip','127.0.0.1','--timing','ntp','--hold-seconds','0']+(['--password-pipe-first'] if direct else []),capture_output=True,text=True,encoding='utf-8',timeout=15)
p.join(2);r.join(2);log=result.stdout+result.stderr
assert result.returncode==0,log
assert 'PASSWORD_NEEDED' in log and 'PASSWORD_ACCEPTED' in log and 'SESSION_ACCEPTED' in log,log
if direct: assert 'mode=automatic' not in log and 'fixed-pin' not in log,log
assert m.SECRET not in log and not errors and not p.is_alive() and not r.is_alive(),errors
print(('Direct retry: ' if direct else '')+'PASS native GUI password pipe: automatic challenge, SRP authentication, cleanup; secret absent from log')
