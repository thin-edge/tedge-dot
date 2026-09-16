#!/usr/bin/env python3
"""Independent reference decoder for the SNMP trap golden vectors.

Implements doc/connectors/snmp-connector-spec.md §4 (message decoding) and §5 (conversions)
a third time, in Python, so the Rust and C decoders are checked against something neither of
them produced.

    genvectors.py captures.txt connectors/snmp/conformance/trap-vectors.json

captures.txt holds one `<hex datagram> # <name>` per line, captured from net-snmp; the SNMPv3
datagrams live in v3captures.txt beside it and are decoded by v3vectors.py (this file's §4.3
semantics on top of its own USM implementation).
"""
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import v3vectors  # noqa: E402  (after sys.path, so the sibling module is importable)

U32 = 0xFFFFFFFF
U64 = 0xFFFFFFFFFFFFFFFF
SAFE = 2**53 - 1
MAX_ARCS = 128
MAX_VARBINDS = 256
SYSUPTIME = "1.3.6.1.2.1.1.3.0"
TRAPOID = "1.3.6.1.6.3.1.1.4.1.0"


class DecodeError(Exception):
    pass


# ---- decoding (spec §4) ---------------------------------------------------------------------

def element(buf, pos, end):
    if pos >= end:
        raise DecodeError("truncated")
    tag = buf[pos]
    if tag & 0x1F == 0x1F:
        raise DecodeError("multi-octet tag")
    pos += 1
    if pos >= end:
        raise DecodeError("truncated")
    first = buf[pos]
    pos += 1
    if first < 0x80:
        length = first
    elif first == 0x80:
        raise DecodeError("indefinite length")
    else:
        n = first & 0x7F
        if n > 4:
            raise DecodeError("length too long")
        if pos + n > end:
            raise DecodeError("truncated")
        length = int.from_bytes(buf[pos:pos + n], "big")
        pos += n
    if length > end - pos:
        raise DecodeError("truncated")
    return tag, pos, pos + length


def expect(buf, pos, end, tag):
    t, s, e = element(buf, pos, end)
    if t != tag:
        raise DecodeError(f"expected tag {tag:#x}, got {t:#x}")
    return s, e


def dec_int(c):
    if not 1 <= len(c) <= 8:
        raise DecodeError("integer length")
    return int.from_bytes(c, "big", signed=True)


def dec_unsigned(c, max_len, limit):
    if not 1 <= len(c) <= max_len:
        raise DecodeError("unsigned length")
    v = int.from_bytes(c, "big")
    if v > limit:
        raise DecodeError("unsigned overflow")
    return v


def dec_oid(c):
    if not c:
        raise DecodeError("empty oid")
    subids = []
    v = 0
    cont = False
    for b in c:
        v = (v << 7) | (b & 0x7F)
        if v > U32:
            raise DecodeError("oid arc overflow")
        cont = bool(b & 0x80)
        if not cont:
            subids.append(v)
            v = 0
    if cont:
        raise DecodeError("truncated oid")
    first = subids[0]
    if first < 40:
        arcs = [0, first]
    elif first < 80:
        arcs = [1, first - 40]
    else:
        arcs = [2, first - 80]
    arcs += subids[1:]
    if len(arcs) > MAX_ARCS:
        raise DecodeError("too many arcs")
    return arcs


def dotted(arcs):
    return ".".join(str(a) for a in arcs)


TYPES = {
    0x02: "integer", 0x04: "octet_string", 0x05: "null", 0x06: "oid",
    0x40: "ip_address", 0x41: "counter32", 0x42: "gauge32", 0x43: "timeticks",
    0x44: "opaque", 0x46: "counter64", 0x80: "no_such_object",
    0x81: "no_such_instance", 0x82: "end_of_mib_view",
}


def enc_int_content(v):
    """Minimal two's-complement content octets of an INTEGER."""
    n = 1
    while not -(1 << (8 * n - 1)) <= v < (1 << (8 * n - 1)):
        n += 1
    return v.to_bytes(n, "big", signed=True)


