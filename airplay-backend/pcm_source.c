#include "windows_port.h"
#include "probe.h"
/* Explicit contract: raw 44100 Hz / S16LE / stereo, finite <= 60 s. */
FILE *probe_pcm_open(const char *path, uint64_t *frames) {
    int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, NULL, 0);
    if (!size) { errno = EINVAL; return NULL; }
    wchar_t *wide = malloc((size_t)size * sizeof(*wide));
    if (!wide) return NULL;
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, wide, size);
    FILE *file = _wfopen(wide, L"rb"); free(wide);
    if (!file) return NULL;
    if (_fseeki64(file, 0, SEEK_END) != 0) { fclose(file); return NULL; }
    int64_t length = _ftelli64(file);
    if (length <= 0 || length % 4 || length > 60LL*44100*4 || _fseeki64(file, 0, SEEK_SET) != 0) {
        fclose(file); errno = EINVAL; return NULL;
    }
    *frames = (uint64_t)length/4; return file;
}
