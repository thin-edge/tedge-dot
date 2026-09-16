#!/usr/bin/env python3
"""Independent reference decoder for the SNMPv3 golden vectors (spec §4, §8).

Implements RFC 3414's User-based Security Model a third time — key localization, the
authentication digest and the privacy decryption — in Python, so the Rust and C decoders are
checked against something neither of them produced. The symmetric ciphers come from the
`openssl` CLI (AES-128-CFB, DES-CBC), which is neither implementation's crypto either.

Used by genvectors.py; the datagrams come from v3captures.txt (see capture-v3.sh).
"""
import hashlib
import hmac
import subprocess

U32 = 0xFFFFFFFF

# The credential sets v3captures.txt refers to. Test fixtures, published on purpose: the whole
# point is that the vectors can be re-derived from the passwords.
USERS = {
    "sha-aes": {
        "user": "trapuser",
        "auth_protocol": "SHA",
        "auth_password": "authpassword",
        "priv_protocol": "AES",
        "priv_password": "privpassword",
    },
    "sha": {"user": "trapuser", "auth_protocol": "SHA", "auth_password": "authpassword"},
    "md5-des": {
        "user": "trapuser",
        "auth_protocol": "MD5",
        "auth_password": "authpassword",
        "priv_protocol": "DES",
        "priv_password": "privpassword",
    },
    "none": {"user": "trapuser"},
}

# This receiver's engine, for the discovery probe: what genvectors.py writes into `v3.receiver`
# and the tests build their LocalEngine from.
RECEIVER = {"engine_id": "800000000574656467652d646f74", "engine_boots": 7, "engine_time": 99}

HASHES = {"MD5": hashlib.md5, "SHA": hashlib.sha1, "SHA1": hashlib.sha1, "SHA256": hashlib.sha256}
TRUNCATION = {"MD5": 12, "SHA": 12, "SHA1": 12, "SHA256": 24}
PRIV_KEY_LEN = {"DES": 16, "AES": 16, "AES128": 16, "AES256": 32}

USM_STATS = {
    1: "1.3.6.1.6.3.15.1.1.1.0",  # unsupportedSecLevels
    2: "1.3.6.1.6.3.15.1.1.2.0",  # notInTimeWindows
    3: "1.3.6.1.6.3.15.1.1.3.0",  # unknownUserNames
    4: "1.3.6.1.6.3.15.1.1.4.0",  # unknownEngineIDs
    5: "1.3.6.1.6.3.15.1.1.5.0",  # wrongDigests
    6: "1.3.6.1.6.3.15.1.1.6.0",  # decryptionErrors
}


class V3Error(Exception):
    pass


# ---- USM keys (RFC 3414 §A.2) ----------------------------------------------------------------

