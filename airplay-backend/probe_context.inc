
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
