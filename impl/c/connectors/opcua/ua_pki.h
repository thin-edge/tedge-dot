/* tedge-dot — OPC UA PKI directory and certificate trust (mbedTLS).
 *
 * The C rendering of impl/rust/crates/connector-opcua/src/pki.rs and of the
 * trust decisions tedge-dot patched into async-opcua-crypto
 * (impl/rust/vendor/async-opcua-crypto/src/trust_list.rs): the same Part 12
 * layout, the same trust model (pinned certificates, CA chains through
 * trusted/ and issuers/, a CRL required for every CA), the same file names and
 * the same text forms, so a PKI directory, the reasons and `tedge-dot pki`
 * output are interchangeable between the two builds. Both are checked against
 * the shared vectors of connectors/opcua/conformance/tools/genpki.py.
 */
#ifndef TDOT_UA_PKI_H
#define TDOT_UA_PKI_H

#include <stdbool.h>
#include <stddef.h>
#include <time.h>

#include <open62541/types.h>

#define UA_PKI_DEFAULT_DIR "/var/lib/tedge-dot/opcua/pki"
#define UA_PKI_OWN_CERT "own/certs/cert.der"
#define UA_PKI_OWN_KEY "own/private/key.pem"
#define UA_PKI_DEFAULT_DAYS (5 * 365)
#define UA_PKI_KEY_BITS 2048
#define UA_PKI_EXPIRY_WARNING_DAYS 30
#define UA_PKI_PATH_MAX 1024

typedef enum { UA_PKI_TRUSTED, UA_PKI_ISSUERS, UA_PKI_REJECTED } ua_pki_group_t;

/* A DER blob and the file it came from. */
typedef struct {
    unsigned char *der;
    size_t len;
    char path[UA_PKI_PATH_MAX];
} ua_blob_t;

typedef struct {
    ua_blob_t *items;
    size_t n;
} ua_blobs_t;

/* Everything a validation reads from the PKI directory. */
typedef struct {
    ua_blobs_t trusted, trusted_crls, issuers, issuer_crls, rejected;
} ua_trust_t;

/* One certificate in a group, for listing. */
typedef struct {
    ua_pki_group_t group;
    char path[UA_PKI_PATH_MAX];
    unsigned char *der;
    size_t len;
    char thumbprint[41];
    char subject[512];
    char not_after[32];
    bool is_ca;
    int has_crl; /* -1: not a CA, 0/1 otherwise */
} ua_cert_entry_t;

typedef struct {
    ua_cert_entry_t *items;
    size_t n;
} ua_cert_list_t;

/* Descriptive fields of a certificate (text forms shared with Rust). */
typedef struct {
    char subject[512];
    char issuer[512];
    char common_name[256];
    char thumbprint[41];
    char not_before[32]; /* YYYY-MM-DDTHH:MM:SSZ */
    char not_after[32];
    time_t not_after_t;
    time_t not_before_t;
    char alt_names[16][256]; /* in certificate order: URIs, DNS names, IPs */
    size_t nalt;
    bool is_ca;
    size_t key_bits;
} ua_cert_info_t;

const char *ua_pki_group_name(ua_pki_group_t g);
int ua_pki_group_parse(const char *s, ua_pki_group_t *out);

/* ---- parsing -------------------------------------------------------- */

/* Every certificate (CRL) in a buffer: one DER object, or every PEM block of
 * the matching label. Appends to out; returns how many were found. */
size_t ua_pki_parse_certs(const unsigned char *buf, size_t len, ua_blobs_t *out,
                          const char *path);
size_t ua_pki_parse_crls(const unsigned char *buf, size_t len, ua_blobs_t *out,
                         const char *path);
void ua_blobs_free(ua_blobs_t *b);

int ua_pki_cert_info(const unsigned char *der, size_t len, ua_cert_info_t *out);
void ua_pki_thumbprint(const unsigned char *der, size_t len, char out[41]);
/* "<CN>_<thumbprint>.der" (unsafe characters of the CN replaced by '_') */
void ua_pki_canonical_name(const unsigned char *der, size_t len, char *out,
                           size_t outlen);
/* Whether the DER CRL was issued (named and signed) by the DER CA. */
bool ua_pki_crl_issued_by(const unsigned char *crl, size_t crl_len,
                          const unsigned char *ca, size_t ca_len);
