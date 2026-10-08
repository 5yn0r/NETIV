//! Mode enseignant : composer un cours à partir de challenges, et le noter.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::builtin;
use crate::profile::Profile;

/// Nature d'un élément de cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    /// Travaux pratiques guidés.
    Lab,
    /// Exercice.
    Exercise,
    /// Challenge.
    Challenge,
    /// Épreuve notée : pas d'indices, temps imparti.
    Exam,
}

impl ItemKind {
    /// Libellé français.
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::Lab => "TP",
            ItemKind::Exercise => "Exercice",
            ItemKind::Challenge => "Challenge",
            ItemKind::Exam => "Examen",
        }
    }
}

fn yes() -> bool {
    true
}

/// Un élément d'un cours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CourseItem {
    /// Nature.
    pub kind: ItemKind,
    /// Titre affiché.
    pub title: String,
    /// Challenge livré à jouer (identifiant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<String>,
    /// Consignes pour l'élève.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    /// Points attribués.
    #[serde(default)]
    pub points: u32,
    /// Durée prévue.
    #[serde(default)]
    pub minutes: u32,
    /// Indices autorisés (toujours refusés pour un examen).
    #[serde(default = "yes")]
    pub allow_hints: bool,
}

impl CourseItem {
    /// Les indices sont-ils réellement disponibles pour cet élément ?
    pub fn hints_allowed(&self) -> bool {
        self.allow_hints && self.kind != ItemKind::Exam
    }
}

/// Un cours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Course {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Objectif pédagogique.
    #[serde(default)]
    pub objective: String,
    /// Niveau visé.
    #[serde(default)]
    pub level: String,
    /// Éléments, dans l'ordre.
    pub items: Vec<CourseItem>,
}

/// Note d'un élève sur un cours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grade {
    /// `(titre, points obtenus, points maximum)`.
    pub items: Vec<(String, u32, u32)>,
    /// Total obtenu.
    pub earned: u32,
    /// Total possible.
    pub max: u32,
}

impl Course {
    /// Lit un cours YAML.
    pub fn from_yaml(text: &str) -> Result<Course, String> {
        serde_yaml_ng::from_str(text).map_err(|e| format!("cours illisible : {e}"))
    }

