"""Build a minimal view of pinned upstream code. Never edit generated files.

HAP/SRP, framing, RTSP parsing and plist implementations remain upstream code.
Control tests stop after session SETUP/events. Tone tests select realtime audio.
"""
import hashlib
import pathlib
import re
import sys

root, out = map(pathlib.Path, sys.argv[1:])
out.mkdir(parents=True, exist_ok=True)
client = (root / 'src/ap2_client.c').read_text(encoding='utf-8')
expected = '5e9e30835193c4b73d954b134f4052dfb8ac8f04b6584deabb042540afcb479e'
# Git on Windows may check out CRLF; compare normalized content as well.
raw_hash = hashlib.sha256((root / 'src/ap2_client.c').read_bytes()).hexdigest()
normalized_hash = hashlib.sha256(client.encode()).hexdigest()
if expected not in (raw_hash, normalized_hash):
    raise SystemExit('Upstream client changed; review extraction before building: ' + raw_hash)

def function(name, source=client):
    pattern = re.compile(r'^(?:static[ \t]+)?[A-Za-z_]\w*(?:[ \t]+[A-Za-z_]\w*)*[ \t*]+' + re.escape(name) + r'\([^;{}]*\)\s*\{', re.M)
    matches = list(pattern.finditer(source))
    if len(matches) != 1:
        raise ValueError(f'{name}: expected one definition, found {len(matches)}')
    start = matches[0].start()
    pos = matches[0].end() - 1
    depth, state = 0, 'code'
    i = pos
    while i < len(source):
        c, pair = source[i], source[i:i+2]
        if state == 'code':
            if pair == '/*': state = 'comment'; i += 1
            elif pair == '//': state = 'line'; i += 1
            elif c == '"': state = 'string'
            elif c == "'": state = 'char'
            elif c == '{': depth += 1
            elif c == '}':
                depth -= 1
                if depth == 0: return source[start:i+1].strip()
        elif state == 'comment':
            if pair == '*/': state = 'code'; i += 1
        elif state == 'line':
            if c == '\n': state = 'code'
        elif c == '\\': i += 1
        elif (state == 'string' and c == '"') or (state == 'char' and c == "'"): state = 'code'
        i += 1
    raise ValueError('Unclosed function ' + name)

