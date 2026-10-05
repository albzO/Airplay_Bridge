#include "raw_alac.h"
#include <stdlib.h>
/* This first audio test supports only the upstream raw stereo escape frame. */
struct alac_codec_s { int frames; };
struct alac_codec_s *alac_create_encoder(int frames, int rate, int bits, int channels) {
    if (frames != 352 || rate != 44100 || bits != 16 || channels != 2) return NULL;
    struct alac_codec_s *codec = malloc(sizeof(*codec));
    if (codec) codec->frames = frames;
    return codec;
}
void alac_delete_encoder(struct alac_codec_s *codec) { free(codec); }
bool pcm_to_alac(struct alac_codec_s *codec, uint8_t *samples, int frames, uint8_t **out, int *size) {
    if (!codec) return false;
    return pcm_to_alac_raw(samples, frames, out, size, codec->frames);
}