    /// Écrit le cours en YAML.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(self).map_err(|e| e.to_string())
    }

    /// Cours d'exemple, point de départ pour un enseignant.
    pub fn template(id: &str) -> Course {
        Course {
            id: id.to_string(),
            title: format!("TP {id} — Routage"),
            objective: "Configurer la communication entre trois réseaux.".to_string(),
            level: "intermediate".to_string(),
            items: vec![
                CourseItem {
                    kind: ItemKind::Lab,
                    title: "Découvrir le réseau".to_string(),
                    challenge: None,
                    instructions: "Charge le réseau « office », envoie un ping puis suis le paquet avec « trace ».".to_string(),
                    points: 0,
                    minutes: 10,
                    allow_hints: true,
                },
                CourseItem {
                    kind: ItemKind::Challenge,
                    title: "Il manque une route".to_string(),
                    challenge: Some("intermediate-02".to_string()),
                    instructions: String::new(),
                    points: 10,
                    minutes: 20,
                    allow_hints: true,
                },
                CourseItem {
                    kind: ItemKind::Exam,
                    title: "Contrôle : la réponse ne revient pas".to_string(),
                    challenge: Some("advanced-01".to_string()),
                    instructions: String::new(),
                    points: 10,
                    minutes: 25,
                    allow_hints: false,
                },
            ],
        }
    }

    /// Durée totale prévue.
    pub fn total_minutes(&self) -> u32 {
        self.items.iter().map(|i| i.minutes).sum()
    }

    /// Total des points.
    pub fn total_points(&self) -> u32 {
        self.items.iter().map(|i| i.points).sum()
    }

    /// Problèmes du cours (liste vide = valide).
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.id.trim().is_empty() {
            p.push("identifiant vide".to_string());
        }
        if self.title.trim().is_empty() {
            p.push("titre vide".to_string());
        }
        if self.items.is_empty() {
            p.push("le cours ne contient aucun élément".to_string());
        }
        let mut titles = BTreeSet::new();
        for (n, item) in self.items.iter().enumerate() {
            let who = format!("élément n°{} « {} »", n + 1, item.title);
            if item.title.trim().is_empty() {
                p.push(format!("élément n°{} : titre vide", n + 1));
            } else if !titles.insert(item.title.clone()) {
                p.push(format!("{who} : titre en double"));
            }
            match (&item.challenge, item.kind) {
                (None, ItemKind::Challenge | ItemKind::Exam) => {
                    p.push(format!(
                        "{who} : un {} doit désigner un challenge",
                        item.kind.label().to_lowercase()
                    ));
                }
                (Some(id), _) => match builtin::challenge(id) {
                    None => p.push(format!(
                        "{who} : challenge inconnu « {id} » (voir « challenge list »)"
                    )),
                    Some(c) => {
                        if item.minutes > 0 && item.minutes < c.time_limit_minutes / 2 {
                            p.push(format!(
                                "{who} : {} min prévues pour un challenge conseillé à {} min",
                                item.minutes, c.time_limit_minutes
                            ));
                        }
                    }
                },
                (None, _) => {}
            }
            if item.kind == ItemKind::Lab
                && item.instructions.trim().is_empty()
                && item.challenge.is_none()
            {
                p.push(format!(
                    "{who} : un TP sans consignes ni challenge est vide"
                ));
            }
            if item.kind == ItemKind::Exam && item.points == 0 {
                p.push(format!("{who} : un examen doit rapporter des points"));
            }
        }
        p
    }

    /// Fiche du cours, prête à distribuer.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{}", self.title);
        if !self.objective.is_empty() {
            let _ = writeln!(s, "\nObjectif :\n{}", self.objective);
        }
        let level = if self.level.is_empty() {
            "—"
        } else {
            &self.level
        };
        let _ = writeln!(
            s,
            "\nDurée : {} minutes    Niveau : {level}    Points : {}",
            self.total_minutes(),
            self.total_points()
        );
        let _ = writeln!(s, "\nCourse");
        let last = self.items.len().saturating_sub(1);
        for (i, item) in self.items.iter().enumerate() {
            let branch = if i == last { "└──" } else { "├──" };
            let pts = if item.points > 0 {
                format!(", {} pts", item.points)
            } else {
                String::new()
            };
            let hints = if item.hints_allowed() {
                ""
            } else {
                ", sans indices"
            };
            let _ = writeln!(
                s,
                " {branch} {} : {} ({} min{pts}{hints})",
                item.kind.label(),
                item.title,
                item.minutes
            );
        }
        s
    }

    /// Fiche au format Markdown.
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# {}\n", self.title);
        if !self.objective.is_empty() {
            let _ = writeln!(s, "**Objectif.** {}\n", self.objective);
        }
        let _ = writeln!(
            s,
            "*Durée : {} min · Niveau : {} · Total : {} points*\n",
            self.total_minutes(),
            if self.level.is_empty() {
                "—"
            } else {
                &self.level
            },
            self.total_points()
        );
        for (i, item) in self.items.iter().enumerate() {
            let _ = writeln!(s, "## {}. {} — {}\n", i + 1, item.kind.label(), item.title);
            let _ = writeln!(s, "- Durée : {} min", item.minutes);
            if item.points > 0 {
                let _ = writeln!(s, "- Points : {}", item.points);
            }
            if !item.hints_allowed() {
                let _ = writeln!(s, "- Indices : interdits");
            }
            if let Some(c) = item.challenge.as_deref().and_then(builtin::challenge) {
                let _ = writeln!(s, "- Challenge : `{}` — {}", c.id, c.title);
                let _ = writeln!(s, "\n> {}", c.description.trim().replace('\n', "\n> "));
            }
            if !item.instructions.is_empty() {
                let _ = writeln!(s, "\n{}", item.instructions.trim());
            }
            let _ = writeln!(s);
        }
        s
    }

    /// Note un élève : points × (meilleur score / 100) pour chaque challenge réussi.
    pub fn grade(&self, profile: &Profile) -> Grade {
        let mut items = Vec::new();
        for item in &self.items {
            let earned = item
                .challenge
                .as_ref()
                .and_then(|id| profile.completed.get(id))
                .map_or(0, |c| (item.points * c.score + 50) / 100);
            items.push((item.title.clone(), earned, item.points));
        }
        Grade {
            earned: items.iter().map(|i| i.1).sum(),
            max: items.iter().map(|i| i.2).sum(),
            items,
        }
    }
}

impl Grade {
    /// Relevé de notes.
    pub fn render(&self, who: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "RELEVÉ — {who}");
        for (title, earned, max) in &self.items {
            if *max > 0 {
                let _ = writeln!(s, "  {earned:>3} / {max:<3} {title}");
            }
        }
        let _ = writeln!(s, "  {:>3} / {:<3} TOTAL", self.earned, self.max);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Score;

