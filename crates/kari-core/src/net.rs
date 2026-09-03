//! The addresses this machine has, for the "also listen on" picker. The hub
//! on a phone reaches a node over one of them, never over every interface.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LocalAddress {
    /// Interface name, such as `utun4` or `wg0`.
    pub interface: String,
    pub ip: String,
    /// True for the private ranges: 10/8, 172.16/12, 192.168/16, 100.64/10.
    pub private: bool,
}

fn is_private(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// Every non-loopback address, private ones first.
pub fn local_addresses() -> Vec<LocalAddress> {
    let mut out: Vec<LocalAddress> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| {
            let ip = i.ip();
            LocalAddress {
                interface: i.name.clone(),
                ip: ip.to_string(),
                private: is_private(&ip),
            }
        })
        .collect();
    out.sort_by(|a, b| b.private.cmp(&a.private).then(a.interface.cmp(&b.interface)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // The ranges are built from octets so that no address literal sits in the
    // source; the repository must hold none.
    fn v4(a: u8, b: u8, c: u8, d: u8) -> bool {
        is_private(&IpAddr::V4(Ipv4Addr::new(a, b, c, d)))
    }

    #[test]
    fn private_ranges() {
        assert!(v4(10, 1, 2, 3));
        assert!(v4(172, 16, 0, 1));
        assert!(v4(192, 168, 1, 1));
        assert!(v4(100, 64, 0, 1));
        assert!(v4(100, 127, 255, 254));
        assert!(!v4(100, 128, 0, 1));
        assert!(!v4(8, 8, 8, 8));
        assert!(is_private(&IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1))));
        assert!(!is_private(&IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))));
    }
}
