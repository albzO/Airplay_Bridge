#include "windows_port.h"
#include "cross_log.h"
#include "probe.h"
#include <stdarg.h>

static log_level level = lDEBUG;
log_level *loglevel = &level;
int probe_debug = 1;
static pthread_mutex_t log_lock = PTHREAD_MUTEX_INITIALIZER;
static const char *secret_to_redact;
void probe_set_secret(const char *secret) { secret_to_redact = secret; }
const char *logtime(void) { return "[DEBUG]"; }
void logprint(const char *format, ...) {
    char buffer[8192];
    va_list args; va_start(args, format);
    vsnprintf(buffer, sizeof(buffer), format, args); va_end(args);
    pthread_mutex_lock(&log_lock);
    if (!probe_debug) level = lINFO;
    char *cursor = buffer;
    const char *secret = secret_to_redact;
    while (secret && *secret) {
        char *match = strstr(cursor, secret);
        if (!match) break;
        fwrite(cursor, 1, (size_t)(match - cursor), stderr);
        fputs("[REDACTED]", stderr);
        cursor = match + strlen(secret);
    }
    fputs(cursor, stderr); fflush(stderr);
    pthread_mutex_unlock(&log_lock);
}

