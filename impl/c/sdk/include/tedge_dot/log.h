/* tedge-dot C SDK — log lines.
 *
 * `tedge-dot run` runs every connector in one process, each on its own threads, and all of them
 * log to stderr (the service's journal). A line is "<RFC 3339 time> <level> <message>", and it
 * is written with ONE call, so a line from another thread can never land inside it.
 */
#ifndef TDOT_LOG_H
#define TDOT_LOG_H

#include <stdarg.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Write one line to stderr: the current time, `level` padded to five columns ("info", "warn",
 * "error", "debug"), the formatted message and a newline. The message is never cut short.
 * errno is left as it was. */
#if defined(__GNUC__)
__attribute__((format(printf, 2, 3)))
#endif
void tdot_log(const char *level, const char *fmt, ...);

/* The same with a va_list, for a module's own log helper. */
#if defined(__GNUC__)
__attribute__((format(printf, 2, 0)))
#endif
void tdot_vlog(const char *level, const char *fmt, va_list ap);

#ifdef __cplusplus
}
#endif

#endif /* TDOT_LOG_H */
