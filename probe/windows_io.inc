bool ap2_io_write_all_deadline(ap2_socket_t fd, const uint8_t *data, size_t len,
                              uint64_t deadline_ms)
{
    u_long mode = 1;
    if (ioctlsocket((SOCKET)fd, FIONBIO, &mode) != 0) { ap2_socket_error(); return false; }
    size_t offset = 0;
    while (offset < len) {
        if (ap2_io_poll_fd(fd, POLLOUT, deadline_ms) <= 0) break;
        int count = send(fd, data + offset, len - offset, 0);
        if (count > 0) offset += (size_t)count;
        else if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR || errno == ENOBUFS)) continue;
        else { if (count == 0) errno = EPIPE; break; }
    }
    return offset == len;
}

ssize_t ap2_io_read_deadline(ap2_socket_t fd, uint8_t *buf, size_t len,
                           uint64_t deadline_ms)
{
    u_long mode = 1;
    if (ioctlsocket((SOCKET)fd, FIONBIO, &mode) != 0) { ap2_socket_error(); return -1; }
    for (;;) {
        if (ap2_io_poll_fd(fd, POLLIN, deadline_ms) <= 0) return -1;
        int count = recv(fd, buf, len, 0);
        if (count < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) continue;
        if (count == 0) errno = ECONNRESET;
        return count;
    }
}
