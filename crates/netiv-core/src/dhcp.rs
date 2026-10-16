//! DHCP : messages, plages d'adresses et baux.
//!
//! Le protocole suit le cycle DORA (Discover, Offer, Request, Ack) de la
//! RFC 2131, simplifié :
//! * les messages de diffusion vont à 255.255.255.255 ; un serveur répond
//!   lui aussi en diffusion, ou en unicast au relais (`giaddr`) ;
//! * pas de renouvellement périodique (T1/T2) : un bail dure toute la
//!   session, sauf `release`. Une simulation sans timers récurrents finit
//!   toujours par s'arrêter, ce qui garde `run_until_idle` utilisable ;
//! * l'attribution est déterministe : la même MAC reçoit la même adresse, et
//!   une nouvelle MAC reçoit la première adresse libre de la plage.
//!
//! La logique (client, serveur, relais) est dans `ip_node::dhcp`.

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::addr::{Ipv4Net, MacAddr};
use crate::time::SimTime;

/// Port UDP du serveur.
pub const SERVER_PORT: u16 = 67;
/// Port UDP du client.
pub const CLIENT_PORT: u16 = 68;
/// Durée de bail par défaut (24 h).
pub const DEFAULT_LEASE_S: u32 = 86_400;
/// Nombre de tentatives d'un client avant abandon.
pub const MAX_ATTEMPTS: u8 = 4;
/// Attente d'une réponse à la première tentative ; doublée à chaque nouvel essai.
pub const RETRY_BASE: SimTime = SimTime::from_millis(2_000);
/// Durée pendant laquelle une adresse offerte reste réservée au client.
pub const OFFER_HOLD: SimTime = SimTime::from_millis(10_000);

/// Type de message DHCP.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DhcpOp {
    /// Le client cherche un serveur (diffusion).
    Discover,
    /// Un serveur propose une adresse.
    Offer,
    /// Le client demande l'adresse proposée par un serveur précis.
    Request,
    /// Le serveur confirme : le bail commence.
    Ack,
    /// Le serveur refuse la demande.
    Nak,
    /// Le client rend son adresse.
    Release,
}

impl DhcpOp {
    /// Message envoyé par un client (et non par un serveur).
    pub fn from_client(self) -> bool {
        matches!(self, DhcpOp::Discover | DhcpOp::Request | DhcpOp::Release)
    }
}

impl fmt::Display for DhcpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            DhcpOp::Discover => "Discover",
            DhcpOp::Offer => "Offer",
            DhcpOp::Request => "Request",
            DhcpOp::Ack => "Ack",
            DhcpOp::Nak => "Nak",
            DhcpOp::Release => "Release",
        })
    }
}

/// Un message DHCP (les champs utiles à la simulation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DhcpMessage {
    /// Type.
    pub op: DhcpOp,
    /// Identifiant de l'échange, choisi par le client.
    pub xid: u32,
    /// MAC du client.
    pub client_mac: MacAddr,
    /// Adresse actuelle du client (`Release`).
    pub ciaddr: Option<Ipv4Addr>,
    /// Adresse attribuée (`Offer`, `Ack`).
    pub yiaddr: Option<Ipv4Addr>,
    /// Adresse du relais qui a transmis le message.
    pub giaddr: Option<Ipv4Addr>,
    /// Serveur choisi (`Request`) ou serveur émetteur (`Offer`, `Ack`).
    pub server_id: Option<Ipv4Addr>,
    /// Adresse demandée (`Request`).
    pub requested: Option<Ipv4Addr>,
    /// Longueur du masque attribué.
    pub prefix: Option<u8>,
    /// Passerelle par défaut proposée.
    pub gateway: Option<Ipv4Addr>,
    /// Serveur DNS proposé.
    pub dns: Option<Ipv4Addr>,
    /// Durée du bail, en secondes.
    pub lease_s: u32,
}

impl DhcpMessage {
    /// Message vide d'un type donné.
    pub fn new(op: DhcpOp, xid: u32, client_mac: MacAddr) -> Self {
        DhcpMessage {
            op,
            xid,
            client_mac,
            ciaddr: None,
            yiaddr: None,
            giaddr: None,
            server_id: None,
            requested: None,
            prefix: None,
            gateway: None,
            dns: None,
            lease_s: 0,
        }
    }
}

