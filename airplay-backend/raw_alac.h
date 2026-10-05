#pragma once
#include <stdbool.h>
#include <stdint.h>
struct alac_codec_s;
struct alac_codec_s *alac_create_encoder(int frames, int rate, int bits, int channels);
void alac_delete_encoder(struct alac_codec_s *codec);
bool pcm_to_alac(struct alac_codec_s *codec, uint8_t *samples, int frames, uint8_t **out, int *size);
bool pcm_to_alac_raw(uint8_t *samples, int frames, uint8_t **out, int *size, int block_size);
