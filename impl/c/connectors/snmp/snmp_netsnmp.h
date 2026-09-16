/* tedge-dot — the SNMP connector's use of libnetsnmp.
 *
 * Everything that touches net-snmp goes through here, for two reasons:
 *
 *  - net-snmp keeps process-wide state (the USM user table, the local engine
 *    ID and its boots/time, the engine-time cache) that is not safe to use
 *    from several threads, and the runtime runs one thread per config file.
 *    tsnmp_lock() initialises the library exactly once (pthread_once) and
 *    serialises every call. Requests release it while they wait on the
 *    network, so one slow agent does not stall another connector.
 *  - the connector owns the notification socket (it routes by UDP source
 *    before decoding, spec §4.1), so decoding, acknowledging and reporting
 *    work on datagrams: snmp_parse / snmp_build, not a net-snmp transport.
 *
 * Functions marked "lock held" must be called between tsnmp_lock() and
 * tsnmp_unlock(); the others take the lock themselves.
 */
#ifndef TDOT_SNMP_NETSNMP_H
#define TDOT_SNMP_NETSNMP_H

#include <net-snmp/net-snmp-config.h>
#include <net-snmp/net-snmp-includes.h>

#include "snmp_value.h"

#ifdef __cplusplus
extern "C" {
#endif

void tsnmp_lock(void);
void tsnmp_unlock(void);

/* ---- decoding (lock held) ------------------------------------------------ */

/* The message's version field (0, 1, 3, ...), or -1 when it has none. */
long tsnmp_message_version(const uint8_t *buf, size_t len);

/* Decode one datagram with the library (for v3 using the USM users
 * installed). Returns 0, or -1 with *lib_errno the net-snmp error (a USM one
 * for a v3 message that did not authenticate). *pdu is set either way when
 * allocation succeeded -- a failed v3 message still carries what a Report
 * needs -- and the caller frees it with snmp_free_pdu. */
int tsnmp_parse_datagram(const uint8_t *buf, size_t len, netsnmp_pdu **pdu,
                         int *lib_errno);

/* What §4.3 derives from a notification. */
typedef struct {
    netsnmp_pdu *pdu; /* borrowed */
    tsnmp_version_t version;
    tsnmp_pdu_t kind;
    bool has_request_id;
    int64_t request_id;
    bool has_agent_addr; /* v1 */
    uint8_t agent_addr[4];
    bool has_uptime;
    uint32_t uptime;
    tsnmp_oid_t trap;
    size_t nvarbinds;
} tsnmp_notification_t;

/* The connector-owned notification rules of §4.3 on a decoded PDU: the PDU
 * kinds each version may carry, the trap OID and uptime, and the limits.
 * Returns 0, or -1 with err. */
int tsnmp_notification_check(netsnmp_pdu *pdu, tsnmp_notification_t *n,
                             char *err, size_t errlen);

/* A varbind's name as arcs (false when it does not fit 128 32-bit arcs). */
bool tsnmp_var_name(const netsnmp_variable_list *v, tsnmp_oid_t *out);
/* A varbind value's type and canonical content octets (§6): *content points
 * into the variable for strings, into buf (TSNMP_CANON_MAX) otherwise. */
tsnmp_type_t tsnmp_var_content(const netsnmp_variable_list *v, uint8_t *buf,
                               const uint8_t **content, size_t *len);
/* The last snmpTrapAddress.0 IpAddress a forwarder added (§3.1). */
bool tsnmp_trap_address(const netsnmp_pdu *pdu, uint8_t addr[4]);
/* tsnmp_oid_t -> net-snmp oid array (MAX_OID_LEN). Returns the arc count. */
size_t tsnmp_oid_to_net(const tsnmp_oid_t *o, oid *out);

/* ---- encoding (lock held) ------------------------------------------------ */

/* Serialise a PDU into a datagram; *out is malloc'ed. Returns 0, or -1. */
int tsnmp_pdu_to_datagram(netsnmp_pdu *pdu, uint8_t **out, size_t *len);
/* The Response to an InformRequest: same request-id and varbinds, error
 * status and index 0 (for v3 with the security state the inform arrived
 * with). NULL on allocation failure. */
netsnmp_pdu *tsnmp_inform_response(netsnmp_pdu *inform);
/* True for the net-snmp errors that mean a v3 message did not authenticate. */
bool tsnmp_usm_failure(int lib_errno);
/* The USM Report a failed v3 message is answered with (unknown engine ID, not
 * in time window, ...), exactly as net-snmp's own receivers do; NULL when none
 * is due (not a USM error, or not a reportable/confirmed message). */
netsnmp_pdu *tsnmp_report_for(netsnmp_pdu *failed, int lib_errno);

/* ---- SNMPv3 ------------------------------------------------------------- */

typedef struct {
    uint8_t engine[32]; /* msgAuthoritativeEngineID */
    size_t engine_len;
    char user[33];
    uint8_t flags; /* msgFlags: auth 1, priv 2, reportable 4 */
} tsnmp_v3_header_t;

/* Read a v3 message's header without authenticating it. 0, or -1. */
int tsnmp_v3_peek(const uint8_t *buf, size_t len, tsnmp_v3_header_t *h);

typedef enum { TSNMP_AUTH_NONE = 0, TSNMP_AUTH_MD5, TSNMP_AUTH_SHA } tsnmp_auth_t;
typedef enum { TSNMP_PRIV_NONE = 0, TSNMP_PRIV_DES, TSNMP_PRIV_AES } tsnmp_priv_t;

/* A USM user as the connector keeps it: the master keys (Ku), never the
 * passwords. Flat, so it can live in a device's configuration state. */
typedef struct {
    char user[33];
    int level; /* SNMP_SEC_LEVEL_NOAUTH / AUTHNOPRIV / AUTHPRIV */
    tsnmp_auth_t auth;
    tsnmp_priv_t priv;
    uint8_t auth_ku[64];
    size_t auth_ku_len;
    uint8_t priv_ku[64];
    size_t priv_ku_len;
    char context[64];
} tsnmp_v3_creds_t;

/* Derive the master keys from the passwords (lock held). 0, or -1 with err. */
int tsnmp_v3_creds_derive(tsnmp_v3_creds_t *c, const char *auth_password,
                          const char *priv_password, char *err, size_t errlen);
/* Make the USM table hold `c`, localised for `engine` (lock held). A remote
 * engine (a trap sender) also gets a first engine-time entry so the library
 * processes its messages, as snmptrapd's createUser -e does. 0, or -1. */
int tsnmp_usm_install(const tsnmp_v3_creds_t *c, const uint8_t *engine,
                      size_t engine_len, bool remote);

/* This process's engine ID, the authoritative one for received informs. */
size_t tsnmp_local_engine_id(uint8_t *buf, size_t cap); /* lock held */
int tsnmp_set_local_engine_id(const uint8_t *id, size_t len); /* lock held */
/* The §3.1 default: 80000000 05 + the first 8 octets of SHA-256 of
 * /etc/machine-id (the hostname when absent). Writes 13 octets. */
size_t tsnmp_default_engine_id(uint8_t *buf);

/* ---- requests (take the lock themselves) --------------------------------- */

typedef struct {
    const char *peer; /* "udp:192.0.2.1:161", "udp6:[2001:db8::1]:161" */
    tsnmp_version_t version;
    const char *community;
    long timeout_us;
    int retries;
    const tsnmp_v3_creds_t *v3;
    const uint8_t *engine; /* NULL: discovered when the session opens */
    size_t engine_len;
} tsnmp_session_params_t;

/* Open a single-session handle (snmp_sess_open). A v3 session without a
 * known engine ID discovers it here, which blocks for up to the request
 * timeout. NULL with err on failure. */
void *tsnmp_session_open(const tsnmp_session_params_t *p, char *err,
                         size_t errlen);
void tsnmp_session_close(void *sessp);
/* The engine ID a v3 session uses (configured or discovered). */
size_t tsnmp_session_engine(void *sessp, uint8_t *buf, size_t cap);

typedef enum {
    TSNMP_REQ_OK = 0,
    TSNMP_REQ_TIMEOUT,
    TSNMP_REQ_SECURITY, /* a USM Report / authentication failure */
    TSNMP_REQ_SEND,
} tsnmp_req_status_t;

/* Send `req` (consumed) and wait for the response, re-sending per the
 * session's retries. On TSNMP_REQ_OK *resp is the response (snmp_free_pdu
 * it); otherwise err says why. */
tsnmp_req_status_t tsnmp_request(void *sessp, netsnmp_pdu *req,
                                 netsnmp_pdu **resp, char *err, size_t errlen);

#ifdef __cplusplus
}
#endif

#endif /* TDOT_SNMP_NETSNMP_H */
