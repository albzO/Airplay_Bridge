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
