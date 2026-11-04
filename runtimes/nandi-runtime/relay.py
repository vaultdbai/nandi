"""Egress relay: the one way out of Nandi's internal `nandi-agents` network.

Runs in the `nandi-egress` container, which sits on both that network and
Docker's default bridge. It forwards every connection on 0.0.0.0:<listen> to
the host's egress proxy, which authenticates each request by the agent's
proxy token and enforces its skill.md. The relay itself decides nothing.

Usage: python relay.py <listen port> <target host> <target port>
"""

import asyncio
import sys


async def pipe(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        writer.close()


async def main() -> None:
    listen, host, port = int(sys.argv[1]), sys.argv[2], int(sys.argv[3])

    async def forward(client_reader, client_writer):
        try:
            upstream_reader, upstream_writer = await asyncio.open_connection(host, port)
        except OSError:
            client_writer.close()
            return
        await asyncio.gather(
            pipe(client_reader, upstream_writer),
            pipe(upstream_reader, client_writer),
        )

    server = await asyncio.start_server(forward, "0.0.0.0", listen)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
