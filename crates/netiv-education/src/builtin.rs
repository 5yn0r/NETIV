//! Contenu livré avec NETIV : réseaux d'entraînement et challenges.

use std::sync::OnceLock;

use crate::challenge::Challenge;

/// Noms des réseaux livrés.
pub const TOPOLOGY_NAMES: [&str; 7] = [
    "office", "dmz", "campus", "triangle", "cyber", "dhcp", "pme",
];

/// Texte YAML d'un réseau livré.
pub fn topology(name: &str) -> Option<&'static str> {
    Some(match name {
        "office" => include_str!("../content/topologies/office.yaml"),
        "dmz" => include_str!("../content/topologies/dmz.yaml"),
        "campus" => include_str!("../content/topologies/campus.yaml"),
        "triangle" => include_str!("../content/topologies/triangle.yaml"),
        "cyber" => include_str!("../../../examples/cyber.yaml"),
        "dhcp" => include_str!("../content/topologies/dhcp.yaml"),
        "pme" => include_str!("../content/topologies/pme.yaml"),
        _ => return None,
    })
}

const CHALLENGE_SOURCES: [(&str, &str); 28] = [
    (
        "beginner/01-passerelle",
        include_str!("../content/challenges/beginner/01-passerelle.yaml"),
    ),
    (
        "beginner/02-cable",
        include_str!("../content/challenges/beginner/02-cable.yaml"),
    ),
    (
        "beginner/03-interface",
        include_str!("../content/challenges/beginner/03-interface.yaml"),
    ),
    (
        "beginner/04-adresse",
        include_str!("../content/challenges/beginner/04-adresse.yaml"),
    ),
    (
        "beginner/05-service",
        include_str!("../content/challenges/beginner/05-service.yaml"),
    ),
    (
        "intermediate/06-masque",
        include_str!("../content/challenges/intermediate/06-masque.yaml"),
    ),
    (
        "intermediate/07-route",
        include_str!("../content/challenges/intermediate/07-route.yaml"),
    ),
    (
        "intermediate/08-transfert",
        include_str!("../content/challenges/intermediate/08-transfert.yaml"),
    ),
    (
        "intermediate/09-vlan",
        include_str!("../content/challenges/intermediate/09-vlan.yaml"),
    ),
    (
        "intermediate/10-pare-feu",
        include_str!("../content/challenges/intermediate/10-pare-feu.yaml"),
    ),
    (
        "advanced/11-retour",
        include_str!("../content/challenges/advanced/11-retour.yaml"),
    ),
    (
        "advanced/12-boucle",
        include_str!("../content/challenges/advanced/12-boucle.yaml"),
    ),
    (
        "advanced/13-arp",
        include_str!("../content/challenges/advanced/13-arp.yaml"),
    ),
    (
        "advanced/14-icmp-bloque",
        include_str!("../content/challenges/advanced/14-icmp-bloque.yaml"),
    ),
    (
        "advanced/15-perte",
        include_str!("../content/challenges/advanced/15-perte.yaml"),
    ),
    (
        "beginner/16-dhcp-serveur",
        include_str!("../content/challenges/beginner/16-dhcp-serveur.yaml"),
    ),
    (
        "intermediate/17-dhcp-relais",
        include_str!("../content/challenges/intermediate/17-dhcp-relais.yaml"),
    ),
    (
        "advanced/18-dhcp-pirate",
        include_str!("../content/challenges/advanced/18-dhcp-pirate.yaml"),
    ),
    // Parcours « support IT » (tickets d'assistance, réseau « pme »).
    (
        "beginner/19-support-port",
        include_str!("../content/challenges/beginner/19-support-port.yaml"),
    ),
    (
        "beginner/20-support-apipa",
        include_str!("../content/challenges/beginner/20-support-apipa.yaml"),
    ),
    (
        "beginner/21-support-imprimante",
        include_str!("../content/challenges/beginner/21-support-imprimante.yaml"),
    ),
    (
        "beginner/22-support-appli",
        include_str!("../content/challenges/beginner/22-support-appli.yaml"),
    ),
    (
        "beginner/23-support-dns-poste",
        include_str!("../content/challenges/beginner/23-support-dns-poste.yaml"),
    ),
    (
        "intermediate/24-support-dns-serveur",
        include_str!("../content/challenges/intermediate/24-support-dns-serveur.yaml"),
    ),
    (
        "intermediate/25-support-dns-enregistrement",
        include_str!("../content/challenges/intermediate/25-support-dns-enregistrement.yaml"),
    ),
    (
        "intermediate/26-support-dhcp-plage",
        include_str!("../content/challenges/intermediate/26-support-dhcp-plage.yaml"),
    ),
    (
        "intermediate/27-support-vlan",
        include_str!("../content/challenges/intermediate/27-support-vlan.yaml"),
    ),
    (
        "advanced/28-support-conflit",
        include_str!("../content/challenges/advanced/28-support-conflit.yaml"),
    ),
];

/// Challenges livrés, dans l'ordre. Un fichier illisible est ignoré ici et
/// signalé par les tests (qui exigent qu'ils soient tous présents).
pub fn challenges() -> &'static [Challenge] {
    static CACHE: OnceLock<Vec<Challenge>> = OnceLock::new();
    CACHE.get_or_init(|| {
        CHALLENGE_SOURCES
            .iter()
            .filter_map(|(_, text)| Challenge::from_yaml(text).ok())
            .collect()
    })
}

/// Challenge livré par identifiant.
pub fn challenge(id: &str) -> Option<&'static Challenge> {
    challenges().iter().find(|c| c.id == id)
}

/// Fichiers sources des challenges (pour les tests de contenu).
pub fn challenge_sources() -> &'static [(&'static str, &'static str)] {
    &CHALLENGE_SOURCES
}

const RANGE_SOURCES: [&str; 3] = [
    include_str!("../content/range/01-balayage.yaml"),
    include_str!("../content/range/02-rebond.yaml"),
    include_str!("../content/range/03-balise.yaml"),
];

/// Scénarios de cyber range livrés.
pub fn ranges() -> &'static [crate::range::RangeScenario] {
    static CACHE: OnceLock<Vec<crate::range::RangeScenario>> = OnceLock::new();
    CACHE.get_or_init(|| {
        RANGE_SOURCES
            .iter()
            .filter_map(|t| crate::range::RangeScenario::from_yaml(t).ok())
            .collect()
    })
}

/// Scénario de cyber range par identifiant.
pub fn range(id: &str) -> Option<&'static crate::range::RangeScenario> {
    ranges().iter().find(|r| r.id == id)
}

/// Sources YAML des scénarios (pour les tests de contenu).
pub fn range_sources() -> &'static [&'static str] {
    &RANGE_SOURCES
}

/// Toutes les épreuves livrées : `(identifiant, thème)`. Les scénarios de
/// cyber range comptent dans le thème « security ».
pub fn catalog() -> Vec<(String, String)> {
    challenges()
        .iter()
        .map(|c| (c.id.clone(), c.topic.clone()))
        .chain(
            ranges()
                .iter()
                .map(|r| (r.id.clone(), "security".to_string())),
        )
        .collect()
}