def password_to_key(password, engine_id, protocol):
    """Ku over a megabyte of the repeated password, localized to the engine: Kul = H(Ku|ID|Ku)."""
    h = HASHES[protocol]()
    password = password.encode()
    buf = (password * (1048576 // len(password) + 1))[:1048576]
    h.update(buf)
    ku = h.digest()
    return HASHES[protocol](ku + engine_id + ku).digest()


def auth_key(user, engine_id):
    return password_to_key(user["auth_password"], engine_id, user["auth_protocol"])


def priv_key(user, engine_id):
    key = password_to_key(user["priv_password"], engine_id, user["auth_protocol"])
    need = PRIV_KEY_LEN[user["priv_protocol"].upper()]
    while len(key) < need:  # Blumenthal extension, for AES-192/256 with a short digest
        key += HASHES[user["auth_protocol"]](key).digest()
    return key[:need]


def openssl(cipher, key, iv, data):
    """Decrypt with the openssl CLI: a third implementation of the cipher."""
    argv = ["openssl", "enc", "-d", "-" + cipher, "-K", key.hex(), "-iv", iv.hex(), "-nopad"]
    if cipher == "des-cbc":  # OpenSSL 3 keeps DES in the legacy provider
        argv += ["-provider", "legacy", "-provider", "default"]
    done = subprocess.run(argv, input=data, capture_output=True)
    if done.returncode != 0:
        raise V3Error(f"openssl {cipher} failed: {done.stderr.decode().strip()}")
    return done.stdout


def decrypt(user, engine_id, boots, time, priv_params, encrypted):
    key = priv_key(user, engine_id)
    protocol = user["priv_protocol"].upper()
    if protocol == "DES":
        if len(priv_params) != 8:
            raise V3Error("DES privacy parameters are eight octets")
        iv = bytes(a ^ b for a, b in zip(key[8:16], priv_params))
        return openssl("des-cbc", key[:8], iv, encrypted)
    if len(priv_params) != 8:
        raise V3Error("AES privacy parameters are eight octets")
    iv = boots.to_bytes(4, "big") + time.to_bytes(4, "big") + priv_params
    bits = len(key) * 8
    return openssl(f"aes-{bits}-cfb", key, iv, encrypted)


# ---- message decoding -------------------------------------------------------------------------

def parse(ber, datagram):
    """The unauthenticated layers: msgGlobalData, the USM parameters and the message data."""
    s, e = ber.expect(datagram, 0, len(datagram), 0x30)
    vs, ve = ber.expect(datagram, s, e, 0x02)
    if ber.dec_int(datagram[vs:ve]) != 3:
        raise V3Error("not an SNMPv3 message")
    gs, ge = ber.expect(datagram, ve, e, 0x30)
    ms, me = ber.expect(datagram, gs, ge, 0x02)
    msg_id = ber.dec_int(datagram[ms:me])
    xs, xe = ber.expect(datagram, me, ge, 0x02)
    max_size = ber.dec_int(datagram[xs:xe])
    fs, fe = ber.expect(datagram, xe, ge, 0x04)
    if fe - fs != 1:
        raise V3Error("msgFlags is one octet")
    flags = datagram[fs]
    ss, se = ber.expect(datagram, fe, ge, 0x02)
    if ber.dec_int(datagram[ss:se]) != 3:
        raise V3Error("not the User-based Security Model")

    ps, pe = ber.expect(datagram, ge, e, 0x04)  # msgSecurityParameters
    us, ue = ber.expect(datagram, ps, pe, 0x30)
    es, ee = ber.expect(datagram, us, ue, 0x04)
    engine_id = datagram[es:ee]
    bs, be = ber.expect(datagram, ee, ue, 0x02)
    boots = ber.dec_int(datagram[bs:be])
    ts, te = ber.expect(datagram, be, ue, 0x02)
    time = ber.dec_int(datagram[ts:te])
    ns, ne = ber.expect(datagram, te, ue, 0x04)
    user_name = datagram[ns:ne]
    as_, ae = ber.expect(datagram, ne, ue, 0x04)
    auth_params, auth_at = datagram[as_:ae], as_
    vs2, ve2 = ber.expect(datagram, ae, ue, 0x04)
    priv_params = datagram[vs2:ve2]

    if flags & 0x02:  # privacy
        ds, de = ber.expect(datagram, pe, e, 0x04)
    else:
        ds, de = ber.expect(datagram, pe, e, 0x30)
    return {
        "msg_id": msg_id,
        "max_size": max_size,
        "flags": flags,
        "engine_id": engine_id,
        "engine_boots": boots,
        "engine_time": time,
        "user_name": user_name,
        "auth_params": auth_params,
        "auth_at": auth_at,
        "priv_params": priv_params,
        "data": datagram[ds:de],
        "message": datagram[:e],
        "level": "authPriv" if flags & 0x02 else ("authNoPriv" if flags & 0x01 else "noAuthNoPriv"),
    }


def verify(header, user):
    """msgAuthenticationParameters: the HMAC of the message with them zeroed."""
    protocol = user["auth_protocol"]
    trunc = TRUNCATION[protocol]
    if len(header["auth_params"]) != trunc:
        raise V3Error("the authentication parameters have the wrong length")
    signed = bytearray(header["message"])
    signed[header["auth_at"]:header["auth_at"] + trunc] = bytes(trunc)
    key = auth_key(user, header["engine_id"])
    digest = hmac.new(key, bytes(signed), HASHES[protocol]).digest()[:trunc]
    if not hmac.compare_digest(digest, header["auth_params"]):
        raise V3Error("the authentication digest does not match")


def scoped(ber, header, user):
    """The scoped PDU: decrypted when the message is private."""
    data = header["data"]
    if header["flags"] & 0x02:
        plaintext = decrypt(
            user,
            header["engine_id"],
            header["engine_boots"],
            header["engine_time"],
            header["priv_params"],
            data,
        )
        s, e = ber.expect(plaintext, 0, len(plaintext), 0x30)
        return plaintext, s, e
    return data, 0, len(data)


def decode(ber, datagram, user):
    """One v3 message, fully: the USM checks and then the notification semantics of §4.3."""
    header = parse(ber, datagram)
    if user.get("auth_protocol"):
        if not header["flags"] & 0x01:
            raise V3Error("an authenticated user, an unauthenticated message")
        verify(header, user)
    elif header["flags"] & 0x01:
        raise V3Error("the user has no keys for an authenticated message")
    if bool(header["flags"] & 0x02) != bool(user.get("priv_protocol")):
        raise V3Error("the message's privacy does not match the user's")
    if header["user_name"].decode() != user["user"]:
        raise V3Error("another user's message")

    buf, s, e = scoped(ber, header, user)
    cs, ce = ber.expect(buf, s, e, 0x04)
    context_engine_id = buf[cs:ce]
    ns, ne = ber.expect(buf, ce, e, 0x04)
    context_name = buf[ns:ne]
    out = ber.decode_pdu(buf, ne, e, version=3, community=header["user_name"])
    out["engine_id"] = header["engine_id"].hex()
    out["engine_boots"] = header["engine_boots"]
    out["engine_time"] = header["engine_time"]
    out["level"] = header["level"]
    out["context_engine_id"] = context_engine_id.hex()
    out["context_name"] = context_name.decode()
    out["msg_id"] = str(header["msg_id"])
    return out


def probe_expectation(ber, datagram):
    """What the Report answering a discovery probe must carry (RFC 3412 §7.1, RFC 3414 §3.2)."""
    header = parse(ber, datagram)
    if header["engine_id"]:
        raise V3Error("a discovery probe carries no engine ID")
    if not header["flags"] & 0x04:
        raise V3Error("a discovery probe is reportable")
    buf, s, e = scoped(ber, header, USERS["none"])
    cs, ce = ber.expect(buf, s, e, 0x04)
    ns, ne = ber.expect(buf, ce, e, 0x04)
    tag, ps, pe = ber.element(buf, ne, e)
    rs, re_ = ber.expect(buf, ps, pe, 0x02)
    return {
        "msg_id": str(header["msg_id"]),
        "request_id": str(ber.dec_int(buf[rs:re_])),
        "counter": USM_STATS[4],
    }


def build(ber, captures_path):
    """The `v3` section of trap-vectors.json, from the captures and the credentials above."""
    messages, rejected, probes = [], [], []
    for line in open(captures_path).read().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        hexdata, kind, credentials, name = (part.strip() for part in line.split("|", 3))
        datagram = bytes.fromhex(hexdata)
        user = USERS[credentials]
        if kind == "message":
            expect = decode(ber, datagram, user)
            messages.append({"name": name, "hex": hexdata, "user": credentials, "expect": expect})
        elif kind == "rejected":
            try:
                decode(ber, datagram, user)
            except V3Error as e:
                rejected.append({"name": name, "hex": hexdata, "user": credentials, "why": str(e)})
                continue
            raise SystemExit(f"v3 vector '{name}' was accepted, but is listed as rejected")
        elif kind == "probe":
            probes.append({"name": name, "hex": hexdata, "expect": probe_expectation(ber, datagram)})
        else:
            raise SystemExit(f"unknown kind '{kind}' in {captures_path}")

    return {
        "about": (
            "SNMPv3 vectors (spec §4, §8). `users` holds the credential sets the captures were "
            "sent with — a set names the user, the authentication and privacy protocols and the "
            "passwords, from which the reader localizes the keys itself (RFC 3414 §A.2). "
            "`messages` decode with their set's keys and carry the same expectation as the v1/v2c "
            "ones plus the v3 header (engine ID, security level, context); `rejected` must be "
            "refused; `probes` are engine-ID discovery probes, each answered with the Report "
            "`expect.counter` names, carrying `expect.msg_id` and `expect.request_id`, from an "
            "engine built with `receiver`. The reference decoder is Python + the openssl CLI."
        ),
        "users": USERS,
        "receiver": RECEIVER,
        "messages": messages,
        "rejected": rejected,
        "probes": probes,
    }
