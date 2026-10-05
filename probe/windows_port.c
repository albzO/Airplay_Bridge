#define AP2_PORT_IMPLEMENTATION
#include "windows_port.h"
#include <iphlpapi.h>

void ap2_socket_error(void) {
    int code = WSAGetLastError();
    switch (code) {
    case WSAEWOULDBLOCK: errno = EWOULDBLOCK; break;
    case WSAEINTR: errno = EINTR; break;
    case WSAETIMEDOUT: errno = ETIMEDOUT; break;
    case WSAECONNRESET: case WSAECONNABORTED: errno = ECONNRESET; break;
    case WSAECONNREFUSED: errno = ECONNREFUSED; break;
    case WSAEADDRINUSE: errno = EADDRINUSE; break;
    case WSAEACCES: errno = EACCES; break;
    case WSAENOBUFS: errno = ENOBUFS; break;
    case WSAEMSGSIZE: errno = EMSGSIZE; break;
    case WSAENOTSOCK: errno = EBADF; break;
    case WSAENETUNREACH: case WSAEHOSTUNREACH: errno = EHOSTUNREACH; break;
    default: errno = EIO; break;
    }
}
static int checked(int result) { if (result == SOCKET_ERROR) ap2_socket_error(); return result; }
ap2_socket_t ap2_socket_open(int family, int kind, int protocol) {
    SOCKET fd = socket(family, kind, protocol);
    if (fd == INVALID_SOCKET) { ap2_socket_error(); return -1; }
    return (ap2_socket_t)fd;
}
int ap2_socket_close(ap2_socket_t fd) { return checked(closesocket((SOCKET)fd)); }
int ap2_socket_bind(ap2_socket_t fd, const struct sockaddr *addr, int len) {
    return checked(bind((SOCKET)fd, addr, len));
}
int ap2_socket_poll(struct pollfd *fds, unsigned long count, int timeout) {
    return checked(WSAPoll(fds, count, timeout));
}
int ap2_socket_connect(ap2_socket_t fd, const struct sockaddr *addr, int len) {
    u_long mode = 1;
    if (checked(ioctlsocket((SOCKET)fd, FIONBIO, &mode)) != 0) return -1;
    int result = connect((SOCKET)fd, addr, len);
    if (result == SOCKET_ERROR) {
        int code = WSAGetLastError();
        if (code != WSAEWOULDBLOCK && code != WSAEINPROGRESS) { ap2_socket_error(); return -1; }
        fd_set writes, errors;
        FD_ZERO(&writes); FD_ZERO(&errors);
        FD_SET((SOCKET)fd, &writes); FD_SET((SOCKET)fd, &errors);
        struct timeval timeout = {.tv_sec = 8};
        result = select(0, NULL, &writes, &errors, &timeout);
        if (result <= 0) { if (result == 0) errno = ETIMEDOUT; else ap2_socket_error(); return -1; }
        int error = 0, size = sizeof(error);
        if (checked(getsockopt((SOCKET)fd, SOL_SOCKET, SO_ERROR, (char *)&error, &size)) != 0) return -1;
        if (error) { WSASetLastError(error); ap2_socket_error(); return -1; }
    }
    mode = 0;
    return checked(ioctlsocket((SOCKET)fd, FIONBIO, &mode));
}
int ap2_socket_send(ap2_socket_t fd, const void *data, size_t len, int flags) {
    return checked(send((SOCKET)fd, (const char *)data, len > INT_MAX ? INT_MAX : (int)len, flags));
}
int ap2_socket_recv(ap2_socket_t fd, void *data, size_t len, int flags) {
    return checked(recv((SOCKET)fd, (char *)data, len > INT_MAX ? INT_MAX : (int)len, flags));
}
int ap2_socket_sendto(ap2_socket_t fd, const void *data, size_t len, int flags,
                     const struct sockaddr *addr, int addr_len) {
    return checked(sendto((SOCKET)fd, data, (int)len, flags, addr, addr_len));
}
int ap2_socket_recvfrom(ap2_socket_t fd, void *data, size_t len, int flags,
                       struct sockaddr *addr, int *addr_len) {
    return checked(recvfrom((SOCKET)fd, data, (int)len, flags, addr, addr_len));
}
int ap2_socket_setsockopt(ap2_socket_t fd, int level, int option, const void *value, int len) {
    if (level == SOL_SOCKET && (option == SO_RCVTIMEO || option == SO_SNDTIMEO) && len == sizeof(struct timeval)) {
        const struct timeval *tv = value;
        DWORD millis = (DWORD)(tv->tv_sec * 1000 + (tv->tv_usec + 999) / 1000);
        return checked(setsockopt((SOCKET)fd, level, option, (const char *)&millis, sizeof(millis)));
    }
    return checked(setsockopt((SOCKET)fd, level, option, (const char *)value, len));
}
int ap2_socket_read(ap2_socket_t fd, void *data, size_t len) {
    return ap2_socket_recv(fd, data, len, 0);
}
int ap2_socket_write(ap2_socket_t fd, const void *data, size_t len) {
    /* Pairing writes may be partial too. A single deadline covers the whole write. */
    u_long mode = 1;
    if (checked(ioctlsocket((SOCKET)fd, FIONBIO, &mode)) != 0) return -1;
    ULONGLONG deadline = GetTickCount64() + 8000;
    size_t offset = 0;
    while (offset < len) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline) { errno = ETIMEDOUT; break; }
        struct pollfd pollfd = {.fd = (SOCKET)fd, .events = POLLOUT};
        int ready = ap2_socket_poll(&pollfd, 1, (int)(deadline - now));
        if (ready <= 0) { if (ready == 0) errno = ETIMEDOUT; break; }
        int sent = ap2_socket_send(fd, (const char *)data + offset, len - offset, 0);
        if (sent > 0) offset += (size_t)sent;
        else if (sent < 0 && (errno == EWOULDBLOCK || errno == EINTR || errno == ENOBUFS)) continue;
        else break;
    }
    int saved = errno;
    mode = 0;
    if (checked(ioctlsocket((SOCKET)fd, FIONBIO, &mode)) != 0) return -1;
    errno = saved;
    return offset == len ? (int)offset : -1;
}
int ap2_clock_gettime(int clock_id, struct timespec *ts) {
    if (clock_id == CLOCK_MONOTONIC) {
        LARGE_INTEGER value, frequency;
        QueryPerformanceCounter(&value); QueryPerformanceFrequency(&frequency);
        ts->tv_sec = value.QuadPart / frequency.QuadPart;
        ts->tv_nsec = (long)((value.QuadPart % frequency.QuadPart) * 1000000000LL / frequency.QuadPart);
    } else {
        FILETIME ft; ULARGE_INTEGER value;
        GetSystemTimePreciseAsFileTime(&ft);
        value.LowPart = ft.dwLowDateTime; value.HighPart = ft.dwHighDateTime;
        uint64_t ticks = value.QuadPart - 116444736000000000ULL;
        ts->tv_sec = ticks / 10000000ULL;
        ts->tv_nsec = (long)((ticks % 10000000ULL) * 100);
    }
    return 0;
}
void ap2_usleep(unsigned long micros) {
    struct timespec delay = {.tv_sec = micros / 1000000UL, .tv_nsec = (micros % 1000000UL) * 1000UL};
    nanosleep(&delay, NULL);
}
char *ap2_strcasestr(const char *haystack, const char *needle) {
    size_t len = strlen(needle);
    for (; *haystack; haystack++) if (strncasecmp(haystack, needle, len) == 0) return (char *)haystack;
    return len == 0 ? (char *)haystack : NULL;
}
struct in_addr get_interface(char *ip, char **name, uint32_t *mask) {
    struct in_addr address = {.s_addr = INADDR_ANY};
    *name = NULL; *mask = 0;
    inet_pton(AF_INET, ip, &address);
    return address;
}
void get_mac(uint8_t mac[6]) {
    memset(mac, 0, 6);
    ULONG size = 0;
    GetAdaptersInfo(NULL, &size);
    IP_ADAPTER_INFO *adapters = malloc(size);
    if (adapters && GetAdaptersInfo(adapters, &size) == NO_ERROR) {
        for (IP_ADAPTER_INFO *item = adapters; item; item = item->Next)
            if (item->AddressLength == 6) { memcpy(mac, item->Address, 6); break; }
    }
    free(adapters);
}