names = '''ap2_env_enabled ap2_status_is_auth ap2_auth_error_kind ap2_set_connect_error
ap2_capture_response ap2_log_failed_response ap2_report_failed_exchange
ap2_rtsp_write_request ap2_rtsp_farewell_teardown ap2_mark_rtsp_dead ap2_rtsp_timeout_ms
ap2_rtsp_send_ex_unlocked ap2_rtsp_send_ex_tracked ap2_rtsp_send_ex ap2_rtsp_send
ap2_gen_uuid ap2_dacp_bytes ap2_colon_hex ap2_txt_hex_field ap2_txt_features
ap2_features_has_ptp ap2_splice_denied ap2_txt_field ap2_model_prefix ap2_txt_version_major
ap2_receiver_os_ge_27 ap2_follow_receiver_clock ap2_audio_format_code
ap2_parse_format_capability ap2_make_timing_peer ap2_native_open_socket
ap2_native_get_info ap2_native_reset_connection ap2_native_transient
ap2_native_pair_verify ap2_native_pair ap2_native_connect'''.split()
selected = []
for name in names:
    body = function(name)
    if name == 'ap2_native_connect':
        marker = '    /* 5. RECORD'
        if marker not in body: raise ValueError('Session boundary changed')
        body = body[:body.index(marker)] + '''
    p->rtsp_established = true;
    ap2_io_status_line("[PROBE] SESSION_ACCEPTED timing=%s", p->use_ptp ? "ptp" : "ntp");
    return true;
}
'''
        body = body.replace('if (!ap2_native_open_socket(p)) return false;',
                            'if (!ap2_native_open_socket(p)) return false;\n    ap2_io_status_line("[PROBE] TCP_CONNECTED");')
        body = body.replace('if (!ap2_native_get_info(p)) return false;',
                            'p->phase = "info";\n    ap2_io_status_line("[PROBE] GET_INFO_SENT");\n    if (!ap2_native_get_info(p)) return false;\n    ap2_io_status_line("[PROBE] GET_INFO_OK");')
        body = body.replace('if (!ap2_native_pair(p)) return false;',
                            'p->phase = "pairing";\n    if (!ap2_native_pair(p)) return false;')
        body = body.replace('LOG_INFO("[AP2] Channel encrypted");',
                            'LOG_INFO("[AP2] Channel encrypted");\n    ap2_io_status_line("[PROBE] ENCRYPTION_KEYS_DERIVED");')
        body = body.replace('int status = ap2_rtsp_send(p, "SETUP",',
                            'p->phase = "session-setup";\n    ap2_io_status_line("[PROBE] RTSP_SETUP_SENT");\n    int status = ap2_rtsp_send(p, "SETUP",')
        body = body.replace('LOG_INFO("[AP2] Session SETUP OK',
                            'ap2_io_status_line("[PROBE] ENCRYPTED_RESPONSE_OK");\n    ap2_io_status_line("[PROBE] RTSP_SETUP_200_OK");\n    LOG_INFO("[AP2] Session SETUP OK')
        body = body.replace('LOG_INFO("[AP2] Events connection OK");',
                            'LOG_INFO("[AP2] Events connection OK");\n                ap2_io_status_line("[PROBE] EVENTS_CONNECTED");')
        body = body.replace('event_port = (int)v;', 'event_port = (int)v;')
        body = body.replace('free(resp);\n\n    /* Open events',
                            'free(resp);\n    p->event_required = event_port > 0;\n\n    /* Open events')
        body = body.replace('p->ptp = ap2_ptp_create();',
                            'p->phase = "timing";\n    if (!p->ptp) p->ptp = ap2_ptp_create();\n    if (!p->ptp) return false;')
        body = body.replace('    if (want_ptp) {',
                            '    if (p->ptp_borrowed) {\n        if (!want_ptp) return false;\n        p->use_ptp = true;\n    } else if (want_ptp) {',1)
        body = body.replace('ap2_gen_uuid(p->group_uuid);','if (!p->group_uuid[0]) ap2_gen_uuid(p->group_uuid);')
        body = body.replace('    /* Buffered (type 103)', '''    if (!p->use_ptp && timing_port <= 0) {
        ap2_set_connect_error(p, AP2_CONNECT_ERROR_GENERIC, 0, "Timing responder failed to start");
        return false;
    }
    ap2_io_status_line("[PROBE] TIMING value=%s", p->use_ptp ? "ptp" : "ntp");

    /* Buffered (type 103)''')
    if name == 'ap2_native_pair':
        marker='    int password_status = err.http_status;'
        if body.count(marker)!=1: raise ValueError('Pairing error boundary changed')
        body=body.replace(marker,'''    if (err.tlv_error == 3 || err.tlv_error == 5) {
        p->probe_error = 12;
        ap2_set_connect_error(p, AP2_CONNECT_ERROR_AUTH_FAILED, err.http_status,
                              "pairing temporarily rate-limited by receiver");
        return false;
    }
    if (err.result == AP2_HAP_ERR_AUTH) {
        if (err.pair_state == 4 && err.tlv_error == 2) p->probe_error = 10;
        ap2_set_connect_error(p, AP2_CONNECT_ERROR_AUTH_FAILED, err.http_status,
                              "device rejected the supplied password");
        return false;
    }
''' + marker)
        body = body.replace('LOG_INFO("[AP2] Device accepted the password',
                            'p->auth_method = "password";\n        ap2_io_status_line("[PROBE] PASSWORD_ACCEPTED");\n        LOG_INFO("[AP2] Device accepted the password')
        body = body.replace('if (ap2_native_pair_verify(p, &err)) return true;',
                            'if (ap2_native_pair_verify(p, &err)) { p->auth_method = "credentials"; return true; }')
        body = body.replace('if (ap2_native_transient(p, NULL, &err)) return true;',
                            'if (ap2_native_transient(p, NULL, &err)) { p->auth_method = "fixed-pin"; return true; }')
        body = body.replace('LOG_INFO("[AP2] Stored credentials accepted',
                            'p->auth_method = "credentials";\n            LOG_INFO("[AP2] Stored credentials accepted')
        body = body.replace('LOG_WARN("[AP2] Device paired with the fixed',
                            'p->auth_method = "fixed-pin";\n        LOG_WARN("[AP2] Device paired with the fixed')
        body = body.replace('static bool ap2_native_pair(', 'static bool probe_pair_once(', 1)
        body += '\n' + (pathlib.Path(__file__).parent / 'auth_auto.inc').read_text(encoding='utf-8')
    if name in ('ap2_native_transient', 'ap2_native_pair_verify'):
        body = body.replace('    LOG_ERROR("[AP2] HAP ', '    p->last_pair_error = *err;\n    LOG_ERROR("[AP2] HAP ', 1)
    body = body.replace('int events_sock = socket(', 'ap2_socket_t events_sock = socket(')
    selected.append(body)

