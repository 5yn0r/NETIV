//! Learning Mode : des parcours de leçons dont la progression se lit dans le profil.

use std::fmt::Write as _;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::profile::Profile;

/// Une leçon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Texte de la leçon.
    pub text: String,
    /// Commandes à essayer.
    #[serde(default)]
    pub r#try: Vec<String>,
    /// Challenges qui valident la leçon. Vide : la leçon est validée en la lisant.
    #[serde(default)]
    pub challenges: Vec<String>,
}

/// Un parcours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningPath {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Description.
    pub description: String,
    /// Leçons, dans l'ordre.
    pub lessons: Vec<Lesson>,
}

/// Les parcours livrés.
pub fn paths() -> &'static [LearningPath] {
    static CACHE: OnceLock<Vec<LearningPath>> = OnceLock::new();
    CACHE.get_or_init(|| {
        [include_str!("../content/paths/fundamentals.yaml")]
            .iter()
            .filter_map(|t| serde_yaml_ng::from_str(t).ok())
            .collect()
    })
}

impl Lesson {
    /// La leçon est-elle validée pour ce profil ?
    pub fn is_done(&self, profile: &Profile) -> bool {
        if self.challenges.is_empty() {
            profile.lessons_read.contains(&self.id)
        } else {
            self.challenges.iter().all(|c| profile.has_completed(c))
        }
    }
}

impl LearningPath {
    /// Leçon par numéro (à partir de 1) ou par identifiant.
    pub fn lesson(&self, key: &str) -> Option<(usize, &Lesson)> {
        match key.parse::<usize>() {
            Ok(n) if n >= 1 => self.lessons.get(n - 1).map(|l| (n, l)),
            _ => self
                .lessons
                .iter()
                .enumerate()
                .find(|(_, l)| l.id == key)
                .map(|(i, l)| (i + 1, l)),
        }
    }

    /// Nombre de leçons validées.
    pub fn done_count(&self, profile: &Profile) -> usize {
        self.lessons.iter().filter(|l| l.is_done(profile)).count()
    }

    /// Première leçon non validée.
    pub fn next_lesson(&self, profile: &Profile) -> Option<(usize, &Lesson)> {
        self.lessons
            .iter()
            .enumerate()
            .find(|(_, l)| !l.is_done(profile))
            .map(|(i, l)| (i + 1, l))
    }

