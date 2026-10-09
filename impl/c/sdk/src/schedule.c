#include "tedge_dot/schedule.h"

#include <stdlib.h>

#include "tedge_dot/report.h"

void tdot_schedule_rebuild(tdot_device_t *dev) {
    /* Point lists never change size while a configuration runs, so the arrays
     * are allocated once at full size and refilled in place. */
    if (!dev->polled && dev->npoints)
        dev->polled = malloc(dev->npoints * sizeof *dev->polled);
    if (!dev->reported && dev->npoints)
        dev->reported = malloc(dev->npoints * sizeof *dev->reported);

    dev->npolled = dev->nreported = dev->npushed = 0;
    dev->fastest_push_s = -1;
    for (size_t j = 0; j < dev->npoints; j++) {
        const tdot_point_t *pt = &dev->points[j];
        if (pt->subscribed) {
            dev->npushed++;
            if (dev->fastest_push_s < 0 || pt->sampling_interval_s < dev->fastest_push_s)
                dev->fastest_push_s = pt->sampling_interval_s;
        } else if ((pt->access & TDOT_ACCESS_READ) && dev->polled) {
            dev->polled[dev->npolled++] = j;
        }
        if (!tdot_report_is_passthrough(&pt->report) && dev->reported)
            dev->reported[dev->nreported++] = j;
    }
}

void tdot_schedule_free(tdot_device_t *dev) {
    free(dev->polled);
    free(dev->reported);
    dev->polled = dev->reported = NULL;
    dev->npolled = dev->nreported = dev->npushed = 0;
    dev->fastest_push_s = -1;
}

int tdot_schedule_tick_ms(const tdot_config_t *cfg) {
    double fastest = -1;
    for (size_t i = 0; i < cfg->ndevices; i++) {
        const tdot_device_t *dev = &cfg->devices[i];
        if (dev->npushed > 0 && (fastest < 0 || dev->fastest_push_s < fastest))
            fastest = dev->fastest_push_s;
    }
    if (fastest < 0)
        return TDOT_TICK_MAX_MS;
    double ms = fastest * 1000.0;
    if (ms < TDOT_TICK_MIN_MS)
        return TDOT_TICK_MIN_MS;
    if (ms > TDOT_TICK_MAX_MS)
        return TDOT_TICK_MAX_MS;
    return (int)ms;
}
