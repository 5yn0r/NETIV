//! Trames et paquets simulés (structurés, pas des octets).

use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::addr::MacAddr;
use crate::ids::TraceId;

/// TTL initial des paquets créés par les hôtes.
pub const DEFAULT_TTL: u8 = 64;

/// Trame Ethernet.
///
/// `trace_id` n'existe pas « sur le fil » : c'est une métadonnée de
/// simulation qui permet de suivre une transaction (voir `TraceId`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    /// MAC source.
    pub src: MacAddr,
    /// MAC destination.
    pub dst: MacAddr,
    /// Étiquette VLAN 802.1Q (`None` = trame non étiquetée).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// Contenu.
    pub payload: EtherPayload,
    /// Transaction à laquelle appartient cette trame.
    pub trace_id: TraceId,
}

/// Contenu d'une trame Ethernet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EtherPayload {
    /// Message ARP.
    Arp(ArpPacket),
    /// Paquet IPv4.
    Ipv4(Ipv4Packet),
}

/// Opération ARP.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArpOp {
    /// « Qui a cette IP ? »
    Request,
    /// « C'est moi, voici ma MAC. »
    Reply,
}

/// Message ARP (Ethernet/IPv4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArpPacket {
    /// Requête ou réponse.
    pub op: ArpOp,
    /// MAC de l'émetteur.
    pub sender_mac: MacAddr,
    /// IP de l'émetteur.
    pub sender_ip: Ipv4Addr,
    /// MAC cherchée (zéro dans une requête).
    pub target_mac: MacAddr,
    /// IP recherchée.
    pub target_ip: Ipv4Addr,
}

/// Paquet IPv4.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4Packet {
    /// IP source.
    pub src: Ipv4Addr,
    /// IP destination.
    pub dst: Ipv4Addr,
    /// Durée de vie restante.
    pub ttl: u8,
    /// Contenu.
    pub payload: IpPayload,
}

impl Ipv4Packet {
    /// `true` si c'est un message d'erreur ICMP (on n'en génère jamais en
    /// réponse à un autre message d'erreur).
    pub fn is_icmp_error(&self) -> bool {
        matches!(&self.payload, IpPayload::Icmp(msg) if msg.is_error())
    }
}

/// Contenu d'un paquet IPv4.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpPayload {
    /// Message ICMP.
    Icmp(IcmpMessage),
    /// Datagramme UDP.
    Udp(UdpDatagram),
    /// Segment TCP.
    Tcp(TcpSegment),
}

/// Protocole de transport (pour les services et les règles).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum L4Proto {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl fmt::Display for L4Proto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            L4Proto::Tcp => "tcp",
            L4Proto::Udp => "udp",
        })
    }
}

/// Datagramme UDP. Le contenu est du texte : la simulation n'a pas besoin
/// d'octets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpDatagram {
    /// Port source.
    pub src_port: u16,
    /// Port destination.
    pub dst_port: u16,
    /// Contenu applicatif.
    pub data: String,
}

/// Drapeaux TCP utilisés par la simulation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpFlags {
    /// Ouverture de connexion.
    pub syn: bool,
    /// Acquittement.
    pub ack: bool,
    /// Fermeture.
    pub fin: bool,
    /// Réinitialisation (port fermé, connexion inconnue).
    pub rst: bool,
}

impl TcpFlags {
    /// SYN seul.
    pub const SYN: TcpFlags = TcpFlags {
        syn: true,
        ack: false,
        fin: false,
        rst: false,
    };
    /// SYN + ACK.
    pub const SYN_ACK: TcpFlags = TcpFlags {
        syn: true,
        ack: true,
        fin: false,
        rst: false,
    };
    /// ACK seul.
    pub const ACK: TcpFlags = TcpFlags {
        syn: false,
        ack: true,
        fin: false,
        rst: false,
    };
    /// FIN + ACK.
    pub const FIN_ACK: TcpFlags = TcpFlags {
        syn: false,
        ack: true,
        fin: true,
        rst: false,
    };
    /// RST + ACK.
    pub const RST_ACK: TcpFlags = TcpFlags {
        syn: false,
        ack: true,
        fin: false,
        rst: true,
    };
}

impl fmt::Display for TcpFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = [
            (self.syn, "SYN"),
            (self.fin, "FIN"),
            (self.rst, "RST"),
            (self.ack, "ACK"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect();
        f.pad(&names.join(","))
    }
}

