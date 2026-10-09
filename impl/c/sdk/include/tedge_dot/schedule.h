/* tedge-dot C SDK — which points a pass of the runtime's main loop visits.
 *
 * The C runtime is one loop per connector. Each pass polls the points that are
 * due, hands over what was pushed, and runs the reporting policy (§5.3). With
 * tens of thousands of points, visiting every point on every pass costs more
 * than the work itself: most points are delivered by push, and most have no
 * policy. So each device keeps two index lists, and the loop visits only those:
 *
 *   polled    readable points not currently delivered by push
 *   reported  points whose effective `report` policy is not passthrough
 *
 * Push state changes at runtime (a subscription is armed on connect and
 * dropped with the transport), so the runtime rebuilds the lists whenever it
 * changes a point's `subscribed` flag. It also derives how long the loop may
 * wait between passes from the fastest pushed point (tdot_schedule_tick_ms).
 *
 * The lists only filter the loop. A device whose lists could not be allocated
 * is walked in full, and the loop's own checks still decide what is done.
 */
#ifndef TDOT_SCHEDULE_H
#define TDOT_SCHEDULE_H

#include <stddef.h>

#include "config.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Bounds of the wait between two passes of the main loop, in milliseconds. */
#define TDOT_TICK_MAX_MS 200
#define TDOT_TICK_MIN_MS 10

/* Rebuild the device's polled and reported lists and its fastest pushed
 * sampling interval from the points' current state. */
void tdot_schedule_rebuild(tdot_device_t *dev);

/* Release the device's lists. */
void tdot_schedule_free(tdot_device_t *dev);

/* How long the main loop may wait between passes: the fastest effective
 * sampling interval among the configuration's pushed points, clamped to
 * [TDOT_TICK_MIN_MS, TDOT_TICK_MAX_MS], and TDOT_TICK_MAX_MS when no point is
 * pushed. Uses each device's value from its last rebuild. */
int tdot_schedule_tick_ms(const tdot_config_t *cfg);

#ifdef __cplusplus
}
#endif

#endif
