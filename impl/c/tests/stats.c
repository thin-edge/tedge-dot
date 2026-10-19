/* The statistics line (stats.h), the same cases as the Rust build's
 * `stats_*` tests in impl/rust/crates/sdk/src/runtime.rs: the interval
 * setting and the line's keys. */
#include <stdio.h>
#include <string.h>

#include "tedge_dot/stats.h"

static int failures = 0;

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            printf("FAIL %s:%d: ", __FILE__, __LINE__);                        \
            printf(__VA_ARGS__);                                               \
            printf("\n");                                                      \
        }                                                                      \
    } while (0)

int main(void) {
    CHECK(tdot_stats_interval(NULL) == 60, "unset -> default");
    CHECK(tdot_stats_interval("15") == 15, "15");
    CHECK(tdot_stats_interval("+15") == 15, "+15");
    CHECK(tdot_stats_interval("0") == 0, "0 -> off");
    CHECK(tdot_stats_interval("soon") == 60, "invalid -> default");
    CHECK(tdot_stats_interval("") == 60, "empty -> default");
    CHECK(tdot_stats_interval("1s") == 60, "a unit is not whole seconds");

    tdot_stats_t st = {.received = 4, .suppressed = 2, .published = 2,
                       .publish_failed = 1, .reconnects = 3};
    char line[256];
    tdot_stats_format(&st, "opcua", line, sizeof line);
    CHECK(!strcmp(line,
                  "{\"connector\":\"opcua\",\"in\":4,\"suppressed\":2,\"published\":2,"
                  "\"publish_failed\":1,\"reconnects\":3}"),
          "line: %s", line);

    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
