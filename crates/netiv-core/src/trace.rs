//! Trace de la simulation : tout ce que fait le moteur y est consigné.
//!
//! Packet Journey, Packet Microscope, Explain Engine et diagnostic sont des
//! *vues* sur cette trace.

use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::addr::MacAddr;
use crate::ids::{DeviceId, IfaceId, TraceId};
use crate::packet::Frame;
use crate::route::RouteDecision;
use crate::time::SimTime;

/// Pourquoi une trame ou un paquet a été abandonné.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropReason {
    /// L'interface est désactivée.
    InterfaceDown,
    /// Aucun câble n'est branché sur l'interface.
    NoLink,
    /// Le câble est coupé.
    LinkDown,
    /// Perte aléatoire sur le lien.
    LinkLoss,
    /// Aucune route vers la destination.
    NoRoute,
    /// Ni l'adresse MAC ni l'adresse IP ne nous concernent.
    NotForUs,
    /// L'adresse MAC du prochain saut est restée introuvable.
    ArpUnresolved,
    /// Un switch ne renvoie pas une trame vers le port d'où elle vient.
    SameSegment,
    /// Le VLAN de la trame n'est pas autorisé sur ce port.
    VlanMismatch,
    /// Rejeté par une règle de filtrage (voir l'événement `AclDecision`).
    Filtered,
}

impl fmt::Display for DropReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DropReason::InterfaceDown => "interface désactivée",
            DropReason::NoLink => "aucun câble branché",
            DropReason::LinkDown => "câble coupé",
            DropReason::LinkLoss => "perdu sur le lien",
            DropReason::NoRoute => "aucune route",
            DropReason::NotForUs => "pas pour nous",
            DropReason::ArpUnresolved => "adresse MAC du prochain saut introuvable",
            DropReason::SameSegment => "destination sur le port d'arrivée",
            DropReason::VlanMismatch => "VLAN non autorisé sur ce port",
            DropReason::Filtered => "bloqué par le filtrage",
        })
    }
}

/// Pourquoi un switch diffuse une trame sur tous ses ports.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FloodReason {
    /// Destination broadcast ou multicast.
    Broadcast,
    /// Destination unicast pas encore apprise.
    UnknownUnicast,
}

impl fmt::Display for FloodReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FloodReason::Broadcast => "destination broadcast",
            FloodReason::UnknownUnicast => "MAC destination inconnue",
        })
    }
}

/// Erreur ICMP reçue en réponse à un ping.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IcmpErrorKind {
    /// TTL expiré en transit.
    TtlExpired,
    /// Réseau inaccessible.
    NetUnreachable,
    /// Port UDP fermé.
    PortUnreachable,
}

impl fmt::Display for IcmpErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IcmpErrorKind::TtlExpired => "TTL expiré en transit",
            IcmpErrorKind::NetUnreachable => "réseau inaccessible",
            IcmpErrorKind::PortUnreachable => "port inaccessible",
        })
    }
}

/// Résultat d'une requête TCP (`curl`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurlOutcome {
    /// Réponse reçue.
    Ok {
        /// Contenu de la réponse.
        response: String,
    },
    /// Le serveur a répondu RST : aucun service n'écoute sur ce port.
    Refused,
    /// Aucune réponse (paquets perdus ou filtrés).
    Timeout,
    /// Erreur ICMP reçue.
    Error(IcmpErrorKind),
}

impl fmt::Display for CurlOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurlOutcome::Ok { response } => write!(f, "« {response} »"),
            CurlOutcome::Refused => {
                f.write_str("connexion refusée (RST) : aucun service sur ce port")
            }
            CurlOutcome::Timeout => f.write_str("délai dépassé : aucune réponse"),
            CurlOutcome::Error(e) => write!(f, "erreur ICMP : {e}"),
        }
    }
}

