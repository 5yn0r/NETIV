//! Tuteur : il guide, il ne résout pas.
//!
//! Deux types de réponses :
//! * **explications de notions** (« c'est quoi l'ARP ? ») : gratuites, avec ou
//!   sans challenge ;
//! * **aide sur le challenge en cours** : le tuteur délivre l'indice suivant
//!   (−10 points, comme `hint`), enrobé d'une remarque sur l'hypothèse de
//!   l'élève. Il refuse de donner la réponse directement.
//!
//! Tout est scripté : mêmes questions, mêmes réponses, aucun appel externe.

use netiv_diagnostics::Layer;

use crate::run::{ChallengeRun, RunError};

/// Réponse du tuteur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// Texte à afficher.
    pub text: String,
    /// La réponse a consommé un indice (−10 points).
    pub cost_hint: bool,
}

struct Concept {
    keys: &'static [&'static str],
    title: &'static str,
    text: &'static str,
}

const CONCEPTS: &[Concept] = &[
    Concept {
        keys: &["arp"],
        title: "ARP",
        text: "ARP associe une adresse IP à une adresse MAC. Une machine qui veut parler à 192.168.1.1 diffuse « qui a cette IP ? » ; le propriétaire répond avec sa MAC, qui est mémorisée dans le cache ARP (« show arp »).",
    },
    Concept {
        keys: &["mac"],
        title: "Adresse MAC",
        text: "Une adresse MAC identifie une interface sur le câble local (6 octets). Les switches apprennent sur quel port se trouve chaque MAC et s'en servent pour transmettre les trames (« show mac »).",
    },
    Concept {
        keys: &["ip", "ipv4", "adresse ip"],
        title: "Adresse IP",
        text: "Une adresse IPv4 identifie une machine ET son réseau. Deux machines ne se parlent directement que si leurs adresses sont dans le même réseau ; sinon, le paquet passe par une passerelle.",
    },
    Concept {
        keys: &[
            "masque",
            "mask",
            "subnet",
            "sous-réseau",
            "sous-reseau",
            "cidr",
        ],
        title: "Masque de sous-réseau",
        text: "Le masque (/24, /16…) dit quelle partie de l'adresse désigne le réseau. À partir de lui, une machine décide si une destination est locale (ARP direct) ou distante (envoi à la passerelle). Un masque faux inverse cette décision.",
    },
    Concept {
        keys: &["passerelle", "gateway", "default gateway"],
        title: "Passerelle par défaut",
        text: "La passerelle est le routeur local vers lequel une machine envoie tout ce qui n'est pas dans son réseau. Elle doit être une adresse du MÊME réseau que la machine, et appartenir à un équipement qui répond.",
    },
    Concept {
        keys: &["route", "routage", "routing", "table de routage"],
        title: "Routage",
        text: "Un routeur compare l'adresse de destination à sa table et choisit la route au plus long préfixe qui correspond. Sans route, il abandonne le paquet et prévient l'émetteur (« réseau inaccessible »). Il faut des routes dans les deux sens.",
    },
    Concept {
        keys: &["ttl"],
        title: "TTL",
        text: "Le TTL est un compteur décrémenté à chaque routeur. À zéro, le paquet est détruit et le routeur prévient l'émetteur. Cela empêche un paquet de tourner indéfiniment dans une boucle de routage.",
    },
    Concept {
        keys: &["icmp", "ping"],
        title: "ICMP et ping",
        text: "ICMP transporte les messages de contrôle. Le ping envoie une requête d'écho et attend la réponse : il teste l'aller ET le retour. Les erreurs ICMP (réseau inaccessible, TTL expiré) disent où et pourquoi le paquet s'est arrêté.",
    },
    Concept {
        keys: &["tcp"],
        title: "TCP",
        text: "TCP établit une connexion en trois temps (SYN, SYN-ACK, ACK) avant d'échanger des données. Un RST signifie « rien n'écoute sur ce port » ; l'absence totale de réponse signifie plutôt un paquet perdu ou filtré.",
    },
    Concept {
        keys: &["udp"],
        title: "UDP",
        text: "UDP envoie des datagrammes sans connexion ni accusé de réception. Un service UDP répond ou se tait ; un port fermé provoque un message ICMP « port inaccessible ».",
    },
    Concept {
        keys: &["vlan", "trunk", "802.1q"],
        title: "VLAN",
        text: "Un VLAN découpe un switch en réseaux de niveau 2 indépendants. Un port d'accès appartient à un VLAN ; un trunk transporte plusieurs VLAN en étiquetant les trames (802.1Q). Passer d'un VLAN à l'autre exige un routeur.",
    },
    Concept {
        keys: &["pare-feu", "parefeu", "firewall", "acl", "filtrage"],
        title: "Pare-feu et ACL",
        text: "Un pare-feu applique des règles dans l'ordre : la première qui correspond gagne, sinon la politique par défaut. Avec suivi de connexion, les réponses aux flux autorisés repassent automatiquement.",
    },
    Concept {
        keys: &["dhcp", "dns", "nat"],
        title: "DHCP, DNS, NAT",
        text: "Ces services ne sont pas encore simulés dans NETIV. Principe : DHCP distribue les adresses, DNS traduit les noms en adresses, NAT traduit les adresses privées en adresses publiques.",
    },
];

