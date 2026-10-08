//! Filtrage de paquets : règles ordonnées, politique par défaut et suivi de
//! connexion (« stateful »).

use std::collections::BTreeSet;
use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::addr::Ipv4Net;
use crate::ids::IfaceId;
use crate::packet::{IcmpMessage, IpPayload, Ipv4Packet};

/// Décision d'une règle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AclAction {
    /// Laisse passer.
    Allow,
    /// Jette silencieusement (pas de réponse : l'émetteur subit un timeout).
    Deny,
}

impl fmt::Display for AclAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            AclAction::Allow => "allow",
            AclAction::Deny => "deny",
        })
    }
}

/// Protocole visé par une règle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleProto {
    /// Tous les protocoles.
    Any,
    /// ICMP.
    Icmp,
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl fmt::Display for RuleProto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            RuleProto::Any => "any",
            RuleProto::Icmp => "icmp",
            RuleProto::Tcp => "tcp",
            RuleProto::Udp => "udp",
        })
    }
}

/// Une règle de filtrage. Un champ `None` signifie « tout ».
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclRule {
    /// Action.
    pub action: AclAction,
    /// Protocole.
    pub proto: RuleProto,
    /// Réseau source.
    pub src: Option<Ipv4Net>,
    /// Réseau destination.
    pub dst: Option<Ipv4Net>,
    /// Port destination minimal (TCP/UDP).
    pub port_from: Option<u16>,
    /// Port destination maximal (TCP/UDP).
    pub port_to: Option<u16>,
    /// Interface d'entrée.
    pub in_iface: Option<IfaceId>,
    /// Commentaire libre.
    #[serde(default)]
    pub comment: String,
}

impl AclRule {
    /// Règle sans restriction : à compléter.
    pub fn new(action: AclAction, proto: RuleProto) -> Self {
        AclRule {
            action,
            proto,
            src: None,
            dst: None,
            port_from: None,
            port_to: None,
            in_iface: None,
            comment: String::new(),
        }
    }

    /// Description lisible ; `iface_name` traduit un `IfaceId` en nom.
    pub fn describe(&self, iface_name: impl Fn(IfaceId) -> String) -> String {
        let net =
            |n: &Option<Ipv4Net>| n.map_or_else(|| "any".to_string(), |n| n.trunc().to_string());
        let ports = match (self.port_from, self.port_to) {
            (Some(a), Some(b)) if a != b => format!(" ports {a}-{b}"),
            (Some(a), _) => format!(" port {a}"),
            _ => String::new(),
        };
        let iface = self
            .in_iface
            .map_or(String::new(), |i| format!(" depuis {}", iface_name(i)));
        let comment = if self.comment.is_empty() {
            String::new()
        } else {
            format!(" — {}", self.comment)
        };
        format!(
            "{} {} {} → {}{ports}{iface}{comment}",
            self.action,
            self.proto,
            net(&self.src),
            net(&self.dst)
        )
    }

    fn matches(&self, ingress: IfaceId, p: &Ipv4Packet) -> bool {
        if self.in_iface.is_some_and(|i| i != ingress) {
            return false;
        }
        if self.src.is_some_and(|n| !n.contains(p.src))
            || self.dst.is_some_and(|n| !n.contains(p.dst))
        {
            return false;
        }
        let (proto, dport) = match &p.payload {
            IpPayload::Icmp(_) => (RuleProto::Icmp, None),
            IpPayload::Tcp(t) => (RuleProto::Tcp, Some(t.dst_port)),
            IpPayload::Udp(u) => (RuleProto::Udp, Some(u.dst_port)),
        };
        if self.proto != RuleProto::Any && self.proto != proto {
            return false;
        }
        match (self.port_from, self.port_to) {
            (None, None) => true,
            (from, to) => match dport {
                None => false,
                Some(port) => port >= from.unwrap_or(0) && port <= to.or(from).unwrap_or(u16::MAX),
            },
        }
    }
}

/// Verdict du filtrage.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Le paquet passe.
    Allow,
    /// Le paquet est jeté ; `rule` = règle responsable (`None` : politique par défaut).
    Deny {
        /// Numéro de la règle.
        rule: Option<usize>,
    },
}

/// Identifie un flux indépendamment du sens (extrémités ordonnées).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FlowKey {
    proto: u8,
    a: (Ipv4Addr, u16),
    b: (Ipv4Addr, u16),
}