# Retain the pinned realtime stream setup, excluding buffered TCP and MRP.
native = function('ap2_native_connect')
audio = native[native.index('    /* 5. RECORD'):native.index('    /* 6b. MRP')]
buffered = audio.index('    /* Buffered audio: open the TCP')
peers = audio.index('    /* 6. SETPEERS')
audio = audio[:buffered] + audio[peers:]
audio = audio.replace('status = ap2_rtsp_send(p, "RECORD",',
                      'p->phase = "record";\n    status = ap2_rtsp_send(p, "RECORD",')
audio = audio.replace('    if (status <= 0) {\n        free(plist_data);',
                      '    if (status != 200) {\n        ap2_report_failed_exchange(p, "RECORD", status);\n        free(plist_data);')
audio = audio.replace('LOG_INFO("[AP2] RECORD OK");',
                      'LOG_INFO("[AP2] RECORD OK");\n        ap2_io_status_line("[PROBE] RECORD_OK");')
audio = audio.replace('    status = ap2_rtsp_send(p, "SETUP",',
                      '    p->phase = "stream-setup";\n    status = ap2_rtsp_send(p, "SETUP",')
audio = audio.replace('    LOG_INFO("[AP2] Stream SETUP OK");', '''    if (!p->data_addr.sin_port || !p->ctrl_addr.sin_port) {
        free(resp);
        ap2_set_connect_error(p, AP2_CONNECT_ERROR_GENERIC, 200, "Stream response has missing/invalid remote ports");
        return false;
    }
    ap2_io_status_line("[PROBE] STREAM_SETUP_OK");
    LOG_INFO("[AP2] Stream SETUP OK");''')
audio = audio.replace('        if (sp_status <= 0) return false;', '''        if (sp_status != 200) {
            ap2_report_failed_exchange(p, "SETPEERS", sp_status);
            return false;
        }
        ap2_io_status_line("[PROBE] SETPEERS_OK");''')