impl fmt::Display for DhcpMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DHCP {} (xid {:#x}, client {})",
            self.op, self.xid, self.client_mac
        )?;
        if let Some(ip) = self.yiaddr {
            write!(f, " adresse {ip}")?;
            if let Some(p) = self.prefix {
                write!(f, "/{p}")?;
            }
        }
        if let Some(ip) = self.requested {
            write!(f, " demande {ip}")?;
        }
        if let Some(ip) = self.server_id {
            write!(f, " serveur {ip}")?;
        }
        if let Some(ip) = self.giaddr {
            write!(f, " relais {ip}")?;
        }
        Ok(())
    }
}

/// Une plage d'adresses distribuée par un serveur.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DhcpPool {
    /// Réseau servi (choisi d'après l'interface d'arrivée ou le relais).
    pub network: Ipv4Net,
    /// Première adresse distribuée.
    pub first: Ipv4Addr,
    /// Dernière adresse distribuée.
    pub last: Ipv4Addr,
    /// Passerelle annoncée aux clients.
    pub gateway: Option<Ipv4Addr>,
    /// Serveur DNS annoncé aux clients.
    pub dns: Option<Ipv4Addr>,
    /// Durée des baux, en secondes.
    pub lease_s: u32,
}

impl DhcpPool {
    /// L'adresse fait-elle partie de la plage ?
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        (u32::from(self.first)..=u32::from(self.last)).contains(&u32::from(ip))
    }

    /// Nombre d'adresses de la plage.
    pub fn size(&self) -> u32 {
        u32::from(self.last).saturating_sub(u32::from(self.first)) + 1
    }

    /// `first-last`, comme dans les fichiers.
    pub fn range_text(&self) -> String {
        format!("{}-{}", self.first, self.last)
    }
}

impl fmt::Display for DhcpPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} : {}", self.network.trunc(), self.range_text())?;
        if let Some(g) = self.gateway {
            write!(f, ", passerelle {g}")?;
        }
        if let Some(d) = self.dns {
            write!(f, ", DNS {d}")?;
        }
        Ok(())
    }
}

/// Un bail (ou une offre en attente de confirmation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// Client.
    pub mac: MacAddr,
    /// Fin du bail (ou de la réservation pour une offre).
    pub expires: SimTime,
    /// `false` : simple offre, pas encore confirmée par le client.
    pub bound: bool,
}

/// Un serveur DHCP : sa configuration et ses baux.
///
/// Les baux font partie de l'état durable du serveur (comme sa base de
/// données) : ils survivent à `reset_runtime`, sinon une copie neuve du
/// réseau pourrait redistribuer une adresse déjà utilisée.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DhcpServer {
    /// Le service tourne-t-il ?
    pub enabled: bool,
    /// Plages servies.
    pub pools: Vec<DhcpPool>,
    /// Baux et offres, par adresse.
    pub leases: BTreeMap<Ipv4Addr, Lease>,
}

impl DhcpServer {
    /// Serveur actif avec ces plages, sans aucun bail.
    pub fn new(pools: Vec<DhcpPool>) -> Self {
        DhcpServer {
            enabled: true,
            pools,
            leases: BTreeMap::new(),
        }
    }

    /// Plage qui sert l'adresse `subnet_hint` (adresse du relais ou de
    /// l'interface d'arrivée).
    pub fn pool_for(&self, subnet_hint: Ipv4Addr) -> Option<usize> {
        self.pools
            .iter()
            .position(|p| p.network.contains(subnet_hint))
    }

    fn usable(&self, ip: Ipv4Addr, mac: MacAddr, now: SimTime) -> bool {
        match self.leases.get(&ip) {
            None => true,
            Some(l) => l.mac == mac || l.expires <= now,
        }
    }

