//! Construction d'un réseau : lire, valider et remplacer la topologie du lab
//! (éditeur de l'interface web, `topology autoconfig`).
//!
//! L'éditeur travaille sur la **description** du réseau (`TopologyDef`) : le
//! moteur sait ajouter des équipements mais pas en retirer, et reconstruire
//! une simulation ne coûte qu'environ une milliseconde. Chaque modification
//! validée reconstruit donc le lab (la trace repart de zéro).

use netiv_topology::autoconf::autoconfigure;
use netiv_topology::{TopologyDef, apply_layout, build, export, layout_of};

use super::*;

/// Pourquoi une modification de topologie est refusée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// Le lab ne peut pas être modifié maintenant (défi en cours…).
    Locked(String),
    /// La topologie proposée est invalide : un message par problème.
    Invalid(Vec<String>),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Locked(why) => write!(f, "{why}"),
            EditError::Invalid(errors) => {
                write!(f, "topologie invalide :")?;
                for e in errors {
                    write!(f, "\n- {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for EditError {}

impl Session {
    /// Topologie actuelle du lab (configuration modifiée et positions
    /// comprises), ou erreur si aucun réseau n'est chargé.
    pub(super) fn current_def(&self) -> Result<TopologyDef, CliError> {
        let mut def = export(self.sim()?, &self.name);
        apply_layout(&mut def, &self.layout);
        Ok(def)
    }

    /// Topologie actuelle du lab ; `None` si aucun réseau n'est chargé.
    pub fn topology_def(&self) -> Option<TopologyDef> {
        self.current_def().ok()
    }

    /// Raison pour laquelle le réseau ne peut pas être modifié, s'il y en a
    /// une : pendant un défi ou un cyber range, redessiner le réseau serait
    /// une façon trop simple de « réparer ».
    pub fn edit_lock(&self) -> Option<String> {
        if self.run.as_ref().is_some_and(|r| !r.is_finished()) {
            return Some(
                "un challenge est en cours : le réseau ne peut pas être redessiné (« challenge giveup » pour arrêter)"
                    .to_string(),
            );
        }
        if self.range_run.as_ref().is_some_and(|r| !r.is_finished()) {
            return Some(
                "un scénario de cyber range est en cours : le réseau ne peut pas être redessiné (« range giveup » pour arrêter)"
                    .to_string(),
            );
        }
        None
    }

    /// Vérifie une topologie sans rien modifier : `(erreurs, avertissements)`.
    pub fn validate_topology(def: &TopologyDef) -> (Vec<String>, Vec<String>) {
        match build(def) {
            Ok(loaded) => (Vec::new(), loaded.warnings),
            Err(e) => (e.errors, Vec::new()),
        }
    }

    /// Remplace le réseau du lab par `def` et rend les avertissements.
    ///
    /// La simulation est reconstruite (trace vide, `t = 0`) ; le nom du lab,
    /// le profil et la ligne de base éventuelle de `chaos` sont oubliés comme
    /// pour un chargement.
    pub fn apply_topology(&mut self, def: &TopologyDef) -> Result<Vec<String>, EditError> {
        if let Some(why) = self.edit_lock() {
            return Err(EditError::Locked(why));
        }
        let loaded = build(def).map_err(|e| EditError::Invalid(e.errors))?;
        self.sim = Some(loaded.sim);
        self.name = loaded.name;
        self.layout = layout_of(def);
        self.chaos = None;
        self.baseline = None;
        self.last_cause = None;
        self.hint_level = 0;
        self.run = None;
        self.range_run = None;
        Ok(loaded.warnings)
    }

    /// Met à jour les positions sur le schéma, sans toucher au réseau
    /// (déplacement d'un équipement dans l'interface). Les noms inconnus sont
    /// ignorés.
    pub fn set_positions(&mut self, positions: &Layout) {
        let Some(sim) = &self.sim else { return };
        for (name, p) in positions {
            if sim.device_id(name).is_ok() {
                self.layout.insert(name.clone(), *p);
            }
        }
    }

    /// Positions connues des équipements.
    pub fn positions(&self) -> &Layout {
        &self.layout
    }

    /// Complète l'adressage du réseau (voir `netiv_topology::autoconf`) et
    /// rend la liste des changements.
    pub fn autoconfigure(&mut self) -> Result<Vec<String>, EditError> {
        if let Some(why) = self.edit_lock() {
            return Err(EditError::Locked(why));
        }
        let mut def = self
            .current_def()
            .map_err(|e| EditError::Invalid(vec![e.0]))?;
        let report = autoconfigure(&mut def);
        if !report.is_empty() {
            self.apply_topology(&def)?;
        }
        Ok(report)
    }

    /// `topology autoconfig` dans la CLI.
    pub(super) fn cmd_autoconfig(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        let report = self.autoconfigure()?;
        if report.is_empty() {
            writeln!(
                out,
                "Rien à compléter : toutes les interfaces câblées ont déjà une adresse."
            )?;
            return Ok(());
        }
        writeln!(
            out,
            "Adressage automatique ({} changement(s)) :",
            report.len()
        )?;
        for line in &report {
            writeln!(out, "  {line}")?;
        }
        writeln!(
            out,
            "La trace repart de zéro. Essaie un ping pour vérifier."
        )?;
        Ok(())
    }
}
