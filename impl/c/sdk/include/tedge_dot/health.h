/* tedge-dot C SDK — when an unreadable point is logged.
 *
 * The link status reports a whole device failing. A single point failing on a
 * device that otherwise reads fine (a wrong register, an unknown node id) has
 * its reason only in the sample payload, so the runtime logs it: a warning
 * when the point starts failing or its error changes, a line when it reads
 * again, nothing on the polls in between. The runtime does not judge a read
 * pass in which no point was readable: that is the device failing, which the
 * link reports, and recording it would make every point "recover" afterwards.
 *
 * Mirrors PointHealth in the Rust runtime (impl/rust/crates/sdk/src/runtime.rs).
 */
#ifndef TDOT_HEALTH_H
#define TDOT_HEALTH_H

#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include "model.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    TDOT_POINT_UNCHANGED,
    TDOT_POINT_FAILED,    /* log "point unreadable: <logged>" */
    TDOT_POINT_RECOVERED, /* log "point readable again" */
} tdot_point_change_t;

/* Judge one reading of a point. `logged` is the error last logged for it, ""
 * while it is readable; it is updated in place. */
static inline tdot_point_change_t tdot_point_health(char logged[TDOT_ERR_MAX],
                                                    bool bad,
                                                    const char *error) {
    if (bad) {
        const char *e = error && error[0] ? error : "bad quality";
        if (strncmp(logged, e, TDOT_ERR_MAX - 1) == 0 && logged[0])
            return TDOT_POINT_UNCHANGED;
        snprintf(logged, TDOT_ERR_MAX, "%s", e);
        return TDOT_POINT_FAILED;
    }
    if (!logged[0])
        return TDOT_POINT_UNCHANGED;
    logged[0] = '\0';
    return TDOT_POINT_RECOVERED;
}

#ifdef __cplusplus
}
#endif

#endif