/// Résultat d'un envoi UDP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UdpOutcome {
    /// Réponse reçue.
    Reply {
        /// Contenu de la réponse.
        data: String,
    },
    /// ICMP « port inaccessible » reçu.
    PortUnreachable,
    /// Aucune réponse.
    Timeout,
    /// Autre erreur ICMP.
    Error(IcmpErrorKind),
}

impl fmt::Display for UdpOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UdpOutcome::Reply { data } => write!(f, "réponse « {data} »"),
            UdpOutcome::PortUnreachable => f.write_str("port inaccessible (ICMP)"),
            UdpOutcome::Timeout => f.write_str("délai dépassé : aucune réponse"),
            UdpOutcome::Error(e) => write!(f, "erreur ICMP : {e}"),
        }
    }
}

/// Ce qui s'est passé.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceKind {
    /// Une commande utilisateur démarre.
    CommandIssued {
        /// Description lisible.
        description: String,
    },
    /// Modification de configuration.
    ConfigChanged {
        /// Description lisible.
        description: String,
    },
    /// Trame émise par une interface.
    FrameSent {
        /// Interface d'émission.
        iface: IfaceId,
        /// La trame.
        frame: Frame,
    },
    /// Trame reçue par une interface.
    FrameReceived {
        /// Interface de réception.
        iface: IfaceId,
        /// La trame.
        frame: Frame,
    },
    /// Trame ou paquet abandonné.
    Dropped {
        /// Raison.
        reason: DropReason,
    },
    /// Un switch apprend (ou déplace) une adresse MAC.
    MacLearned {
        /// Adresse apprise.
        mac: MacAddr,
        /// VLAN dans lequel elle a été vue.
        vlan: u16,
        /// Port où elle a été vue.
        iface: IfaceId,
        /// Ancien port, si l'adresse a changé de place.
        previous: Option<IfaceId>,
    },
    /// Un switch diffuse une trame.
    Flood {
        /// Raison.
        reason: FloodReason,
        /// Ports de sortie.
        out_ifaces: Vec<IfaceId>,
    },
    /// Un switch transmet une trame vers un seul port.
    Forward {
        /// Port de sortie.
        out_iface: IfaceId,
    },
    /// Requête ARP émise.
    ArpRequestSent {
        /// IP dont on cherche la MAC.
        target: Ipv4Addr,
        /// Numéro de tentative (à partir de 1).
        attempt: u8,
    },
    /// Réponse ARP émise.
    ArpReplySent {
        /// Demandeur.
        to: Ipv4Addr,
    },
    /// Le cache ARP a été mis à jour.
    ArpCacheUpdated {
        /// IP.
        ip: Ipv4Addr,
        /// MAC associée.
        mac: MacAddr,
    },
    /// Plus de réponse ARP : abandon.
    ArpTimeout {
        /// IP cherchée.
        target: Ipv4Addr,
        /// Paquets abandonnés en attente.
        dropped: usize,
    },
    /// Décision de routage.
    RouteLookup(RouteDecision),
    /// Le routeur décrémente le TTL.
    TtlDecremented {
        /// Ancienne valeur.
        from: u8,
        /// Nouvelle valeur.
        to: u8,
    },
    /// Le TTL arrive à zéro.
    TtlExpired,
    /// Un ping part.
    PingSent {
        /// Destination.
        dst: Ipv4Addr,
        /// Numéro de séquence.
        seq: u16,
    },
    /// Réponse de ping reçue.
    PingReply {
        /// Répondant.
        from: Ipv4Addr,
        /// Numéro de séquence.
        seq: u16,
        /// TTL du paquet reçu.
        ttl: u8,
        /// Temps aller-retour simulé.
        rtt: SimTime,
    },
    /// Erreur ICMP reçue en réponse à un ping.
    PingError {
        /// Émetteur de l'erreur.
        from: Ipv4Addr,
        /// Numéro de séquence.
        seq: u16,
        /// Nature de l'erreur.
        error: IcmpErrorKind,
    },
    /// Aucune réponse au ping.
    PingTimeout {
        /// Destination.
        dst: Ipv4Addr,
        /// Numéro de séquence.
        seq: u16,
    },
    /// Une requête TCP démarre.
    CurlStarted {
        /// Serveur visé.
        dst: Ipv4Addr,
        /// Port visé.
        port: u16,
    },
    /// Fin d'une requête TCP.
    CurlResult {
        /// Serveur visé.
        dst: Ipv4Addr,
        /// Port visé.
        port: u16,
        /// Résultat.
        outcome: CurlOutcome,
    },
    /// Une connexion TCP est établie.
    TcpEstablished {
        /// Machine distante.
        peer: Ipv4Addr,
        /// Port distant.
        peer_port: u16,
        /// Port local.
        local_port: u16,
    },
    /// Un RST est envoyé (port fermé ou connexion inconnue).
    TcpResetSent {
        /// Destinataire.
        to: Ipv4Addr,
        /// Port local concerné.
        port: u16,
    },
    /// Un service traite une requête.
    ServiceRequest {
        /// Client.
        from: Ipv4Addr,
        /// Port du service.
        port: u16,
        /// Nom du service.
        service: String,
    },
    /// Un datagramme UDP part.
    UdpSent {
        /// Destination.
        dst: Ipv4Addr,
        /// Port destination.
        port: u16,
    },
    /// Fin d'un échange UDP.
    UdpResult {
        /// Destination.
        dst: Ipv4Addr,
        /// Port destination.
        port: u16,
        /// Résultat.
        outcome: UdpOutcome,
    },
    /// Une panne est injectée (visible uniquement si l'injection est publique).
    FaultApplied {
        /// Description lisible.
        description: String,
    },
    /// Une panne est réparée.
    FaultRepaired {
        /// Description lisible.
        description: String,
    },
    /// Une règle de filtrage refuse un paquet.
    AclDecision {
        /// Numéro de la règle (`None` : politique par défaut).
        rule: Option<usize>,
        /// Texte de la règle ou de la politique.
        text: String,
    },
}

