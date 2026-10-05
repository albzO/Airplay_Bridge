#pragma once
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <stdint.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <io.h>
#include <fcntl.h>
#include <errno.h>
#include <limits.h>
#include <time.h>
#include <sys/types.h>
#include <pthread.h>

/* Signed pointer-sized handle preserves the upstream -1 invalid sentinel. */
typedef intptr_t ap2_socket_t;
#ifndef MSG_DONTWAIT
#define MSG_DONTWAIT 0
#endif
#ifndef SHUT_RDWR
#define SHUT_RDWR SD_BOTH
#endif
#ifndef EPROTO
#define EPROTO EINVAL
#endif
#ifdef __cplusplus
extern "C" {
#endif
void ap2_socket_error(void);
ap2_socket_t ap2_socket_open(int family, int kind, int protocol);
int ap2_socket_connect(ap2_socket_t fd, const struct sockaddr *addr, int len);
int ap2_socket_bind(ap2_socket_t fd, const struct sockaddr *addr, int len);
int ap2_socket_close(ap2_socket_t fd);
int ap2_socket_send(ap2_socket_t fd, const void *data, size_t len, int flags);
int ap2_socket_recv(ap2_socket_t fd, void *data, size_t len, int flags);
int ap2_socket_sendto(ap2_socket_t fd, const void *data, size_t len, int flags,
                      const struct sockaddr *addr, int addr_len);
int ap2_socket_recvfrom(ap2_socket_t fd, void *data, size_t len, int flags,
                       struct sockaddr *addr, int *addr_len);
int ap2_socket_setsockopt(ap2_socket_t fd, int level, int option, const void *value, int len);
int ap2_socket_poll(struct pollfd *fds, unsigned long count, int timeout);
int ap2_socket_read(ap2_socket_t fd, void *data, size_t len);
int ap2_socket_write(ap2_socket_t fd, const void *data, size_t len);
int ap2_clock_gettime(int clock_id, struct timespec *ts);
void ap2_usleep(unsigned long micros);
char *ap2_strcasestr(const char *haystack, const char *needle);
struct in_addr get_interface(char *ip, char **name, uint32_t *mask);
void get_mac(uint8_t mac[6]);
#ifdef __cplusplus
}
#endif
#ifndef AP2_PORT_IMPLEMENTATION
#define socket ap2_socket_open
#define connect ap2_socket_connect
#define bind ap2_socket_bind
#define close ap2_socket_close
#define send ap2_socket_send
#define recv ap2_socket_recv
#define sendto ap2_socket_sendto
#define recvfrom ap2_socket_recvfrom
#define setsockopt ap2_socket_setsockopt
#define poll ap2_socket_poll
#define read ap2_socket_read
#define write ap2_socket_write
#define clock_gettime ap2_clock_gettime
#define usleep ap2_usleep
#define strcasestr ap2_strcasestr
#endif

