/* tedge-dot — the SNMP connector's use of libnetsnmp (see snmp_netsnmp.h). */
#include "snmp_netsnmp.h"

#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/select.h>
#include <unistd.h>

/* Exported by snmplib but not declared in its public headers: the same packet
 * builder snmp_sess_send uses (reverse encoding into a growing buffer). */
struct snmp_internal_session;
extern int netsnmp_build_packet(struct snmp_internal_session *isp,
                                netsnmp_session *sp, netsnmp_pdu *pdu,
                                u_char **pktbuf_p, size_t *pktbuf_len_p,
                                u_char **pkt_p, size_t *len_p);

static pthread_once_t tsnmp_once = PTHREAD_ONCE_INIT;
static pthread_mutex_t tsnmp_mutex = PTHREAD_MUTEX_INITIALIZER;
static netsnmp_session tsnmp_parse_session;

static void tsnmp_init(void) {
    /* A library, not an application: no snmp.conf, no persistent state in
     * /var/net-snmp, no RFC 5343 context probes answered behind our back, and
     * no log output (errors reach the connector as return codes). */
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID,
                           NETSNMP_DS_LIB_DONT_READ_CONFIGS, 1);
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID,
                           NETSNMP_DS_LIB_DONT_PERSIST_STATE, 1);
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID,
                           NETSNMP_DS_LIB_DISABLE_PERSISTENT_LOAD, 1);
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID,
                           NETSNMP_DS_LIB_DISABLE_PERSISTENT_SAVE, 1);
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID, NETSNMP_DS_LIB_NO_DISCOVERY,
                           1);
    /* Build into a growing buffer, so a large inform still gets its Response. */
    netsnmp_ds_set_boolean(NETSNMP_DS_LIBRARY_ID,
                           NETSNMP_DS_LIB_REVERSE_ENCODE, 1);
    netsnmp_register_loghandler(NETSNMP_LOGHANDLER_NONE, LOG_DEBUG);
    init_snmp("tedge-dot");
    snmp_sess_init(&tsnmp_parse_session);
}

void tsnmp_lock(void) {
    pthread_once(&tsnmp_once, tsnmp_init);
    pthread_mutex_lock(&tsnmp_mutex);
}

void tsnmp_unlock(void) { pthread_mutex_unlock(&tsnmp_mutex); }

/* ---- decoding ------------------------------------------------------------ */

long tsnmp_message_version(const uint8_t *buf, size_t len) {
    u_char type;
    size_t l = len;
    u_char *p = asn_parse_sequence((u_char *)buf, &l, &type,
                                   ASN_SEQUENCE | ASN_CONSTRUCTOR, "message");
    if (!p)
        return -1;
    long version;
    if (!asn_parse_int(p, &l, &type, &version, sizeof version))
        return -1;
    return version;
}

int tsnmp_parse_datagram(const uint8_t *buf, size_t len, netsnmp_pdu **pdu,
                         int *lib_errno) {
    *lib_errno = 0;
    *pdu = calloc(1, sizeof **pdu);
    if (!*pdu)
        return -1;
    /* snmp_parse takes a writable buffer; it does not keep it. */
    u_char *copy = malloc(len ? len : 1);
    if (!copy)
        return -1;
    memcpy(copy, buf, len);
    tsnmp_parse_session.s_snmp_errno = 0;
    int rc = snmp_parse(NULL, &tsnmp_parse_session, *pdu, copy, len);
    *lib_errno = tsnmp_parse_session.s_snmp_errno;
    free(copy);
    return rc == 0 ? 0 : -1;
}

static const oid SNMP_TRAP_OID_0[] = {1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0};
static const oid SYS_UPTIME_0[] = {1, 3, 6, 1, 2, 1, 1, 3, 0};
static const oid SNMP_TRAP_ADDRESS_0[] = {1, 3, 6, 1, 6, 3, 18, 1, 3, 0};

static bool net_to_oid(const oid *arcs, size_t n, tsnmp_oid_t *out) {
    if (n > TSNMP_MAX_ARCS)
        return false;
    for (size_t i = 0; i < n; i++) {
        if (arcs[i] > 0xFFFFFFFFUL)
            return false;
        out->arcs[i] = (uint32_t)arcs[i];
    }
    out->n = n;
    return true;
}

size_t tsnmp_oid_to_net(const tsnmp_oid_t *o, oid *out) {
    for (size_t i = 0; i < o->n; i++)
        out[i] = o->arcs[i];
    return o->n;
}

bool tsnmp_var_name(const netsnmp_variable_list *v, tsnmp_oid_t *out) {
    return net_to_oid(v->name, v->name_length, out);
}