/* Whether the PEM/DER private key belongs to the DER certificate. */
bool ua_pki_key_matches(const unsigned char *cert, size_t cert_len,
                        const unsigned char *key, size_t key_len);

/* ---- trust ---------------------------------------------------------- */

/* Read the trust lists of `root` (missing directories are empty). Files that
 * do not parse are skipped with a warning on stderr. */
void ua_pki_load_trust(const char *root, ua_trust_t *t);
void ua_trust_free(ua_trust_t *t);

/* The trust decision alone (pinning, chain, CRLs, CA validity; rejected/certs
 * does not override trust):
 * UA_STATUSCODE_GOOD or BadCertificateUntrusted / Revoked / IssuerRevoked /
 * RevocationUnknown / IssuerRevocationUnknown / IssuerTimeInvalid. */
UA_StatusCode ua_trust_verify(const ua_trust_t *t, const unsigned char *der,
                              size_t len, time_t now);

/* Full validation of a server certificate, as a session does it: trust (an
 * untrusted certificate is copied to rejected/certs), key length for the
 * policy (min/max bits), validity period, host name (a SAN after the first),
 * application URI (the first SAN). `hostname`/`application_uri` may be NULL
 * to skip those checks. */
UA_StatusCode ua_pki_validate(const char *root, const unsigned char *der,
                              size_t len, size_t min_bits, size_t max_bits,
                              const char *hostname,
                              const char *application_uri);

/* ---- directory ------------------------------------------------------ */

int ua_pki_ensure_layout(const char *root, char *err, size_t errlen);
/* Write through a temporary file and rename(2). */
int ua_pki_write_atomic(const char *path, const unsigned char *data,
                        size_t len, int mode, char *err, size_t errlen);
/* Exclusive flock on <root>/own/.lock; returns the fd or -1. */
int ua_pki_lock_own(const char *root, char *err, size_t errlen);
void ua_pki_unlock(int fd);
/* Store a certificate in a group under its canonical name. */
int ua_pki_store(const char *root, ua_pki_group_t g, const unsigned char *der,
                 size_t len, char *path_out, size_t pathlen, char *err,
                 size_t errlen);
void ua_pki_list(const char *root, ua_pki_group_t g, ua_cert_list_t *out);
void ua_cert_list_free(ua_cert_list_t *l);
/* Delete every file of the entry's group holding the entry's certificate. */
int ua_pki_remove(const char *root, const ua_cert_entry_t *e, char *err,
                  size_t errlen);
/* Store a CRL next to the CA (trusted or issuers) that issued it. */
int ua_pki_add_crl(const char *root, const unsigned char *der, size_t len,
                   char *path_out, size_t pathlen, char *err, size_t errlen);

/* Read a whole file; caller frees. Returns NULL on error (errno set). */
unsigned char *ua_pki_read_file(const char *path, size_t *len);
/* Refuse a key file others may read (group/other permission bits). A missing
 * file is not an error here. */
int ua_pki_check_key_mode(const char *path, const char *field, char *err,
                          size_t errlen);

/* ---- application certificate --------------------------------------- */

typedef struct {
    const char *application_name;
    const char *application_uri;
    const char *const *hostnames; /* DNS names / IPs; NULL or empty: the host name */
    size_t nhostnames;
    int days;
} ua_cert_request_t;

/* Generate a self-signed application instance certificate (RSA 2048,
 * SHA-256, random serial, URI SAN first) as DER plus a PEM key. */
int ua_pki_generate(const ua_cert_request_t *req, unsigned char **cert_der,
                    size_t *cert_len, char **key_pem, char *err,
                    size_t errlen);

/* Load the application certificate from cert_path/key_path; when both are
 * missing and `create`, generate it first (under the own/.lock of `root`).
 * `explicit_paths`: the paths were configured (never generated, no layout
 * created). Checks the key mode, the key/certificate match and that
 * `application_uri` is one of the certificate's SANs. On success the caller
 * owns *cert and *key (UA_ByteString_clear). */
int ua_pki_load_or_create_own(const char *root, const char *cert_path,
                              const char *key_path, bool explicit_paths,
                              bool create, const ua_cert_request_t *req,
                              UA_ByteString *cert, UA_ByteString *key,
                              bool *generated, char *err, size_t errlen);

/* "YYYY-MM-DDTHH:MM:SSZ" */
void ua_pki_format_time(time_t t, char *out, size_t outlen);

#endif /* TDOT_UA_PKI_H */
