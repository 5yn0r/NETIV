//! Les causes racines que le moteur de diagnostic sait reconnaître.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Couche concernée.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Layer {
    /// Physique : câbles, interfaces, qualité du lien.
    L1,
    /// Liaison : VLAN, ARP.
    L2,
    /// Réseau : adressage, routage.
    L3,
    /// Services applicatifs.
    Services,
    /// Filtrage.
    Security,
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Layer::L1 => "couche 1 (physique)",
            Layer::L2 => "couche 2 (liaison)",
            Layer::L3 => "couche 3 (réseau)",
            Layer::Services => "services",
            Layer::Security => "filtrage",
        })
    }
}

/// Nature de la panne identifiée.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CauseKind {
    /// Câble coupé.
    LinkDown,
    /// Interface désactivée.
    InterfaceDown,
    /// Perte de paquets sur un lien.
    PacketLoss,
    /// Latence anormale.
    HighLatency,
    /// VLAN incohérent.
    WrongVlan,
    /// Entrée ARP statique erronée.
    ArpMismatch,
    /// Mauvaise adresse IP.
    WrongIp,
    /// Mauvais masque.
    WrongMask,
    /// Mauvaise passerelle par défaut.
    WrongGateway,
    /// Route absente.
    MissingRoute,
    /// Route incorrecte.
    WrongRoute,
    /// Boucle de routage.
    RoutingLoop,
    /// Un routeur ne transfère plus.
    ForwardingDisabled,
    /// Service arrêté.
    ServiceDown,
    /// Paquet bloqué par une règle de filtrage.
    Filtered,
    /// Serveur DHCP arrêté ou absent.
    DhcpServerDown,
    /// Relais DHCP absent ou mal configuré.
    DhcpRelayMissing,
    /// Plage DHCP épuisée ou absente pour ce réseau.
    DhcpPoolExhausted,
    /// Serveur DHCP pirate (une machine distribue une mauvaise configuration).
    RogueDhcp,
    /// Mauvais serveur DNS (ou aucun).
    DnsWrongServer,
    /// Serveur DNS arrêté.
    DnsServerDown,
    /// Enregistrement DNS erroné ou absent.
    DnsRecordWrong,
    /// Cause non identifiée.
    Unknown,
}

impl CauseKind {
    /// Couche de la panne.
    pub fn layer(self) -> Layer {
        match self {
            CauseKind::LinkDown
            | CauseKind::InterfaceDown
            | CauseKind::PacketLoss
            | CauseKind::HighLatency => Layer::L1,
            CauseKind::WrongVlan | CauseKind::ArpMismatch => Layer::L2,
            CauseKind::WrongIp
            | CauseKind::WrongMask
            | CauseKind::WrongGateway
            | CauseKind::MissingRoute
            | CauseKind::WrongRoute
            | CauseKind::RoutingLoop
            | CauseKind::ForwardingDisabled
            | CauseKind::DhcpServerDown
            | CauseKind::DhcpRelayMissing
            | CauseKind::DhcpPoolExhausted
            | CauseKind::Unknown => Layer::L3,
            CauseKind::ServiceDown
            | CauseKind::DnsWrongServer
            | CauseKind::DnsServerDown
            | CauseKind::DnsRecordWrong => Layer::Services,
            CauseKind::Filtered | CauseKind::RogueDhcp => Layer::Security,
        }
    }

