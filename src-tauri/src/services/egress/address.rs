//! Where the proxy may connect once a request is allowed.
//!
//! Names are resolved on the host, so an allowed name that points at the
//! host itself or its network (DNS rebinding, or a `*` rule) would reach what
//! no agent should: loopback services, the LAN, a cloud metadata endpoint.
//! After resolving, an internal address is refused unless the agent's rule
//! names that IP literally, and the proxy connects to the addresses checked
//! here, never resolving again. The tunnel and `/fetch` both ask here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// The addresses `host:port` may be reached at: an IP literal as is, a name
/// as resolved. An internal address is kept only when `named(ip)` (a rule
/// names that IP itself).
pub async fn reachable(
    host: &str,
    port: u16,
    named: impl Fn(IpAddr) -> bool,
) -> Result<Vec<SocketAddr>, String> {
    // An IPv6 literal comes in brackets from a URL
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    let resolved: Vec<SocketAddr> = match literal.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("Couldn't resolve {host}: {e}"))?
            .collect(),
    };
    let reachable: Vec<SocketAddr> = resolved
        .iter()
        .copied()
        .filter(|addr| !is_internal(addr.ip()) || named(addr.ip()))
        .collect();
    match (reachable.is_empty(), resolved.first()) {
        (false, _) => Ok(reachable),
        (true, Some(addr)) => Err(format!(
            "{host} is {}, an internal address. Name that IP in network.allow to reach it",
            addr.ip()
        )),
        (true, None) => Err(format!("{host} has no address")),
    }
}

/// The host itself or a private network: loopback, link-local, private,
/// CGNAT (100.64.0.0/10), unique-local, unspecified or broadcast
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_internal_v4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => is_internal_v4(v4),
            None => is_internal_v6(ip),
        },
    }
}

fn is_internal_v4(ip: Ipv4Addr) -> bool {
    let cgnat = ip.octets()[0] == 100 && (ip.octets()[1] & 0b1100_0000) == 64;
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.octets()[0] == 0
        || cgnat
}

fn is_internal_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local() || ip.is_unicast_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_addresses_are_the_host_and_its_networks() {
        let internal = [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.4.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ];
        let public = [
            "93.184.216.34",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1111",
        ];
        for ip in internal {
            assert!(is_internal(ip.parse().unwrap()), "{ip}");
        }
        for ip in public {
            assert!(!is_internal(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn an_internal_address_is_reached_only_when_a_rule_names_it() {
        let nobody = |_: IpAddr| false;
        let named = |ip: IpAddr| ip == IpAddr::from([127, 0, 0, 1]);

        // A name that resolves to loopback (as a rebound name would)
        let error = reachable("localhost", 443, nobody).await.unwrap_err();
        assert!(error.contains("internal address"), "{error}");
        assert!(reachable("169.254.169.254", 80, nobody).await.is_err());

        let addrs = reachable("127.0.0.1", 8080, named).await.unwrap();
        assert_eq!(addrs, [SocketAddr::from(([127, 0, 0, 1], 8080))]);
        let public = reachable("93.184.216.34", 443, nobody).await.unwrap();
        assert_eq!(public.len(), 1);
    }
}
