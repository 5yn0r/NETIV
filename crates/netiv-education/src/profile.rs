//! Progression d'un élève : XP, niveaux, badges, statistiques.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::challenge::Challenge;
use crate::run::Score;

/// Seuils d'XP et noms des niveaux.
pub const LEVELS: [(u32, &str); 6] = [
    (0, "Apprenti"),
    (150, "Technicien"),
    (400, "Administrateur"),
    (800, "Ingénieur"),
    (1_300, "Architecte"),
    (1_900, "Expert"),
];

/// Un badge à débloquer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    /// Identifiant.
    pub id: &'static str,
    /// Nom affiché.
    pub name: &'static str,
    /// Comment l'obtenir.
    pub how: &'static str,
}

/// Les badges du cahier des charges (§25).
pub const BADGES: [Badge; 6] = [
    Badge {
        id: "network-beginner",
        name: "Network Beginner",
        how: "réussir un premier challenge",
    },
    Badge {
        id: "arp-explorer",
        name: "ARP Explorer",
        how: "réussir un challenge de couche 2 (ARP ou VLAN)",
    },
    Badge {
        id: "routing-specialist",
        name: "Routing Specialist",
        how: "réussir 3 challenges de routage",
    },
    Badge {
        id: "packet-analyst",
        name: "Packet Analyst",
        how: "réussir 3 challenges sans aucun indice",
    },
    Badge {
        id: "troubleshooter",
        name: "Troubleshooter",
        how: "réussir 5 challenges",
    },
    Badge {
        id: "network-defender",
        name: "Network Defender",
        how: "réussir 2 challenges de sécurité",
    },
];

/// Un challenge réussi (meilleur résultat).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    /// Meilleur score (0 à 100).
    pub score: u32,
    /// XP correspondants.
    pub xp: u32,
    /// Indices utilisés lors de ce meilleur résultat.
    pub hints: u32,
    /// Durée de ce meilleur résultat.
    pub elapsed_secs: u64,
    /// Thème.
    pub topic: String,
    /// Nature de la panne.
    pub cause: String,
    /// Nombre de réussites (une même épreuve peut être refaite).
    pub times: u32,
}

/// Le profil d'un élève.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Nom.
    pub name: String,
    /// Challenges réussis.
    pub completed: BTreeMap<String, Completion>,
    /// Challenges tentés (identifiant → nombre de tentatives).
    pub attempts: BTreeMap<String, u32>,
    /// Leçons lues.
    pub lessons_read: BTreeSet<String>,
    /// Badges obtenus.
    pub badges: BTreeSet<String>,
}

/// Ce qu'un résultat a changé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// XP supplémentaires (0 si le meilleur score n'est pas battu).
    pub xp_gained: u32,
    /// Le meilleur résultat a été amélioré.
    pub improved: bool,
    /// Badges obtenus à cette occasion.
    pub new_badges: Vec<Badge>,
    /// Niveau avant.
    pub level_before: u32,
    /// Niveau après.
    pub level_after: u32,
}

/// Erreurs de lecture / écriture du profil.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// Fichier illisible ou inaccessible.
    #[error("profil « {path} » : {reason}")]
    Io {
        /// Chemin.
        path: String,
        /// Cause.
        reason: String,
    },
    /// Contenu invalide.
    #[error("profil « {path} » corrompu : {reason}")]
    Corrupt {
        /// Chemin.
        path: String,
        /// Cause.
        reason: String,
    },
}

/// Niveau pour un total d'XP : (numéro à partir de 1, nom, seuil suivant).
pub fn level_for(xp: u32) -> (u32, &'static str, Option<u32>) {
    let idx = LEVELS.iter().rposition(|(t, _)| xp >= *t).unwrap_or(0);
    let next = LEVELS.get(idx + 1).map(|(t, _)| *t);
    (idx as u32 + 1, LEVELS[idx].1, next)
}

impl Profile {
    /// Profil vide.
    pub fn new(name: &str) -> Self {
        Profile {
            name: name.to_string(),
            ..Profile::default()
        }
    }

    /// XP total.
    pub fn xp(&self) -> u32 {
        self.completed.values().map(|c| c.xp).sum()
    }

    /// Niveau actuel.
    pub fn level(&self) -> u32 {
        level_for(self.xp()).0
    }

    /// Challenges réussis.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Un challenge a-t-il été réussi ?
    pub fn has_completed(&self, id: &str) -> bool {
        self.completed.contains_key(id)
    }