const EXPLAIN_MARKERS: &[&str] = &[
    "c'est quoi",
    "c’est quoi",
    "qu'est-ce",
    "qu’est-ce",
    "qu'est ce",
    "explique",
    "expliquer",
    "définition",
    "definition",
    "à quoi sert",
    "a quoi sert",
    "comment fonctionne",
    "what is",
];

const GIVE_UP_MARKERS: &[&str] = &[
    "solution",
    "réponse",
    "reponse",
    "dis-moi",
    "dis moi",
    "donne-moi",
    "donne moi",
    "corrige",
    "répare",
    "repare",
    "fais-le",
    "fais le",
    "la panne est",
    "c'est quoi la panne",
    "quelle est la panne",
];

fn contains_word(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    let mut start = 0;
    while let Some(pos) = h[start..].find(&n) {
        let i = start + pos;
        let before = h[..i].chars().next_back();
        let after = h[i + n.len()..].chars().next();
        let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if boundary(before) && boundary(after) {
            return true;
        }
        start = i + n.len();
    }
    false
}

/// Explication d'une notion, si la question en demande une.
pub fn concept(question: &str) -> Option<String> {
    let q = question.to_lowercase();
    if !EXPLAIN_MARKERS.iter().any(|m| q.contains(m)) {
        return None;
    }
    // La notion la plus longuement citée l'emporte (« adresse ip » avant « ip »).
    CONCEPTS
        .iter()
        .filter_map(|c| {
            c.keys
                .iter()
                .filter(|k| contains_word(&q, k))
                .map(|k| k.len())
                .max()
                .map(|len| (len, c))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, c)| format!("{} — {}", c.title, c.text))
}

/// Couche suggérée par la question, d'après son vocabulaire.
pub fn intent(question: &str) -> Option<Layer> {
    const MAP: &[(Layer, &[&str])] = &[
        (
            Layer::Security,
            &[
                "pare-feu", "parefeu", "firewall", "acl", "filtr", "bloqu", "règle", "regle",
                "deny",
            ],
        ),
        (
            Layer::Services,
            &[
                "service",
                "port",
                "curl",
                "http",
                "tcp",
                "udp",
                "serveur web",
                "refus",
            ],
        ),
        (
            Layer::L2,
            &["vlan", "arp", "mac", "switch", "trunk", "commutat"],
        ),
        (
            Layer::L1,
            &[
                "câble",
                "cable",
                "interface",
                "physique",
                "lien",
                "débranch",
                "debranch",
                "éteint",
                "eteint",
                "perte",
                "coupé",
                "coupe",
            ],
        ),
        (
            Layer::L3,
            &[
                "ip",
                "masque",
                "passerelle",
                "gateway",
                "route",
                "routage",
                "routeur",
                "ttl",
                "adresse",
            ],
        ),
    ];
    let q = question.to_lowercase();
    MAP.iter()
        .find(|(_, words)| {
            words
                .iter()
                .any(|w| contains_word(&q, w) || (w.len() > 4 && q.contains(w)))
        })
        .map(|(l, _)| *l)
}

