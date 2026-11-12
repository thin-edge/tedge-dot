/* Log lines are written whole (log.h): several threads log at once into stderr, as the
 * connectors of one `tedge-dot run` process do, and every line that comes out is complete
 * ("<time> <level> <message>") and arrives exactly once. Writing the time, the message and the
 * newline as separate calls let the lines of different threads end up inside each other.
 * Every 97th line is longer than tdot_vlog's stack buffer, so the heap path is covered too, and
 * no line may be cut short. */
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tedge_dot/log.h"

#define THREADS 8
#define LINES 2000
#define LONG_EVERY 97
#define LONG_LEN 3000
#define MAX_REPORTED 20

static int failures = 0;

/* Only the first few failures are printed: interleaved output fails thousands of lines. */
#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            if (failures++ < MAX_REPORTED) {                                   \
                printf("FAIL %s:%d: ", __FILE__, __LINE__);                    \
                printf(__VA_ARGS__);                                           \
                printf("\n");                                                  \
            }                                                                  \
        }                                                                      \
    } while (0)

static char filler[LONG_LEN + 1];
static const char *const levels[] = {"info", "warn", "error"};

static int line_len(int i) { return i % LONG_EVERY == 0 ? LONG_LEN : 20 + i % 40; }

static void *writer(void *arg) {
    int t = (int)(intptr_t)arg;
    for (int i = 0; i < LINES; i++) {
        int n = line_len(i);
        tdot_log(levels[i % 3], "thread %d line %d len %d %.*s", t, i, n, n, filler);
    }
    return NULL;
}

/* tdot_now_rfc3339's shape: 2026-11-12T09:10:07.585Z */
static int is_timestamp(const char *s) {
    static const char shape[] = "dddd-dd-ddTdd:dd:dd.dddZ";
    for (size_t k = 0; k < sizeof shape - 1; k++) {
        if (shape[k] == 'd' ? (s[k] < '0' || s[k] > '9') : s[k] != shape[k])
            return 0;
    }
    return 1;
}

int main(void) {
    memset(filler, 'x', LONG_LEN);

    char path[] = "/tmp/tedge-dot-log-XXXXXX";
    int fd = mkstemp(path);
    if (fd < 0) {
        perror("mkstemp");
        return 1;
    }
    /* stderr stays the unbuffered stream it is in the service; only its descriptor moves. */
    fflush(stderr);
    int saved = dup(STDERR_FILENO);
    if (saved < 0 || dup2(fd, STDERR_FILENO) < 0) {
        perror("dup2");
        return 1;
    }

    pthread_t threads[THREADS];
    for (intptr_t t = 0; t < THREADS; t++)
        pthread_create(&threads[t], NULL, writer, (void *)t);
    for (int t = 0; t < THREADS; t++)
        pthread_join(threads[t], NULL);

    fflush(stderr);
    dup2(saved, STDERR_FILENO);
    close(saved);
    close(fd);

    FILE *in = fopen(path, "r");
    if (!in) {
        perror("fopen");
        return 1;
    }
    static unsigned char seen[THREADS][LINES];
    char *line = NULL;
    size_t cap = 0;
    ssize_t got;
    size_t count = 0;
    while ((got = getline(&line, &cap, in)) > 0) {
        count++;
        if (line[got - 1] == '\n')
            line[--got] = '\0';
        else
            CHECK(0, "line %zu has no newline", count);

        /* "<24-char time> <level padded to 5> <message>" */
        if (got < 31 || !is_timestamp(line) || line[24] != ' ' || line[30] != ' ') {
            CHECK(0, "line %zu is not \"<time> <level> <message>\": %.80s", count, line);
            continue;
        }
        int t, i, n, off = 0;
        if (sscanf(line + 31, "thread %d line %d len %d %n", &t, &i, &n, &off) != 3 ||
            off == 0 || t < 0 || t >= THREADS || i < 0 || i >= LINES) {
            CHECK(0, "line %zu has a message no writer sent: %.80s", count, line + 31);
            continue;
        }
        char level[6], want[6];
        memcpy(level, line + 25, 5);
        level[5] = '\0';
        snprintf(want, sizeof want, "%-5s", levels[i % 3]);
        CHECK(strcmp(level, want) == 0, "thread %d line %d: level '%s', want '%s'", t, i,
              level, want);

        const char *body = line + 31 + off;
        size_t len = strlen(body);
        CHECK(n == line_len(i) && len == (size_t)n && strspn(body, "x") == len,
              "thread %d line %d: %zu filler bytes, want %d", t, i, len, line_len(i));
        CHECK(!seen[t][i], "thread %d line %d appears twice", t, i);
        seen[t][i] = 1;
    }
    free(line);
    fclose(in);
    unlink(path);

    CHECK(count == (size_t)THREADS * LINES, "%zu lines, want %d", count, THREADS * LINES);
    int missing = 0;
    for (int t = 0; t < THREADS; t++)
        for (int i = 0; i < LINES; i++)
            missing += !seen[t][i];
    CHECK(missing == 0, "%d lines missing or unreadable", missing);

    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    printf("ok: %d lines from %d threads, each whole and once\n", THREADS * LINES, THREADS);
    return 0;
}
