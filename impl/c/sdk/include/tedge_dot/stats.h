#ifndef TEDGE_DOT_STATS_H
#define TEDGE_DOT_STATS_H

/* The counters behind the periodic statistics line, the C side of `Stats` in
 * impl/rust/crates/sdk/src/runtime.rs. Plain integers owned by the runtime
 * loop that increments them; the loop logs them every
 * TEDGE_DOT_STATS_INTERVAL seconds (default 60, 0 = off). */

#include <stddef.h>
#include <stdint.h>

#define TDOT_STATS_INTERVAL_ENV "TEDGE_DOT_STATS_INTERVAL"
#define TDOT_STATS_DEFAULT_S 60

typedef struct {
    uint64_t received;       /* readings offered to the reporting policy */
    uint64_t suppressed;     /* of those, dropped or held back by it (a held
                              * one counts again as published when released) */
    uint64_t published;      /* samples the MQTT client took */
    uint64_t publish_failed; /* samples it refused or that were dropped */
    uint64_t reconnects;     /* broker sessions re-established */
} tdot_stats_t;

/* The interval for the environment value `value` (NULL = unset), in whole
 * seconds; 0 when the line is off. Anything but digits gives the default. */
unsigned tdot_stats_interval(const char *value);

/* The statistics as one JSON object, the same keys as the Rust build:
 * {"connector":..,"in":..,"suppressed":..,"published":..,"publish_failed":..,
 * "reconnects":..}. */
void tdot_stats_format(const tdot_stats_t *st, const char *connector, char *out,
                       size_t len);

#endif