    /// Adresse à proposer à `mac` dans la plage `pool` : son adresse actuelle
    /// s'il en a une, sinon la première libre. La réserve pour `OFFER_HOLD`.
    pub fn offer(&mut self, pool: usize, mac: MacAddr, now: SimTime) -> Option<Ipv4Addr> {
        let p = self.pools.get(pool)?.clone();
        let current = self
            .leases
            .iter()
            .find(|(ip, l)| l.mac == mac && p.contains(**ip))
            .map(|(ip, _)| *ip);
        let ip = current.or_else(|| {
            (u32::from(p.first)..=u32::from(p.last))
                .map(Ipv4Addr::from)
                .find(|ip| self.usable(*ip, mac, now))
        })?;
        let entry = self.leases.entry(ip).or_insert(Lease {
            mac,
            expires: now,
            bound: false,
        });
        if !entry.bound || entry.mac != mac {
            *entry = Lease {
                mac,
                expires: now.saturating_add(OFFER_HOLD),
                bound: false,
            };
        }
        Some(ip)
    }

    /// Confirme le bail de `ip` pour `mac`. `false` : adresse prise ou hors plage.
    pub fn commit(&mut self, ip: Ipv4Addr, mac: MacAddr, now: SimTime) -> Option<&DhcpPool> {
        let idx = self.pools.iter().position(|p| p.contains(ip))?;
        if !self.usable(ip, mac, now) {
            return None;
        }
        let lease_s = self.pools[idx].lease_s;
        self.leases.insert(
            ip,
            Lease {
                mac,
                expires: now.saturating_add(SimTime::from_millis(u64::from(lease_s) * 1_000)),
                bound: true,
            },
        );
        self.pools.get(idx)
    }

    /// Libère l'adresse de `mac` (offre déclinée ou `Release`).
    pub fn release(&mut self, ip: Ipv4Addr, mac: MacAddr) -> bool {
        if self.leases.get(&ip).is_some_and(|l| l.mac == mac) {
            self.leases.remove(&ip);
            return true;
        }
        false
    }

    /// Oublie les offres non confirmées (état dynamique).
    pub fn forget_offers(&mut self) {
        self.leases.retain(|_, l| l.bound);
    }

    /// Baux confirmés et encore valides à `now`, dans la plage `pool`.
    pub fn bound_in(&self, pool: usize, now: SimTime) -> usize {
        let Some(p) = self.pools.get(pool) else {
            return 0;
        };
        self.leases
            .iter()
            .filter(|(ip, l)| l.bound && l.expires > now && p.contains(**ip))
            .count()
    }
}

/// État du client DHCP d'une interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// Discover envoyé, en attente d'une offre.
    Selecting {
        /// Identifiant de l'échange.
        xid: u32,
        /// Numéro de la tentative (à partir de 1).
        attempt: u8,
    },
    /// Request envoyé, en attente de l'Ack.
    Requesting {
        /// Identifiant de l'échange.
        xid: u32,
        /// Numéro de la tentative.
        attempt: u8,
        /// L'offre retenue.
        offer: Box<DhcpMessage>,
    },
    /// Adresse obtenue.
    Bound(Binding),
    /// Aucune réponse après toutes les tentatives.
    Failed,
}

/// Ce qu'un client a obtenu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Adresse et masque.
    pub ip: Ipv4Net,
    /// Serveur qui a attribué l'adresse.
    pub server: Ipv4Addr,
    /// Passerelle reçue.
    pub gateway: Option<Ipv4Addr>,
    /// Serveur DNS reçu.
    pub dns: Option<Ipv4Addr>,
    /// Durée du bail.
    pub lease_s: u32,
}

