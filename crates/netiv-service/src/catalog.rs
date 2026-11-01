//! Catalogue des épreuves livrées (challenges et cyber ranges), pour les
//! plateformes qui les présentent hors du simulateur : pages de catalogue,
//! référencement, filtres. Il ne contient aucune solution : elles sont à part
//! (`solutions`), pour le seul serveur de la plateforme (tuteur, explications).

use netiv_education::{Ticket, builtin};
use serde::Serialize;

/// Version du format du catalogue.
pub const CATALOG_FORMAT_VERSION: u32 = 1;

/// Une épreuve du catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    /// `challenge` ou `range` (cyber range).
    pub kind: &'static str,
    /// Identifiant stable (`beginner-01`, `range-01`) : à passer à
    /// `POST /api/labs` (`{"challenge": id}` ou `{"range": id}`).
    pub id: String,
    /// Titre.
    pub title: String,
    /// Énoncé (contexte et objectif), sans la solution.
    pub description: String,
    /// Thème : `physical`, `addressing`, `switching`, `routing`, `services`
    /// ou `security` (toujours `security` pour un cyber range).
    pub topic: String,
    /// Difficulté : `easy`, `medium` ou `hard`.
    pub difficulty: &'static str,
    /// Durée conseillée, en minutes.
    pub minutes: u32,
    /// Parcours (`support` pour les tickets de support IT), absent sinon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
    /// Ticket de support à l'origine de l'épreuve, absent sinon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<Ticket>,
}

/// Le catalogue complet.
#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    /// Version du format.
    pub format_version: u32,
    /// Version de NETIV qui l'a produit.
    pub netiv_version: &'static str,
    /// Épreuves : les challenges puis les cyber ranges, dans l'ordre livré.
    pub entries: Vec<CatalogEntry>,
}

/// Le catalogue des épreuves livrées avec cette version de NETIV.
pub fn catalog() -> Catalog {
    let challenges = builtin::challenges().iter().map(|c| CatalogEntry {
        kind: "challenge",
        id: c.id.clone(),
        title: c.title.clone(),
        description: c.description.trim().to_string(),
        topic: c.topic.clone(),
        difficulty: c.difficulty.slug(),
        minutes: c.time_limit_minutes,
        track: c.track.clone(),
        ticket: c.ticket.clone(),
    });
    let ranges = builtin::ranges().iter().map(|r| CatalogEntry {
        kind: "range",
        id: r.id.clone(),
        title: r.title.clone(),
        description: r.description.trim().to_string(),
        topic: "security".to_string(),
        difficulty: r.difficulty.slug(),
        minutes: r.time_limit_minutes,
        track: None,
        ticket: None,
    });
    Catalog {
        format_version: CATALOG_FORMAT_VERSION,
        netiv_version: env!("CARGO_PKG_VERSION"),
        entries: challenges.chain(ranges).collect(),
    }
}

/// La solution d'une épreuve : réservée au serveur de la plateforme, jamais
/// envoyée au navigateur.
#[derive(Debug, Clone, Serialize)]
pub struct SolutionEntry {
    /// Identifiant de l'épreuve.
    pub id: String,
    /// `challenge` ou `range`.
    pub kind: &'static str,
    /// Équipement en faute (challenge) ou machine coupable (range).
    pub device: String,
    /// Cause (`wrong-gateway`…) ou comportement (`port-scan`…).
    pub cause: String,
    /// Explication de la panne ou de l'attaque.
    pub explanation: String,
    /// Commandes NETIV qui corrigent.
    pub commands: Vec<String>,
}

/// Les solutions des épreuves livrées, dans l'ordre du catalogue.
pub fn solutions() -> Vec<SolutionEntry> {
    let challenges = builtin::challenges().iter().map(|c| SolutionEntry {
        id: c.id.clone(),
        kind: "challenge",
        device: c.solution.device.clone(),
        cause: c.solution.cause.clone(),
        explanation: c.solution.explanation.trim().to_string(),
        commands: c.solution.commands.clone(),
    });
    let ranges = builtin::ranges().iter().map(|r| SolutionEntry {
        id: r.id.clone(),
        kind: "range",
        device: r.culprit.device.clone(),
        cause: r.culprit.behavior.clone(),
        explanation: r.solution.explanation.trim().to_string(),
        commands: r.solution.commands.clone(),
    });
    challenges.chain(ranges).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_lists_every_shipped_challenge_and_range() {
        let c = catalog();
        let count = |k: &str| c.entries.iter().filter(|e| e.kind == k).count();
        assert_eq!(count("challenge"), 36);
        assert_eq!(count("range"), 7);
        for e in &c.entries {
            assert!(!e.title.is_empty() && !e.description.is_empty(), "{e:?}");
            assert!(["easy", "medium", "hard"].contains(&e.difficulty), "{e:?}");
            assert!(e.minutes > 0, "{e:?}");
        }
        let mut ids: Vec<&str> = c.entries.iter().map(|e| e.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), c.entries.len(), "identifiants uniques");
        let support: Vec<_> = c
            .entries
            .iter()
            .filter(|e| e.track.as_deref() == Some("support"))
            .collect();
        assert_eq!(support.len(), 10);
        assert!(support.iter().all(|e| e.ticket.is_some()), "{support:?}");
        let system: Vec<_> = c
            .entries
            .iter()
            .filter(|e| e.track.as_deref() == Some("system"))
            .collect();
        assert_eq!(system.len(), 4);
        let sol = solutions();
        assert_eq!(
            sol.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            c.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>()
        );
        assert!(
            sol.iter().all(|s| !s.cause.is_empty()
                && !s.explanation.is_empty()
                && !s.commands.is_empty())
        );
        assert!(system.iter().all(|e| e.ticket.is_some()), "{system:?}");
    }
}
