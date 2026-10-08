//! Chaos Engine : choisit et injecte des pannes pertinentes.
//!
//! Une panne tirée au hasard sur un réseau n'a de sens que si elle **touche
//! le flux qu'on observe**. Le moteur commence donc par relever le chemin réel
//! d'un flux sain (équipements, câbles, ports, tables ARP utilisées), puis ne
//! propose que des pannes qui coupent ce chemin.

mod catalog;
mod path;

use std::net::Ipv4Addr;

use netiv_core::packet::L4Proto;
use netiv_core::{DeviceId, NetivError, SimRng, Simulation};
use netiv_diagnostics::{CauseKind, Layer};

pub use catalog::{Template, catalog};
pub use path::PathInfo;

/// Difficulté d'un diagnostic.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    /// Symptôme direct, cause visible dans la configuration.
    Easy,
    /// Demande de suivre le paquet saut par saut.
    Medium,
    /// Cause indirecte (retour, boucle, ARP...).
    Hard,
}

impl Difficulty {
    /// Nom court.
    pub fn slug(self) -> &'static str {
        match self {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        }
    }

    /// Lit un nom court.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "easy" | "facile" | "beginner" => Some(Difficulty::Easy),
            "medium" | "moyen" | "intermediate" => Some(Difficulty::Medium),
            "hard" | "difficile" | "advanced" => Some(Difficulty::Hard),
            _ => None,
        }
    }
}

/// Le flux observé : « `src` doit pouvoir joindre `dst` (sur ce service) ».
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    /// Émetteur.
    pub src: DeviceId,
    /// Destination.
    pub dst: Ipv4Addr,
    /// Service visé ; `None` = ping.
    pub service: Option<(L4Proto, u16)>,
}

impl Flow {
    /// La requête de diagnostic équivalente.
    pub fn request(&self) -> netiv_diagnostics::Request {
        netiv_diagnostics::Request {
            src: self.src,
            dst: self.dst,
            service: self.service,
        }
    }
}

/// Restrictions du tirage.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Couche voulue.
    pub layer: Option<Layer>,
    /// Difficulté voulue.
    pub difficulty: Option<Difficulty>,
    /// Natures de panne autorisées.
    pub kinds: Option<Vec<CauseKind>>,
    /// Identifiants de gabarits autorisés (voir `catalog()`).
    pub templates: Option<Vec<&'static str>>,
}

/// Une panne injectée par le moteur.
#[derive(Clone, Debug)]
pub struct Injection {
    /// Gabarit utilisé.
    pub template: &'static str,
    /// Nature de la panne (la réponse attendue du diagnostic).
    pub kind: CauseKind,
    /// Difficulté.
    pub difficulty: Difficulty,
    /// Numéro de la panne dans la simulation.
    pub fault_id: u64,
    /// Description lisible.
    pub description: String,
}

/// Erreurs du Chaos Engine.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChaosError {
    /// Le flux ne fonctionne pas sur le réseau sain.
    #[error(
        "le flux ne fonctionne pas sur le réseau sain : impossible de choisir une panne pertinente"
    )]
    FlowNotWorking,
    /// Aucune panne ne correspond aux critères.
    #[error("aucune panne du catalogue ne correspond à ces critères sur ce chemin")]
    NothingApplicable,
    /// Erreur du moteur.
    #[error(transparent)]
    Engine(#[from] NetivError),
}

/// Le Chaos Engine pour un réseau et un flux donnés.
#[derive(Clone, Debug)]
pub struct Chaos {
    /// Le réseau tel qu'il doit fonctionner (copie saine).
    pub baseline: Simulation,
    /// Le flux observé.
    pub flow: Flow,
    path: PathInfo,
    /// Pannes injectées par ce moteur.
    pub injected: Vec<Injection>,
    draws: u64,
}

impl Chaos {
    /// Prépare le moteur : relève le chemin du flux sur `sim` (qui doit être sain).
    pub fn new(sim: &Simulation, flow: Flow) -> Result<Self, ChaosError> {
        let path = PathInfo::trace(sim, &flow)?;
        Ok(Chaos {
            baseline: sim.fresh_clone(),
            flow,
            path,
            injected: Vec::new(),
            draws: 0,
        })
    }

    /// Chemin relevé.
    pub fn path(&self) -> &PathInfo {
        &self.path
    }

    /// Gabarits applicables à ce chemin (avant tirage), selon `filter`.
    pub fn applicable(&self, filter: &Filter) -> Vec<&'static Template> {
        catalog()
            .iter()
            .filter(|t| filter.layer.is_none_or(|l| t.kind.layer() == l))
            .filter(|t| filter.difficulty.is_none_or(|d| t.difficulty == d))
            .filter(|t| filter.kinds.as_ref().is_none_or(|k| k.contains(&t.kind)))
            .filter(|t| {
                filter
                    .templates
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&t.id))
            })
            .collect()
    }

    /// Tire et injecte une panne. `visible` : annoncée dans la trace.
    ///
    /// Même graine + même réseau + mêmes critères → même panne.
    pub fn start(
        &mut self,
        sim: &mut Simulation,
        filter: &Filter,
        seed: u64,
        visible: bool,
    ) -> Result<Injection, ChaosError> {
        let mut rng = SimRng::new(seed ^ self.draws.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        self.draws += 1;
        let mut candidates = self.applicable(filter);
        // Mélange de Fisher-Yates déterministe.
        for i in (1..candidates.len()).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            candidates.swap(i, j);
        }
        for t in candidates {
            let Some(fault) = (t.build)(sim, &self.path, &mut rng) else {
                continue;
            };
            let id = sim.inject_fault(fault, visible)?;
            let description = sim
                .faults()
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.description.clone())
                .unwrap_or_default();
            let inj = Injection {
                template: t.id,
                kind: t.kind,
                difficulty: t.difficulty,
                fault_id: id,
                description,
            };
            self.injected.push(inj.clone());
            return Ok(inj);
        }
        Err(ChaosError::NothingApplicable)
    }

    /// Répare toutes les pannes injectées et en vigueur. Retourne leur nombre.
    pub fn undo_all(&mut self, sim: &mut Simulation, visible: bool) -> usize {
        let mut n = 0;
        for inj in self.injected.iter().rev() {
            if sim.repair_fault(inj.fault_id, visible).is_ok() {
                n += 1;
            }
        }
        n
    }
}