    /// Liste à cocher (§20).
    pub fn render(&self, profile: &Profile) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "PARCOURS : {}", self.title.to_uppercase());
        let _ = writeln!(s, "{}", self.description);
        let _ = writeln!(s);
        for (i, l) in self.lessons.iter().enumerate() {
            let mark = if l.is_done(profile) { "✓" } else { " " };
            let how = if l.challenges.is_empty() {
                String::new()
            } else {
                format!("  [challenges : {}]", l.challenges.join(", "))
            };
            let _ = writeln!(s, "[{mark}] {:>2}. {}{how}", i + 1, l.title);
        }
        let _ = writeln!(
            s,
            "\nProgression : {}/{}",
            self.done_count(profile),
            self.lessons.len()
        );
        if let Some((n, l)) = self.next_lesson(profile) {
            let _ = writeln!(s, "Prochaine leçon : {n}. {} (« learn read {n} »)", l.title);
        } else {
            let _ = writeln!(s, "Parcours terminé, bravo !");
        }
        s
    }

    /// Texte d'une leçon.
    pub fn render_lesson(&self, n: usize, lesson: &Lesson) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "LEÇON {n}/{} — {}", self.lessons.len(), lesson.title);
        let _ = writeln!(s);
        let _ = writeln!(s, "{}", lesson.text.trim_end());
        if !lesson.r#try.is_empty() {
            let _ = writeln!(s, "\nÀ essayer (sur examples/mvp.yaml) :");
            for t in &lesson.r#try {
                let _ = writeln!(s, "  netiv> {t}");
            }
        }
        if !lesson.challenges.is_empty() {
            let _ = writeln!(
                s,
                "\nPour valider cette leçon : réussis {} (« challenge start <id> »).",
                lesson.challenges.join(" et ")
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin;
    use crate::run::Score;

    fn score() -> Score {
        Score {
            diagnosis: 40,
            fix: 40,
            time: 20,
            hint_penalty: 0,
            total: 100,
            xp: 100,
            elapsed_secs: 5,
            hints: 0,
        }
    }

    #[test]
    fn the_shipped_path_is_well_formed() {
        let p = &paths()[0];
        assert_eq!(p.id, "fundamentals");
        assert_eq!(p.lessons.len(), 10);
        let ids: std::collections::BTreeSet<_> = p.lessons.iter().map(|l| &l.id).collect();
        assert_eq!(ids.len(), p.lessons.len(), "identifiants uniques");
        for l in &p.lessons {
            assert!(!l.text.trim().is_empty());
            for c in &l.challenges {
                assert!(
                    builtin::challenge(c).is_some(),
                    "leçon {} : challenge inconnu {c}",
                    l.id
                );
            }
        }
    }

    #[test]
    fn lessons_are_validated_by_reading_or_by_challenges() {
        let p = &paths()[0];
        let mut profile = Profile::new("a");
        assert_eq!(p.done_count(&profile), 0);
        assert_eq!(p.next_lesson(&profile).unwrap().1.id, "ethernet-mac");

        profile.mark_lesson_read("ethernet-mac");
        profile.mark_lesson_read("arp");
        assert_eq!(p.done_count(&profile), 2);
        assert_eq!(p.next_lesson(&profile).unwrap().1.id, "ipv4");

        // Lire une leçon à challenges ne suffit pas.
        profile.mark_lesson_read("ipv4");
        assert!(!p.lesson("ipv4").unwrap().1.is_done(&profile));
        profile.record(builtin::challenge("beginner-04").unwrap(), &score());
        assert!(p.lesson("ipv4").unwrap().1.is_done(&profile));

        // Une leçon à deux challenges exige les deux.
        profile.record(builtin::challenge("beginner-02").unwrap(), &score());
        assert!(!p.lesson("physical").unwrap().1.is_done(&profile));
        profile.record(builtin::challenge("beginner-03").unwrap(), &score());
        assert!(p.lesson("physical").unwrap().1.is_done(&profile));
    }

    #[test]
    fn lookup_by_number_or_id() {
        let p = &paths()[0];
        assert_eq!(p.lesson("2").unwrap().1.id, "arp");
        assert_eq!(p.lesson("arp").unwrap().0, 2);
        assert!(p.lesson("0").is_none());
        assert!(p.lesson("99").is_none());
        assert!(p.lesson("nope").is_none());
    }

    #[test]
    fn rendering_shows_a_checklist() {
        let p = &paths()[0];
        let mut profile = Profile::new("a");
        profile.mark_lesson_read("ethernet-mac");
        let text = p.render(&profile);
        assert!(
            text.contains("PARCOURS : FONDAMENTAUX DES RÉSEAUX"),
            "{text}"
        );
        assert!(text.contains("[✓]  1. Ethernet et adresses MAC"), "{text}");
        assert!(text.contains("[ ]  2. ARP"), "{text}");
        assert!(text.contains("Progression : 1/10"), "{text}");
        assert!(text.contains("learn read 2"), "{text}");
    }

    #[test]
    fn lesson_text_includes_things_to_try() {
        let p = &paths()[0];
        let (n, l) = p.lesson("arp").unwrap();
        let text = p.render_lesson(n, l);
        assert!(text.contains("LEÇON 2/10"));
        assert!(text.contains("netiv> show arp PC1"));
        let (n, l) = p.lesson("ipv4").unwrap();
        assert!(p.render_lesson(n, l).contains("beginner-04"));
    }
}
