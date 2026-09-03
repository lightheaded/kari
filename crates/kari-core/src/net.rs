//! The addresses this machine has, and the ones the API listens on now.
//!
//! A hub on a phone cannot open an SSH forward, so it reaches a node over one
//! of the node's private addresses. The listener binds the private addresses
//! only; a public address is never bound. The node advertises what it bound in
//! its identity, and a pairing code carries the list to the phone.

use serde::Serialize;
use std::net::SocketAddr;
use std::sync::RwLock;

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
    out.sort_by(|a, b| {
        b.private
            .cmp(&a.private)
            .then(a.interface.cmp(&b.interface))
    });
    out
}

/// Every private, non-loopback address of this machine, with `port`.
pub fn private_sockets(port: u16) -> Vec<SocketAddr> {
    local_addresses()
        .into_iter()
        .filter(|a| a.private)
        .filter_map(|a| a.ip.parse::<std::net::IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

/// What the API listens on now. The listener writes it; identity reads it.
static BOUND: RwLock<Vec<SocketAddr>> = RwLock::new(Vec::new());

pub fn set_bound(addrs: Vec<SocketAddr>) {
    if let Ok(mut b) = BOUND.write() {
        *b = addrs;
    }
}

/// The bound addresses a remote hub can use: private, never loopback.
pub fn bound_reachable() -> Vec<String> {
    let b = match BOUND.read() {
        Ok(b) => b.clone(),
        Err(_) => return Vec::new(),
    };
    b.into_iter()
        .filter(|a| !a.ip().is_loopback() && is_private(&a.ip()))
        .map(|a| a.to_string())
        .collect()
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
        assert!(is_private(&IpAddr::V6(Ipv6Addr::new(
            0xfd00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_private(&IpAddr::V6(Ipv6Addr::new(
            0x2001, 0xdb8, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn bound_list_drops_loopback_and_public() {
        let port = 47311;
        set_bound(vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)), port),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), port),
        ]);
        let out = bound_reachable();
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].ends_with(":47311"));
        set_bound(Vec::new());
    }
}
