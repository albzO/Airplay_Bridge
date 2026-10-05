"""Focused auth routing checks; no audio or physical receiver involved."""
from mock_receiver import SECRET, test_case

if __name__=='__main__':
    test_case('auto-open', SECRET, 0, ['AUTH_ATTEMPT host=127.0.0.1 mode=automatic',
        'AUTH_METHOD value=fixed-pin','SESSION_ACCEPTED'], ['PASSWORD_NEEDED','PASSWORD_ACCEPTED'], automatic=True)
    test_case('auto-password', SECRET, 0, ['PASSWORD_NEEDED','mode=password',
        'PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
    test_case('auto-wrong-password', 'wrong-local-password', 10, ['PASSWORD_NEEDED',
        'ERROR code=PASSWORD_REJECTED exit=10'], ['PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
    for mode, code, name in [('pair403',15,'ACCESS_DENIED'),('pair470',14,'PAIRING_REQUIRED'),
                             ('pair-policy',13,'AUTH_REJECTED'),('pair-backoff',12,'PAIRING_BACKOFF')]:
        test_case(mode, SECRET, code, [f'ERROR code={name} exit={code}'],
            ['PASSWORD_NEEDED','PASSWORD_ACCEPTED','SESSION_ACCEPTED'], automatic=True)
