#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tedge_dot/log.h"
#include "tedge_dot/runtime.h"

void tdot_log(const char *level, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    tdot_vlog(level, fmt, ap);
    va_end(ap);
}

void tdot_vlog(const char *level, const char *fmt, va_list ap) {
    int saved_errno = errno;
    char ts[40];
    tdot_now_rfc3339(ts, sizeof ts);

    /* The whole line is built first and then written with one call: stderr is unbuffered, so
     * the time, the message and the newline written as separate calls were separate writes,
     * and a line another connector's thread wrote in between ended up inside this one. One
     * stdio call holds the stream's lock throughout, and on an unbuffered stream it is one
     * write. Most lines fit the stack buffer; a longer one is built on the heap rather than
     * cut short. */
    char stack[1024];
    char *line = stack;
    size_t cap = sizeof stack;
    int head = snprintf(stack, cap, "%s %-5s ", ts, level ? level : "");
    if (head < 0 || (size_t)head >= cap - 1) {
        errno = saved_errno;
        return;
    }
    va_list again;
    va_copy(again, ap);
    int body = vsnprintf(stack + head, cap - (size_t)head, fmt, ap);
    if (body < 0) {
        va_end(again);
        errno = saved_errno;
        return;
    }
    size_t len = (size_t)head + (size_t)body;
    if (len + 2 > cap) { /* + newline + NUL */
        char *heap = malloc(len + 2);
        if (heap) {
            memcpy(heap, stack, (size_t)head);
            vsnprintf(heap + head, len + 2 - (size_t)head, fmt, again);
            line = heap;
        } else {
            len = cap - 2; /* out of memory: the line as far as it fits */
        }
    }
    va_end(again);

    line[len] = '\n';
    fwrite(line, 1, len + 1, stderr);
    if (line != stack)
        free(line);
    errno = saved_errno;
}