int tsnmp_notification_check(netsnmp_pdu *pdu, tsnmp_notification_t *n,
                             char *err, size_t errlen) {
    memset(n, 0, sizeof *n);
    n->pdu = pdu;
    switch (pdu->version) {
    case SNMP_VERSION_1:
        n->version = TSNMP_V1;
        break;
    case SNMP_VERSION_2c:
        n->version = TSNMP_V2C;
        break;
    case SNMP_VERSION_3:
        n->version = TSNMP_V3;
        break;
    default:
        snprintf(err, errlen, "unsupported version");
        return -1;
    }

    /* Only these PDUs are notifications, each in its own versions. */
    if (n->version == TSNMP_V1) {
        if (pdu->command != SNMP_MSG_TRAP) {
            snprintf(err, errlen, "not a v1 trap (pdu 0x%x)", pdu->command);
            return -1;
        }
        n->kind = TSNMP_PDU_TRAP;
    } else if (pdu->command == SNMP_MSG_TRAP2) {
        n->kind = TSNMP_PDU_TRAP;
    } else if (pdu->command == SNMP_MSG_INFORM) {
        n->kind = TSNMP_PDU_INFORM;
    } else {
        snprintf(err, errlen, "not a notification (pdu 0x%x)", pdu->command);
        return -1;
    }

    for (netsnmp_variable_list *v = pdu->variables; v; v = v->next_variable)
        n->nvarbinds++;
    if (n->nvarbinds > TSNMP_MAX_VARBINDS) {
        snprintf(err, errlen, "more than %d variable bindings",
                 TSNMP_MAX_VARBINDS);
        return -1;
    }

    if (n->version == TSNMP_V1) {
        /* RFC 3584 §3.1 */
        n->has_agent_addr = true;
        memcpy(n->agent_addr, pdu->agent_addr, 4);
        n->has_uptime = true;
        n->uptime = (uint32_t)pdu->time;
        long generic = pdu->trap_type, specific = pdu->specific_type;
        if (generic >= 0 && generic <= 5) {
            static const uint32_t GENERIC[] = {1, 3, 6, 1, 6, 3, 1, 1, 5};
            memcpy(n->trap.arcs, GENERIC, sizeof GENERIC);
            n->trap.arcs[9] = (uint32_t)generic + 1;
            n->trap.n = 10;
        } else if (generic == 6) {
            if (specific < 0 || (unsigned long)specific > 0xFFFFFFFFUL) {
                snprintf(err, errlen, "specific trap out of range");
                return -1;
            }
            if (pdu->enterprise_length + 2 > TSNMP_MAX_ARCS ||
                !net_to_oid(pdu->enterprise, pdu->enterprise_length, &n->trap)) {
                snprintf(err, errlen, "trap oid longer than %d arcs",
                         TSNMP_MAX_ARCS);
                return -1;
            }
            n->trap.arcs[n->trap.n++] = 0;
            n->trap.arcs[n->trap.n++] = (uint32_t)specific;
        } else {
            snprintf(err, errlen, "generic trap out of range");
            return -1;
        }
        return 0;
    }

    n->has_request_id = true;
    n->request_id = pdu->reqid;
    bool have_trap = false, have_uptime = false;
    for (netsnmp_variable_list *v = pdu->variables; v; v = v->next_variable) {
        if (!have_trap &&
            netsnmp_oid_equals(v->name, v->name_length, SNMP_TRAP_OID_0,
                               OID_LENGTH(SNMP_TRAP_OID_0)) == 0) {
            have_trap = true;
            if (v->type != ASN_OBJECT_ID ||
                !net_to_oid(v->val.objid, v->val_len / sizeof(oid), &n->trap)) {
                snprintf(err, errlen, "no snmpTrapOID.0 (its value is not an oid)");
                return -1;
            }
        } else if (!have_uptime &&
                   netsnmp_oid_equals(v->name, v->name_length, SYS_UPTIME_0,
                                      OID_LENGTH(SYS_UPTIME_0)) == 0) {
            have_uptime = true;
            if (v->type == ASN_TIMETICKS) {
                n->has_uptime = true;
                n->uptime = (uint32_t)(unsigned long)*v->val.integer;
            }
        }
    }
    if (!have_trap) {
        snprintf(err, errlen, "no snmpTrapOID.0");
        return -1;
    }
    return 0;
}

