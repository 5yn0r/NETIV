//! Équipements, interfaces et échanges entre le moteur et les équipements.
//!
//! Un équipement ne touche jamais au monde : il reçoit un événement et
//! remplit un [`Effects`] ; seul le moteur applique ces effets.

use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use crate::addr::{Ipv4Net, MacAddr};
use crate::ids::{DeviceId, IfaceId, LinkId, TimerId, TraceId};
use crate::ip_node::IpNode;
use crate::packet::Frame;
use crate::switch::Switch;
use crate::time::SimTime;
use crate::trace::TraceKind;

/// Type d'équipement.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// Poste de travail.
    Host,
    /// Serveur (un hôte, avec une autre étiquette).
    Server,
    /// Routeur.
    Router,
    /// Switch de niveau 2.
    Switch,
    /// Pare-feu : un routeur qui filtre (politique par défaut : refuser).
    Firewall,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            DeviceKind::Host => "host",
            DeviceKind::Server => "server",
            DeviceKind::Router => "router",
            DeviceKind::Switch => "switch",
            DeviceKind::Firewall => "firewall",
        })
    }
}

impl FromStr for DeviceKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "host" | "pc" => Ok(DeviceKind::Host),
            "server" | "serveur" => Ok(DeviceKind::Server),
            "router" | "routeur" => Ok(DeviceKind::Router),
            "switch" => Ok(DeviceKind::Switch),
            "firewall" | "fw" | "pare-feu" => Ok(DeviceKind::Firewall),
            _ => Err(format!("type d'équipement inconnu : « {s} »")),
        }
    }
}

/// Interface réseau d'un équipement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// Identifiant, local à l'équipement (index dans `Device::ifaces`).
    pub id: IfaceId,
    /// Nom (`eth0`, `port1`...).
    pub name: String,
    /// Adresse MAC.
    pub mac: MacAddr,
    /// Adresse IP et préfixe.
    pub ip: Option<Ipv4Net>,
    /// Interface activée par l'administrateur.
    pub admin_up: bool,
    /// Interface réservée à l'administration de l'équipement.
    pub management: bool,
    /// Lien branché sur cette interface.
    pub link: Option<LinkId>,
}

/// Comportement d'un équipement.
// Un `Box<IpNode>` compliquerait chaque `match` pour économiser quelques
// centaines d'octets sur une poignée d'équipements.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Behavior {
    /// Hôte ou routeur (pile IP).
    Ip(IpNode),
    /// Switch.
    Switch(Switch),
}

/// Un équipement du réseau.
#[derive(Clone, Debug)]
pub struct Device {
    /// Identifiant.
    pub id: DeviceId,
    /// Nom unique (`PC1`, `R1`...).
    pub name: String,
    /// Type.
    pub kind: DeviceKind,
    /// Rôle dans l'architecture (`user`, `server`, `dmz`, `internet`...). `None` :
    /// déduit du type d'équipement.
    pub role: Option<String>,
    /// Interfaces, indexées par `IfaceId`.
    pub ifaces: Vec<Interface>,
    /// Comportement et état protocolaire.
    pub behavior: Behavior,
}

/// Rôles reconnus.
pub const ROLES: [&str; 7] = [
    "user", "server", "dmz", "internet", "network", "guest", "iot",
];

impl Device {
    /// Rôle effectif : le rôle déclaré, sinon celui que suggère le type.
    pub fn effective_role(&self) -> &str {
        if let Some(r) = &self.role {
            return r;
        }
        match self.kind {
            DeviceKind::Host => "user",
            DeviceKind::Server => "server",
            DeviceKind::Router | DeviceKind::Firewall | DeviceKind::Switch => "network",
        }
    }

    /// Interface par identifiant.
    pub fn iface(&self, id: IfaceId) -> Option<&Interface> {
        self.ifaces.get(id.0 as usize)
    }

    /// Interface par nom.
    pub fn iface_by_name(&self, name: &str) -> Option<&Interface> {
        self.ifaces.iter().find(|i| i.name == name)
    }
}

/// Contexte en lecture seule fourni à un équipement.
pub struct Ctx<'a> {
    /// Date courante.
    pub now: SimTime,
    /// Équipement concerné.
    pub device: DeviceId,
    /// Interfaces de l'équipement.
    pub ifaces: &'a [Interface],
}