impl FlowKey {
    fn of(p: &Ipv4Packet) -> Option<(FlowKey, bool)> {
        let (proto, sp, dp, is_new) = match &p.payload {
            IpPayload::Tcp(t) => (6, t.src_port, t.dst_port, t.flags.syn && !t.flags.ack),
            IpPayload::Udp(u) => (17, u.src_port, u.dst_port, true),
            IpPayload::Icmp(IcmpMessage::EchoRequest { id, .. }) => (1, *id, *id, true),
            IpPayload::Icmp(IcmpMessage::EchoReply { id, .. }) => (1, *id, *id, false),
            IpPayload::Icmp(_) => return None,
        };
        let (x, y) = ((p.src, sp), (p.dst, dp));
        let (a, b) = if x <= y { (x, y) } else { (y, x) };
        Some((FlowKey { proto, a, b }, is_new))
    }
}

/// Règles de filtrage d'un équipement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acl {
    /// Règles, évaluées dans l'ordre : la première qui correspond gagne.
    pub rules: Vec<AclRule>,
    /// Politique si aucune règle ne correspond.
    pub default: AclAction,
    /// Autorise automatiquement le trafic de retour des flux acceptés.
    pub stateful: bool,
    #[serde(skip)]
    conntrack: BTreeSet<FlowKey>,
}

impl Default for Acl {
    fn default() -> Self {
        Acl {
            rules: Vec::new(),
            default: AclAction::Allow,
            stateful: false,
            conntrack: BTreeSet::new(),
        }
    }
}

impl Acl {
    /// Politique d'un pare-feu : tout refuser, suivi de connexion actif.
    pub fn firewall() -> Self {
        Acl {
            default: AclAction::Deny,
            stateful: true,
            ..Acl::default()
        }
    }

    /// Nombre de flux suivis.
    pub fn tracked_flows(&self) -> usize {
        self.conntrack.len()
    }

    /// Oublie les flux suivis (après un changement de règles).
    pub fn reset_tracking(&mut self) {
        self.conntrack.clear();
    }