/// Le tuteur répond. Sans challenge en cours, seules les explications de
/// notions sont possibles.
pub fn ask(run: Option<&mut ChallengeRun>, question: &str) -> Result<Reply, String> {
    let question = question.trim();
    if question.is_empty() {
        return Err("pose une question, par ex. « tutor c'est quoi l'ARP ? » ou « tutor pourquoi le ping échoue ? »".to_string());
    }
    if let Some(text) = concept(question) {
        return Ok(Reply {
            text: format!("Tuteur : {text}"),
            cost_hint: false,
        });
    }
    let Some(run) = run.filter(|r| !r.is_finished()) else {
        return Err("aucun challenge en cours : je peux seulement expliquer des notions (« tutor c'est quoi le TTL ? »)".to_string());
    };

    let q = question.to_lowercase();
    let asks_answer = GIVE_UP_MARKERS.iter().any(|m| q.contains(m));
    let opening = if asks_answer {
        "Je ne donne pas la réponse : mon rôle est de te guider, pas de résoudre à ta place. Voici plutôt un indice. ".to_string()
    } else if run.hints_used() >= 1 {
        // La couche a déjà été révélée par le premier indice : on peut commenter l'hypothèse.
        match intent(question) {
            Some(l) if l == run.reveal().kind.layer() => "Tu es sur la bonne piste. ".to_string(),
            Some(_) => "Rien ne désigne cette couche pour l'instant : reviens à la méthode, couche par couche. ".to_string(),
            None => String::new(),
        }
    } else {
        String::new()
    };

    let hint = run.next_hint().map_err(|e: RunError| e.to_string())?;
    Ok(Reply {
        text: format!("Tuteur : {opening}{hint}"),
        cost_hint: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;

    fn run(id: &str) -> ChallengeRun {
        ChallengeRun::start(builtin::challenge(id).unwrap().clone(), 0)
            .unwrap()
            .0
    }

    #[test]
    fn concepts_are_explained_for_free() {
        for (q, needle) in [
            ("c'est quoi l'ARP ?", "ARP associe"),
            ("Explique-moi le TTL", "compteur"),
            ("qu'est-ce qu'un VLAN ?", "réseaux de niveau 2"),
            ("à quoi sert une passerelle", "routeur local"),
            ("c'est quoi un masque de sous-réseau", "Un masque faux"),
            ("What is TCP?", "trois temps"),
            ("explique le pare-feu", "première qui correspond"),
            ("c'est quoi le DNS ?", "pas encore simulés"),
        ] {
            let r = ask(None, q).unwrap_or_else(|e| panic!("{q} : {e}"));
            assert!(r.text.contains(needle), "{q} → {}", r.text);
            assert!(!r.cost_hint, "une explication ne coûte rien");
        }
    }

    #[test]
    fn the_longest_matching_term_wins() {
        let t = concept("c'est quoi une adresse ip ?").unwrap();
        assert!(t.starts_with("Adresse IP"), "{t}");
        // « ping » ne doit pas être pris pour « ip » : on cherche des mots entiers.
        let t = concept("c'est quoi le ping ?").unwrap();
        assert!(t.starts_with("ICMP et ping"), "{t}");
    }

    #[test]
    fn a_plain_question_without_a_known_term_is_not_a_concept() {
        assert!(concept("pourquoi ça ne marche pas").is_none());
        assert!(concept("c'est quoi la couleur du ciel ?").is_none());
        assert!(concept("").is_none());
    }

    #[test]
    fn without_a_challenge_only_concepts_are_available() {
        let e = ask(None, "pourquoi le ping échoue").unwrap_err();
        assert!(e.contains("aucun challenge en cours"), "{e}");
        assert!(ask(None, "").unwrap_err().contains("pose une question"));
    }

    #[test]
    fn help_walks_through_the_hint_levels_and_costs_points() {
        let mut r = run("beginner-01");
        let a = ask(Some(&mut r), "pourquoi le ping échoue ?").unwrap();
        assert!(
            a.cost_hint && a.text.starts_with("Tuteur : Indice 1"),
            "{}",
            a.text
        );
        let b = ask(Some(&mut r), "je ne vois pas").unwrap();
        assert!(b.text.contains("Indice 2"), "{}", b.text);
        assert_eq!(r.hints_used(), 2);
    }

    #[test]
    fn the_tutor_refuses_to_hand_over_the_answer_but_still_helps() {
        let mut r = run("beginner-01");
        for q in [
            "donne-moi la solution",
            "quelle est la réponse ?",
            "corrige le réseau",
            "dis-moi la panne",
        ] {
            let a = ask(Some(&mut r), q).unwrap();
            assert!(
                a.text.contains("Je ne donne pas la réponse"),
                "{q} → {}",
                a.text
            );
            assert!(
                !a.text.contains("gateway PC1 192.168.1.1") || a.text.contains("Solution"),
                "{}",
                a.text
            );
        }
        // Les trois premières réponses n'ont jamais livré la commande.
        let mut r = run("beginner-01");
        for _ in 0..3 {
            let a = ask(Some(&mut r), "donne-moi la solution").unwrap();
            assert!(!a.text.contains("gateway PC1 192.168.1.1"), "{}", a.text);
        }
        // Le dernier niveau est la solution, mais elle arrive par la même échelle de coût.
        let last = ask(Some(&mut r), "donne-moi la solution").unwrap();
        assert!(last.text.contains("Solution"), "{}", last.text);
        assert_eq!(r.hints_used(), 4);
    }

    #[test]
    fn the_tutor_comments_on_the_students_hypothesis_only_after_the_layer_is_known() {
        // beginner-01 : couche 3 (passerelle).
        let mut r = run("beginner-01");
        let first = ask(Some(&mut r), "c'est peut-être un problème de VLAN").unwrap();
        assert!(
            !first.text.contains("bonne piste") && !first.text.contains("Rien ne désigne"),
            "{}",
            first.text
        );
        let right = ask(Some(&mut r), "je regarde la passerelle et la route").unwrap();
        assert!(right.text.contains("bonne piste"), "{}", right.text);
        let wrong = ask(Some(&mut r), "peut-être le pare-feu ?").unwrap();
        assert!(
            wrong.text.contains("Rien ne désigne cette couche"),
            "{}",
            wrong.text
        );
    }

    #[test]
    fn intents_follow_the_vocabulary() {
        assert_eq!(intent("le pare-feu bloque ?"), Some(Layer::Security));
        assert_eq!(intent("le port 80 est fermé"), Some(Layer::Services));
        assert_eq!(intent("problème de vlan ou d'arp"), Some(Layer::L2));
        assert_eq!(intent("le câble est débranché"), Some(Layer::L1));
        assert_eq!(intent("mauvaise passerelle"), Some(Layer::L3));
        assert_eq!(intent("bonjour"), None);
    }

    #[test]
    fn a_finished_challenge_no_longer_gets_help() {
        let mut r = run("beginner-01");
        r.give_up().unwrap();
        assert!(
            ask(Some(&mut r), "un indice ?")
                .unwrap_err()
                .contains("aucun challenge en cours")
        );
        // Mais les notions restent disponibles.
        assert!(ask(Some(&mut r), "c'est quoi l'ARP ?").is_ok());
    }

    #[test]
    fn replies_are_deterministic() {
        let go = || {
            let mut r = run("intermediate-04");
            [
                "pourquoi ça échoue ?",
                "c'est le vlan ?",
                "donne-moi la solution",
                "encore un indice",
            ]
            .iter()
            .map(|q| ask(Some(&mut r), q).unwrap().text)
            .collect::<Vec<_>>()
        };
        assert_eq!(go(), go());
    }
}
