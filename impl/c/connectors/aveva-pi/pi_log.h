/* AVEVA PI connector -- log lines in the runtime's format (stderr, one line each). Never pass a
 * secret: not the Authorization header, not a secret file's content (spec §10). */
#ifndef TDOT_PI_LOG_H
#define TDOT_PI_LOG_H

#include <stdarg.h>

#include "tedge_dot/log.h"

/* One whole line per call (log.h): the push workers log from their own threads. */
#if defined(__GNUC__)
__attribute__((format(printf, 2, 3)))
#endif
static inline void pi_log(const char *level, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    tdot_vlog(level, fmt, ap);
    va_end(ap);
}

#endif
