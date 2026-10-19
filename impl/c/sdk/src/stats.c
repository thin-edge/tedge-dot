#include "tedge_dot/stats.h"

#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

#include "cjson/cJSON.h"

unsigned tdot_stats_interval(const char *value) {
    if (!value)
        return TDOT_STATS_DEFAULT_S;
    const char *p = *value == '+' ? value + 1 : value;
    bool digits = *p != '\0';
    for (const char *c = p; *c && digits; c++)
        digits = *c >= '0' && *c <= '9';
    if (!digits)
        return TDOT_STATS_DEFAULT_S;
    unsigned long v = strtoul(p, NULL, 10);
    return v > 86400UL * 365 ? 86400U * 365 : (unsigned)v;
}

void tdot_stats_format(const tdot_stats_t *st, const char *connector, char *out,
                       size_t len) {
    cJSON *obj = cJSON_CreateObject();
    cJSON_AddStringToObject(obj, "connector", connector);
    cJSON_AddNumberToObject(obj, "in", (double)st->received);
    cJSON_AddNumberToObject(obj, "suppressed", (double)st->suppressed);
    cJSON_AddNumberToObject(obj, "published", (double)st->published);
    cJSON_AddNumberToObject(obj, "publish_failed", (double)st->publish_failed);
    cJSON_AddNumberToObject(obj, "reconnects", (double)st->reconnects);
    char *json = cJSON_PrintUnformatted(obj);
    snprintf(out, len, "%s", json ? json : "{}");
    free(json);
    cJSON_Delete(obj);
}
