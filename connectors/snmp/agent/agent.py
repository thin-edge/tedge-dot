#!/usr/bin/env python3
"""SNMP agent simulator for the tedge-dot SNMP connector (pysnmp 7.1).

THIS HEADER IS THE ONE PLACE THE SEEDS ARE DOCUMENTED. The e2e suite
(connectors/snmp/tests/snmp_e2e.robot), connectors/snmp/connector.toml, the demo point library
(demo/points.d/snmp/demo-sim.toml) and impl/c/ci/smoke.sh rely on them; change them together.

Access
------
  v1/v2c  community "public"   read-only  (the whole tree)
  v1/v2c  community "private"  read-write (the whole tree)
  v3      user "ro-auth"    authNoPriv  SHA (SHA-1)           auth "ro-auth-pass-1"             read-only
  v3      user "rw-priv"    authPriv    SHA (SHA-1) + AES-128 auth "rw-auth-pass-1"
                                                              priv "rw-priv-pass-1"             read-write
  v3      user "sha2-priv"  authPriv    SHA256 + AES256       auth "sha2-auth-pass-1"
                            (Blumenthal key extension,        priv "sha2-priv-pass-1"           read-write
                             net-snmp's "-a SHA-256 -x AES-256")
  engine ID: $SNMP_ENGINE_ID (hex), default 80001f8880e2e0000000000001

Objects (E = 1.3.6.1.4.1.99999.1)
----------------------------------
  OID           SNMP type      seed                                   access
  E.1.0         INTEGER        -42                                    read-only   (SET -> notWritable)
  E.2.0         INTEGER        1234                                   read-only
  E.3.0         OCTET STRING   "Pump station 7 – Zürich" (UTF-8)       read-only
  E.4.0         OCTET STRING   de ad be ef 00 ff (not UTF-8)          read-only
  E.5.0         OID            1.3.6.1.4.1.99999.42.7                 read-only
  E.6.0         IpAddress      192.168.10.2                           read-only
  E.7.0         Counter32      4000000000                             read-only
  E.8.0         Gauge32        42                                     read-only
  E.9.0         TimeTicks      123456                                 read-only
  E.10.0        Counter64      9007199254740993 (2^53 + 1)            read-only
  E.11.0        Opaque         01 02 03 04                            read-only
  E.12.0        Counter32      seconds since the agent started        read-only   (changes every second)
  E.20.0        INTEGER        10                                     read-write
  E.21.0        OCTET STRING   "initial"                              read-write
  E.22.0        Gauge32        100                                    read-write
  1.3.6.1.2.1.1.1.0  sysDescr.0   "tedge-dot SNMP agent simulator"    read-only
  1.3.6.1.2.1.1.3.0  sysUpTime.0  (pysnmp engine uptime)              read-only
  (all of SNMPv2-MIB that pysnmp serves by default is reachable too)

Not served (for exception handling): E.99.0 (noSuchObject), E.1.1 (noSuchInstance). In v1 both
answer error-status noSuchName.

Writes are kept in memory: restarting the container restores the seeds.

Usage:  agent.py            serve on 0.0.0.0:$SNMP_AGENT_PORT (default 161)
        agent.py --check    exit 0 when an agent answers sysDescr.0 on 127.0.0.1 (healthcheck)
"""

import asyncio
import logging
import os
import sys
import time
import warnings

# pysnmp's AES uses cryptography's CFB mode from its old import location: a deprecation warning
# on the first authPriv request, nothing more (the pinned versions work together).
warnings.filterwarnings("ignore", message="CFB has been moved")

from pysnmp.carrier.asyncio.dgram import udp
from pysnmp.entity import config, engine
from pysnmp.entity.rfc3413 import cmdrsp, context
from pysnmp.proto import rfc1902

PORT = int(os.environ.get("SNMP_AGENT_PORT", "161"))
ENGINE_ID = os.environ.get("SNMP_ENGINE_ID", "80001f8880e2e0000000000001")
SYS_DESCR = "tedge-dot SNMP agent simulator"
ENTERPRISE = (1, 3, 6, 1, 4, 1, 99999, 1)
STARTED = time.monotonic()

V3_USERS = [
    # (user, auth protocol, auth password, priv protocol, priv password, VACM level, writable)
    ("ro-auth", config.USM_AUTH_HMAC96_SHA, "ro-auth-pass-1", config.USM_PRIV_NONE, None, "authNoPriv", False),
    ("rw-priv", config.USM_AUTH_HMAC96_SHA, "rw-auth-pass-1", config.USM_PRIV_CFB128_AES, "rw-priv-pass-1", "authPriv", True),
    ("sha2-priv", config.USM_AUTH_HMAC192_SHA256, "sha2-auth-pass-1", config.USM_PRIV_CFB256_AES_BLUMENTHAL, "sha2-priv-pass-1", "authPriv", True),
]

# (arc under ENTERPRISE, seed value, max-access)
SCALARS = [
    (1, rfc1902.Integer32(-42), "read-only"),
    (2, rfc1902.Integer32(1234), "read-only"),
    (3, rfc1902.OctetString("Pump station 7 – Zürich".encode("utf-8")), "read-only"),
    (4, rfc1902.OctetString(bytes.fromhex("deadbeef00ff")), "read-only"),
    (5, rfc1902.ObjectIdentifier("1.3.6.1.4.1.99999.42.7"), "read-only"),
    (6, rfc1902.IpAddress("192.168.10.2"), "read-only"),
    (7, rfc1902.Counter32(4000000000), "read-only"),
    (8, rfc1902.Gauge32(42), "read-only"),
    (9, rfc1902.TimeTicks(123456), "read-only"),
    (10, rfc1902.Counter64(9007199254740993), "read-only"),
    (11, rfc1902.Opaque(bytes.fromhex("01020304")), "read-only"),
    (12, rfc1902.Counter32(0), "read-only"),  # ticking, see Ticking below
    (20, rfc1902.Integer32(10), "read-write"),
    (21, rfc1902.OctetString(b"initial"), "read-write"),
    (22, rfc1902.Gauge32(100), "read-write"),
]
TICKING_ARC = 12