def enc_unsigned_content(v):
    """Minimal content octets of an unsigned type, DER-style: a leading zero keeps it positive."""
    b = v.to_bytes(max(1, (v.bit_length() + 7) // 8), "big")
    return b if b[0] < 0x80 else b"\x00" + b


def dec_value(tag, c):
    """The type, the text form, and the CANONICAL content octets (spec §6): what decodes to a
    number or an OID is re-encoded minimally, so a non-minimal encoding on the wire and the
    canonical one give the same `raw`; octets stay as received."""
    kind = TYPES.get(tag, "unknown")
    raw = c
    if kind == "integer":
        n = dec_int(c)
        value, raw = str(n), enc_int_content(n)
    elif kind in ("counter32", "gauge32", "timeticks"):
        n = dec_unsigned(c, 5, U32)
        value, raw = str(n), enc_unsigned_content(n)
    elif kind == "counter64":
        n = dec_unsigned(c, 9, U64)
        value, raw = str(n), enc_unsigned_content(n)
    elif kind == "ip_address":
        if len(c) != 4:
            raise DecodeError("ip address length")
        value = ".".join(str(b) for b in c)
    elif kind == "oid":
        arcs = dec_oid(c)
        value, raw = dotted(arcs), enc_oid(arcs)
    elif kind in ("octet_string", "opaque"):
        value = c.hex()
    else:
        value = None
    return kind, value, raw


def decode(buf):
    s, e = expect(buf, 0, len(buf), 0x30)
    vs, ve = expect(buf, s, e, 0x02)
    version = dec_int(buf[vs:ve])
    if version == 3:
        raise DecodeError("SNMPv3 needs credentials (see v3vectors.py)")
    if version not in (0, 1):
        raise DecodeError("unknown version")
    cs, ce = expect(buf, ve, e, 0x04)
    return decode_pdu(buf, ce, e, version, buf[cs:ce])


def decode_pdu(buf, pos, e, version, community):
    """The notification semantics of spec §4.3 over one PDU, from `pos` to `e`. Shared by the
    v1/v2c decoder above and the v3 one in v3vectors.py (which has already decrypted it)."""
    tag, ps, pe = element(buf, pos, e)
    out = {"version": {0: "v1", 1: "v2c", 3: "v3"}[version], "community": community.hex()}
    if version == 0:
        if tag != 0xA4:
            raise DecodeError("not a v1 trap")
        es, ee = expect(buf, ps, pe, 0x06)
        enterprise = dec_oid(buf[es:ee])
        as_, ae = expect(buf, ee, pe, 0x40)
        if ae - as_ != 4:
            raise DecodeError("agent addr length")
        agent = ".".join(str(b) for b in buf[as_:ae])
        gs, ge = expect(buf, ae, pe, 0x02)
        generic = dec_int(buf[gs:ge])
        ss, se = expect(buf, ge, pe, 0x02)
        specific = dec_int(buf[ss:se])
        ts, te = expect(buf, se, pe, 0x43)
        uptime = dec_unsigned(buf[ts:te], 5, U32)
        vl_s, vl_e = expect(buf, te, pe, 0x30)
        out.update(pdu="trap", request_id=None, agent_addr=agent, uptime=str(uptime))
    else:
        if tag not in (0xA6, 0xA7):
            raise DecodeError("not a notification")
        rs, re_ = expect(buf, ps, pe, 0x02)
        request_id = dec_int(buf[rs:re_])
        es, ee = expect(buf, re_, pe, 0x02)
        dec_int(buf[es:ee])
        is_, ie = expect(buf, ee, pe, 0x02)
        dec_int(buf[is_:ie])
        vl_s, vl_e = expect(buf, ie, pe, 0x30)
        out.update(pdu="inform" if tag == 0xA6 else "trap", request_id=str(request_id),
                   agent_addr=None)
    varbinds = []
    pos = vl_s
    while pos < vl_e:
        vs_, ve_ = expect(buf, pos, vl_e, 0x30)
        ns, ne = expect(buf, vs_, ve_, 0x06)
        name = dotted(dec_oid(buf[ns:ne]))
        vtag, xs, xe = element(buf, ne, ve_)
        kind, value, raw = dec_value(vtag, buf[xs:xe])
        if len(varbinds) == MAX_VARBINDS:
            raise DecodeError("too many varbinds")
        varbinds.append({"oid": name, "type": kind, "value": value, "raw": raw.hex()})
        pos = ve_
    if version == 0:
        if 0 <= generic <= 5:
            trap = [1, 3, 6, 1, 6, 3, 1, 1, 5, generic + 1]
        elif generic == 6:
            if not 0 <= specific <= U32:
                raise DecodeError("specific trap out of range")
            trap = enterprise + [0, specific]
            if len(trap) > MAX_ARCS:
                raise DecodeError("too many arcs")
        else:
            raise DecodeError("generic trap out of range")
    else:
        trap_vb = next((v for v in varbinds if v["oid"] == TRAPOID), None)
        if trap_vb is None or trap_vb["type"] != "oid":
            raise DecodeError("no snmpTrapOID.0")
        trap = [int(a) for a in trap_vb["value"].split(".")]
        up = next((v for v in varbinds if v["oid"] == SYSUPTIME), None)
        out["uptime"] = up["value"] if up and up["type"] == "timeticks" else None
    out["trap"] = dotted(trap)
    out["trap_raw"] = enc_oid(trap).hex()
    out["varbinds"] = varbinds
    return out


# ---- encoding helpers (for synthetic and malformed vectors) ----------------------------------

def enc_len(n):
    if n < 0x80:
        return bytes([n])
    b = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return bytes([0x80 | len(b)]) + b


def tlv(tag, content):
    return bytes([tag]) + enc_len(len(content)) + content


def seq(*parts):
    return tlv(0x30, b"".join(parts))


def integer(v, tag=0x02):
    n = 1
    while not -(1 << (8 * n - 1)) <= v < (1 << (8 * n - 1)):
        n += 1
    return tlv(tag, v.to_bytes(n, "big", signed=True))


def octets(b, tag=0x04):
    return tlv(tag, b)


def enc_oid(arcs):
    out = bytearray()
    subids = [arcs[0] * 40 + arcs[1]] + list(arcs[2:])
    for s in subids:
        chunk = [s & 0x7F]
        s >>= 7
        while s:
            chunk.append(0x80 | (s & 0x7F))
            s >>= 7
        out += bytes(reversed(chunk))
    return bytes(out)


def arcs(text):
    return [int(a) for a in text.split(".")]


def oid(text):
    return tlv(0x06, enc_oid(arcs(text)))


def vb(name, value):
    return seq(oid(name), value)


def v2(pdu_tag=0xA7, varbinds=None, version=1, community=b"public", request_id=7):
    if varbinds is None:
        varbinds = [vb(SYSUPTIME, integer(5, 0x43)), vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9"))]
    return seq(integer(version), octets(community),
               tlv(pdu_tag, integer(request_id) + integer(0) + integer(0) + seq(*varbinds)))


def v1(generic=6, specific=1, varbinds=(), enterprise="1.3.6.1.4.1.99999", pdu_tag=0xA4, version=0):
    return seq(integer(version), octets(b"public"),
               tlv(pdu_tag, oid(enterprise) + tlv(0x40, bytes([10, 0, 0, 1])) + integer(generic)
                   + integer(specific) + integer(42, 0x43) + seq(*varbinds)))


# ---- conversions (spec §5) -------------------------------------------------------------------

def convert(kind, raw_hex, datatype):
    c = bytes.fromhex(raw_hex)
    numeric = {"integer": lambda: dec_int(c),
               "counter32": lambda: dec_unsigned(c, 5, U32),
               "gauge32": lambda: dec_unsigned(c, 5, U32),
               "timeticks": lambda: dec_unsigned(c, 5, U32),
               "counter64": lambda: dec_unsigned(c, 9, U64)}
    ranges = {"int8": (-2**7, 2**7 - 1), "uint8": (0, 2**8 - 1), "int16": (-2**15, 2**15 - 1),
              "uint16": (0, 2**16 - 1), "int32": (-2**31, 2**31 - 1), "uint32": (0, 2**32 - 1),
              "int64": (-2**63, 2**63 - 1), "uint64": (0, 2**64 - 1)}
    if kind in numeric:
        v = numeric[kind]()  # the conversion cases give `raw` as it arrives on the wire
        if datatype == "bool":
            return {"value": v != 0, "repr": "boolean"}
        if datatype in ranges:
            lo, hi = ranges[datatype]
            if not lo <= v <= hi:
                return {"bad": True}
            if abs(v) > SAFE:
                return {"value": str(v), "repr": "string"}
            return {"value": v, "repr": "number"}
        if datatype == "float32":
            return {"value": struct.unpack("f", struct.pack("f", float(v)))[0], "repr": "number"}
        if datatype == "float64":
            return {"value": float(v), "repr": "number"}
        if datatype == "string":
            return {"value": str(v), "repr": "string"}
        return {"bad": True}
    if kind == "octet_string" and datatype == "string":
        try:
            return {"value": c.decode("utf-8"), "repr": "string"}
        except UnicodeDecodeError:
            return {"bad": True}
    if kind == "oid" and datatype == "string":
        return {"value": dotted(dec_oid(c)), "repr": "string"}
    if kind == "ip_address" and datatype == "string":
        return {"value": ".".join(str(b) for b in c), "repr": "string"}
    return {"bad": True}


def main(captures_path, out_path):
    captures = []
    for line in open(captures_path).read().splitlines():
        if line.strip():
            hexdata, name = line.split("#", 1)
            captures.append((name.strip(), bytes.fromhex(hexdata.strip())))

    # Accepted, but not something net-snmp sends: the lenient corners of §4.
    synthetic = [
        ("(synthetic) Counter32 of 0xFFFFFFFF in four octets, no leading zero",
         v2(varbinds=[vb(SYSUPTIME, integer(5, 0x43)), vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")),
                      vb("1.3.6.1.4.1.99999.2.1", tlv(0x41, b"\xff\xff\xff\xff"))])),
        ("(synthetic) non-minimal long-form lengths",
         bytes.fromhex("30 81 2c 02 81 01 01 04 06") + b"public" +
         bytes.fromhex("a7 1e 02 01 07 02 01 00 02 01 00 30 13 30 11 06 0a") +
         enc_oid(arcs(TRAPOID)) + bytes.fromhex("06 03") + enc_oid(arcs("1.3.6.1"))),
        ("(synthetic) bytes after the message are ignored", v2() + b"\x00\x01garbage"),
        ("(synthetic) extra element inside a varbind is ignored",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")),
                      seq(oid("1.3.6.1.4.1.99999.2.1"), integer(1), integer(2))])),
        ("(synthetic) v2c trap without sysUpTime.0, snmpTrapOID.0 not second",
         v2(varbinds=[vb("1.3.6.1.4.1.99999.2.1", integer(1)), vb("1.3.6.1.4.1.99999.2.2", integer(2)),
                      vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9"))])),
        ("(synthetic) sysUpTime.0 of the wrong type leaves uptime unknown",
         v2(varbinds=[vb(SYSUPTIME, integer(5)), vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9"))])),
        ("(synthetic) unknown value tag and exception values",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")),
                      vb("1.3.6.1.4.1.99999.2.1", tlv(0x47, b"\x01")),
                      vb("1.3.6.1.4.1.99999.2.2", tlv(0x80, b"")),
                      vb("1.3.6.1.4.1.99999.2.3", tlv(0x81, b"")),
                      vb("1.3.6.1.4.1.99999.2.4", tlv(0x82, b"")),
                      vb("1.3.6.1.4.1.99999.2.5", tlv(0x05, b"")),
                      vb("1.3.6.1.4.1.99999.2.6", tlv(0x44, b"\xca\xfe"))])),
        ("(synthetic) OID arcs at the limits: 2.999 and a 32-bit arc",
         v2(varbinds=[vb(TRAPOID, oid("2.999.4294967295.0.1"))])),
        ("(synthetic) OID with a non-minimal 0x80 padding octet",
         v2(varbinds=[vb(TRAPOID, tlv(0x06, bytes.fromhex("2b0601040180868d1f0009")))])),
        ("(synthetic) negative request-id and INTEGER of eight octets",
         v2(request_id=-5, varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")),
                                     vb("1.3.6.1.4.1.99999.2.1", tlv(0x02, bytes.fromhex("8000000000000000")))])),
        ("(synthetic) v1 enterprise-specific trap with specific-trap 0",
         v1(generic=6, specific=0)),
        ("(synthetic) empty community", v2(community=b"")),
        ("(synthetic) inform", v2(pdu_tag=0xA6)),
        ("(synthetic) 256 varbinds", v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9"))] +
                                      [vb(f"1.3.6.1.4.1.99999.2.{i}", integer(i)) for i in range(255)])),
        ("(synthetic) OID of 128 arcs",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb(".".join(["1"] * 128), integer(1))])),
    ]

    messages = []
    for name, data in captures + synthetic:
        messages.append({"name": name, "hex": data.hex(), "expect": decode(data)})

    good = captures[0][1]
    malformed = [
        ("empty datagram", b""),
        ("truncated in the varbind list", good[:-3]),
        ("truncated inside a length octet", bytes.fromhex("3082")),
        ("outer element is not a SEQUENCE", b"\x31" + good[1:]),
        ("indefinite length", bytes.fromhex("30800201010000")),
        ("length form with five length octets", bytes.fromhex("3085000000000302 0101".replace(" ", ""))),
        ("multi-octet tag", bytes.fromhex("30031f0100")),
        ("SNMPv3 message", v2(version=3)),
        ("unknown version 2", v2(version=2)),
        ("negative version", v2(version=-1)),
        ("community is not an OCTET STRING", seq(integer(1), integer(5), tlv(0xA7, b""))),
        ("v2c get-request is not a notification", v2(pdu_tag=0xA0)),
        ("v2c response is not a notification", v2(pdu_tag=0xA2)),
        ("v2c message carrying a v1 trap PDU", v1(version=1)),
        ("v1 message carrying a v2 trap PDU", v2(version=0)),
        ("v2c trap without snmpTrapOID.0", v2(varbinds=[vb(SYSUPTIME, integer(5, 0x43))])),
        ("v2c trap with no varbinds", v2(varbinds=[])),
        ("snmpTrapOID.0 that is not an OID", v2(varbinds=[vb(TRAPOID, integer(9))])),
        ("v1 generic-trap 7", v1(generic=7)),
        ("v1 generic-trap -1", v1(generic=-1)),
        ("v1 enterprise-specific with negative specific-trap", v1(generic=6, specific=-1)),
        ("v1 enterprise-specific with specific-trap above 32 bits", v1(generic=6, specific=2**32)),
        ("v1 agent-addr of three octets",
         seq(integer(0), octets(b"public"), tlv(0xA4, oid("1.3.6.1.4.1.99999") + tlv(0x40, b"\x0a\x00\x01")
             + integer(6) + integer(1) + integer(42, 0x43) + seq()))),
        ("v1 time-stamp that is an INTEGER",
         seq(integer(0), octets(b"public"), tlv(0xA4, oid("1.3.6.1.4.1.99999") + tlv(0x40, b"\x0a\x00\x00\x01")
             + integer(6) + integer(1) + integer(42) + seq()))),
        ("v1 enterprise-specific trap OID longer than 128 arcs",
         v1(enterprise=".".join(["1"] * 127), generic=6, specific=1)),
        ("OID arc larger than 32 bits",
         v2(varbinds=[vb(TRAPOID, tlv(0x06, bytes.fromhex("2b060190808080 00".replace(" ", ""))))])),
        ("OID ending with a continuation bit", v2(varbinds=[vb(TRAPOID, tlv(0x06, bytes.fromhex("2b86")))])),
        ("empty OID", v2(varbinds=[vb(TRAPOID, tlv(0x06, b""))])),
        ("OID of 129 arcs",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb(".".join(["1"] * 129), integer(1))])),
        ("INTEGER with nine content octets", seq(tlv(0x02, bytes(9)), octets(b"public"), tlv(0xA7, b""))),
        ("INTEGER with no content octets", seq(tlv(0x02, b""), octets(b"public"), tlv(0xA7, b""))),
        ("Counter32 value above 32 bits",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x41, bytes.fromhex("0100000000")))])),
        ("Gauge32 with six content octets",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x42, bytes(6)))])),
        ("TimeTicks with no content octets",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x43, b""))])),
        ("Counter64 with ten content octets",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x46, bytes(10)))])),
        ("Counter64 of nine octets above 64 bits",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x46, b"\x01" + bytes(8)))])),
        ("IpAddress of five octets",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x40, bytes(5)))])),
        ("OID-typed varbind value with an arc overflow",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), vb("1.3.6.1.4.1.99999.2.1", tlv(0x06, bytes.fromhex("2bffffffff7f")))])),
        ("varbind that is not a SEQUENCE",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), tlv(0x31, oid("1.3.6") + integer(1))])),
        ("varbind without a value", v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), seq(oid("1.3.6"))])),
        ("varbind name that is not an OID",
         v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9")), seq(integer(1), integer(1))])),
        ("257 varbinds", v2(varbinds=[vb(TRAPOID, oid("1.3.6.1.4.1.99999.0.9"))] +
                           [vb(f"1.3.6.1.4.1.99999.2.{i}", integer(i)) for i in range(256)])),
        ("length exceeding the datagram", good[:2] + bytes([good[1] + 1]) + good[3:] if False else
         bytes([0x30, len(good) - 1]) + good[2:] if good[1] < 0x80 else bytes([0x30, 0x7f]) + good[2:]),
    ]
    out_malformed = []
    for name, data in malformed:
        try:
            decode(data)
        except DecodeError as e:
            out_malformed.append({"name": name, "hex": data.hex(), "why": str(e)})
            continue
        sys.exit(f"malformed vector '{name}' decoded successfully: fix the vector")

    conversions = []
    cases = [
        ("integer", "d6", ["int8", "uint8", "int16", "int32", "int64", "uint64", "float32", "float64", "bool", "string"]),
        ("integer", "00", ["bool", "uint8"]),
        ("integer", "7fffffff", ["int16", "int32", "uint32", "float32"]),
        ("integer", "01000001", ["float32", "float64"]),
        ("integer", "8000000000000000", ["int64", "uint64", "string", "float64"]),
        ("counter32", "ffffffff", ["int32", "uint32", "int64", "string"]),
        ("gauge32", "00ffffffff", ["uint32"]),
        ("timeticks", "63", ["uint16", "uint8", "int8"]),
        ("counter64", "00ffffffffffffffff", ["uint64", "int64", "float64", "string"]),
        ("counter64", "1fffffffffffff", ["uint64", "int64"]),
        ("counter64", "20000000000000", ["uint64", "int64"]),
        ("octet_string", "68c3a96c6c6f", ["string", "bool", "uint32"]),
        ("octet_string", "ff", ["string"]),
        ("octet_string", "c3", ["string"]),
        ("octet_string", "eda080", ["string"]),
        ("octet_string", "f09f9880", ["string"]),
        ("octet_string", "", ["string"]),
        ("oid", "2b06010401bf08", ["string", "uint32"]),
        ("ip_address", "c0a80114", ["string", "uint32"]),
        ("opaque", "deadbeef", ["string", "uint32"]),
        ("null", "", ["string", "bool", "int32"]),
        ("no_such_object", "", ["string"]),
        ("no_such_instance", "", ["bool"]),
        ("end_of_mib_view", "", ["int32"]),
        ("unknown", "01", ["string"]),
    ]
    for kind, raw, datatypes in cases:
        for dt in datatypes:
            entry = {"type": kind, "raw": raw, "datatype": dt}
            entry.update(convert(kind, raw, dt))
            conversions.append(entry)

    v3 = v3vectors.build(sys.modules[__name__],
                         os.path.join(os.path.dirname(os.path.abspath(captures_path)), "v3captures.txt"))

    doc = {
        "about": ("Golden vectors for the SNMP connector (doc/connectors/snmp-connector-spec.md). "
                  "`messages` are datagrams (captured from net-snmp, or synthetic where the name says so) "
                  "with their decoding (§4); `malformed` datagrams must be rejected (§4); `conversions` "
                  "turn one varbind value into one point datatype (§6); `v3` holds the SNMPv3 vectors and "
                  "the credentials they were sent with (see its own `about`). A varbind's `raw` is the "
                  "CANONICAL content octets of its value (§6): numbers and OIDs re-encoded minimally, "
                  "octet strings as received. Numbers that can exceed 2^53 are decimal strings, and "
                  "`community` and OCTET STRING/Opaque values are hex. Generated by a reference decoder "
                  "independent of both implementations; the Rust (connector-snmp) and C "
                  "(impl/c/tests/snmp.c) tests read this file."),
        "messages": messages,
        "malformed": out_malformed,
        "conversions": conversions,
        "v3": v3,
    }
    with open(out_path, "w") as f:
        json.dump(doc, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"{len(messages)} messages, {len(out_malformed)} malformed, {len(conversions)} conversions, "
          f"{len(v3['messages'])} v3 messages, {len(v3['rejected'])} v3 rejected, {len(v3['probes'])} v3 probes")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