audio = '''static bool ap2_set_nonblocking(ap2_socket_t fd, const char *name) {
    u_long mode = 1;
    if (ioctlsocket((SOCKET)fd, FIONBIO, &mode) == 0) return true;
    ap2_socket_error();
    LOG_ERROR("[AP2] Cannot make %s nonblocking: %s", name, strerror(errno));
    return false;
}
static bool probe_audio_setup(struct ap2cl_s *p) {
    int len, plist_len = 0; uint8_t *plist_data = NULL;
    uint8_t *resp = NULL; int resp_len = 0, status = 0;
    p->phase = "stream-setup";
    struct sockaddr_in local; len = sizeof(local);
    if (getsockname(p->sock_fd, (struct sockaddr *)&local, &len) != 0) return false;
    char our_addr[INET_ADDRSTRLEN];
    inet_ntop(AF_INET, &local.sin_addr, our_addr, sizeof(our_addr));
''' + audio + '''
    p->first_packet = true;
    p->alac = alac_create_encoder(352, 44100, 16, 2);
    if (!p->alac || !ap2_rtx_start(p)) return false;
    return true;
}
'''
for name in ['ap2_rtx_store', 'ap2_rtx_resend', 'ap2_rtx_thread_main', 'ap2_rtx_start', 'ap2_rtx_stop',
             'ap2_send_sync_packet', 'ap2_send_sync_packet_ptp', 'ap2_encrypt_audio', 'ap2_native_send_chunk']:
    body = function(name)
    body = body.replace('    if (p->use_buffered) return ap2_buffered_send_chunk(p, sample, frames);', '')
    if name == 'ap2_native_send_chunk':
        original_nonce = '    uint16_t seq16 = (uint16_t)p->seq_number;\n    memcpy(nonce + 4, &seq16, 2);'
        assert body.count(original_nonce) == 1, 'Pinned audio nonce block changed'
        body = body.replace(original_nonce, '''    if (!probe_audio_nonce(&p->audio_nonce_counter, nonce)) {
        free(encoded);
        return AP2_SEND_FATAL;
    }''')
        body = body.replace('Nonce: 12 bytes, all zero except the 2-byte sequence number at [4..5].',
                            'Nonce: four zero bytes followed by an independent 64-bit counter.')
        body = body.replace('/* seqnum at offset 4 in native (little-endian) byte order, matching owntone\n     * (memcpy(nonce+4, &seqnum, 2)). The same bytes are appended to the wire. */',
                            '/* Independent little-endian counter survives 16-bit RTP wrap. */')
    selected.append(body)
selected.append(audio)