tsnmp_type_t tsnmp_var_content(const netsnmp_variable_list *v, uint8_t *buf,
                               const uint8_t **content, size_t *len) {
    *content = buf;
    *len = 0;
    switch (v->type) {
    case ASN_INTEGER:
        *len = tsnmp_encode_integer((int64_t)*v->val.integer, buf);
        return TSNMP_TYPE_INTEGER;
    case ASN_COUNTER:
    case ASN_GAUGE:
    case ASN_TIMETICKS:
    case ASN_UINTEGER: {
        uint64_t u = (uint64_t)(unsigned long)*v->val.integer & 0xFFFFFFFFULL;
        *len = tsnmp_encode_unsigned(u, buf);
        return v->type == ASN_COUNTER     ? TSNMP_TYPE_COUNTER32
               : v->type == ASN_GAUGE     ? TSNMP_TYPE_GAUGE32
               : v->type == ASN_TIMETICKS ? TSNMP_TYPE_TIMETICKS
                                          : TSNMP_TYPE_UNKNOWN;
    }
    case ASN_COUNTER64: {
        uint64_t u = ((uint64_t)(v->val.counter64->high & 0xFFFFFFFFUL) << 32) |
                     (uint64_t)(v->val.counter64->low & 0xFFFFFFFFUL);
        *len = tsnmp_encode_unsigned(u, buf);
        return TSNMP_TYPE_COUNTER64;
    }
    case ASN_OCTET_STR:
    case ASN_IPADDRESS:
    case ASN_OPAQUE:
        *content = v->val.string;
        *len = v->val_len;
        return v->type == ASN_OCTET_STR   ? TSNMP_TYPE_OCTET_STRING
               : v->type == ASN_IPADDRESS ? TSNMP_TYPE_IP_ADDRESS
                                          : TSNMP_TYPE_OPAQUE;
    case ASN_OBJECT_ID: {
        tsnmp_oid_t o;
        if (net_to_oid(v->val.objid, v->val_len / sizeof(oid), &o))
            *len = tsnmp_oid_encode(&o, buf, TSNMP_CANON_MAX);
        return TSNMP_TYPE_OID;
    }
    case ASN_NULL:
        return TSNMP_TYPE_NULL;
    case SNMP_NOSUCHOBJECT:
        return TSNMP_TYPE_NO_SUCH_OBJECT;
    case SNMP_NOSUCHINSTANCE:
        return TSNMP_TYPE_NO_SUCH_INSTANCE;
    case SNMP_ENDOFMIBVIEW:
        return TSNMP_TYPE_END_OF_MIB_VIEW;
    default:
        return TSNMP_TYPE_UNKNOWN;
    }
}

bool tsnmp_trap_address(const netsnmp_pdu *pdu, uint8_t addr[4]) {
    bool found = false;
    for (netsnmp_variable_list *v = pdu->variables; v; v = v->next_variable)
        if (netsnmp_oid_equals(v->name, v->name_length, SNMP_TRAP_ADDRESS_0,
                               OID_LENGTH(SNMP_TRAP_ADDRESS_0)) == 0 &&
            v->type == ASN_IPADDRESS && v->val_len == 4) {
            memcpy(addr, v->val.string, 4); /* the last one wins */
            found = true;
        }
    return found;
}

/* ---- encoding ------------------------------------------------------------ */

int tsnmp_pdu_to_datagram(netsnmp_pdu *pdu, uint8_t **out, size_t *len) {
    netsnmp_session s;
    snmp_sess_init(&s);
    s.version = pdu->version;
    s.community = pdu->community;
    s.community_len = pdu->community_len;
    if (pdu->msgMaxSize == 0)
        pdu->msgMaxSize = 65507;
    size_t buflen = 2048;
    u_char *buf = malloc(buflen);
    if (!buf)
        return -1;
    u_char *pkt = NULL;
    size_t pktlen = 0;
    if (netsnmp_build_packet(NULL, &s, pdu, &buf, &buflen, &pkt, &pktlen) != 0 ||
        !pkt) {
        free(buf);
        return -1;
    }
    *out = malloc(pktlen ? pktlen : 1);
    if (!*out) {
        free(buf);
        return -1;
    }
    memcpy(*out, pkt, pktlen);
    *len = pktlen;
    free(buf);
    return 0;
}

netsnmp_pdu *tsnmp_inform_response(netsnmp_pdu *inform) {
    netsnmp_pdu *r = snmp_clone_pdu(inform);
    if (!r)
        return NULL;
    r->command = SNMP_MSG_RESPONSE;
    r->errstat = 0;
    r->errindex = 0;
    r->flags &= ~UCD_MSG_FLAG_EXPECT_RESPONSE;
    r->flags |= UCD_MSG_FLAG_RESPONSE_PDU;
    return r;
}

static bool usm_reportable(int e) {
    return e == SNMPERR_USM_UNKNOWNENGINEID ||
           e == SNMPERR_USM_UNKNOWNSECURITYNAME ||
           e == SNMPERR_USM_UNSUPPORTEDSECURITYLEVEL ||
           e == SNMPERR_USM_AUTHENTICATIONFAILURE ||
           e == SNMPERR_USM_NOTINTIMEWINDOW || e == SNMPERR_USM_DECRYPTIONERROR;
}