/// Un événement de trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Numéro global, croissant, égal à l'index dans le journal.
    pub seq: u64,
    /// Date simulée.
    pub time: SimTime,
    /// Équipement concerné.
    pub device: DeviceId,
    /// Transaction concernée, s'il y en a une.
    pub trace_id: Option<TraceId>,
    /// Événement qui a provoqué celui-ci.
    pub caused_by: Option<u64>,
    /// Contenu.
    pub kind: TraceKind,
}

/// Journal des événements de trace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceLog {
    events: Vec<TraceEvent>,
}

impl TraceLog {
    /// Ajoute un événement et retourne son numéro.
    pub(crate) fn push(
        &mut self,
        time: SimTime,
        device: DeviceId,
        trace_id: Option<TraceId>,
        caused_by: Option<u64>,
        kind: TraceKind,
    ) -> u64 {
        let seq = self.events.len() as u64;
        self.events.push(TraceEvent {
            seq,
            time,
            device,
            trace_id,
            caused_by,
            kind,
        });
        seq
    }

    /// Vide le journal.
    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }

    /// Tous les événements.
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    /// Nombre d'événements.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// `true` si le journal est vide.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Événement numéro `seq`.
    pub fn get(&self, seq: u64) -> Option<&TraceEvent> {
        usize::try_from(seq).ok().and_then(|i| self.events.get(i))
    }

    /// Événements à partir de l'index `start` (flux incrémental).
    pub fn since(&self, start: usize) -> &[TraceEvent] {
        self.events.get(start..).unwrap_or(&[])
    }

    /// Événements d'une transaction, dans l'ordre.
    pub fn by_trace(&self, id: TraceId) -> impl Iterator<Item = &TraceEvent> {
        self.events.iter().filter(move |e| e.trace_id == Some(id))
    }

    /// Événements d'un équipement, dans l'ordre.
    pub fn by_device(&self, id: DeviceId) -> impl Iterator<Item = &TraceEvent> {
        self.events.iter().filter(move |e| e.device == id)
    }
}