    /// Note une tentative.
    pub fn record_attempt(&mut self, id: &str) {
        *self.attempts.entry(id.to_string()).or_insert(0) += 1;
    }

    /// Enregistre un résultat. Seul le meilleur score compte pour l'XP.
    pub fn record(&mut self, challenge: &Challenge, score: &Score) -> Recorded {
        self.record_result(
            &challenge.id,
            &challenge.topic,
            &challenge.solution.cause,
            score,
        )
    }

    /// Enregistre le résultat d'une épreuve quelconque (challenge, cyber range).
    pub fn record_result(&mut self, id: &str, topic: &str, cause: &str, score: &Score) -> Recorded {
        let level_before = self.level();
        let entry = self.completed.entry(id.to_string()).or_default();
        let old_xp = entry.xp;
        let improved = entry.times == 0 || score.total > entry.score;
        entry.times += 1;
        if improved {
            entry.score = score.total;
            entry.xp = score.xp;
            entry.hints = score.hints;
            entry.elapsed_secs = score.elapsed_secs;
            entry.topic = topic.to_string();
            entry.cause = cause.to_string();
        }
        let xp_gained = entry.xp.saturating_sub(old_xp);

        let earned = self.evaluate_badges();
        let new_badges: Vec<Badge> = BADGES
            .iter()
            .filter(|b| earned.contains(b.id) && !self.badges.contains(b.id))
            .copied()
            .collect();
        self.badges.extend(earned.into_iter().map(str::to_string));
        Recorded {
            xp_gained,
            improved,
            new_badges,
            level_before,
            level_after: self.level(),
        }
    }

    fn evaluate_badges(&self) -> BTreeSet<&'static str> {
        let done = &self.completed;
        let count_topic = |t: &str| done.values().filter(|c| c.topic == t).count();
        let mut out = BTreeSet::new();
        if !done.is_empty() {
            out.insert("network-beginner");
        }
        if done
            .values()
            .any(|c| c.cause == "arp-mismatch" || c.cause == "wrong-vlan")
        {
            out.insert("arp-explorer");
        }
        if count_topic("routing") >= 3 {
            out.insert("routing-specialist");
        }
        if done.values().filter(|c| c.hints == 0).count() >= 3 {
            out.insert("packet-analyst");
        }
        if done.len() >= 5 {
            out.insert("troubleshooter");
        }
        if count_topic("security") >= 2 {
            out.insert("network-defender");
        }
        out
    }

    /// Marque une leçon comme lue.
    pub fn mark_lesson_read(&mut self, id: &str) {
        self.lessons_read.insert(id.to_string());
    }

    /// Charge un profil ; un fichier absent donne un profil neuf.
    pub fn load(path: &Path, name: &str) -> Result<Profile, ProfileError> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| ProfileError::Corrupt {
                path: path.display().to_string(),
                reason: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Profile::new(name)),
            Err(e) => Err(ProfileError::Io {
                path: path.display().to_string(),
                reason: e.to_string(),
            }),
        }
    }

    /// Sauvegarde (écriture atomique : fichier temporaire puis renommage).
    pub fn save(&self, path: &Path) -> Result<(), ProfileError> {
        let io = |e: std::io::Error| ProfileError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| ProfileError::Corrupt {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    /// Tableau de bord (§24), à partir du catalogue de challenges.
    pub fn dashboard(&self, catalog: &[(String, String)]) -> String {
        let (level, level_name, next) = level_for(self.xp());
        let mut s = String::new();
        let _ = writeln!(s, "TABLEAU DE BORD — {}", self.name);
        let progress = match next {
            Some(n) => format!("{} / {n}", self.xp()),
            None => format!("{} (niveau maximum)", self.xp()),
        };
        let _ = writeln!(s, "Niveau {level} « {level_name} » — XP : {progress}");
        let _ = writeln!(s);

        let mut topics: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (id, topic) in catalog {
            let e = topics.entry(topic.as_str()).or_default();
            e.1 += 1;
            if self.has_completed(id) {
                e.0 += 1;
            }
        }
        // Ordre pédagogique : du câble à la sécurité.
        const ORDER: [&str; 6] = [
            "physical",
            "addressing",
            "switching",
            "routing",
            "services",
            "security",
        ];
        let mut ordered: Vec<(&str, (usize, usize))> = ORDER
            .iter()
            .filter_map(|t| topics.remove(t).map(|v| (*t, v)))
            .collect();
        ordered.extend(topics);
        for (topic, (done, total)) in &ordered {
            let pct = if *total == 0 { 0 } else { done * 100 / total };
            let _ = writeln!(
                s,
                "{:<12} {} {:>3}%  ({done}/{total})",
                topic_label(topic),
                bar(pct),
                pct
            );
        }
        let _ = writeln!(s);
        let known: usize = catalog
            .iter()
            .filter(|(id, _)| self.has_completed(id))
            .count();
        let _ = writeln!(s, "Challenges réussis : {known} / {}", catalog.len());
        let clean = self.completed.values().filter(|c| c.hints == 0).count();
        let _ = writeln!(s, "Sans aucun indice  : {clean}");
        if self.badges.is_empty() {
            let _ = writeln!(s, "Badges : aucun pour l'instant");
        } else {
            let names: Vec<&str> = BADGES
                .iter()
                .filter(|b| self.badges.contains(b.id))
                .map(|b| b.name)
                .collect();
            let _ = writeln!(s, "Badges : {}", names.join(", "));
        }
        s
    }
}

/// Une ligne du classement local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderEntry {
    /// Nom du profil.
    pub name: String,
    /// XP total.
    pub xp: u32,
    /// Niveau.
    pub level: u32,
    /// Nom du niveau.
    pub level_name: &'static str,
    /// Challenges réussis.
    pub completed: usize,
    /// Badges obtenus.
    pub badges: usize,
}