bool tsnmp_usm_failure(int e) {
    return usm_reportable(e) || e == SNMPERR_USM_GENERICERROR ||
           e == SNMPERR_USM_ENCRYPTIONERROR || e == SNMPERR_USM_PARSEERROR;
}

netsnmp_pdu *tsnmp_report_for(netsnmp_pdu *failed, int lib_errno) {
    if (!failed || failed->version != SNMP_VERSION_3 ||
        !usm_reportable(lib_errno))
        return NULL;
    /* as net-snmp's usm_handle_report: a confirmed PDU, or one whose scoped
     * PDU could not be read but that asked for a report */
    if (!(SNMP_CMD_CONFIRMED(failed->command) ||
          (failed->command == 0 && (failed->flags & SNMP_MSG_FLAG_RPRT_BIT))))
        return NULL;
    int flags = failed->flags;
    failed->flags |= UCD_MSG_FLAG_FORCE_PDU_COPY;
    netsnmp_pdu *r = snmp_clone_pdu(failed);
    failed->flags = flags;
    if (!r)
        return NULL;
    r->flags = flags & ~UCD_MSG_FLAG_EXPECT_RESPONSE;
    if (snmpv3_make_report(r, lib_errno) != SNMPERR_SUCCESS) {
        snmp_free_pdu(r);
        return NULL;
    }
    return r;
}

/* ---- SNMPv3 ------------------------------------------------------------- */

int tsnmp_v3_peek(const uint8_t *buf, size_t len, tsnmp_v3_header_t *h) {
    memset(h, 0, sizeof *h);
    u_char type;
    size_t left = len;
    u_char *c = asn_parse_sequence((u_char *)buf, &left, &type,
                                   ASN_SEQUENCE | ASN_CONSTRUCTOR, "message");
    if (!c)
        return -1;
    long x;
    if (!(c = asn_parse_int(c, &left, &type, &x, sizeof x)) || x != 3)
        return -1;

    /* msgGlobalData: msgID, msgMaxSize, msgFlags, msgSecurityModel */
    size_t glen = left;
    u_char *g = asn_parse_sequence(c, &glen, &type,
                                   ASN_SEQUENCE | ASN_CONSTRUCTOR, "global");
    if (!g || (size_t)(g - c) + glen > left)
        return -1;
    size_t gl = glen;
    u_char flags[4];
    size_t fl = sizeof flags;
    u_char *q = asn_parse_int(g, &gl, &type, &x, sizeof x);
    if (!q || !(q = asn_parse_int(q, &gl, &type, &x, sizeof x)) ||
        !(q = asn_parse_string(q, &gl, &type, flags, &fl)) || fl != 1)
        return -1;
    h->flags = flags[0];
    left -= (size_t)(g + glen - c);
    c = g + glen;

    /* msgSecurityParameters: an OCTET STRING holding the USM SEQUENCE */
    u_char sec[1024];
    size_t sl = sizeof sec;
    if (!asn_parse_string(c, &left, &type, sec, &sl))
        return -1;
    size_t ul = sl;
    u_char *s = asn_parse_sequence(sec, &ul, &type,
                                   ASN_SEQUENCE | ASN_CONSTRUCTOR, "usm");
    size_t el = sizeof h->engine;
    size_t nl = sizeof h->user - 1;
    if (!s || !(s = asn_parse_string(s, &ul, &type, h->engine, &el)) ||
        !(s = asn_parse_int(s, &ul, &type, &x, sizeof x)) ||
        !(s = asn_parse_int(s, &ul, &type, &x, sizeof x)) ||
        !(s = asn_parse_string(s, &ul, &type, (u_char *)h->user, &nl)))
        return -1;
    h->engine_len = el;
    h->user[nl] = '\0';
    return 0;
}

static const oid *auth_oid(tsnmp_auth_t a) {
    return a == TSNMP_AUTH_MD5 ? usmHMACMD5AuthProtocol
           : a == TSNMP_AUTH_SHA ? usmHMACSHA1AuthProtocol
                                 : usmNoAuthProtocol;
}

static const oid *priv_oid(tsnmp_priv_t p) {
    return p == TSNMP_PRIV_DES ? usmDESPrivProtocol
           : p == TSNMP_PRIV_AES ? usmAESPrivProtocol
                                 : usmNoPrivProtocol;
}

#define TSNMP_PROTO_LEN 10 /* every usm*Protocol OID above */

