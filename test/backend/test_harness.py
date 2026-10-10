import socket
import struct
import subprocess
import sys
import unittest
from audio_sequence import AudioSequence
from harness import Resources


def packet(sequence, timestamp, nonce, first=False):
    return struct.pack('!BBHII', 0x80, 0xe0 if first else 0x60, sequence, timestamp, 0) + b'x'*16 + nonce.to_bytes(8, 'little')


class HarnessTests(unittest.TestCase):
    def test_rtp_and_timestamp_wrap_while_nonce_increases(self):
        counter = AudioSequence(packet=65535, timestamp=0xffffff00, nonce=65535)
        counter.inspect(packet(65535, 0xffffff00, 65535, first=True))
        counter.inspect(packet(0, (0xffffff00+352) & 0xffffffff, 65536))
        counter.inspect(packet(1, (0xffffff00+704) & 0xffffffff, 65537))

    def test_wrapped_nonce_and_reset_timestamp_are_rejected(self):
        counter = AudioSequence(packet=65536, nonce=65536)
        with self.assertRaises(AssertionError):
            counter.inspect(packet(0, 441000, 0, first=True))
        counter = AudioSequence(packet=65535, timestamp=0xffffff00, nonce=65535)
        counter.inspect(packet(65535, 0xffffff00, 65535, first=True))
        with self.assertRaises(AssertionError):
            counter.inspect(packet(0, 441000, 65536))

    def test_failure_reaps_child_closes_socket_and_joins_worker(self):
        resources = Resources()
        with self.assertRaisesRegex(AssertionError, 'fixture failure'):
            with resources:
                endpoint = resources.socket()
                endpoint.bind(('127.0.0.1', 0))
                worker = resources.thread(resources.stop.wait)
                child = resources.popen([sys.executable, '-c', 'import time; time.sleep(30)'],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                raise AssertionError('fixture failure')
        self.assertIsNotNone(child.poll())
        self.assertFalse(worker.is_alive())
        self.assertEqual(endpoint.fileno(), -1)
        self.assertTrue(child.stdin.closed and child.stdout.closed and child.stderr.closed)


if __name__ == '__main__':
    unittest.main()