/// Classement des profils d'un dossier : XP décroissant, puis nom. Les fichiers
/// qui ne sont pas des profils lisibles sont ignorés (et comptés dans le
/// second élément du résultat).
pub fn leaderboard(dir: &Path) -> Result<(Vec<LeaderEntry>, usize), ProfileError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ProfileError::Io {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })?;
    let mut board = Vec::new();
    let mut skipped = 0;
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("?")
            .to_string();
        match Profile::load(&path, &stem) {
            Ok(p) if !p.name.is_empty() => {
                let xp = p.xp();
                let (level, level_name, _) = level_for(xp);
                board.push(LeaderEntry {
                    name: p.name.clone(),
                    xp,
                    level,
                    level_name,
                    completed: p.completed_count(),
                    badges: p.badges.len(),
                });
            }
            _ => skipped += 1,
        }
    }
    board.sort_by(|a, b| b.xp.cmp(&a.xp).then_with(|| a.name.cmp(&b.name)));
    Ok((board, skipped))
}

/// Nom français d'un thème.
pub fn topic_label(topic: &str) -> &str {
    match topic {
        "physical" => "Physique",
        "addressing" => "Adressage",
        "switching" => "Commutation",
        "routing" => "Routage",
        "services" => "Services",
        "security" => "Sécurité",
        other => other,
    }
}