impl Ctx<'_> {
    /// Interface par identifiant.
    pub fn iface(&self, id: IfaceId) -> Option<&Interface> {
        self.ifaces.get(id.0 as usize)
    }

    /// `true` si `ip` est l'adresse d'une de nos interfaces.
    pub fn is_local_ip(&self, ip: Ipv4Addr) -> bool {
        self.ifaces
            .iter()
            .any(|i| i.ip.is_some_and(|n| n.addr() == ip))
    }
}

/// Commande utilisateur adressée à un équipement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Envoie `count` requêtes d'écho vers `dst`, une par seconde simulée.
    Ping {
        /// Destination.
        dst: Ipv4Addr,
        /// Nombre de requêtes.
        count: u16,
    },
    /// Ouvre une connexion TCP, envoie une requête et attend la réponse.
    Curl {
        /// Serveur.
        dst: Ipv4Addr,
        /// Port TCP.
        port: u16,
    },
    /// Envoie un datagramme UDP et attend une réponse.
    SendUdp {
        /// Destination.
        dst: Ipv4Addr,
        /// Port UDP.
        port: u16,
        /// Contenu.
        data: String,
    },
    /// Demande (ou redemande) une adresse par DHCP.
    DhcpRenew {
        /// Interface ; `None` : toutes les interfaces DHCP.
        iface: Option<IfaceId>,
        /// `true` : oublie d'abord l'adresse actuelle (démarrage à froid).
        fresh: bool,
    },
    /// Rend l'adresse obtenue par DHCP.
    DhcpRelease {
        /// Interface ; `None` : toutes les interfaces DHCP.
        iface: Option<IfaceId>,
    },
    /// Résout un nom par DNS, puis lance éventuellement une commande vers
    /// l'adresse trouvée.
    Resolve {
        /// Nom à résoudre.
        name: String,
        /// Suite à donner.
        then: crate::dns::AfterResolve,
    },
}

/// Ce qu'un équipement demande au moteur de faire.
#[derive(Clone, Debug)]
pub enum Effect {
    /// Émettre une trame par une interface.
    Send {
        /// Interface d'émission.
        iface: IfaceId,
        /// Trame.
        frame: Frame,
    },
    /// Programmer l'expiration d'un timer.
    SetTimer {
        /// Délai.
        after: SimTime,
        /// Timer (propre à l'équipement).
        timer: TimerId,
    },
    /// Consigner un événement de trace.
    Trace {
        /// Transaction concernée.
        trace_id: Option<TraceId>,
        /// Contenu.
        kind: TraceKind,
    },
    /// Changer l'adresse d'une de ses interfaces (client DHCP).
    ConfigureIface {
        /// Interface.
        iface: IfaceId,
        /// Nouvelle adresse (`None` : aucune).
        ip: Option<crate::addr::Ipv4Net>,
    },
}

/// Liste d'effets, avec un allocateur d'identifiants de transaction.
#[derive(Debug)]
pub struct Effects {
    list: Vec<Effect>,
    next_trace: u64,
}

impl Effects {
    /// Crée une liste vide ; `next_trace` est le prochain `TraceId` libre.
    pub fn new(next_trace: u64) -> Self {
        Effects {
            list: Vec::new(),
            next_trace,
        }
    }

    /// Alloue un nouvel identifiant de transaction.
    pub fn new_trace_id(&mut self) -> TraceId {
        let id = TraceId(self.next_trace);
        self.next_trace += 1;
        id
    }

    /// Prochain `TraceId` libre (à rendre au moteur).
    pub fn next_trace(&self) -> u64 {
        self.next_trace
    }

    /// Demande l'envoi d'une trame.
    pub fn send(&mut self, iface: IfaceId, frame: Frame) {
        self.list.push(Effect::Send { iface, frame });
    }

    /// Demande un timer.
    pub fn set_timer(&mut self, after: SimTime, timer: TimerId) {
        self.list.push(Effect::SetTimer { after, timer });
    }

    /// Consigne un événement de trace.
    pub fn trace(&mut self, trace_id: Option<TraceId>, kind: TraceKind) {
        self.list.push(Effect::Trace { trace_id, kind });
    }

    /// Demande le changement d'adresse d'une interface.
    pub fn configure_iface(&mut self, iface: IfaceId, ip: Option<crate::addr::Ipv4Net>) {
        self.list.push(Effect::ConfigureIface { iface, ip });
    }

    /// Consomme la liste.
    pub fn into_vec(self) -> Vec<Effect> {
        self.list
    }

    /// Effets accumulés (utile aux tests).
    pub fn as_slice(&self) -> &[Effect] {
        &self.list
    }
}
