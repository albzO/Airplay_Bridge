#pragma once
#include <stdbool.h>
#include <stdio.h>
#include <stdint.h>
typedef const char *(*probe_password_request)(const char *host, const char *name, void *arg);
typedef struct {
    const char *host, *name, *txt, *password, *credentials, *identity, *bind_ip;
    const char *pcm_file;
    const char *active_remote;
    const char *peer_host, *peer_name, *peer_txt, *peer_identity, *peer_active_remote;
    int peer_port, peer_volume_control_port;
    int port, hold_seconds, lead_ms, buffer_ms, volume_control_port;
    bool use_ptp, play_tone, pcm_stdin;
    bool peer_password_first;
    probe_password_request request_password;
    void *password_arg;
} probe_options;
int probe_run(const probe_options *options);
extern int probe_debug;
void probe_set_secret(const char *secret);
FILE *probe_pcm_open(const char *path, uint64_t *frames);
bool probe_audio_nonce(uint64_t *counter, uint8_t nonce[12]);
bool probe_nonce_selfcheck(void);

