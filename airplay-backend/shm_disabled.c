#include "ap2_ptp_shm.h"
/* Shared multi-room daemon is explicitly unavailable in the AirPlay backend. */
bool ap2_ptp_shm_reader_open(struct ap2_ptp_shm_reader *r) { (void)r; return false; }
bool ap2_ptp_shm_read(struct ap2_ptp_shm_reader *r, struct ap2_ptp_shm_sample *out) { (void)r; (void)out; return false; }
void ap2_ptp_shm_reader_close(struct ap2_ptp_shm_reader *r) { (void)r; }
bool ap2_ptp_ctrl_send(const char *cmd, int timeout, char *ack, int len) {
    (void)cmd; (void)timeout; if (ack && len > 0) ack[0] = 0; return false;
}

