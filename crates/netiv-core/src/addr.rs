//! Adresses de couche 2 et 3.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::NetivError;

/// Adresse MAC (48 bits).
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// Adresse de broadcast `FF:FF:FF:FF:FF:FF`.
    pub const BROADCAST: MacAddr = MacAddr([0xFF; 6]);

    /// Génère une MAC déterministe à partir d'un compteur.
    ///
    /// Préfixe `02:4E:49` : bit « localement administré » positionné, bit
    /// multicast à zéro, donc jamais en conflit avec un vrai constructeur.
    pub const fn from_index(index: u32) -> Self {
        let b = index.to_be_bytes();
        MacAddr([0x02, 0x4E, 0x49, b[1], b[2], b[3]])
    }

    /// `true` pour `FF:FF:FF:FF:FF:FF`.
    pub fn is_broadcast(self) -> bool {
        self == Self::BROADCAST
    }

    /// `true` si le bit multicast est positionné (le broadcast en fait partie).
    pub fn is_multicast(self) -> bool {
        self.0[0] & 0x01 == 1
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        f.pad(&format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        ))
    }
}

impl FromStr for MacAddr {
    type Err = NetivError;

    /// Accepte `AA:BB:CC:DD:EE:FF` (séparateur `:` ou `-`, casse libre).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || NetivError::InvalidMac(s.to_string());
        let mut out = [0u8; 6];
        let mut parts = s.split([':', '-']);
        for byte in &mut out {
            let part = parts.next().ok_or_else(err)?;
            if part.len() != 2 {
                return Err(err());
            }
            *byte = u8::from_str_radix(part, 16).map_err(|_| err())?;
        }
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(MacAddr(out))
    }
}

/// Adresse IPv4 d'interface avec son préfixe : `192.168.1.10/24`.
///
/// Conserve l'adresse *hôte* (pas seulement le réseau), comme la
/// configuration d'une interface réelle.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Ipv4Net {
    addr: Ipv4Addr,
    prefix: u8,
}

impl Ipv4Net {
    /// Crée une adresse avec préfixe ; `prefix` doit être dans `0..=32`.
    pub fn new(addr: Ipv4Addr, prefix: u8) -> Result<Self, NetivError> {
        if prefix > 32 {
            return Err(NetivError::InvalidPrefix(prefix));
        }
        Ok(Ipv4Net { addr, prefix })
    }

    /// Adresse de l'hôte (`192.168.1.10`).
    pub fn addr(self) -> Ipv4Addr {
        self.addr
    }

    /// Longueur du préfixe (`24`).
    pub fn prefix(self) -> u8 {
        self.prefix
    }

    /// Masque de sous-réseau (`255.255.255.0`).
    pub fn mask(self) -> Ipv4Addr {
        Ipv4Addr::from(self.mask_bits())
    }

    /// Adresse du réseau (`192.168.1.0`).
    pub fn network(self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) & self.mask_bits())
    }

    /// Adresse de broadcast du réseau (`192.168.1.255`).
    pub fn broadcast(self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.addr) | !self.mask_bits())
    }

    /// Le même réseau, avec l'adresse hôte remplacée par l'adresse réseau.
    /// Utile pour les destinations de table de routage.
    pub fn trunc(self) -> Ipv4Net {
        Ipv4Net {
            addr: self.network(),
            prefix: self.prefix,
        }
    }

    /// `ip` appartient-elle à ce sous-réseau ?
    pub fn contains(self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & self.mask_bits() == u32::from(self.network())
    }

    fn mask_bits(self) -> u32 {
        // `<< 32` déborderait : /0 est traité à part.
        if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(self.prefix))
        }
    }
}

impl fmt::Display for Ipv4Net {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl FromStr for Ipv4Net {
    type Err = NetivError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, prefix) = s
            .split_once('/')
            .ok_or_else(|| NetivError::InvalidCidr(s.to_string()))?;
        let addr = addr
            .parse::<Ipv4Addr>()
            .map_err(|_| NetivError::InvalidIpv4(addr.to_string()))?;
        let prefix = prefix
            .parse::<u8>()
            .map_err(|_| NetivError::InvalidCidr(s.to_string()))?;
        Ipv4Net::new(addr, prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(s: &str) -> Ipv4Net {
        s.parse().expect("CIDR de test valide")
    }

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().expect("IPv4 de test valide")
    }

    // ---- MacAddr ----

    #[test]
    fn mac_roundtrip() {
        let mac: MacAddr = "aa:bb:cc:dd:ee:01".parse().unwrap();
        assert_eq!(mac.to_string(), "AA:BB:CC:DD:EE:01");
        assert_eq!(mac.to_string().parse::<MacAddr>().unwrap(), mac);
    }

    #[test]
    fn mac_accepts_dash_separator() {
        assert!(
            "FF-FF-FF-FF-FF-FF"
                .parse::<MacAddr>()
                .unwrap()
                .is_broadcast()
        );
    }