    /// Nom court (utilisé aussi pour répondre dans un challenge).
    pub fn slug(self) -> &'static str {
        match self {
            CauseKind::LinkDown => "link-down",
            CauseKind::InterfaceDown => "interface-down",
            CauseKind::PacketLoss => "packet-loss",
            CauseKind::HighLatency => "high-latency",
            CauseKind::WrongVlan => "wrong-vlan",
            CauseKind::ArpMismatch => "arp-mismatch",
            CauseKind::WrongIp => "wrong-ip",
            CauseKind::WrongMask => "wrong-mask",
            CauseKind::WrongGateway => "wrong-gateway",
            CauseKind::MissingRoute => "missing-route",
            CauseKind::WrongRoute => "wrong-route",
            CauseKind::RoutingLoop => "routing-loop",
            CauseKind::ForwardingDisabled => "forwarding-disabled",
            CauseKind::ServiceDown => "service-down",
            CauseKind::Filtered => "filtered",
            CauseKind::DhcpServerDown => "dhcp-server-down",
            CauseKind::DhcpRelayMissing => "dhcp-relay-missing",
            CauseKind::DhcpPoolExhausted => "dhcp-pool-exhausted",
            CauseKind::RogueDhcp => "rogue-dhcp",
            CauseKind::DnsWrongServer => "dns-wrong-server",
            CauseKind::DnsServerDown => "dns-server-down",
            CauseKind::DnsRecordWrong => "dns-record-wrong",
            CauseKind::Unknown => "unknown",
        }
    }

    /// Retrouve une cause à partir de son nom court.
    pub fn from_slug(s: &str) -> Option<CauseKind> {
        CauseKind::ALL.iter().copied().find(|k| k.slug() == s)
    }

    /// Toutes les causes (sauf `Unknown`).
    pub const ALL: [CauseKind; 22] = [
        CauseKind::LinkDown,
        CauseKind::InterfaceDown,
        CauseKind::PacketLoss,
        CauseKind::HighLatency,
        CauseKind::WrongVlan,
        CauseKind::ArpMismatch,
        CauseKind::WrongIp,
        CauseKind::WrongMask,
        CauseKind::WrongGateway,
        CauseKind::MissingRoute,
        CauseKind::WrongRoute,
        CauseKind::RoutingLoop,
        CauseKind::ForwardingDisabled,
        CauseKind::ServiceDown,
        CauseKind::Filtered,
        CauseKind::DhcpServerDown,
        CauseKind::DhcpRelayMissing,
        CauseKind::DhcpPoolExhausted,
        CauseKind::RogueDhcp,
        CauseKind::DnsWrongServer,
        CauseKind::DnsServerDown,
        CauseKind::DnsRecordWrong,
    ];

    /// Libellé lisible.
    pub fn label(self) -> &'static str {
        match self {
            CauseKind::LinkDown => "câble coupé",
            CauseKind::InterfaceDown => "interface désactivée",
            CauseKind::PacketLoss => "perte de paquets",
            CauseKind::HighLatency => "latence anormale",
            CauseKind::WrongVlan => "VLAN incohérent",
            CauseKind::ArpMismatch => "entrée ARP erronée",
            CauseKind::WrongIp => "mauvaise adresse IP",
            CauseKind::WrongMask => "mauvais masque de sous-réseau",
            CauseKind::WrongGateway => "mauvaise passerelle par défaut",
            CauseKind::MissingRoute => "route manquante",
            CauseKind::WrongRoute => "route incorrecte",
            CauseKind::RoutingLoop => "boucle de routage",
            CauseKind::ForwardingDisabled => "transfert de paquets désactivé",
            CauseKind::ServiceDown => "service arrêté",
            CauseKind::Filtered => "paquet bloqué par le filtrage",
            CauseKind::DhcpServerDown => "serveur DHCP arrêté ou absent",
            CauseKind::DhcpRelayMissing => "relais DHCP absent ou erroné",
            CauseKind::DhcpPoolExhausted => "plage DHCP épuisée ou absente",
            CauseKind::RogueDhcp => "serveur DHCP pirate",
            CauseKind::DnsWrongServer => "mauvais serveur DNS",
            CauseKind::DnsServerDown => "serveur DNS arrêté",
            CauseKind::DnsRecordWrong => "enregistrement DNS erroné ou absent",
            CauseKind::Unknown => "cause non identifiée",
        }
    }
}

impl fmt::Display for CauseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.label())
    }
}

/// La cause racine d'un problème.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootCause {
    /// Nature.
    pub kind: CauseKind,
    /// Équipement fautif.
    pub device: String,
    /// Détail (interface, route...).
    pub detail: String,
    /// Valeur attendue, si on la connaît.
    pub expected: Option<String>,
    /// Valeur constatée.
    pub actual: Option<String>,
    /// Commande NETIV qui répare.
    pub fix: Option<String>,
    /// Le problème touche le trajet de retour, pas l'aller.
    pub on_return_path: bool,
}

impl RootCause {
    pub(crate) fn new(kind: CauseKind, device: &str, detail: impl Into<String>) -> Self {
        RootCause {
            kind,
            device: device.to_string(),
            detail: detail.into(),
            expected: None,
            actual: None,
            fix: None,
            on_return_path: false,
        }
    }
}
