#include "probe.h"
#include <string.h>
/* The RTP sequence wraps at 16 bits. AEAD nonces must never wrap with it.
   AP2 carries all eight counter bytes in the packet suffix; retransmission
   reuses the original encrypted packet without consuming a new nonce. */
bool probe_audio_nonce(uint64_t *counter, uint8_t nonce[12]) {
    if (*counter == UINT64_MAX) return false;
    memset(nonce, 0, 12);
    uint64_t value = (*counter)++;
    for (int i=0; i<8; i++) nonce[4+i]=(uint8_t)(value>>(i*8));
    return true;
}
bool probe_nonce_selfcheck(void) {
    uint64_t counter=0;
    uint8_t nonce[12];
    for (uint64_t packet=0; packet<65538; packet++) {
        if (!probe_audio_nonce(&counter, nonce)) return false;
        uint64_t decoded=0;
        for (int i=0; i<4; i++) if (nonce[i]) return false;
        for (int i=0; i<8; i++) decoded |= (uint64_t)nonce[4+i]<<(8*i);
        if (decoded!=packet) return false;
    }
    counter=UINT64_MAX;
    return !probe_audio_nonce(&counter, nonce);
}