    #[test]
    fn mac_rejects_garbage() {
        for bad in [
            "",
            "AA:BB",
            "AA:BB:CC:DD:EE:FF:00",
            "GG:BB:CC:DD:EE:FF",
            "A:BB:CC:DD:EE:FF",
        ] {
            assert!(bad.parse::<MacAddr>().is_err(), "{bad:?} aurait dû échouer");
        }
    }

    #[test]
    fn mac_from_index_is_deterministic_unicast() {
        let a = MacAddr::from_index(1);
        assert_eq!(a, MacAddr::from_index(1));
        assert_ne!(a, MacAddr::from_index(2));
        assert_eq!(a.to_string(), "02:4E:49:00:00:01");
        assert!(!a.is_multicast());
    }

    #[test]
    fn broadcast_is_multicast() {
        assert!(MacAddr::BROADCAST.is_broadcast());
        assert!(MacAddr::BROADCAST.is_multicast());
    }

    // ---- Ipv4Net ----

    #[test]
    fn parse_and_display() {
        let n = net("192.168.1.10/24");
        assert_eq!(n.addr(), ip("192.168.1.10"));
        assert_eq!(n.prefix(), 24);
        assert_eq!(n.to_string(), "192.168.1.10/24");
    }

    #[test]
    fn mask_network_broadcast() {
        let n = net("192.168.1.10/24");
        assert_eq!(n.mask(), ip("255.255.255.0"));
        assert_eq!(n.network(), ip("192.168.1.0"));
        assert_eq!(n.broadcast(), ip("192.168.1.255"));
        assert_eq!(n.trunc().to_string(), "192.168.1.0/24");
    }

    #[test]
    fn point_to_point_slash_30() {
        let n = net("10.0.0.1/30");
        assert_eq!(n.network(), ip("10.0.0.0"));
        assert_eq!(n.broadcast(), ip("10.0.0.3"));
    }

    #[test]
    fn prefix_edge_cases() {
        let all = net("1.2.3.4/0");
        assert_eq!(all.mask(), ip("0.0.0.0"));
        assert!(all.contains(ip("200.1.1.1")));

        let host = net("1.2.3.4/32");
        assert_eq!(host.mask(), ip("255.255.255.255"));
        assert!(host.contains(ip("1.2.3.4")));
        assert!(!host.contains(ip("1.2.3.5")));
    }

    #[test]
    fn contains() {
        let n = net("192.168.1.10/24");
        assert!(n.contains(ip("192.168.1.200")));
        assert!(!n.contains(ip("192.168.2.1")));
    }

    #[test]
    fn wrong_mask_changes_locality() {
        // Le scénario pédagogique du cahier des charges : /16 au lieu de /24.
        let target = ip("192.168.2.10");
        assert!(!net("192.168.1.10/24").contains(target));
        assert!(net("192.168.1.10/16").contains(target));
    }

    #[test]
    fn rejects_invalid_cidr() {
        assert!(matches!(
            "192.168.1.10".parse::<Ipv4Net>(),
            Err(NetivError::InvalidCidr(_))
        ));
        assert!(matches!(
            "192.168.1.10/".parse::<Ipv4Net>(),
            Err(NetivError::InvalidCidr(_))
        ));
        assert!(matches!(
            "192.168.1.10/abc".parse::<Ipv4Net>(),
            Err(NetivError::InvalidCidr(_))
        ));
        assert!(matches!(
            "300.1.1.1/24".parse::<Ipv4Net>(),
            Err(NetivError::InvalidIpv4(_))
        ));
        assert_eq!(
            "10.0.0.1/33".parse::<Ipv4Net>(),
            Err(NetivError::InvalidPrefix(33))
        );
    }
}

macro_rules! serde_as_string {
    ($t:ty) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let text = String::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}
serde_as_string!(MacAddr);
serde_as_string!(Ipv4Net);

#[cfg(test)]
mod serde_tests {
    use super::*;

    #[test]
    fn addresses_serialize_as_readable_strings() {
        let mac = MacAddr::from_index(1);
        let net: Ipv4Net = "10.0.0.1/24".parse().unwrap();
        assert_eq!(
            serde_json::to_string(&mac).unwrap(),
            "\"02:4E:49:00:00:01\""
        );
        assert_eq!(serde_json::to_string(&net).unwrap(), "\"10.0.0.1/24\"");
        assert_eq!(
            serde_json::from_str::<MacAddr>("\"02:4E:49:00:00:01\"").unwrap(),
            mac
        );
        assert_eq!(
            serde_json::from_str::<Ipv4Net>("\"10.0.0.1/24\"").unwrap(),
            net
        );
        assert!(serde_json::from_str::<Ipv4Net>("\"nope\"").is_err());
    }
}