/// Événement DHCP consigné dans la trace (côté client, serveur ou relais).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DhcpEvent {
    /// Le client cherche un serveur.
    ClientDiscover {
        /// Tentative (à partir de 1).
        attempt: u8,
    },
    /// Le client accepte une offre et la demande.
    ClientRequest {
        /// Adresse demandée.
        ip: Ipv4Addr,
        /// Serveur choisi.
        server: Ipv4Addr,
    },
    /// Le client a obtenu une adresse.
    ClientBound(Binding),
    /// Le serveur a refusé : le client recommence.
    ClientNak,
    /// Aucune réponse : le client abandonne.
    ClientFailed {
        /// Tentatives effectuées.
        attempts: u8,
    },
    /// Le client rend son adresse.
    ClientReleased {
        /// Adresse rendue.
        ip: Ipv4Addr,
    },
    /// Le serveur propose une adresse.
    ServerOffer {
        /// Adresse proposée.
        ip: Ipv4Addr,
        /// Client.
        mac: MacAddr,
    },
    /// Le serveur confirme un bail.
    ServerAck {
        /// Adresse attribuée.
        ip: Ipv4Addr,
        /// Client.
        mac: MacAddr,
    },
    /// Le serveur refuse une demande.
    ServerNak {
        /// Adresse demandée.
        ip: Ipv4Addr,
        /// Client.
        mac: MacAddr,
    },
    /// Le serveur n'a plus d'adresse libre dans la plage.
    ServerExhausted {
        /// Réseau de la plage.
        network: Ipv4Net,
    },
    /// Le serveur n'a aucune plage pour ce réseau.
    ServerNoPool {
        /// Adresse qui désigne le réseau du client.
        subnet: Ipv4Addr,
    },
    /// Le serveur libère un bail.
    ServerReleased {
        /// Adresse libérée.
        ip: Ipv4Addr,
    },
    /// Un relais transmet une demande au serveur.
    Relayed {
        /// Serveur visé.
        to: Ipv4Addr,
    },
    /// Un relais rediffuse la réponse du serveur sur le réseau du client.
    RelayedBack {
        /// Réponse transmise.
        op: DhcpOp,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn server(first: &str, last: &str) -> DhcpServer {
        DhcpServer::new(vec![DhcpPool {
            network: "10.0.1.0/24".parse().unwrap(),
            first: ip(first),
            last: ip(last),
            gateway: Some(ip("10.0.1.1")),
            dns: None,
            lease_s: 60,
        }])
    }

    #[test]
    fn same_mac_same_address_and_first_free_for_others() {
        let mut s = server("10.0.1.100", "10.0.1.110");
        let (a, b) = (MacAddr::from_index(1), MacAddr::from_index(2));
        let t = SimTime::ZERO;
        assert_eq!(s.offer(0, a, t), Some(ip("10.0.1.100")));
        assert!(s.commit(ip("10.0.1.100"), a, t).is_some());
        assert_eq!(s.offer(0, b, t), Some(ip("10.0.1.101")));
        assert_eq!(
            s.offer(0, a, t),
            Some(ip("10.0.1.100")),
            "même MAC → même adresse"
        );
        // b ne peut pas prendre l'adresse de a.
        assert!(s.commit(ip("10.0.1.100"), b, t).is_none());
    }

    #[test]
    fn exhausted_pool_and_expired_offers() {
        let mut s = server("10.0.1.100", "10.0.1.100");
        let (a, b) = (MacAddr::from_index(1), MacAddr::from_index(2));
        assert!(s.offer(0, a, SimTime::ZERO).is_some());
        assert_eq!(
            s.offer(0, b, SimTime::ZERO),
            None,
            "plage d'une adresse, déjà offerte"
        );
        // L'offre non confirmée expire.
        let later = SimTime::ZERO.saturating_add(OFFER_HOLD);
        assert_eq!(s.offer(0, b, later), Some(ip("10.0.1.100")));
        s.forget_offers();
        assert!(s.leases.is_empty());
    }

    #[test]
    fn pool_selection_and_release() {
        let mut s = server("10.0.1.100", "10.0.1.110");
        assert_eq!(s.pool_for(ip("10.0.1.1")), Some(0));
        assert_eq!(s.pool_for(ip("10.0.2.1")), None);
        let a = MacAddr::from_index(7);
        s.offer(0, a, SimTime::ZERO);
        s.commit(ip("10.0.1.100"), a, SimTime::ZERO);
        assert_eq!(s.bound_in(0, SimTime::ZERO), 1);
        assert!(!s.release(ip("10.0.1.100"), MacAddr::from_index(8)));
        assert!(s.release(ip("10.0.1.100"), a));
        assert_eq!(s.bound_in(0, SimTime::ZERO), 0);
    }
}