    /// Évalue un paquet entrant par `ingress`.
    ///
    /// Les messages d'erreur ICMP sont toujours acceptés (ils portent le
    /// diagnostic d'un flux déjà autorisé).
    pub fn check(&mut self, ingress: IfaceId, p: &Ipv4Packet) -> Verdict {
        let Some((key, is_new)) = FlowKey::of(p) else {
            return Verdict::Allow;
        };
        if self.stateful && self.conntrack.contains(&key) {
            return Verdict::Allow;
        }
        let verdict = match self.rules.iter().position(|r| r.matches(ingress, p)) {
            Some(i) => match self.rules[i].action {
                AclAction::Allow => Verdict::Allow,
                AclAction::Deny => Verdict::Deny { rule: Some(i) },
            },
            None => match self.default {
                AclAction::Allow => Verdict::Allow,
                AclAction::Deny => Verdict::Deny { rule: None },
            },
        };
        if verdict == Verdict::Allow && self.stateful && is_new {
            self.conntrack.insert(key);
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{TcpFlags, TcpSegment, UdpDatagram};

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn tcp(src: &str, sp: u16, dst: &str, dp: u16, flags: TcpFlags) -> Ipv4Packet {
        Ipv4Packet {
            src: ip(src),
            dst: ip(dst),
            ttl: 64,
            payload: IpPayload::Tcp(TcpSegment {
                src_port: sp,
                dst_port: dp,
                seq: 0,
                ack: 0,
                flags,
                data: String::new(),
            }),
        }
    }

    fn udp(src: &str, dst: &str, dp: u16) -> Ipv4Packet {
        Ipv4Packet {
            src: ip(src),
            dst: ip(dst),
            ttl: 64,
            payload: IpPayload::Udp(UdpDatagram {
                src_port: 5000,
                dst_port: dp,
                data: String::new(),
            }),
        }
    }

    fn rule(action: AclAction, proto: RuleProto) -> AclRule {
        AclRule::new(action, proto)
    }

    #[test]
    fn default_allow_with_no_rules() {
        let mut acl = Acl::default();
        assert_eq!(
            acl.check(IfaceId(0), &udp("1.1.1.1", "2.2.2.2", 53)),
            Verdict::Allow
        );
    }

    #[test]
    fn first_matching_rule_wins() {
        let mut acl = Acl::default();
        let mut allow = rule(AclAction::Allow, RuleProto::Tcp);
        allow.port_from = Some(80);
        let mut deny = rule(AclAction::Deny, RuleProto::Tcp);
        deny.port_from = Some(1);
        deny.port_to = Some(1000);
        acl.rules = vec![allow, deny];
        let syn = |port| tcp("1.1.1.1", 40000, "2.2.2.2", port, TcpFlags::SYN);
        assert_eq!(acl.check(IfaceId(0), &syn(80)), Verdict::Allow);
        assert_eq!(
            acl.check(IfaceId(0), &syn(443)),
            Verdict::Deny { rule: Some(1) }
        );
        assert_eq!(acl.check(IfaceId(0), &syn(8080)), Verdict::Allow); // défaut
    }

    #[test]
    fn default_deny_reports_no_rule() {
        let mut acl = Acl::firewall();
        assert_eq!(
            acl.check(IfaceId(0), &udp("1.1.1.1", "2.2.2.2", 53)),
            Verdict::Deny { rule: None }
        );
    }

    #[test]
    fn network_and_interface_constraints() {
        let mut acl = Acl::default();
        let mut r = rule(AclAction::Deny, RuleProto::Any);
        r.src = Some("10.0.0.0/8".parse().unwrap());
        r.dst = Some("192.168.2.0/24".parse().unwrap());
        r.in_iface = Some(IfaceId(1));
        acl.rules = vec![r];
        let p = udp("10.1.1.1", "192.168.2.5", 9);
        assert_eq!(acl.check(IfaceId(1), &p), Verdict::Deny { rule: Some(0) });
        assert_eq!(acl.check(IfaceId(0), &p), Verdict::Allow, "autre interface");
        assert_eq!(
            acl.check(IfaceId(1), &udp("11.1.1.1", "192.168.2.5", 9)),
            Verdict::Allow
        );
        assert_eq!(
            acl.check(IfaceId(1), &udp("10.1.1.1", "192.168.3.5", 9)),
            Verdict::Allow
        );
    }

    #[test]
    fn port_rule_does_not_match_icmp() {
        let mut acl = Acl::default();
        let mut r = rule(AclAction::Deny, RuleProto::Any);
        r.port_from = Some(80);
        acl.rules = vec![r];
        let ping = Ipv4Packet {
            src: ip("1.1.1.1"),
            dst: ip("2.2.2.2"),
            ttl: 64,
            payload: IpPayload::Icmp(IcmpMessage::EchoRequest { id: 1, seq: 1 }),
        };
        assert_eq!(acl.check(IfaceId(0), &ping), Verdict::Allow);
    }

    #[test]
    fn stateful_firewall_lets_replies_back_but_not_new_inbound_flows() {
        let mut acl = Acl::firewall();
        let mut out = rule(AclAction::Allow, RuleProto::Tcp);
        out.in_iface = Some(IfaceId(0)); // LAN
        out.dst = Some("203.0.113.0/24".parse().unwrap());
        acl.rules = vec![out];

        let syn = tcp("192.168.1.10", 40000, "203.0.113.5", 80, TcpFlags::SYN);
        assert_eq!(acl.check(IfaceId(0), &syn), Verdict::Allow);
        // Retour, vu depuis l'interface WAN : accepté grâce au suivi.
        let synack = tcp("203.0.113.5", 80, "192.168.1.10", 40000, TcpFlags::SYN_ACK);
        assert_eq!(acl.check(IfaceId(1), &synack), Verdict::Allow);
        // Un nouveau flux entrant, lui, est refusé.
        let inbound = tcp("203.0.113.5", 4444, "192.168.1.10", 22, TcpFlags::SYN);
        assert_eq!(
            acl.check(IfaceId(1), &inbound),
            Verdict::Deny { rule: None }
        );
        assert_eq!(acl.tracked_flows(), 1);
    }

    #[test]
    fn stateless_filter_does_not_track() {
        let mut acl = Acl {
            default: AclAction::Deny,
            rules: vec![rule(AclAction::Allow, RuleProto::Udp)],
            ..Acl::default()
        };
        assert_eq!(
            acl.check(IfaceId(0), &udp("1.1.1.1", "2.2.2.2", 53)),
            Verdict::Allow
        );
        assert_eq!(acl.tracked_flows(), 0);
    }

    #[test]
    fn icmp_errors_always_pass() {
        let mut acl = Acl::firewall();
        let orig = udp("1.1.1.1", "2.2.2.2", 53);
        let err = Ipv4Packet {
            src: ip("9.9.9.9"),
            dst: ip("1.1.1.1"),
            ttl: 64,
            payload: IpPayload::Icmp(IcmpMessage::TimeExceeded {
                original: Box::new(orig),
            }),
        };
        assert_eq!(acl.check(IfaceId(0), &err), Verdict::Allow);
    }

    #[test]
    fn rule_description_is_readable() {
        let mut r = rule(AclAction::Deny, RuleProto::Tcp);
        r.src = Some("10.0.0.0/8".parse().unwrap());
        r.port_from = Some(443);
        r.in_iface = Some(IfaceId(2));
        r.comment = "pas de HTTPS".into();
        assert_eq!(
            r.describe(|i| format!("eth{}", i.0)),
            "deny tcp 10.0.0.0/8 → any port 443 depuis eth2 — pas de HTTPS"
        );
    }
}