int tsnmp_v3_creds_derive(tsnmp_v3_creds_t *c, const char *auth_password,
                          const char *priv_password, char *err,
                          size_t errlen) {
    c->auth_ku_len = c->priv_ku_len = 0;
    if (c->auth != TSNMP_AUTH_NONE) {
        c->auth_ku_len = sizeof c->auth_ku;
        if (generate_Ku(auth_oid(c->auth), TSNMP_PROTO_LEN,
                        (const u_char *)auth_password, strlen(auth_password),
                        c->auth_ku, &c->auth_ku_len) != SNMPERR_SUCCESS) {
            snprintf(err, errlen, "cannot derive the authentication key");
            return -1;
        }
    }
    if (c->priv != TSNMP_PRIV_NONE) {
        /* RFC 3414: the privacy key is derived with the auth hash */
        c->priv_ku_len = sizeof c->priv_ku;
        if (generate_Ku(auth_oid(c->auth), TSNMP_PROTO_LEN,
                        (const u_char *)priv_password, strlen(priv_password),
                        c->priv_ku, &c->priv_ku_len) != SNMPERR_SUCCESS) {
            snprintf(err, errlen, "cannot derive the privacy key");
            return -1;
        }
    }
    return 0;
}

int tsnmp_usm_install(const tsnmp_v3_creds_t *c, const uint8_t *engine,
                      size_t engine_len, bool remote) {
    u_char authkey[64], privkey[64];
    size_t alen = 0, plen = 0;
    if (c->auth != TSNMP_AUTH_NONE) {
        alen = sizeof authkey;
        if (generate_kul(auth_oid(c->auth), TSNMP_PROTO_LEN, engine, engine_len,
                         c->auth_ku, c->auth_ku_len, authkey, &alen) !=
            SNMPERR_SUCCESS)
            return -1;
    }
    if (c->priv != TSNMP_PRIV_NONE) {
        plen = sizeof privkey;
        if (generate_kul(auth_oid(c->auth), TSNMP_PROTO_LEN, engine, engine_len,
                         c->priv_ku, c->priv_ku_len, privkey, &plen) !=
            SNMPERR_SUCCESS)
            return -1;
    }

    struct usmUser *u = usm_get_user(engine, engine_len, c->user);
    if (u && u->authKeyLen == alen && u->privKeyLen == plen &&
        (alen == 0 || memcmp(u->authKey, authkey, alen) == 0) &&
        (plen == 0 || memcmp(u->privKey, privkey, plen) == 0) &&
        snmp_oid_compare(u->authProtocol, u->authProtocolLen, auth_oid(c->auth),
                         TSNMP_PROTO_LEN) == 0 &&
        snmp_oid_compare(u->privProtocol, u->privProtocolLen, priv_oid(c->priv),
                         TSNMP_PROTO_LEN) == 0)
        goto time; /* already exactly this user */
    if (u) {
        /* another device's credentials under the same name and engine: the
         * table holds one user per (engine, name), so swap them */
        usm_remove_user(u);
        usm_free_user(u);
    }

    u = usm_create_user();
    if (!u)
        return -1;
    u->engineID = netsnmp_memdup(engine, engine_len);
    u->engineIDLen = engine_len;
    u->name = strdup(c->user);
    u->secName = strdup(c->user);
    SNMP_FREE(u->authProtocol);
    u->authProtocol = snmp_duplicate_objid(auth_oid(c->auth), TSNMP_PROTO_LEN);
    u->authProtocolLen = TSNMP_PROTO_LEN;
    SNMP_FREE(u->privProtocol);
    u->privProtocol = snmp_duplicate_objid(priv_oid(c->priv), TSNMP_PROTO_LEN);
    u->privProtocolLen = TSNMP_PROTO_LEN;
    if (alen) {
        u->authKey = netsnmp_memdup(authkey, alen);
        u->authKeyLen = alen;
    }
    if (plen) {
        u->privKey = netsnmp_memdup(privkey, plen);
        u->privKeyLen = plen;
    }
    u->userStatus = RS_ACTIVE;
    u->userStorageType = ST_READONLY;
    if (!u->engineID || !u->name || !u->secName || !u->authProtocol ||
        !u->privProtocol || (alen && !u->authKey) || (plen && !u->privKey)) {
        usm_free_user(u);
        return -1;
    }
    usm_add_user(u);

time:
    if (remote) {
        u_int boots, now;
        if (get_enginetime(engine, (u_int)engine_len, &boots, &now, FALSE) !=
            SNMPERR_SUCCESS)
            set_enginetime(engine, (u_int)engine_len, 0, 0, FALSE);
    }
    return 0;
}

size_t tsnmp_local_engine_id(uint8_t *buf, size_t cap) {
    return snmpv3_get_engineID(buf, cap);
}

