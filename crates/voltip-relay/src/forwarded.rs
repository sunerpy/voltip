//! The client of a connection that comes through a reverse proxy.
//!
//! The per-IP limits (`RelayConfig::join_rate`, `create_rate`, `attach_rate`) count against the
//! client's address. Behind a reverse proxy every TCP peer is the proxy, so the relay reads the
//! address the proxy forwards in `X-Forwarded-For`, and only from a peer listed in
//! `RelayConfig::trusted_proxies`: any client can send the header itself. The rule is nginx's
//! `real_ip_recursive on`: walk the list from the right, skip the trusted proxies, and the first
//! other address is the client.

use std::net::{IpAddr, SocketAddr};

use ipnet::IpNet;

/// The address `peer`'s requests count against. `forwarded_for` holds the `X-Forwarded-For` lines
/// in the order they came (each a comma-separated list, the nearest proxy's entry last).
///
/// A peer that is not a trusted proxy is its own client, whatever it sends. An entry that is not an
/// address ends the walk, and the nearest trusted hop before it stands for the client, as in nginx.
pub fn client_ip<I, S>(peer: IpAddr, forwarded_for: I, trusted: &[IpNet]) -> IpAddr
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let peer = peer.to_canonical();
    if !is_trusted(peer, trusted) {
        return peer;
    }
    let lines: Vec<S> = forwarded_for.into_iter().collect();
    let mut nearest = peer;
    for entry in lines.iter().rev().flat_map(|line| line.as_ref().rsplit(',')) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some(ip) = parse_entry(entry) else { return nearest };
        if !is_trusted(ip, trusted) {
            return ip;
        }
        nearest = ip;
    }
    nearest
}

/// `--trust-proxy`: an address (`127.0.0.1`, `::1`) or a network (`172.31.0.0/16`).
pub fn parse_trusted_proxy(value: &str) -> Result<IpNet, String> {
    let value = value.trim();
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(IpNet::from(ip.to_canonical()));
    }
    value.parse::<IpNet>().map(|net| net.trunc()).map_err(|_| format!("not an IP address or network: {value:?}"))
}

/// An address, also with a port (`203.0.113.7:51234`, `[2001:db8::7]:443`).
fn parse_entry(entry: &str) -> Option<IpAddr> {
    let ip = entry.parse::<IpAddr>().or_else(|_| entry.parse::<SocketAddr>().map(|a| a.ip())).ok()?;
    Some(ip.to_canonical())
}

fn is_trusted(ip: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|net| net.contains(&ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROXY: &str = "127.0.0.1";

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn nets(list: &[&str]) -> Vec<IpNet> {
        list.iter().map(|s| parse_trusted_proxy(s).unwrap()).collect()
    }

    #[test]
    fn a_peer_that_is_not_trusted_is_its_own_client_whatever_it_forwards() {
        let trusted = nets(&[PROXY]);
        assert_eq!(client_ip(ip("198.51.100.9"), ["203.0.113.1"], &trusted), ip("198.51.100.9"));
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1"], &[]), ip(PROXY), "nothing is trusted unless listed");
    }

    #[test]
    fn a_trusted_proxy_names_the_client() {
        let trusted = nets(&[PROXY]);
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1"], &trusted), ip("203.0.113.1"));
        // Entries the client wrote itself sit left of the one the proxy appended and change nothing.
        assert_eq!(client_ip(ip(PROXY), ["10.9.9.9, 192.0.2.1, 203.0.113.1"], &trusted), ip("203.0.113.1"));
        assert_eq!(client_ip(ip(PROXY), Vec::<&str>::new(), &trusted), ip(PROXY), "no header: the proxy itself");
    }

    #[test]
    fn trusted_hops_are_skipped_from_the_right_across_lines() {
        let trusted = nets(&[PROXY, "172.31.0.0/16"]);
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1, 172.31.4.5", "172.31.9.9"], &trusted), ip("203.0.113.1"));
        assert_eq!(client_ip(ip(PROXY), ["172.31.4.5"], &trusted), ip("172.31.4.5"), "every hop trusted: the farthest");
    }

    #[test]
    fn an_entry_that_is_not_an_address_ends_the_walk_at_the_nearest_trusted_hop() {
        let trusted = nets(&[PROXY, "172.31.0.0/16"]);
        assert_eq!(client_ip(ip(PROXY), ["unknown"], &trusted), ip(PROXY));
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1, junk, 172.31.4.5"], &trusted), ip("172.31.4.5"));
        assert_eq!(client_ip(ip(PROXY), ["\u{fffd}"], &trusted), ip(PROXY), "a header that was not text");
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1, "], &trusted), ip("203.0.113.1"), "empty entries are skipped");
    }

    #[test]
    fn ports_and_mapped_addresses_are_read_as_the_address() {
        let trusted = nets(&[PROXY]);
        assert_eq!(client_ip(ip(PROXY), ["203.0.113.1:51234"], &trusted), ip("203.0.113.1"));
        assert_eq!(client_ip(ip(PROXY), ["[2001:db8::7]:443"], &trusted), ip("2001:db8::7"));
        assert_eq!(client_ip(ip(PROXY), ["::ffff:203.0.113.1"], &trusted), ip("203.0.113.1"));
        // A dual-stack listener sees an IPv4 proxy as a mapped address.
        assert_eq!(client_ip(ip("::ffff:127.0.0.1"), ["203.0.113.1"], &trusted), ip("203.0.113.1"));
    }

    #[test]
    fn trusted_proxies_are_addresses_or_networks() {
        assert_eq!(parse_trusted_proxy("127.0.0.1").unwrap().to_string(), "127.0.0.1/32");
        assert_eq!(parse_trusted_proxy(" ::1 ").unwrap().to_string(), "::1/128");
        assert_eq!(parse_trusted_proxy("172.31.7.9/16").unwrap().to_string(), "172.31.0.0/16");
        assert_eq!(parse_trusted_proxy("::ffff:10.0.0.1").unwrap().to_string(), "10.0.0.1/32");
        assert!(parse_trusted_proxy("nginx").is_err());
        assert!(parse_trusted_proxy("10.0.0.0/33").is_err());
    }
}
