// Egress relay: the one way out of Nandi's internal `nandi-agents` network.
//
// Runs in the `nandi-egress` container, which sits on both that network and
// Docker's default bridge. It forwards every connection on 0.0.0.0:<listen>
// to the host's egress proxy, which authenticates each request by the
// agent's proxy token and enforces its skill.md. The relay decides nothing.
//
// Usage: node relay.mjs <listen port> <target host> <target port>
import net from 'node:net'
import process from 'node:process'

const [listen, host, port] = process.argv.slice(2)
net
  .createServer(client => {
    const upstream = net.connect(Number(port), host)
    client.pipe(upstream).pipe(client)
    const close = () => {
      client.destroy()
      upstream.destroy()
    }
    client.on('error', close)
    upstream.on('error', close)
  })
  .listen(Number(listen), '0.0.0.0')
