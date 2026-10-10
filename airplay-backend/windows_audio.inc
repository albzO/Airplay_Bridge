static bool ap2_set_nonblocking(ap2_socket_t fd, const char *name) {
    u_long mode = 1;
    if (ioctlsocket((SOCKET)fd, FIONBIO, &mode) == 0) return true;
    ap2_socket_error();
    LOG_ERROR("[AP2] Cannot make %s nonblocking: %s", name, strerror(errno));
    return false;
}