def check():
    """Healthcheck: GET sysDescr.0 over v2c from localhost."""
    from pysnmp.hlapi.v3arch.asyncio import (
        CommunityData, ContextData, ObjectIdentity, ObjectType, SnmpEngine, UdpTransportTarget, get_cmd,
    )

    async def probe():
        target = await UdpTransportTarget.create(("127.0.0.1", PORT), timeout=1, retries=0)
        err, status, _index, binds = await get_cmd(
            SnmpEngine(), CommunityData("public"), target, ContextData(),
            ObjectType(ObjectIdentity("1.3.6.1.2.1.1.1.0")),
        )
        return not err and not status and SYS_DESCR in str(binds[0][1])

    return 0 if asyncio.run(probe()) else 1


def build_engine():
    snmp_engine = engine.SnmpEngine(snmpEngineID=rfc1902.OctetString(hexValue=ENGINE_ID))
    config.add_transport(
        snmp_engine, udp.DOMAIN_NAME, udp.UdpTransport().open_server_mode(("0.0.0.0", PORT))
    )

    everything = (1, 3, 6)
    # A read-only principal still needs a write view that EXISTS: pysnmp lets a SET through when
    # the write view has no entries at all. This one only includes a subtree nothing is served in.
    nothing = (1, 3, 6, 1, 4, 1, 99999, 255)
    # v1 (security model 1) and v2c (2): one read-only and one read-write community.
    config.add_v1_system(snmp_engine, "public-area", "public")
    config.add_v1_system(snmp_engine, "private-area", "private")
    for model in (1, 2):
        config.add_vacm_user(
            snmp_engine, model, "public-area", "noAuthNoPriv", readSubTree=everything, writeSubTree=nothing
        )
        config.add_vacm_user(
            snmp_engine, model, "private-area", "noAuthNoPriv", readSubTree=everything, writeSubTree=everything
        )

    for user, auth, auth_key, priv, priv_key, level, writable in V3_USERS:
        config.add_v3_user(snmp_engine, user, auth, auth_key, priv, priv_key)
        config.add_vacm_user(
            snmp_engine, 3, user, level, readSubTree=everything, writeSubTree=everything if writable else nothing
        )

    snmp_context = context.SnmpContext(snmp_engine)
    mib_builder = snmp_context.get_mib_instrum().get_mib_builder()
    MibScalar, MibScalarInstance = mib_builder.import_symbols("SNMPv2-SMI", "MibScalar", "MibScalarInstance")

    class Ticking(MibScalarInstance):
        def getValue(self, name, **ctx):
            return self.getSyntax().clone(int(time.monotonic() - STARTED))

    symbols = []
    for arc, value, access in SCALARS:
        oid = ENTERPRISE + (arc,)
        cls = Ticking if arc == TICKING_ARC else MibScalarInstance
        symbols.append(MibScalar(oid, value.clone()).setMaxAccess(access))
        symbols.append(cls(oid, (0,), value.clone()))
    mib_builder.export_symbols("__TEDGE-DOT-SIM-MIB", *symbols)

    (sys_descr,) = mib_builder.import_symbols("__SNMPv2-MIB", "sysDescr")
    sys_descr.syntax = sys_descr.syntax.clone(SYS_DESCR)

    if os.environ.get("SNMP_LOG_REQUESTS", "1") not in ("", "0"):
        # One line per request: which PDU a client sent (the suite tells GET and GETBULK devices
        # apart by it). Never the community or keys.
        def log_request(_engine, _execpoint, variables, _ctx):
            version = {0: "v1", 1: "v2c", 3: "v3"}.get(int(variables["messageProcessingModel"]), "?")
            level = {1: "noAuthNoPriv", 2: "authNoPriv", 3: "authPriv"}.get(int(variables["securityLevel"]), "?")
            logging.info(
                "request from %s: %s %s (%s)",
                variables["transportAddress"][0], version, variables["pdu"].__class__.__name__, level,
            )

        snmp_engine.observer.register_observer(log_request, "rfc3412.receiveMessage:request")

    cmdrsp.GetCommandResponder(snmp_engine, snmp_context)
    cmdrsp.NextCommandResponder(snmp_engine, snmp_context)
    cmdrsp.BulkCommandResponder(snmp_engine, snmp_context)
    cmdrsp.SetCommandResponder(snmp_engine, snmp_context)
    return snmp_engine


def main():
    if "--check" in sys.argv[1:]:
        return check()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
    snmp_engine = build_engine()
    logging.info(
        "SNMP agent simulator on udp/%d (engine %s): communities public/private, v3 users %s",
        PORT, ENGINE_ID, ", ".join(u[0] for u in V3_USERS),
    )
    snmp_engine.transport_dispatcher.job_started(1)
    try:
        snmp_engine.open_dispatcher()
    finally:
        snmp_engine.close_dispatcher()
    return 0


if __name__ == "__main__":
    sys.exit(main())
