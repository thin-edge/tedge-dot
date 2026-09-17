"""OPC-UA simulator for the tedge-dot e2e harness.

Exposes a handful of nodes with stable string NodeIds under namespace index 2
(urn:tedge:opcua-sim) so the connector can address them as `ns=2;s=<name>`:

    ns=2;s=Temperature   Double  21.5            (read)
    ns=2;s=Count         UInt32  617001          (read)
    ns=2;s=Setpoint      Int32   0   (writable)  (read/write round-trip)
    ns=2;s=Running       Boolean false (writable)(read/write round-trip)
    ns=2;s=Ticks         UInt32  incremented every second (subscription/push tests)

Reading ns=2;s=DoesNotExist yields a Bad status, exercising bad-quality handling.

The endpoint host is taken from OPCUA_ENDPOINT_HOST so the advertised endpoint URL
matches the docker service name (avoids OPC-UA hostname-rewrite connection failures).

OPCUA_SIM_DYNAMIC=1 makes Temperature drift (21.5 +/- 2.5 over a 5 minute cycle) and
Count increment every second. The demo stacks enable it: a subscription only notifies
on change, so with static values a push-delivered device goes silent after its first
notification and the cloud marks it unavailable. Off by default because the e2e and
smoke tests assert the static values above.

SECURED MODE (connectors/opcua/docker-compose.secure.yaml): with OPCUA_SECURE_SERVERS set,
the container instead serves one server per PKI scenario, each on its own port, all with the
address space above:

    OPCUA_PKI_VECTORS      directory for the genpki.py tree (generated when it has no
                           expected.json yet; a volume shared with the connector)
    OPCUA_CERT_HOSTNAMES   comma-separated SAN host names of the valid server certificates
    OPCUA_SECURE_SERVERS   comma-separated <port>=<scenario>: the server on <port> presents
                           scenarios/<scenario>/server/cert.der
    OPCUA_SECURE_POLICIES  comma-separated asyncua SecurityPolicyType names they offer
    OPCUA_PLAIN_PORT       an extra server offering only NoSecurity (a password sent to it
                           travels in plaintext)
    OPCUA_USERS            comma-separated <user>:<password> accepted by every server; the
                           X.509 user users/operator.der is accepted too
"""

import asyncio
import math
import os

from asyncua import Server, ua
from asyncua.crypto import uacrypto
from asyncua.server.user_managers import UserManager
from asyncua.crypto.permission_rules import User, UserRole

ENDPOINT_HOST = os.environ.get("OPCUA_ENDPOINT_HOST", "0.0.0.0")
DYNAMIC = os.environ.get("OPCUA_SIM_DYNAMIC", "").strip().lower() in ("1", "true", "yes", "on")
NS_URI = "urn:tedge:opcua-sim"
# The application URI of a secured server (its certificate's URI SAN). Distinct from NS_URI,
# which would otherwise take namespace index 1 and move the nodes off ns=2.
SERVER_URI = "urn:tedge:opcua-sim:server"

TEMPERATURE = 21.5
COUNT = 617001


class SimUserManager(UserManager):
    """Anonymous sessions, the OPCUA_USERS passwords and one trusted X.509 user."""

    def __init__(self, users, user_certificate=None):
        self.users = users
        self.user_certificate = user_certificate

    def get_user(self, iserver, username=None, password=None, certificate=None):
        # asyncua passes the channel's CLIENT certificate for every token type on a secured
        # channel, and the verified user certificate for an X.509 token -- so a certificate
        # that is not the user's is an anonymous (or username) session.
        if username is not None:
            if self.users.get(username) == password:
                return User(role=UserRole.Admin)
            return None
        return User(role=UserRole.Admin)


def parse_users(text):
    users = {}
    for entry in filter(None, (e.strip() for e in text.split(","))):
        user, _, password = entry.partition(":")
        users[user] = password
    return users