context = r'''
#include "windows_port.h"
#include "probe.h"
#include "cross_log.h"
#include "ap2_hap.h"
#include "ap2_io.h"
#include "ap2_ptp.h"
#include "ap2_plist.h"
#include "ap2_bplist.h"
#include <stdatomic.h>
#include <stdarg.h>
#include <inttypes.h>
#include <openssl/rand.h>
#include <openssl/evp.h>
#include <math.h>
#include "raw_alac.h"
extern log_level *loglevel;
#define NFREE(p) do { free(p); (p) = NULL; } while (0)
typedef struct { int sample_rate, bit_depth, channels; } ap2_audio_format_t;
typedef struct { char *name, *hostname, *address; int port; char *txt_records; } ap2_device_info_t;
typedef enum { AP2_CONNECT_ERROR_NONE, AP2_CONNECT_ERROR_GENERIC,
               AP2_CONNECT_ERROR_AUTH_REQUIRED, AP2_CONNECT_ERROR_AUTH_FAILED } ap2_connect_error_t;
#define AP2_RTX_MAX_PKT (12 + 352 * 6 + 8 + 16 + 8)
#define AP2_RTX_RING_SLOTS 512
#define AP2_RTX_CTRL_POLL_MS 200
#define MS2TS(ms, rate) (((uint64_t)(ms) * (rate)) / 1000)
#define NTP2TS(ntp, rate) (((ntp) >> 32) * (rate) + ((((ntp) & 0xffffffffULL) * (rate)) >> 32))
static uint64_t raopcl_get_ntp(void *unused) {
    (void)unused; struct timespec now; clock_gettime(CLOCK_REALTIME, &now);
    return ((uint64_t)now.tv_sec << 32) | (((uint64_t)now.tv_nsec << 32) / 1000000000ULL);
}
struct ap2_rtx_slot { uint16_t seq, len; bool valid; uint8_t data[AP2_RTX_MAX_PKT]; };
struct ap2cl_s {
    ap2_device_info_t device;
    ap2_audio_format_t format;
    ap2_socket_t sock_fd, events_sock;
    struct probe_events *events;
    pthread_mutex_t rtsp_lock;
    atomic_bool rtsp_dead;
    atomic_uint feedback_failures;
    bool rtsp_established, event_required;
    uint8_t rtsp_carry[16384]; int rtsp_carry_len;
    char *password, *auth_credentials, *dacp_id, *active_remote, *iface, *publish_ip, *am;
    const char *auth_method, *phase;
    probe_password_request request_password;
    void *password_arg;
    ap2_hap_error_t last_pair_error;
    struct in_addr bind_addr;
    struct ap2_hap_ctx *hap;
    struct ap2_ptp_ctx *ptp;
    bool ptp_borrowed;
    struct ap2cl_s *group_peer;
    bool use_ptp, ptp_forced, ptp_enabled, ptp_shared, use_buffered, buffered_requested, splice_timeline;
    uint64_t audio_format, realtime_formats, buffered_formats;
    bool realtime_formats_known, buffered_formats_known, realtime_formats_extended, buffered_formats_extended;
    char session_url[128], session_uuid[40], group_uuid[40];
    uint32_t session_id; int cseq;
    ap2_connect_error_t connect_error; int connect_http_status;
    int probe_error;
    char connect_detail[192];
    uint8_t last_error_response[1536]; int last_error_response_len;
    ap2_socket_t data_sock, ctrl_sock;
    struct sockaddr_in data_addr, ctrl_addr;
    struct alac_codec_s *alac;
    uint8_t audio_key[32];
    uint16_t seq_number; uint32_t rtp_timestamp, ssrc;
    int lead_ms, dev_render_ms; uint32_t dev_latency_min, dev_latency_max;
    uint64_t start_ntp, head_ts, rt_anchor_wall0; uint32_t rt_anchor_pos0;
    atomic_uint rtp_offset;
    bool first_packet, rt_anchor_valid;
    atomic_bool media_healthy;
    uint64_t audio_packets_sent, audio_packets_dropped, sync_packets_sent, sync_packets_dropped;
    uint64_t audio_nonce_counter;
    pthread_mutex_t rtx_lock; pthread_t rtx_thread;
    atomic_bool rtx_stop; bool rtx_thread_started; struct ap2_rtx_slot *rtx_ring;
    atomic_ullong rtx_requested, rtx_answered, rtx_expired;
    pthread_t tone_feedback_thread; bool tone_feedback_started;
    atomic_bool tone_feedback_stop;
    ap2_socket_t volume_sock; bool volume_known; double volume_db;
    char volume_line[128]; int volume_line_len;
};
#define AP2_FEAT(f,n) (((f) >> (n)) & 1ULL)
#define AP2_FEAT_PTP 41
'''
constant_names = ['AP2_CHACHA_TAG_SIZE', 'AP2_FMT_ALAC_44100_16_2', 'AP2_FMT_ALAC_44100_24_2',
                  'AP2_FMT_ALAC_48000_16_2', 'AP2_FMT_ALAC_48000_24_2', 'AP2_RTSP_SETUP_TIMEOUT_MS',
                  'AP2_RTSP_CONTROL_TIMEOUT_MS', 'AP2_RTSP_FEEDBACK_TIMEOUT_MS', 'AP2_RTSP_METADATA_TIMEOUT_MS',
                  'AP2_RTSP_ARTWORK_TIMEOUT_MS', 'AP2_RTSP_FAREWELL_TIMEOUT_MS',
                  'AP2_FEEDBACK_MAX_CONSECUTIVE_MISSES', 'AP2_RTSP_RX_BUF_SIZE',
                  'AP2_DIAG_RESPONSE_MAX', 'AP2_DIAG_BODY_MAX', 'AP2_UDP_SEND_TIMEOUT_MS']
for name in constant_names:
    context += ('#define AP2_DIAG_BODY_MAX 0' if name == 'AP2_DIAG_BODY_MAX' else
                re.search(r'^#define\s+' + name + r'\s+[^\n]+', client, re.M).group()) + '\n'
footer = (pathlib.Path(__file__).parent / 'probe_entry.inc').read_text(encoding='utf-8')
footer = (pathlib.Path(__file__).parent / 'audio_entry.inc').read_text(encoding='utf-8') + footer
footer = (pathlib.Path(__file__).parent / 'volume_entry.inc').read_text(encoding='utf-8') + footer
footer = (pathlib.Path(__file__).parent / 'events_entry.inc').read_text(encoding='utf-8') + footer
(out / 'probe_client.c').write_text('/* Generated from pinned airplay-cli.\n * Copyright (C) 2024-2026 Music Assistant Contributors\n * SPDX-License-Identifier: Apache-2.0\n * Local changes: session extraction, Windows sockets and diagnostic markers.\n */\n' + context + '\n\n'.join(selected) + footer, encoding='utf-8')

