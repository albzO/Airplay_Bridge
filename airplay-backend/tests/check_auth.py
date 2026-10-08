"""Focused auth routing checks; no audio or physical receiver involved."""
import subprocess
from mock_receiver import BACKEND, SECRET, test_case

if __name__=='__main__':
    # 参数阶段拒绝超长或多行密码，不能先截断再进入认证与日志路径。
    # Reject oversized/multiline passwords during argument parsing, before authentication or logging.
    for secret in ['x' * 1024, 'example\npassword']:
        result = subprocess.run([str(BACKEND), '--host', '127.0.0.1', '--password', secret],
                                capture_output=True, text=True, encoding='utf-8', timeout=5)
        assert result.returncode == 2 and secret not in result.stderr
    print('PASS password bounds: oversized and multiline arguments rejected')
    test_case('auto-open', SECRET, 0, ['AUTH_ATTEMPT host=127.0.0.1 mode=automatic',
        'AUTH_METHOD value=fixed-pin','SESSION_ACCEPTED'], ['PASSWORD_NEEDED','PASSWORD_ACCEPTED'], automatic=True)
    test_case('auto-password', SECRET, 0, ['PASSWORD_NEEDED','mode=password',
        'PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
    test_case('auto-wrong-password', 'wrong-local-password', 10, ['PASSWORD_NEEDED',
        'ERROR code=PASSWORD_REJECTED exit=10'], ['PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
    for mode, code, name in [('pair403',15,'ACCESS_DENIED'),('pair470',14,'PAIRING_REQUIRED'),
                             ('pair-policy',13,'AUTH_REJECTED'),('pair-backoff',12,'PAIRING_BACKOFF'),
                             ('pair-max-tries',16,'PAIRING_MAX_TRIES'),('pair-max-peers',17,'PAIRING_MAX_PEERS'),
                             ('pair-unavailable',18,'PAIRING_UNAVAILABLE'),('pair-busy',19,'PAIRING_BUSY')]:
        test_case(mode, SECRET, code, [f'ERROR code={name} exit={code}'],
            ['PASSWORD_NEEDED','PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