int tsnmp_set_local_engine_id(const uint8_t *id, size_t len) {
    return set_exact_engineID(id, len) == SNMPERR_SUCCESS ? 0 : -1;
}

void tsnmp_set_local_engine_boots(const uint8_t *id, size_t len,
                                  uint32_t boots) {
    /* snmpEngineBoots for the authoritative role. It has to be set explicitly:
     * usm_check_and_update_timeliness compares an inform's boots against
     * snmpv3_local_snmpEngineBoots() and refuses any mismatch with
     * usmStatsNotInTimeWindows, and set_exact_engineID -- unlike the engine ID
     * change inside snmpv3_store -- sets neither the counter nor the LCD entry.
     * Left alone it stays 0, which RFC 3414 §2.2 reserves for an uninitialised
     * engine: a sender then refuses to adopt the boots/time our discovery
     * Report offers, sends 0/0, is refused, re-synchronises and repeats -- an
     * inform that never completes rather than one that fails.
     *
     * engineBoots_conf is the only exported way in (the setter beside it is
     * behind NETSNMP_ENABLE_TESTING_CODE and is not built), and it stores
     * `atoi(cptr) + 1`, hence the -1 here. The LCD entry is then stamped for
     * our own engine exactly as snmpv3_store does. */
    char text[16];
    snprintf(text, sizeof text, "%u", boots > 0 ? boots - 1 : 0);
    engineBoots_conf("engineBoots", text);
    set_enginetime(id, (u_int)len, snmpv3_local_snmpEngineBoots(),
                   snmpv3_local_snmpEngineTime(), TRUE);
}

/* ---- SHA-256 (FIPS 180-4), only for the default engine ID ---------------- */

typedef struct {
    uint32_t h[8];
    uint8_t block[64];
    size_t fill;
    uint64_t bits;
} sha256_t;

static uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

static void sha256_compress(sha256_t *s) {
    static const uint32_t K[64] = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
        0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};
    uint32_t w[64];
    for (int i = 0; i < 16; i++)
        w[i] = (uint32_t)s->block[4 * i] << 24 | (uint32_t)s->block[4 * i + 1] << 16 |
               (uint32_t)s->block[4 * i + 2] << 8 | (uint32_t)s->block[4 * i + 3];
    for (int i = 16; i < 64; i++) {
        uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    uint32_t a = s->h[0], b = s->h[1], c = s->h[2], d = s->h[3], e = s->h[4],
             f = s->h[5], g = s->h[6], hh = s->h[7];
    for (int i = 0; i < 64; i++) {
        uint32_t t1 = hh + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) +
                      ((e & f) ^ (~e & g)) + K[i] + w[i];
        uint32_t t2 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) +
                      ((a & b) ^ (a & c) ^ (b & c));
        hh = g;
        g = f;
        f = e;
        e = d + t1;
        d = c;
        c = b;
        b = a;
        a = t1 + t2;
    }
    s->h[0] += a; s->h[1] += b; s->h[2] += c; s->h[3] += d;
    s->h[4] += e; s->h[5] += f; s->h[6] += g; s->h[7] += hh;
}

static void sha256(const uint8_t *data, size_t len, uint8_t out[32]) {
    sha256_t s = {.h = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19}};
    s.bits = (uint64_t)len * 8;
    for (size_t i = 0; i < len; i++) {
        s.block[s.fill++] = data[i];
        if (s.fill == 64) {
            sha256_compress(&s);
            s.fill = 0;
        }
    }
    s.block[s.fill++] = 0x80;
    if (s.fill > 56) {
        while (s.fill < 64)
            s.block[s.fill++] = 0;
        sha256_compress(&s);
        s.fill = 0;
    }
    while (s.fill < 56)
        s.block[s.fill++] = 0;
    for (int i = 7; i >= 0; i--)
        s.block[s.fill++] = (uint8_t)(s.bits >> (8 * i));
    sha256_compress(&s);
    for (int i = 0; i < 8; i++)
        for (int k = 0; k < 4; k++)
            out[4 * i + k] = (uint8_t)(s.h[i] >> (24 - 8 * k));
}

size_t tsnmp_default_engine_id(uint8_t *buf) {
    char seed[256] = "";
    FILE *fp = fopen("/etc/machine-id", "r");
    if (fp) {
        if (!fgets(seed, sizeof seed, fp))
            seed[0] = '\0';
        fclose(fp);
        seed[strcspn(seed, "\r\n")] = '\0';
    }
    if (seed[0] == '\0' && gethostname(seed, sizeof seed - 1) != 0)
        snprintf(seed, sizeof seed, "tedge-dot");
    seed[sizeof seed - 1] = '\0';
    uint8_t digest[32];
    sha256((const uint8_t *)seed, strlen(seed), digest);
    static const uint8_t PREFIX[5] = {0x80, 0x00, 0x00, 0x00, 0x05};
    memcpy(buf, PREFIX, sizeof PREFIX);
    memcpy(buf + 5, digest, 8);
    return 13;
}