/// Segment TCP simplifié.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpSegment {
    /// Port source.
    pub src_port: u16,
    /// Port destination.
    pub dst_port: u16,
    /// Numéro de séquence.
    pub seq: u32,
    /// Numéro d'acquittement.
    pub ack: u32,
    /// Drapeaux.
    pub flags: TcpFlags,
    /// Contenu applicatif.
    pub data: String,
}

/// Cause d'un « destination inaccessible ».
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnreachCode {
    /// Aucune route vers le réseau.
    Net,
    /// Aucun service n'écoute sur ce port UDP.
    Port,
}

/// Message ICMP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IcmpMessage {
    /// Requête d'écho (ping).
    EchoRequest {
        /// Identifiant de la session de ping.
        id: u16,
        /// Numéro de séquence.
        seq: u16,
    },
    /// Réponse d'écho.
    EchoReply {
        /// Identifiant de la session de ping.
        id: u16,
        /// Numéro de séquence.
        seq: u16,
    },
    /// Destination inaccessible.
    DestUnreachable {
        /// Cause.
        code: UnreachCode,
        /// Paquet qui n'a pas pu être acheminé.
        original: Box<Ipv4Packet>,
    },
    /// TTL expiré en transit.
    TimeExceeded {
        /// Paquet dont le TTL a expiré.
        original: Box<Ipv4Packet>,
    },
}

impl IcmpMessage {
    /// `true` pour les messages d'erreur.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::DestUnreachable { .. } | Self::TimeExceeded { .. }
        )
    }
}

impl fmt::Display for ArpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ArpOp::Request => "Request",
            ArpOp::Reply => "Reply",
        })
    }
}

impl fmt::Display for IcmpMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EchoRequest { id, seq } => write!(f, "Echo Request (id {id}, seq {seq})"),
            Self::EchoReply { id, seq } => write!(f, "Echo Reply (id {id}, seq {seq})"),
            Self::DestUnreachable { .. } => f.write_str("Destination Unreachable"),
            Self::TimeExceeded { .. } => f.write_str("Time Exceeded"),
        }
    }
}

impl fmt::Display for Frame {
    /// Résumé sur une ligne.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.payload {
            EtherPayload::Arp(a) => match a.op {
                ArpOp::Request => write!(
                    f,
                    "ARP Request : qui a {} ? dis à {}",
                    a.target_ip, a.sender_ip
                ),
                ArpOp::Reply => write!(f, "ARP Reply : {} est à {}", a.sender_ip, a.sender_mac),
            },
            EtherPayload::Ipv4(p) => match &p.payload {
                IpPayload::Icmp(msg) => {
                    write!(f, "ICMP {msg} {} → {} (ttl {})", p.src, p.dst, p.ttl)
                }
                IpPayload::Udp(u) => write!(
                    f,
                    "UDP {}:{} → {}:{} (ttl {})",
                    p.src, u.src_port, p.dst, u.dst_port, p.ttl
                ),
                IpPayload::Tcp(t) => write!(
                    f,
                    "TCP [{}] {}:{} → {}:{} (ttl {})",
                    t.flags, p.src, t.src_port, p.dst, t.dst_port, p.ttl
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn echo(ttl: u8) -> Ipv4Packet {
        Ipv4Packet {
            src: ip("10.0.0.1"),
            dst: ip("10.0.0.2"),
            ttl,
            payload: IpPayload::Icmp(IcmpMessage::EchoRequest { id: 1, seq: 2 }),
        }
    }

    #[test]
    fn icmp_error_detection() {
        assert!(!echo(64).is_icmp_error());
        let err = Ipv4Packet {
            payload: IpPayload::Icmp(IcmpMessage::TimeExceeded {
                original: Box::new(echo(1)),
            }),
            ..echo(64)
        };
        assert!(err.is_icmp_error());
    }

    #[test]
    fn tcp_flags_display() {
        assert_eq!(TcpFlags::SYN.to_string(), "SYN");
        assert_eq!(TcpFlags::SYN_ACK.to_string(), "SYN,ACK");
        assert_eq!(TcpFlags::RST_ACK.to_string(), "RST,ACK");
        assert_eq!(TcpFlags::FIN_ACK.to_string(), "FIN,ACK");
    }

    #[test]
    fn frame_summary() {
        let f = Frame {
            src: MacAddr::from_index(1),
            dst: MacAddr::from_index(2),
            vlan: None,
            payload: EtherPayload::Ipv4(echo(64)),
            trace_id: TraceId(1),
        };
        assert_eq!(
            f.to_string(),
            "ICMP Echo Request (id 1, seq 2) 10.0.0.1 → 10.0.0.2 (ttl 64)"
        );
    }
}