def platform_view(text):
    # Include Windows declarations before anything that can include windows.h.
    text = re.sub(r'^#include <(?:arpa/inet.h|netinet/in.h|netinet/tcp.h|sys/socket.h|netdb.h|poll.h|unistd.h)>\n', '', text, flags=re.M)
    text = re.sub(r'^#include "\.\./libraop/(?:crosstools/src/(?:platform|cross_net)|src/raop_client)\.h"\n', '', text, flags=re.M)
    text = text.replace('int sock_fd', 'ap2_socket_t sock_fd').replace('int fd,', 'ap2_socket_t fd,')
    return '#include "windows_port.h"\n' + text

for name in ['ap2_hap.c', 'ap2_hap.h', 'ap2_io.c', 'ap2_io.h', 'ap2_ptp.c', 'ap2_ptp.h',
             'ap2_ptp_shm.h', 'ap2_plist.c', 'ap2_plist.h', 'ap2_bplist.cpp', 'ap2_bplist.h']:
    text = platform_view((root / 'src' / name).read_text(encoding='utf-8'))
    if name == 'ap2_io.c':
        # Windows I/O uses nonblocking Winsock sockets, preserving upstream deadlines.
        begin = text.index('bool ap2_io_write_all_deadline(')
        end = text.index('ap2_send_result_t ap2_io_send_datagram_deadline(', begin)
        text = text[:begin] + (pathlib.Path(__file__).parent / 'windows_io.inc').read_text() + text[end:]
    if name == 'ap2_hap.c':
        # Preserve the actual TLV state; M2 rejection is a pairing/policy issue,
        # whereas M4 error 2 rejects the client's SRP secret.
        transient = function('ap2_hap_pair_setup_transient', text)
        text = text.replace(transient, transient.replace('    if (err && err_len > 0) {',
                            '    if (err_out && state_val && state_len == 1) err_out->pair_state = *state_val;\n    if (err && err_len > 0) {'), 1)
        # Derive reverse-event keys before the full transient SRP key is wiped.
        # Audio intentionally retains only 32 bytes; it cannot supply these keys.
        text = text.replace('uint8_t shared_secret[32];\n',
            'uint8_t shared_secret[32];\n    uint8_t event_read_key[32], event_write_key[32];\n', 1)
        for secret, size in [('shared_secret', '32'), ('session_key', 'SRP_HASH_LEN')]:
            old = f'''!hkdf_sha512({secret}, {size},
                     "Control-Salt", "Control-Read-Encryption-Key",
                     ctx->read_key, 32))'''
            new = old[:-1] + f''' ||
        !hkdf_sha512({secret}, {size}, "Events-Salt", "Events-Write-Encryption-Key",
                     ctx->event_read_key, 32) ||
        !hkdf_sha512({secret}, {size}, "Events-Salt", "Events-Read-Encryption-Key",
                     ctx->event_write_key, 32))'''
            if text.count(old) != 1: raise ValueError('HAP key derivation boundary changed')
            text = text.replace(old, new)
        text += '''
/* Local reverse-channel context: independent keys and nonce counters. */
struct ap2_hap_ctx *probe_hap_events_create(struct ap2_hap_ctx *control) {
    if (!control || !control->verified) return NULL;
    struct ap2_hap_ctx *events = calloc(1, sizeof(*events));
    if (!events) return NULL;
    memcpy(events->read_key, control->event_read_key, 32);
    memcpy(events->write_key, control->event_write_key, 32);
    events->verified = true;
    return events;
}
'''
        # Keep status/header diagnostics while excluding raw authentication bodies.
        text = re.sub(r'(#define\s+HAP_DIAG_BODY_MAX\s+)\d+', r'\g<1>0', text)
        text = text.replace('LOG_INFO("[HAP] SRP challenge computed, sending M3...");',
                            'ap2_io_status_line("[PROBE] PAIR_M2_VALIDATED");\n    ap2_io_status_line("[PROBE] SRP_SALT_AND_SERVER_KEY_RECEIVED");\n    LOG_INFO("[HAP] SRP challenge computed, sending M3...");')
        text = text.replace('LOG_INFO("[HAP] Transient pair-setup completed successfully");',
                            'ap2_io_status_line("[PROBE] SRP_VERIFIED");\n    LOG_INFO("[HAP] Transient pair-setup completed successfully");')
        text = text.replace('int status = hap_post_pair_setup_path(sock_fd, "/pair-setup", 1, 4,',
                            'ap2_io_status_line("[PROBE] PAIR_M1_SENT");\n    int status = hap_post_pair_setup_path(sock_fd, "/pair-setup", 1, 4,')
        text = text.replace('status = hap_post_pair_setup_path(sock_fd, "/pair-setup", 2, 4,',
                            'ap2_io_status_line("[PROBE] PAIR_M3_SENT");\n    status = hap_post_pair_setup_path(sock_fd, "/pair-setup", 2, 4,')
    if name == 'ap2_hap.h':
        text = text.replace('    int tlv_error;', '    int pair_state; /* actual transient M2/M4 state */\n    int tlv_error;')
        text = text.replace('#endif /*', 'struct ap2_hap_ctx *probe_hap_events_create(struct ap2_hap_ctx *control);\n\n#endif /*', 1)
    if name == 'ap2_ptp.c':
        text = text[:text.index('/* ---- PTP daemon ---- */')]
        text = re.sub(r'\bint (timing_sock|event_sock|general_sock|sock)\b', r'ap2_socket_t \1', text)
        text = text.replace('static int ptp_open_socket(', 'static ap2_socket_t ptp_open_socket(')
        text = text.replace('int s = socket(', 'ap2_socket_t s = socket(')
        # Thread flags are shared; make them atomic rather than relying on volatile.
        text = re.sub(r'\b(?:volatile )?bool (running|ptp_running);', r'atomic_bool \1;', text)
        text = '#include <stdatomic.h>\n' + text
        # NTP shutdown can otherwise race closing a socket with a blocking recv.
        text = text.replace('close(ctx->timing_sock);\n            ctx->timing_sock = -1;',
                            'shutdown(ctx->timing_sock, SHUT_RDWR);\n            close(ctx->timing_sock);\n            ctx->timing_sock = -1;')
    if name == 'ap2_bplist.cpp':
        text = text.replace('../libraop/src/bplist.h', 'bplist.h')
    (out / name).write_text(text, encoding='utf-8')