/* ---- requests ------------------------------------------------------------ */

void *tsnmp_session_open(const tsnmp_session_params_t *p, char *err,
                         size_t errlen) {
    netsnmp_session s;
    tsnmp_lock();
    snmp_sess_init(&s);
    s.peername = (char *)p->peer;
    s.version = p->version == TSNMP_V1    ? SNMP_VERSION_1
                : p->version == TSNMP_V2C ? SNMP_VERSION_2c
                                          : SNMP_VERSION_3;
    s.timeout = p->timeout_us;
    s.retries = p->retries;
    if (p->version != TSNMP_V3) {
        s.community = (u_char *)p->community;
        s.community_len = strlen(p->community);
    } else {
        const tsnmp_v3_creds_t *c = p->v3;
        s.securityModel = SNMP_SEC_MODEL_USM;
        s.securityName = (char *)c->user;
        s.securityNameLen = strlen(c->user);
        s.securityLevel = c->level;
        if (c->auth != TSNMP_AUTH_NONE) {
            s.securityAuthProto = (oid *)auth_oid(c->auth);
            s.securityAuthProtoLen = TSNMP_PROTO_LEN;
            memcpy(s.securityAuthKey, c->auth_ku, c->auth_ku_len);
            s.securityAuthKeyLen = c->auth_ku_len;
        }
        if (c->priv != TSNMP_PRIV_NONE) {
            s.securityPrivProto = (oid *)priv_oid(c->priv);
            s.securityPrivProtoLen = TSNMP_PROTO_LEN;
            memcpy(s.securityPrivKey, c->priv_ku, c->priv_ku_len);
            s.securityPrivKeyLen = c->priv_ku_len;
        }
        if (c->context[0]) {
            s.contextName = (char *)c->context;
            s.contextNameLen = strlen(c->context);
        }
        if (p->engine && p->engine_len) {
            s.securityEngineID = (u_char *)p->engine;
            s.securityEngineIDLen = p->engine_len;
        }
    }
    void *sessp = snmp_sess_open(&s);
    if (!sessp)
        snprintf(err, errlen, "%s", snmp_api_errstring(s.s_snmp_errno));
    if (sessp && p->version == TSNMP_V3) {
        /* Give the agent's engine a first entry in the engine-time cache, as
         * net-snmp does for a user read from its configuration ("so that it's a
         * known engineID and won't return a report PDU"). Engine discovery
         * learns the ID from an unauthenticated Report, which carries boots and
         * time 0; without an entry, the authenticated notInTimeWindow Report
         * that carries the real values is dropped for want of a previous value,
         * and every request then stays outside the agent's time window. */
        netsnmp_session *ss = snmp_sess_session(sessp);
        if (ss && ss->securityEngineIDLen) {
            u_int boots = 0, when = 0;
            if (get_enginetime(ss->securityEngineID,
                               (u_int)ss->securityEngineIDLen, &boots, &when,
                               FALSE) != SNMPERR_SUCCESS)
                set_enginetime(ss->securityEngineID,
                               (u_int)ss->securityEngineIDLen, 0, 0, FALSE);
        }
    }
    tsnmp_unlock();
    return sessp;
}

void tsnmp_session_close(void *sessp) {
    if (!sessp)
        return;
    tsnmp_lock();
    snmp_sess_close(sessp);
    tsnmp_unlock();
}

size_t tsnmp_session_engine(void *sessp, uint8_t *buf, size_t cap) {
    size_t n = 0;
    tsnmp_lock();
    netsnmp_session *ss = snmp_sess_session(sessp);
    if (ss && ss->securityEngineIDLen && ss->securityEngineIDLen <= cap) {
        memcpy(buf, ss->securityEngineID, ss->securityEngineIDLen);
        n = ss->securityEngineIDLen;
    }
    tsnmp_unlock();
    return n;
}

typedef struct {
    bool done;
    tsnmp_req_status_t status;
    netsnmp_pdu *resp;
    bool time_sync; /* a notInTimeWindow Report arrived: worth one more try */
    char why[120];
} request_state_t;

