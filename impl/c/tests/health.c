/* When an unreadable point is logged (health.h): the same cases as
 * an_unreadable_point_is_logged_when_it_fails_and_when_it_recovers in
 * impl/rust/crates/sdk/src/runtime.rs. Whether a pass is judged at all (not
 * when no point in it was readable) is the runtime's, and is covered there. */
#include <stdio.h>
#include <string.h>

#include "tedge_dot/health.h"

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
    char logged[TDOT_ERR_MAX] = "";

    /* a readable point has nothing to say */
    CHECK(tdot_point_health(logged, false, "") == TDOT_POINT_UNCHANGED, "good");

    /* a point that starts failing is logged once, with its error */
    CHECK(tdot_point_health(logged, true, "illegal data address") == TDOT_POINT_FAILED,
          "first failure");
    CHECK(strcmp(logged, "illegal data address") == 0, "logged '%s'", logged);
    CHECK(tdot_point_health(logged, true, "illegal data address") == TDOT_POINT_UNCHANGED,
          "same failure again");

    /* a different error is logged again */
    CHECK(tdot_point_health(logged, true, "timeout") == TDOT_POINT_FAILED,
          "changed failure");
    CHECK(strcmp(logged, "timeout") == 0, "logged '%s'", logged);

    /* recovery is logged once */
    CHECK(tdot_point_health(logged, false, "") == TDOT_POINT_RECOVERED, "recovery");
    CHECK(logged[0] == '\0', "cleared");
    CHECK(tdot_point_health(logged, false, "") == TDOT_POINT_UNCHANGED, "still good");

    /* a bad sample without an error still says why */
    CHECK(tdot_point_health(logged, true, "") == TDOT_POINT_FAILED, "no error text");
    CHECK(strcmp(logged, "bad quality") == 0, "logged '%s'", logged);
    CHECK(tdot_point_health(logged, true, NULL) == TDOT_POINT_UNCHANGED, "NULL error");

    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    printf("health: ok\n");
    return 0;
}
