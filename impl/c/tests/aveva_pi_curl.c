/* The libcurl the AVEVA PI connector links (cmake/curl.cmake): TLS on mbedTLS, HTTP, HTTPS
 * and the WebSocket schemes, and none of the protocols or libraries the build turns off. */
#include <stdio.h>
#include <string.h>

#include <curl/curl.h>

static int failures;

#define CHECK(cond, ...)                                                                   \
    do {                                                                                   \
        if (!(cond)) {                                                                     \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);                            \
            fprintf(stderr, __VA_ARGS__);                                                  \
            fputc('\n', stderr);                                                           \
            failures++;                                                                    \
        }                                                                                  \
    } while (0)

static int has_protocol(const curl_version_info_data *v, const char *name) {
    for (const char *const *p = v->protocols; *p; p++)
        if (strcmp(*p, name) == 0)
            return 1;
    return 0;
}

int main(void) {
    curl_global_init(CURL_GLOBAL_DEFAULT);
    const curl_version_info_data *v = curl_version_info(CURLVERSION_NOW);
    CHECK(v->version_num >= 0x080b00, "libcurl %s is older than 8.11", v->version);
    CHECK(v->ssl_version && strstr(v->ssl_version, "mbedTLS"), "TLS backend is %s",
          v->ssl_version ? v->ssl_version : "(none)");
    CHECK(v->features & CURL_VERSION_SSL, "no TLS");
    for (const char *want[] = {"http", "https", "ws", "wss", NULL}, **p = want; *p; p++)
        CHECK(has_protocol(v, *p), "protocol %s missing", *p);
    for (const char *unwanted[] = {"ftp", "file", "ldap", "smtp", "telnet", "dict", NULL},
                    **p = unwanted;
         *p; p++)
        CHECK(!has_protocol(v, *p), "protocol %s should be disabled", *p);
    CHECK(!(v->features & CURL_VERSION_LIBZ), "zlib should be disabled");
    CHECK(!(v->features & CURL_VERSION_HTTP2), "HTTP/2 should be disabled");
    CHECK(!(v->features & CURL_VERSION_IDN), "IDN should be disabled");
    CHECK(!(v->features & CURL_VERSION_PSL), "libpsl should be disabled");
    printf("aveva-pi-curl: libcurl %s, %s\n", v->version, v->ssl_version);
    curl_global_cleanup();
    return failures ? 1 : 0;
}