static int request_callback(int op, netsnmp_session *session, int reqid,
                            netsnmp_pdu *pdu, void *magic) {
    (void)session;
    (void)reqid;
    request_state_t *st = magic;
    if (st->done)
        return 1;
    switch (op) {
    case NETSNMP_CALLBACK_OP_RECEIVED_MESSAGE:
        if (pdu->command == SNMP_MSG_REPORT) {
            int type = snmpv3_get_report_type(pdu);
            /* notInTimeWindow: the library re-sends, and the engine's boots and
             * time are now known, so a fresh request would be in the window */
            if (type == SNMPERR_NOT_IN_TIME_WINDOW) {
                st->time_sync = true;
                return 1;
            }
            st->status = TSNMP_REQ_SECURITY;
            snprintf(st->why, sizeof st->why, "%s", snmp_api_errstring(type));
        } else {
            st->resp = snmp_clone_pdu(pdu);
            st->status = st->resp ? TSNMP_REQ_OK : TSNMP_REQ_SEND;
            if (!st->resp)
                snprintf(st->why, sizeof st->why, "out of memory");
        }
        st->done = true;
        break;
    case NETSNMP_CALLBACK_OP_TIMED_OUT:
        st->status = TSNMP_REQ_TIMEOUT;
        snprintf(st->why, sizeof st->why, "request timed out");
        st->done = true;
        break;
    case NETSNMP_CALLBACK_OP_SEC_ERROR: {
        /* the library leaves s_snmp_errno unset on some of these */
        const char *text = session ? snmp_api_errstring(session->s_snmp_errno)
                                   : NULL;
        if (!text || !*text)
            text = "wrong user, password or engine";
        st->status = TSNMP_REQ_SECURITY;
        snprintf(st->why, sizeof st->why, "%s", text);
        st->done = true;
        break;
    }
    case NETSNMP_CALLBACK_OP_SEND_FAILED:
        st->status = TSNMP_REQ_SEND;
        snprintf(st->why, sizeof st->why, "send failed");
        st->done = true;
        break;
    default:
        break;
    }
    return 1;
}

/* One send and wait. `req` is consumed. */
static tsnmp_req_status_t request_once(void *sessp, netsnmp_pdu *req,
                                       netsnmp_pdu **resp, bool *time_sync,
                                       char *err, size_t errlen) {
    request_state_t st = {.status = TSNMP_REQ_SEND};
    *resp = NULL;
    tsnmp_lock();
    if (snmp_sess_async_send(sessp, req, request_callback, &st) == 0) {
        int clib = 0, lib = 0;
        char *text = NULL;
        snmp_sess_error(sessp, &clib, &lib, &text);
        snprintf(err, errlen, "send failed: %s", text ? text : "unknown error");
        free(text);
        snmp_free_pdu(req);
        tsnmp_unlock();
        return TSNMP_REQ_SEND;
    }
    while (!st.done) {
        int nfds = 0, block = 1;
        fd_set fds;
        FD_ZERO(&fds);
        struct timeval tv = {0, 0};
        snmp_sess_select_info(sessp, &nfds, &fds, &tv, &block);
        if (block) { /* nothing pending: should not happen, but never hang */
            tv.tv_sec = 1;
            tv.tv_usec = 0;
        }
        /* wait on the network without the lock */
        tsnmp_unlock();
        int n = select(nfds, &fds, NULL, NULL, &tv);
        int e = errno;
        tsnmp_lock();
        if (n > 0) {
            snmp_sess_read(sessp, &fds);
        } else if (n == 0) {
            snmp_sess_timeout(sessp);
        } else if (e != EINTR) {
            st.status = TSNMP_REQ_SEND;
            snprintf(st.why, sizeof st.why, "select: %s", strerror(e));
            st.done = true;
        }
    }
    tsnmp_unlock();
    *resp = st.resp;
    *time_sync = st.time_sync;
    if (st.status != TSNMP_REQ_OK)
        snprintf(err, errlen, "%s", st.why);
    return st.status;
}

tsnmp_req_status_t tsnmp_request(void *sessp, netsnmp_pdu *req,
                                 netsnmp_pdu **resp, char *err, size_t errlen) {
    /* A v3 agent answers the first authenticated request of a session with a
     * notInTimeWindow Report carrying its engine boots and time (RFC 3414
     * §3.2). The library re-sends from the request it already built, so the
     * second attempt can still carry the old values; one request built afresh
     * afterwards is in the window. Spec §5.1 allows the re-sends anyway. */
    netsnmp_pdu *spare = NULL;
    if (req->version == SNMP_VERSION_3) {
        tsnmp_lock();
        spare = snmp_clone_pdu(req);
        tsnmp_unlock();
    }
    bool time_sync = false;
    tsnmp_req_status_t status =
        request_once(sessp, req, resp, &time_sync, err, errlen);
    if (status == TSNMP_REQ_SECURITY && time_sync && spare) {
        status = request_once(sessp, spare, resp, &time_sync, err, errlen);
        spare = NULL;
    }
    if (spare) {
        tsnmp_lock();
        snmp_free_pdu(spare);
        tsnmp_unlock();
    }
    return status;
}
