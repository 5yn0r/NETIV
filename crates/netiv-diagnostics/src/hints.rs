//! Indices progressifs : du plus vague (la couche) à la solution complète.
//!
//! Le dernier indice donne la réponse ; les précédents guident sans la donner.
//! Les mêmes indices alimentent le Tutor des challenges.

use crate::cause::{CauseKind, RootCause};

/// Nombre d'indices par cause (le dernier est la solution).
pub const LEVELS: usize = 4;

/// Les quatre indices d'une cause racine.
pub fn hints_for(cause: &RootCause) -> [String; LEVELS] {
    let d = &cause.device;
    let layer = cause.kind.layer();
    let h1 = format!(
        "Indice 1 — Le problème se situe au niveau de la {layer}. Remonte le trajet du paquet couche par couche."
    );
    let (h2, h3) = match cause.kind {
        CauseKind::LinkDown => (
            "Le paquet est-il sorti de l'équipement ? Regarde l'état des câbles avec « show ifaces » sur chaque équipement du chemin.".to_string(),
            "Un câble du chemin est signalé comme coupé (COUPÉ) dans « show ifaces ».".to_string(),
        ),
        CauseKind::InterfaceDown => (
            "Les interfaces des équipements du chemin sont-elles toutes actives ?".to_string(),
            format!("Vérifie l'état (up/down) des interfaces de {d} avec « show ifaces {d} »."),
        ),
        CauseKind::PacketLoss => (
            "Le réseau répond parfois, parfois non : qu'est-ce qui peut perdre des trames de façon intermittente ?".to_string(),
            "Cherche dans la trace (« log ») les trames « perdues sur le lien » : elles désignent le câble fautif.".to_string(),
        ),
        CauseKind::HighLatency => (
            "Le réseau répond, mais lentement. Compare le temps de réponse à ce que donne le même trajet sain.".to_string(),
            "Dans « trace », cherche l'étape où l'écart de temps entre deux événements est anormal.".to_string(),
        ),
        CauseKind::WrongVlan => (
            "Les deux machines sont dans le même sous-réseau mais la requête ARP n'arrive pas : quelle fonction de couche 2 cloisonne un switch ?".to_string(),
            format!("Examine les ports de {d} avec « show vlan {d} » : tous les ports du chemin transportent-ils le même VLAN ?"),
        ),
        CauseKind::ArpMismatch => (
            "L'adresse IP est bonne et la machine existe, pourtant les trames ne trouvent pas leur destinataire. À qui sont-elles envoyées ?".to_string(),
            format!("Compare la MAC enregistrée dans « show arp {d} » avec celle de l'interface du voisin (« show ifaces »)."),
        ),
        CauseKind::WrongIp => (
            "Compare l'adresse de la machine avec celle de sa passerelle : sont-elles dans le même réseau ?".to_string(),
            format!("Regarde l'adresse IP de {d} avec « show ifaces {d} » et compare-la au plan d'adressage."),
        ),
        CauseKind::WrongMask => (
            "La machine considère-t-elle la destination comme locale ou distante ? De quoi dépend cette décision ?".to_string(),
            format!("Regarde le masque (le /xx) configuré sur {d} avec « show ifaces {d} »."),
        ),
        CauseKind::WrongGateway => (
            "Le paquet doit quitter le réseau local : vers quelle adresse la machine l'envoie-t-elle ?".to_string(),
            format!("Vérifie la passerelle par défaut de {d} (« show route {d} ») et assure-toi qu'une machine répond vraiment à cette adresse."),
        ),
        CauseKind::MissingRoute => (
            "Un routeur du trajet sait-il où envoyer ce paquet ? Pense aussi au chemin de retour de la réponse.".to_string(),
            format!("Consulte la table de routage de {d} (« show route {d} ») : manque-t-il un réseau ?"),
        ),
        CauseKind::WrongRoute => (
            "Une route existe mais ne mène nulle part. Le prochain saut est-il joignable ?".to_string(),
            format!("Dans « show route {d} », un prochain saut est signalé comme inutilisable ou incorrect."),
        ),
        CauseKind::RoutingLoop => (
            "Le TTL expire : le paquet tourne en rond. Quels équipements voient passer le même paquet plusieurs fois ?".to_string(),
            format!("Compare les routes de {d} et de son voisin pour ce réseau : se renvoient-elles le paquet l'une à l'autre ?"),
        ),
        CauseKind::ForwardingDisabled => (
            "Le paquet arrive à un routeur puis s'arrête. Que fait un routeur d'un paquet qui ne lui est pas destiné ?".to_string(),
            format!("Regarde la trace de {d} : il reçoit le paquet mais l'abandonne « pas pour nous »."),
        ),
        CauseKind::ServiceDown => (
            "La machine répond au ping mais refuse la connexion. Que signifie un RST ?".to_string(),
            format!("Liste les services de {d} avec « show services {d} » : le port visé y figure-t-il ?"),
        ),
        CauseKind::Filtered => (
            "Le paquet arrive à un équipement de filtrage puis disparaît sans réponse. Qui décide de le laisser passer ?".to_string(),
            format!("Lis les règles de {d} avec « show acl {d} » et la décision citée dans la trace."),
        ),
        CauseKind::DhcpServerDown => (
            "La machine n'a pas d'adresse IP. Qui est censé lui en donner une, et ce service répond-il ?".to_string(),
            format!("Regarde le serveur DHCP avec « show dhcp {d} » : tourne-t-il ? Ensuite, le client doit redemander une adresse (« dhcp <poste> renew »)."),
        ),
        CauseKind::DhcpRelayMissing => (
            "La demande DHCP du client est une diffusion : un routeur ne la transmet jamais tel quel. Où est le serveur par rapport au client ?".to_string(),
            format!("Le serveur est dans un autre réseau : l'interface de {d} côté client doit relayer les demandes (« show dhcp {d} »)."),
        ),
        CauseKind::DhcpPoolExhausted => (
            "Le serveur DHCP répond-il au client, ou n'a-t-il rien à lui proposer ?".to_string(),
            format!("Compare la taille de la plage de {d} et le nombre de baux déjà attribués (« show dhcp {d} »)."),
        ),
        CauseKind::RogueDhcp => (
            "Le client a bien une adresse, mais d'où vient sa configuration (passerelle, DNS) ?".to_string(),
            format!("Regarde quel serveur a répondu au client (« show dhcp <poste> ») : {d} est-il censé distribuer des adresses ?"),
        ),
        CauseKind::DnsWrongServer => (
            "Le réseau fonctionne avec les adresses IP, pas avec les noms. Qui la machine interroge-t-elle pour traduire un nom ?".to_string(),
            format!("Regarde le serveur DNS utilisé par {d} (« show dns {d} ») : une machine répond-elle aux questions DNS à cette adresse ?"),
        ),
        CauseKind::DnsServerDown => (
            "La question DNS part bien, mais quelle réponse revient ?".to_string(),
            format!("Le serveur DNS {d} est-il démarré (« show dns {d} ») ?"),
        ),
        CauseKind::DnsRecordWrong => (
            "Le serveur DNS répond : la réponse est-elle la bonne ?".to_string(),
            format!("Compare l'enregistrement du nom dans la zone de {d} (« show dns {d} ») avec l'adresse réelle du serveur visé."),
        ),
        CauseKind::Unknown => (
            "Reprends la trace pas à pas avec « trace » et note où le paquet disparaît.".to_string(),
            "Compare les tables (ARP, routes, ACL) des équipements autour de cet endroit.".to_string(),
        ),
    };
    let fix = cause
        .fix
        .as_deref()
        .map(|f| format!(" Commande : {f}"))
        .unwrap_or_default();
    let h4 = format!("Solution — {} sur {d} : {}.{fix}", cause.kind, cause.detail);
    [
        h1,
        format!("Indice 2 — {h2}"),
        format!("Indice 3 — {h3}"),
        h4,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(kind: CauseKind) -> RootCause {
        RootCause {
            kind,
            device: "PC1".into(),
            detail: "la passerelle est fausse".into(),
            expected: Some("192.168.1.1".into()),
            actual: Some("192.168.1.254".into()),
            fix: Some("gateway PC1 192.168.1.1".into()),
            on_return_path: false,
        }
    }

    #[test]
    fn every_cause_has_four_distinct_hints() {
        for kind in CauseKind::ALL.iter().chain([&CauseKind::Unknown]) {
            let h = hints_for(&cause(*kind));
            assert_eq!(h.len(), LEVELS);
            for (i, text) in h.iter().enumerate() {
                assert!(!text.is_empty());
                assert_eq!(
                    h.iter().filter(|x| *x == text).count(),
                    1,
                    "indice {i} en double"
                );
            }
        }
    }

    #[test]
    fn only_the_last_hint_gives_the_answer() {
        for kind in CauseKind::ALL {
            let c = cause(kind);
            let h = hints_for(&c);
            let fix = c.fix.as_deref().unwrap();
            for early in &h[..3] {
                assert!(
                    !early.contains(fix),
                    "{kind:?} : le correctif fuit trop tôt"
                );
                assert!(
                    !early.contains("192.168.1.1"),
                    "{kind:?} : la valeur attendue fuit trop tôt"
                );
            }
            assert!(h[3].contains(fix));
        }
    }
}
