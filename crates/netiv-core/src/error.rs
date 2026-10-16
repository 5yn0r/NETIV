//! Erreurs d'utilisation (entrées invalides). Les événements réseau
//! (paquet perdu, TTL expiré...) ne sont *pas* des erreurs : ils passent
//! par la trace.

use thiserror::Error;

use crate::time::SimTime;

/// Erreur d'utilisation du moteur.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum NetivError {
    /// Adresse MAC mal formée.
    #[error("adresse MAC invalide : « {0} »")]
    InvalidMac(String),

    /// Adresse IPv4 mal formée.
    #[error("adresse IPv4 invalide : « {0} »")]
    InvalidIpv4(String),

    /// Notation CIDR mal formée (`a.b.c.d/n`).
    #[error("notation CIDR invalide : « {0} »")]
    InvalidCidr(String),

    /// Longueur de préfixe hors de `0..=32`.
    #[error("préfixe invalide : /{0} (attendu entre /0 et /32)")]
    InvalidPrefix(u8),

    /// Tentative de programmer un événement avant la date courante.
    #[error("impossible de programmer un événement à {requested} : la simulation est déjà à {now}")]
    ScheduleInPast {
        /// Date demandée.
        requested: SimTime,
        /// Date courante de la simulation.
        now: SimTime,
    },

    /// Dépassement de l'horloge simulée.
    #[error("dépassement de l'horloge simulée")]
    TimeOverflow,

    /// La simulation a dépassé sa limite d'événements (boucle probable).
    #[error("limite de {0} événements atteinte : la simulation ne semble pas se terminer")]
    EventLimitExceeded(u64),

    /// Nom d'équipement ou d'interface inutilisable.
    #[error("nom invalide : « {0} » (ni vide, ni espace, ni « : »)")]
    InvalidName(String),

    /// Deux équipements portent le même nom.
    #[error("l'équipement « {0} » existe déjà")]
    DuplicateDevice(String),

    /// Deux interfaces d'un équipement portent le même nom.
    #[error("l'interface « {iface} » existe déjà sur {device}")]
    DuplicateInterface {
        /// Équipement.
        device: String,
        /// Interface.
        iface: String,
    },

    /// Équipement introuvable.
    #[error("équipement inconnu : « {0} »")]
    UnknownDevice(String),

    /// Interface introuvable.
    #[error("interface inconnue : « {iface} » sur {device}")]
    UnknownInterface {
        /// Équipement.
        device: String,
        /// Interface.
        iface: String,
    },

    /// Une interface a déjà un câble.
    #[error("{device}/{iface} est déjà reliée à un câble")]
    InterfaceAlreadyConnected {
        /// Équipement.
        device: String,
        /// Interface.
        iface: String,
    },

    /// Un câble relie une interface à elle-même.
    #[error("impossible de relier {device}/{iface} à elle-même")]
    SelfLink {
        /// Équipement.
        device: String,
        /// Interface.
        iface: String,
    },

    /// Opération réservée aux équipements dotés d'une pile IP.
    #[error("{0} est un switch : cette opération n'a pas de sens pour lui")]
    NotAnIpDevice(String),

    /// Taux de perte hors de `[0, 1]`.
    #[error("taux de perte invalide : {0} (attendu entre 0 et 1)")]
    InvalidLoss(f64),

    /// Aucun événement de trace avec ce numéro.
    #[error("aucun événement n°{0} dans la trace")]
    NoSuchEvent(u64),

    /// Aucune transaction avec ce numéro.
    #[error("aucune transaction #{0} dans la trace")]
    NoSuchTrace(u64),

    /// L'événement existe mais n'est pas du bon type pour cette opération.
    #[error("l'événement n°{seq} n'est pas {expected}")]
    WrongEventKind {
        /// Numéro de l'événement.
        seq: u64,
        /// Ce qui était attendu.
        expected: &'static str,
    },

    /// Opération réservée aux switches.
    #[error("{0} n'est pas un switch")]
    NotASwitch(String),

    /// Numéro de règle inexistant.
    #[error("la règle n°{0} n'existe pas")]
    NoSuchRule(usize),

    /// Numéro de panne inexistant.
    #[error("la panne n°{0} n'existe pas")]
    NoSuchFault(u64),

    /// La panne a déjà été réparée.
    #[error("la panne n°{0} est déjà réparée")]
    FaultAlreadyRepaired(u64),

    /// Configuration impossible (message complet).
    #[error("{0}")]
    InvalidConfig(String),

    /// Rôle d'équipement inconnu.
    #[error("rôle inconnu : « {0} » (rôles : user, server, dmz, internet, network, guest, iot)")]
    UnknownRole(String),
}