    fn done(profile: &mut Profile, id: &str, total: u32) {
        profile.record(
            builtin::challenge(id).unwrap(),
            &Score {
                diagnosis: 0,
                fix: 0,
                time: 0,
                hint_penalty: 0,
                total,
                xp: total,
                elapsed_secs: 1,
                hints: 0,
            },
        );
    }

    #[test]
    fn the_template_is_valid_and_roundtrips() {
        let c = Course::template("04");
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        let again = Course::from_yaml(&c.to_yaml().unwrap()).unwrap();
        assert_eq!(c, again);
        assert_eq!(c.total_points(), 20);
        assert_eq!(c.total_minutes(), 55);
    }

    #[test]
    fn exams_never_allow_hints() {
        let c = Course::template("x");
        assert!(c.items[1].hints_allowed());
        let mut exam = c.items[2].clone();
        exam.allow_hints = true;
        assert!(
            !exam.hints_allowed(),
            "un examen interdit toujours les indices"
        );
    }

    #[test]
    fn invalid_courses_are_diagnosed() {
        let mut c = Course::template("x");
        c.id = String::new();
        c.items[1].challenge = Some("nope-99".into());
        c.items[2].title = c.items[1].title.clone();
        c.items[2].challenge = None;
        c.items[0].instructions.clear();
        c.items[0].minutes = 5;
        let all = c.problems().join("\n");
        for needle in [
            "identifiant vide",
            "challenge inconnu « nope-99 »",
            "titre en double",
            "doit désigner un challenge",
            "TP sans consignes",
        ] {
            assert!(all.contains(needle), "« {needle} » manque dans :\n{all}");
        }
        assert!(
            Course {
                items: vec![],
                ..Course::template("x")
            }
            .problems()
            .iter()
            .any(|p| p.contains("aucun élément"))
        );
    }

    #[test]
    fn an_unrealistic_duration_is_flagged() {
        let mut c = Course::template("x");
        c.items[1].minutes = 2;
        assert!(
            c.problems().iter().any(|p| p.contains("conseillé à")),
            "{:?}",
            c.problems()
        );
    }

    #[test]
    fn an_exam_without_points_is_flagged() {
        let mut c = Course::template("x");
        c.items[2].points = 0;
        assert!(
            c.problems()
                .iter()
                .any(|p| p.contains("doit rapporter des points"))
        );
    }

    #[test]
    fn rendering_matches_the_course_sheet_layout() {
        let text = Course::template("04").render();
        assert!(text.contains("TP 04 — Routage"), "{text}");
        assert!(text.contains("Objectif :"), "{text}");
        assert!(text.contains("Durée : 55 minutes"), "{text}");
        assert!(
            text.contains("├── TP : Découvrir le réseau (10 min)"),
            "{text}"
        );
        assert!(
            text.contains(
                "└── Examen : Contrôle : la réponse ne revient pas (25 min, 10 pts, sans indices)"
            ),
            "{text}"
        );
    }

    #[test]
    fn markdown_export_embeds_the_challenge_statement() {
        let md = Course::template("04").to_markdown();
        assert!(md.starts_with("# TP 04 — Routage"), "{md}");
        assert!(md.contains("## 2. Challenge — Il manque une route"), "{md}");
        assert!(md.contains("`intermediate-02`"), "{md}");
        assert!(md.contains("> PC1 ne joint plus SRV"), "{md}");
        assert!(md.contains("Indices : interdits"), "{md}");
    }

    #[test]
    fn grading_scales_points_by_the_best_score() {
        let c = Course::template("x");
        let mut p = Profile::new("alice");
        let g = c.grade(&p);
        assert_eq!((g.earned, g.max), (0, 20));
        done(&mut p, "intermediate-02", 80);
        let g = c.grade(&p);
        assert_eq!(g.items[1], ("Il manque une route".to_string(), 8, 10));
        done(&mut p, "advanced-01", 55);
        let g = c.grade(&p);
        assert_eq!(
            (g.earned, g.max),
            (8 + 6, 20),
            "55 % de 10 points arrondi à 6"
        );
        let text = g.render("alice");
        assert!(
            text.contains("RELEVÉ — alice") && text.contains("TOTAL"),
            "{text}"
        );
        assert!(
            !text.contains("Découvrir"),
            "les éléments sans points n'apparaissent pas au relevé"
        );
    }

    #[test]
    fn unknown_fields_in_a_course_file_are_refused() {
        assert!(Course::from_yaml("id: x\ntitle: y\nitems: []\nextra: 1").is_err());
        assert!(Course::from_yaml("id: x").is_err());
    }
}
