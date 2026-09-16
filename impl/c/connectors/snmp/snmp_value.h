/* tedge-dot — SNMP value handling that net-snmp does not provide
 * (doc/connectors/snmp-connector-spec.md §3.3, §6).
 *
 * Decoding a message is libnetsnmp's job (snmp_netsnmp.c). What stays here is
 * the connector's own part: OID text in configuration, subtree matching, the
 * canonical content octets published as `raw`, strict UTF-8, and converting a
 * value to a point datatype. Independent of net-snmp, so it is unit-testable on
 * its own. Everything is prefixed tsnmp_ to stay clear of net-snmp's snmp_*.
 */
#ifndef TDOT_SNMP_VALUE_H
#define TDOT_SNMP_VALUE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "tedge_dot/model.h"

#ifdef __cplusplus
extern "C" {
#endif

#define TSNMP_MAX_ARCS 128     /* §4.3: at most 128 arcs per OID */
#define TSNMP_MAX_VARBINDS 256 /* §4.3: at most 256 variable bindings */
/* Dotted text of the longest OID: 128 arcs of up to 10 digits plus dots. */
#define TSNMP_OID_STR_MAX (TSNMP_MAX_ARCS * 11 + 1)
/* Canonical content octets of any non-string value (a 128-arc OID). */
#define TSNMP_CANON_MAX (TSNMP_MAX_ARCS * 5 + 16)

typedef struct {
    uint32_t arcs[TSNMP_MAX_ARCS];
    size_t n;
} tsnmp_oid_t;

/* Value types; tsnmp_type_name gives the vectors' spelling. */
typedef enum {
    TSNMP_TYPE_INTEGER = 0,
    TSNMP_TYPE_OCTET_STRING,
    TSNMP_TYPE_NULL,
    TSNMP_TYPE_OID,
    TSNMP_TYPE_IP_ADDRESS,
    TSNMP_TYPE_COUNTER32,
    TSNMP_TYPE_GAUGE32, /* also Unsigned32: the same tag */
    TSNMP_TYPE_TIMETICKS,
    TSNMP_TYPE_OPAQUE,
    TSNMP_TYPE_COUNTER64,
    TSNMP_TYPE_NO_SUCH_OBJECT,
    TSNMP_TYPE_NO_SUCH_INSTANCE,
    TSNMP_TYPE_END_OF_MIB_VIEW,
    TSNMP_TYPE_UNKNOWN,
} tsnmp_type_t;

const char *tsnmp_type_name(tsnmp_type_t type);
/* Returns 0 and fills *type, or -1 when the name is not one of the above. */
int tsnmp_type_parse(const char *name, tsnmp_type_t *type);
/* The SET types of point.address.type (§3.3): "integer", "unsigned32",
 * "gauge32", "counter32", "counter64", "timeticks", "octet_string",
 * "ip_address", "oid". Returns 0, or -1. */
int tsnmp_set_type_parse(const char *name, tsnmp_type_t *type);
bool tsnmp_type_is_exception(tsnmp_type_t type);
/* "noSuchObject", "noSuchInstance", "endOfMibView" (the SMI spelling). */
const char *tsnmp_exception_name(tsnmp_type_t type);

typedef enum { TSNMP_V1 = 0, TSNMP_V2C, TSNMP_V3 } tsnmp_version_t;
typedef enum { TSNMP_PDU_TRAP = 0, TSNMP_PDU_INFORM } tsnmp_pdu_t;

const char *tsnmp_version_name(tsnmp_version_t v); /* "v1", "v2c", "v3" */

/* OID content octets -> arcs. Returns 0, or -1 with err. */
int tsnmp_oid_decode(const uint8_t *content, size_t len, tsnmp_oid_t *out,
                     char *err, size_t errlen);
/* Canonical (minimal) BER content octets of an OID. Writes at most cap bytes
 * and returns the full length, so a short buffer truncates. */
size_t tsnmp_oid_encode(const tsnmp_oid_t *oid, uint8_t *dst, size_t cap);
/* Configuration syntax (§3.3): dotted decimal, a leading '.' accepted, 2-128
 * arcs, first 0-2, second below 40 unless the first is 2 (then at most
 * 2^32-1-80, so the first subidentifier fits 32 bits), every arc 32-bit.
 * Returns 0, or -1 with err. */
int tsnmp_oid_parse(const char *text, tsnmp_oid_t *out, char *err,
                    size_t errlen);
/* Dotted decimal, snprintf semantics (returns the untruncated length). */
size_t tsnmp_oid_format(const tsnmp_oid_t *oid, char *dst, size_t cap);
bool tsnmp_oid_equal(const tsnmp_oid_t *a, const tsnmp_oid_t *b);
/* True when `oid` equals `prefix` or lies beneath it. */
bool tsnmp_oid_under(const tsnmp_oid_t *oid, const tsnmp_oid_t *prefix);

/* Canonical content octets (§6): minimal two's complement for INTEGER, the
 * minimal unsigned encoding (a leading 00 when the top bit is set) for the
 * unsigned types. Return the length; dst holds 8 / 9 bytes. */
size_t tsnmp_encode_integer(int64_t v, uint8_t *dst);
size_t tsnmp_encode_unsigned(uint64_t v, uint8_t *dst);

/* Content rules, also used by conversion. Return 0, or -1. */
int tsnmp_integer_decode(const uint8_t *content, size_t len, int64_t *out);
int tsnmp_unsigned_decode(const uint8_t *content, size_t len,
                          size_t max_octets, uint64_t max, uint64_t *out);

/* Strict UTF-8 (§6): no overlong forms, no surrogates, nothing above U+10FFFF. */
bool tsnmp_utf8_valid(const uint8_t *s, size_t len);

/* Convert one value, given as its type and content octets, to a typed point
 * datatype (§6): 0 with *out filled, or -1 with err saying why the sample is
 * bad. A string longer than out->str holds is truncated at a UTF-8 character
 * boundary. The transform is not applied here. */
int tsnmp_convert(tsnmp_type_t type, const uint8_t *content, size_t len,
                  tdot_datatype_t dt, tdot_value_t *out, char *err,
                  size_t errlen);

/* The SNMP error-status names (RFC 3416), e.g. "notWritable". */
const char *tsnmp_error_status_name(long status);

#ifdef __cplusplus
}
#endif

#endif /* TDOT_SNMP_VALUE_H */