/// Barre de progression de 10 cases.
pub fn bar(percent: usize) -> String {
    let filled = (percent.min(100) + 5) / 10;
    format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;

    fn score(total: u32, hints: u32, xp: u32) -> Score {
        Score {
            diagnosis: 0,
            fix: 0,
            time: 0,
            hint_penalty: 0,
            total,
            xp,
            elapsed_secs: 30,
            hints,
        }
    }

    fn ch(id: &str) -> &'static Challenge {
        builtin::challenge(id).unwrap()
    }

    #[test]
    fn levels() {
        assert_eq!(level_for(0), (1, "Apprenti", Some(150)));
        assert_eq!(level_for(149).0, 1);
        assert_eq!(level_for(150), (2, "Technicien", Some(400)));
        assert_eq!(level_for(1_900), (6, "Expert", None));
        assert_eq!(level_for(99_999).0, 6);
    }

    #[test]
    fn recording_accumulates_xp_and_keeps_the_best_result() {
        let mut p = Profile::new("alice");
        let r = p.record(ch("beginner-01"), &score(80, 2, 80));
        assert_eq!((r.xp_gained, r.improved), (80, true));
        assert_eq!(p.xp(), 80);

        // Un moins bon résultat n'enlève rien et ne rapporte rien.
        let r = p.record(ch("beginner-01"), &score(60, 0, 60));
        assert_eq!((r.xp_gained, r.improved), (0, false));
        assert_eq!(p.xp(), 80);
        assert_eq!(
            p.completed["beginner-01"].hints, 2,
            "le meilleur résultat est conservé"
        );

        // Un meilleur résultat ne rapporte que la différence.
        let r = p.record(ch("beginner-01"), &score(100, 0, 100));
        assert_eq!((r.xp_gained, r.improved), (20, true));
        assert_eq!(p.xp(), 100);
        assert_eq!(p.completed["beginner-01"].times, 3);
        assert_eq!(p.completed_count(), 1);
    }

    #[test]
    fn level_ups_are_reported() {
        let mut p = Profile::new("a");
        let r = p.record(ch("advanced-01"), &score(100, 0, 200));
        assert_eq!((r.level_before, r.level_after), (1, 2));
    }

    #[test]
    fn badges_unlock_once_with_clear_rules() {
        let mut p = Profile::new("a");
        let r = p.record(ch("beginner-01"), &score(100, 0, 100));
        assert_eq!(
            r.new_badges.iter().map(|b| b.id).collect::<Vec<_>>(),
            ["network-beginner"]
        );
        // Le même badge n'est pas annoncé deux fois.
        let r = p.record(ch("beginner-02"), &score(100, 0, 100));
        assert!(r.new_badges.is_empty());

        // arp-explorer : panne ARP ou VLAN.
        let r = p.record(ch("advanced-03"), &score(70, 1, 140));
        assert!(r.new_badges.iter().any(|b| b.id == "arp-explorer"));
        // packet-analyst : 3 réussites sans indice (01, 02 déjà) + une de plus.
        let r = p.record(ch("beginner-03"), &score(90, 0, 90));
        assert!(
            r.new_badges.iter().any(|b| b.id == "packet-analyst"),
            "{:?}",
            r.new_badges
        );
        // troubleshooter : 5 challenges.
        let r = p.record(ch("beginner-04"), &score(90, 0, 90));
        assert!(r.new_badges.iter().any(|b| b.id == "troubleshooter"));
        // routing-specialist : 3 de routage (07, 08, 11).
        p.record(ch("intermediate-02"), &score(80, 0, 120));
        p.record(ch("intermediate-03"), &score(80, 0, 120));
        let r = p.record(ch("advanced-01"), &score(80, 0, 160));
        assert!(r.new_badges.iter().any(|b| b.id == "routing-specialist"));
        // network-defender : 2 de sécurité.
        p.record(ch("intermediate-05"), &score(80, 0, 120));
        let r = p.record(ch("advanced-04"), &score(80, 0, 160));
        assert!(r.new_badges.iter().any(|b| b.id == "network-defender"));
        assert_eq!(p.badges.len(), BADGES.len());
    }

    #[test]
    fn saving_and_loading_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles").join("alice.json");
        let mut p = Profile::new("alice");
        p.record(ch("beginner-01"), &score(100, 0, 100));
        p.record_attempt("beginner-02");
        p.mark_lesson_read("arp");
        p.save(&path).unwrap();
        assert!(
            !path.with_extension("json.tmp").exists(),
            "pas de fichier temporaire résiduel"
        );
        assert_eq!(Profile::load(&path, "alice").unwrap(), p);
    }

    #[test]
    fn a_missing_profile_is_a_fresh_one_and_a_corrupt_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = Profile::load(&dir.path().join("nope.json"), "bob").unwrap();
        assert_eq!(fresh, Profile::new("bob"));
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{ pas du json").unwrap();
        let e = Profile::load(&bad, "x").unwrap_err();
        assert!(e.to_string().contains("corrompu"), "{e}");
    }

    #[test]
    fn dashboard_shows_levels_topics_and_badges() {
        let mut p = Profile::new("alice");
        p.record(ch("beginner-01"), &score(100, 0, 100));
        p.record(ch("beginner-04"), &score(100, 0, 100));
        let text = p.dashboard(&builtin::catalog());
        assert!(text.contains("TABLEAU DE BORD — alice"), "{text}");
        assert!(
            text.contains("Niveau 2 « Technicien » — XP : 200 / 400"),
            "{text}"
        );
        assert!(text.contains("Adressage") && text.contains("66%"), "{text}");
        let order: Vec<usize> = [
            "Physique",
            "Adressage",
            "Commutation",
            "Routage",
            "Services",
            "Sécurité",
        ]
        .iter()
        .map(|t| text.find(t).unwrap())
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "ordre des thèmes : {text}"
        );
        assert!(text.contains("Routage") && text.contains("0%"), "{text}");
        assert!(text.contains("Challenges réussis : 2 / 18"), "{text}");
        assert!(text.contains("Network Beginner"), "{text}");
    }

    #[test]
    fn progress_bars() {
        assert_eq!(bar(0), "░░░░░░░░░░");
        assert_eq!(bar(50), "█████░░░░░");
        assert_eq!(bar(100), "██████████");
        assert_eq!(bar(67), "███████░░░");
        assert_eq!(bar(500), "██████████");
    }
}
