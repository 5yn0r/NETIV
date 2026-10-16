//! DNS : messages, zones et résolution de noms.
//!
//! Volontairement minimal : des enregistrements A (nom → IPv4), une question
//! par message, pas de récursion. Un client résout un nom auprès de son
//! serveur DNS (configuré, ou reçu par DHCP) et garde la réponse en cache
//! pendant `CACHE_TTL`.
//!
//! La logique (client, serveur) est dans `ip_node::dns`.

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::time::SimTime;

/// Port UDP du serveur.
pub const PORT: u16 = 53;
/// Durée de vie des réponses en cache.
pub const CACHE_TTL: SimTime = SimTime::from_millis(300_000);
/// Attente d'une réponse.
pub const TIMEOUT: SimTime = SimTime::from_millis(2_000);
/// Nombre d'envois d'une question avant abandon.
pub const MAX_ATTEMPTS: u8 = 2;

/// Code de réponse.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rcode {
    /// Réponse trouvée.
    NoError,
    /// Le nom n'existe pas dans la zone.
    NxDomain,
}

/// Un message DNS : une question, et la réponse s'il s'agit d'une réponse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsMessage {
    /// Identifiant de la question.
    pub id: u16,
    /// Nom demandé.
    pub name: String,
    /// `true` pour une réponse.
    pub response: bool,
    /// Code de réponse (réponses seulement).
    pub rcode: Rcode,
    /// Adresse trouvée.
    pub answer: Option<Ipv4Addr>,
}

impl fmt::Display for DnsMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.response {
            return write!(f, "DNS question #{} : A {} ?", self.id, self.name);
        }
        match (self.rcode, self.answer) {
            (Rcode::NoError, Some(ip)) => {
                write!(f, "DNS réponse #{} : {} = {ip}", self.id, self.name)
            }
            _ => write!(
                f,
                "DNS réponse #{} : {} introuvable (NXDOMAIN)",
                self.id, self.name
            ),
        }
    }
}

/// Un serveur DNS faisant autorité sur une zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsServer {
    /// Le service tourne-t-il ?
    pub enabled: bool,
    /// Enregistrements A, par nom (en minuscules).
    pub zone: BTreeMap<String, Ipv4Addr>,
}

impl DnsServer {
    /// Serveur actif avec ces enregistrements.
    pub fn new(zone: BTreeMap<String, Ipv4Addr>) -> Self {
        DnsServer {
            enabled: true,
            zone: zone
                .into_iter()
                .map(|(n, ip)| (normalize(&n), ip))
                .collect(),
        }
    }

    /// Réponse à une question.
    pub fn lookup(&self, name: &str) -> Option<Ipv4Addr> {
        self.zone.get(&normalize(name)).copied()
    }
}

/// Forme canonique d'un nom : minuscules, sans point final.
pub fn normalize(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Un nom de domaine acceptable : lettres, chiffres, `-` et `.`, 253
/// caractères au plus, pas de libellé vide.
pub fn is_valid_name(name: &str) -> bool {
    let n = name.trim_end_matches('.');
    !n.is_empty()
        && n.len() <= 253
        && n.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // Un nom n'est pas une adresse IP.
        && n.parse::<Ipv4Addr>().is_err()
}

/// Issue d'une résolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsOutcome {
    /// Nom résolu.
    Resolved(Ipv4Addr),
    /// Le serveur dit que le nom n'existe pas.
    NxDomain,
    /// Aucune réponse du serveur.
    Timeout,
    /// Aucun serveur DNS configuré.
    NoServer,
    /// Le serveur est injoignable (erreur ICMP, pas de route).
    Unreachable,
}

impl fmt::Display for DnsOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DnsOutcome::Resolved(ip) => write!(f, "{ip}"),
            DnsOutcome::NxDomain => f.write_str("nom introuvable (NXDOMAIN)"),
            DnsOutcome::Timeout => f.write_str("aucune réponse du serveur DNS (délai dépassé)"),
            DnsOutcome::NoServer => f.write_str("aucun serveur DNS configuré"),
            DnsOutcome::Unreachable => f.write_str("serveur DNS injoignable"),
        }
    }
}

/// Ce qu'il faut faire une fois le nom résolu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterResolve {
    /// Rien (`nslookup`).
    Nothing,
    /// Lancer des pings.
    Ping {
        /// Nombre de requêtes.
        count: u16,
    },
    /// Ouvrir une connexion TCP.
    Curl {
        /// Port.
        port: u16,
    },
    /// Envoyer un datagramme UDP.
    Udp {
        /// Port.
        port: u16,
        /// Contenu.
        data: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized_and_validated() {
        let mut zone = BTreeMap::new();
        zone.insert("Web.LAN.".to_string(), "10.0.2.10".parse().unwrap());
        let s = DnsServer::new(zone);
        assert_eq!(s.lookup("web.lan"), Some("10.0.2.10".parse().unwrap()));
        assert_eq!(s.lookup("WEB.lan."), Some("10.0.2.10".parse().unwrap()));
        assert_eq!(s.lookup("mail.lan"), None);
        assert!(is_valid_name("web.lan"));
        assert!(is_valid_name("srv-1"));
        assert!(!is_valid_name("10.0.0.1"));
        assert!(!is_valid_name("a..b"));
        assert!(!is_valid_name("bad_name"));
        assert!(!is_valid_name(""));
    }
}
