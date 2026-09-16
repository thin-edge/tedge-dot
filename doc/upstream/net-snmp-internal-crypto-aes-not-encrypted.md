# net-snmp: with `--with-openssl=internal`, an AES privacy user sends the scoped PDU in clear

**Project:** [net-snmp](https://github.com/net-snmp/net-snmp) · **Found in:** 5.9.4 ·
**Severity:** silent loss of confidentiality · **Status:** draft, not filed

## What happens

Configured with `--with-openssl=internal` (net-snmp's own copy of the OpenSSL routines USM
needs, so no system OpenSSL is required), a USM user with `usmAESPrivProtocol` produces
**unencrypted** SNMPv3 messages whose `msgFlags` claim privacy. Nothing fails: `sc_encrypt()`
returns `SNMPERR_SUCCESS`.

## Why

`snmplib/scapi.c` implements AES only for a real OpenSSL:

```c
#if defined(NETSNMP_USE_OPENSSL) && defined(HAVE_AES)
    if (USM_CREATE_USER_PRIV_AES == (pai->type & USM_PRIV_MASK_ALG)) { ... }
#endif
```

With the bundled crypto the build defines `NETSNMP_USE_INTERNAL_CRYPTO` (not
`NETSNMP_USE_OPENSSL`) while **`HAVE_AES` is still defined**, so:

- `configure` reports `Crypto support from: internal` and accepts AES users;
- an AES user matches no branch in `sc_encrypt()`/`sc_decrypt()`;
- the function falls through to its exit path, leaves `*ctlen` at the plaintext length and
  returns success, so USM sends the scoped PDU **as plaintext with the privacy flag set**.

DES is unaffected: it has an internal-crypto branch.

## Reproduction

1. `./configure --with-defaults --disable-agent --disable-applications --disable-mibs
   --with-mib-modules="" --without-perl-modules --with-openssl=internal --disable-shared
   --enable-static --with-security-modules=usm && make`
2. Send an authPriv notification with an AES user (`snmptrap -v 3 -l authPriv -a SHA -A … -x AES
   -X … …`) from a binary linked against that library.
3. Capture the datagram: the scoped PDU is readable, and `msgFlags` has the privacy bit set. A
   receiver with the same user rejects it ("error parsing ScopedPDU").

## Suggested fix

Add internal-crypto branches beside the OpenSSL ones in `sc_encrypt()` and `sc_decrypt()`. The
bundled copy already carries AES CFB-128 (`snmplib/openssl/openssl_aes_cfb.c`,
`AES_set_encrypt_key` + `AES_cfb128_encrypt`), so the branch is a few lines — the shape tedge-dot
carries in `impl/c/third_party/net-snmp/tedge-dot.patch` (item 6).

Failing that, `configure` should **refuse** AES privacy when only the internal crypto is
available, so the build fails loudly instead of the library silently not encrypting.

## How tedge-dot is affected

The C implementation links this static, OpenSSL-free build, and patches in the missing branches;
with the patch, v3 authPriv (SHA + AES-128) notifications work end to end against net-snmp and
pysnmp peers. The patch can be dropped once a release carries the fix.