for name in ['bplist.cpp', 'bplist.h']:
    text = (root / 'libraop/src' / name).read_text(encoding='utf-8')
    text = '#include <cstdint>\n' + text
    text = text.replace('#define be64toh ntohll', '#define be64toh __builtin_bswap64')
    text = text.replace('#define htobe64 htonll', '#define htobe64 __builtin_bswap64')
    (out / name).write_text(text, encoding='utf-8')
# Only the pinned raw 16-bit stereo ALAC escape-frame encoder is needed here.
raw_source = (root / 'src/alac_ext.cpp').read_text(encoding='utf-8').replace('extern "C" ', '')
raw = function('pcm_to_alac_raw', raw_source).replace('std::min(frames, bsize)', '(frames < bsize ? frames : bsize)')
raw = raw.replace('    /* Raw ALAC framing', '    if (!sample || !out || !size || frames <= 0 || frames > bsize || bsize > 352) return false;\n    /* Raw ALAC framing')
raw = raw.replace('    p = *out;', '    if (!*out) return false;\n    p = *out;')
(out / 'raw_alac.c').write_text('/* Copyright (C) 2024-2026 Music Assistant Contributors\n * SPDX-License-Identifier: Apache-2.0\n * Selected from alac_ext.cpp; allocation/argument checks added. */\n#include <stdint.h>\n#include <stdlib.h>\n#include <stdbool.h>\n' + raw, encoding='utf-8')
print('Selected pinned control/realtime source; buffered audio/FIFO/shared-daemon excluded.')

