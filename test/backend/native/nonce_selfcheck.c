/* 音频 nonce 的边界检查，供后端 --self-check 入口调用。
 * Audio nonce boundary checks, called by the backend --self-check entry. */
#include "probe.h"

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