async def build_server(port, policies, user_manager=None, cert=None, key=None):
    """A server with the simulator's address space; returns (server, nodes)."""
    server = Server(user_manager=user_manager)
    await server.init()
    server.set_endpoint(f"opc.tcp://{ENDPOINT_HOST}:{port}/")
    server.set_server_name("tedge OPC-UA simulator")
    if cert and key:
        await server.set_application_uri(SERVER_URI)
        await server.load_certificate(cert)
        await server.load_private_key(key)
    server.set_security_policy(policies)
    if user_manager is not None:
        server.set_identity_tokens(
            [ua.AnonymousIdentityToken, ua.UserNameIdentityToken, ua.X509IdentityToken]
        )

    idx = await server.register_namespace(NS_URI)
    plc = await server.nodes.objects.add_object(
        ua.NodeId("Plc", idx), ua.QualifiedName("Plc", idx)
    )

    temperature = await plc.add_variable(
        ua.NodeId("Temperature", idx), ua.QualifiedName("Temperature", idx), TEMPERATURE
    )
    count = await plc.add_variable(
        ua.NodeId("Count", idx),
        ua.QualifiedName("Count", idx),
        ua.Variant(COUNT, ua.VariantType.UInt32),
    )
    setpoint = await plc.add_variable(
        ua.NodeId("Setpoint", idx),
        ua.QualifiedName("Setpoint", idx),
        ua.Variant(0, ua.VariantType.Int32),
    )
    running = await plc.add_variable(
        ua.NodeId("Running", idx), ua.QualifiedName("Running", idx), False
    )
    await setpoint.set_writable()
    await running.set_writable()

    # Changes every second so OPC-UA subscriptions (monitored items) have data-change
    # notifications to deliver; the static nodes above only ever notify once.
    ticks = await plc.add_variable(
        ua.NodeId("Ticks", idx),
        ua.QualifiedName("Ticks", idx),
        ua.Variant(0, ua.VariantType.UInt32),
    )
    return server, {"idx": idx, "temperature": temperature, "count": count, "ticks": ticks}


def secured_servers():
    """The (port, policies, cert, key) of every server of the secured mode."""
    import subprocess
    import sys

    vectors = os.environ["OPCUA_PKI_VECTORS"]
    if not os.path.exists(os.path.join(vectors, "expected.json")):
        hosts = [h for h in os.environ.get("OPCUA_CERT_HOSTNAMES", "").split(",") if h]
        cmd = [
            sys.executable,
            os.path.join(os.path.dirname(__file__), "genpki.py"),
            vectors,
            "--application-uri",
            SERVER_URI,
        ]
        for h in hosts:
            cmd += ["--hostname", h]
        subprocess.run(cmd, check=True)
        print(f"generated the PKI vectors in {vectors}", flush=True)
    policies = [
        getattr(ua.SecurityPolicyType, name.strip())
        for name in os.environ.get("OPCUA_SECURE_POLICIES", "Basic256Sha256_SignAndEncrypt").split(",")
        if name.strip()
    ]
    servers = []
    for entry in filter(None, (e.strip() for e in os.environ["OPCUA_SECURE_SERVERS"].split(","))):
        port, _, scenario = entry.partition("=")
        base = os.path.join(vectors, "scenarios", scenario, "server")
        servers.append((int(port), policies, os.path.join(base, "cert.der"), os.path.join(base, "key.pem")))
    plain = os.environ.get("OPCUA_PLAIN_PORT")
    if plain:
        servers.append((int(plain), [ua.SecurityPolicyType.NoSecurity], None, None))
    return vectors, servers


async def main():
    users = parse_users(os.environ.get("OPCUA_USERS", ""))
    if os.environ.get("OPCUA_SECURE_SERVERS"):
        vectors, specs = secured_servers()
        user_cert = await uacrypto.load_certificate(os.path.join(vectors, "users", "operator.der"))
        manager = SimUserManager(users, user_cert)
    else:
        specs = [(4840, [ua.SecurityPolicyType.NoSecurity], None, None)]
        manager = SimUserManager(users) if users else None

    servers = []
    for port, policies, cert, key in specs:
        servers.append(await build_server(port, policies, manager, cert, key))
    for server, nodes in servers:
        await server.start()
        print(
            f"OPC-UA simulator listening on {server.endpoint.geturl()} "
            f"(namespace idx={nodes['idx']}, dynamic={DYNAMIC})",
            flush=True,
        )
    try:
        n = 0
        while True:
            await asyncio.sleep(1)
            n += 1
            for _server, nodes in servers:
                await nodes["ticks"].write_value(ua.Variant(n, ua.VariantType.UInt32))
                if DYNAMIC:
                    drift = 2.5 * math.sin(2 * math.pi * n / 300)
                    await nodes["temperature"].write_value(round(TEMPERATURE + drift, 2))
                    await nodes["count"].write_value(
                        ua.Variant((COUNT + n) & 0xFFFFFFFF, ua.VariantType.UInt32)
                    )
    finally:
        for server, _nodes in servers:
            await server.stop()


if __name__ == "__main__":
    asyncio.run(main())
