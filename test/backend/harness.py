"""Bounded lifecycle for localhost protocol fixtures, including failure paths."""
from functools import wraps
import socket
import subprocess
import threading
import time


class Resources:
    def __init__(self):
        self.stop = threading.Event()
        self.sockets = []
        self.threads = []
        self.children = []
        self.failures = []
        self.lock = threading.Lock()
        self.closed = False

    def socket(self, endpoint=None):
        endpoint = endpoint if endpoint is not None else socket.socket()
        with self.lock:
            if self.closed:
                endpoint.close()
                raise RuntimeError('fixture already closed')
            self.sockets.append(endpoint)
        return endpoint

    def thread(self, target, args=()):
        def run():
            try:
                target(*args)
            except Exception as error:
                if not self.stop.is_set():
                    self.failures.append(repr(error))
        worker = threading.Thread(target=run, daemon=True)
        with self.lock:
            if self.closed:
                raise RuntimeError('fixture already closed')
            self.threads.append(worker)
            worker.start()
        return worker

    def popen(self, command, **kwargs):
        child = subprocess.Popen(command, **kwargs)
        self.children.append(child)
        return child

    def __enter__(self):
        return self

    def __exit__(self, exc_type, error, traceback):
        # 先终止子进程，再关闭管道，避免关闭 stdin 时被尚未消费的写入阻塞。
        # Kill/reap children before closing pipes so stdin flush cannot wait on a stalled reader.
        self.stop.set()
        for child in self.children:
            try:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=3)
            except (OSError, subprocess.TimeoutExpired) as cleanup_error:
                self.failures.append(repr(cleanup_error))
            if child.poll() is not None and child.stdin and not child.stdin.closed:
                try:
                    child.stdin.close()
                except OSError:
                    pass
        with self.lock:
            self.closed = True
            for endpoint in self.sockets:
                try:
                    endpoint.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    endpoint.close()
                except OSError as cleanup_error:
                    self.failures.append(repr(cleanup_error))
        deadline = time.monotonic() + 6
        for worker in self.threads:
            worker.join(max(0, deadline - time.monotonic()))
        alive = [worker.name for worker in self.threads if worker.is_alive()]
        # 不争抢仍在读取的 Python 管道锁；报告超时而非在 close 中再次无限等待。
        # Avoid a live reader's Python pipe lock; report timeout instead of blocking again in close.
        running = [child.pid for child in self.children if child.poll() is None]
        if not alive and not running:
            for child in self.children:
                for stream in (child.stdout, child.stderr):
                    if stream:
                        stream.close()
        if alive or running or self.failures:
            message = f'fixture cleanup failed: children={running}, threads={alive}, errors={self.failures}'
            if error is not None:
                if hasattr(error, 'add_note'):
                    error.add_note(message)
                else:
                    import warnings
                    warnings.warn(message, RuntimeWarning)
            else:
                raise AssertionError(message)


def managed_case(function):
    @wraps(function)
    def run(*args, **kwargs):
        with Resources() as resources:
            return function(*args, **kwargs, resources=resources)
    return run
