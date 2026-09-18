#!/usr/bin/env python3
"""Render the interop connector configuration with the reference server's real namespace index.

The UA-.NETStandard reference server documents its namespace indices as *typical*, not
guaranteed, and `connector-opcua` can only address a node by index (`ns=1;s=...`); there is no
namespace-URI form. Hardcoding an index would make the suite fail for a reason that has nothing
to do with the connector, so the index is read from the server's namespace array and
substituted for `@NS@` in the template.

Each server is resolved separately. They run the same image, so their indices agree today --
but the whole reason this script exists is that the index is the server's to choose, and
`ref-discovery` in particular starts with a different feature set. Assuming one server's answer
holds for another would reintroduce exactly the bug being avoided, in a place where it would
show up as a node read returning the wrong value rather than as an error.

Usage: resolve-ns.py <template> <output> <placeholder>=<endpoint> ...
"""
import asyncio
import sys

from asyncua import Client, ua

NAMESPACE_URI = "urn:opcua:testserver:nodes"


class PlainClient(Client):
    """`asyncua` with the invented `ServerUri` removed.

    asyncua always sets `CreateSessionParameters.ServerUri` to `urn:<host>:<path>`, which it
    makes up from the URL it was given. UA-.NETStandard rejects anything that is not its own
    ApplicationUri with `BadServerUriInvalid`, so an unpatched asyncua cannot open a session
    against it at all. ServerUri is only meaningful when connecting through a gateway
    (OPC UA Part 4, CreateSession), so sending it empty is correct, not a workaround for us.
    """

    async def create_session(self):
        original = ua.CreateSessionParameters

        class Blanked(original):
            def __setattr__(self, name, value):
                if name == "ServerUri":
                    value = ""
                object.__setattr__(self, name, value)

        ua.CreateSessionParameters = Blanked
        try:
            return await super().create_session()
        finally:
            ua.CreateSessionParameters = original


async def resolve(endpoint: str) -> int:
    last = None
    # The server is healthy (its port is open) before its address space is fully built.
    for _ in range(60):
        try:
            async with PlainClient(url=endpoint) as client:
                namespaces = await client.get_namespace_array()
                for index, uri in enumerate(namespaces):
                    print(f"  ns={index}  {uri}", flush=True)
                if NAMESPACE_URI not in namespaces:
                    raise RuntimeError(
                        f"{endpoint} does not serve {NAMESPACE_URI}; it has {namespaces}"
                    )
                return namespaces.index(NAMESPACE_URI)
        except Exception as exc:  # noqa: BLE001 - retry whatever the server is not ready for
            last = exc
            await asyncio.sleep(2)
    raise SystemExit(f"could not read the namespace array of {endpoint}: {last}")


async def main() -> None:
    template, output = sys.argv[1:3]
    targets = [arg.split("=", 1) for arg in sys.argv[3:]]
    if not targets:
        raise SystemExit("no <placeholder>=<endpoint> pairs given")

    with open(template, encoding="utf-8") as handle:
        rendered = handle.read()

    for placeholder, endpoint in targets:
        # Checked before substituting, not after: replacing every occurrence and then looking
        # for one can only ever find nothing. What is worth catching is a template that lost a
        # placeholder, which would otherwise render a configuration with no namespace at all.
        if placeholder not in rendered:
            raise SystemExit(f"{template} contains no {placeholder} placeholder to render")
        index = await resolve(endpoint)
        print(f"{NAMESPACE_URI} is ns={index} on {endpoint}", flush=True)
        rendered = rendered.replace(placeholder, str(index))

    with open(output, "w", encoding="utf-8") as handle:
        handle.write(rendered)
    print(f"rendered {output}", flush=True)


asyncio.run(main())
