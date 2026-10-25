//! Logique de l'API NETIV, sans HTTP ni runtime asynchrone.
//!
//! Un [`Service`] gère un ensemble de labs (chacun est une [`Session`] de la
//! CLI, en bac à sable) et répond aux opérations de l'API : créer un lab,
//! exécuter une commande, lire la trace, analyser, éditer la topologie...
//! Chaque opération est une méthode typée qui rend une structure `serde` ou
//! une [`ServiceError`] (sans code HTTP : c'est l'appelant qui traduit).
//!
//! Deux couches s'appuient dessus, pour que la logique ne soit écrite qu'une
//! fois :
//! * `netiv-api` : le serveur HTTP de `netiv serve` (axum) ;
//! * `netiv-wasm` : le moteur dans le navigateur, via [`Service::request`]
//!   qui reproduit les routes de l'API HTTP.

pub mod catalog;
mod request;
pub mod snapshot;

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use netiv_analysis::{DocFormat, generate_docs, health, what_if};
use netiv_cli::storage::{default_storage, load_profile};
use netiv_cli::{EditError, Flow, Session, Storage};
use netiv_core::{Simulation, TraceId};
use netiv_education::profile::{level_for, rank};
use netiv_education::{BADGES, Profile, builtin, learn};
use netiv_security::analyze;
use netiv_topology::{
    Bundle, BundleMeta, Layout, NetworkDef, TOPOLOGY_SCHEMA_VERSION, TopologyDef,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use catalog::{Catalog, CatalogEntry, catalog};
pub use request::Reply;

/// Nombre maximal de labs ouverts en même temps.
pub const MAX_LABS: usize = 16;
/// Longueur maximale d'une ligne de commande.
pub const MAX_LINE: usize = 1_000;
/// Longueur maximale d'un nom de réseau créé depuis l'interface.
pub const MAX_NAME: usize = 60;
/// Nombre maximal d'équipements d'une topologie envoyée par l'éditeur.
pub const MAX_DEVICES: usize = 2_000;
/// Nombre maximal de câbles d'une topologie envoyée par l'éditeur.
pub const MAX_LINKS: usize = 4_000;
/// Taille maximale d'une requête (corps JSON).
pub const MAX_BODY: usize = 1 << 20;

/// État complet d'un lab, tel que l'affiche l'interface (voir [`snapshot`]).
pub type Snapshot = Value;

// ---------------------------------------------------------------------------
// Erreurs
// ---------------------------------------------------------------------------

/// Nature d'une erreur métier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Demande invalide (paramètre, topologie, commande de création...).
    BadRequest,
    /// Lab, transaction ou équipement inconnu.
    NotFound,
    /// Action interdite (analyse pendant un challenge).
    Forbidden,
    /// Action impossible dans l'état actuel (lab verrouillé par un challenge).
    Conflict,
}

/// Erreur d'une opération du service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    /// Nature de l'erreur.
    pub kind: ErrorKind,
    /// Message pour l'utilisateur, en français.
    pub message: String,
}

impl ServiceError {
    /// Demande invalide.
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadRequest, msg)
    }

    /// Élément introuvable.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, msg)
    }

    fn new(kind: ErrorKind, msg: impl Into<String>) -> Self {
        ServiceError {
            kind,
            message: msg.into(),
        }
    }

    /// Code HTTP équivalent (400, 404, 403, 409).
    pub fn status(&self) -> u16 {
        match self.kind {
            ErrorKind::BadRequest => 400,
            ErrorKind::NotFound => 404,
            ErrorKind::Forbidden => 403,
            ErrorKind::Conflict => 409,
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

/// Résultat d'une opération.
pub type ServiceResult<T> = Result<T, ServiceError>;

fn no_network() -> ServiceError {
    ServiceError::bad("aucun réseau chargé")
}

fn unknown_lab(id: &str) -> ServiceError {
    ServiceError::not_found(format!("lab inconnu : « {id} »"))
}

/// Erreur d'édition : 409 si le lab est verrouillé (défi en cours), 400 avec
/// la liste des problèmes si la topologie est invalide.
fn edit_error(e: EditError) -> ServiceError {
    match e {
        EditError::Locked(why) => ServiceError::new(ErrorKind::Conflict, why),
        EditError::Invalid(errors) => {
            ServiceError::bad(format!("topologie invalide :\n- {}", errors.join("\n- ")))
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration et règles de sécurité
// ---------------------------------------------------------------------------

/// Réglages du service.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Nom du profil d'élève.
    pub profile_name: String,
    /// Dossier des profils (progression sauvegardée) ; `None` : en mémoire.
    pub profile_dir: Option<PathBuf>,
    /// Dossier de la bibliothèque de labs.
    pub labs_dir: PathBuf,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            profile_name: "eleve".to_string(),
            profile_dir: None,
            labs_dir: PathBuf::from("labs"),
        }
    }
}

/// Nom de fichier sûr : lettres, chiffres, `-` et `_`.
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "eleve".to_string() } else { s }
}

/// Mots sûrs pour être recopiés dans une commande (jamais de séparateur).
pub fn token(s: &str, what: &str) -> ServiceResult<String> {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Ok(s.to_string())
    } else {
        Err(ServiceError::bad(format!("{what} invalide : « {s} »")))
    }
}

/// Les analyses qui trahiraient la panne d'un challenge sont verrouillées.
pub fn not_during_challenge(s: &Session) -> ServiceResult<()> {
    if s.challenge_info().is_some_and(|c| !c.finished) {
        return Err(ServiceError::new(
            ErrorKind::Forbidden,
            "indisponible pendant un challenge : c'est à toi de trouver",
        ));
    }
    Ok(())
}

/// Refuse les topologies démesurées envoyées par l'éditeur.
pub fn check_size(def: &TopologyDef) -> ServiceResult<()> {
    if def.devices.len() > MAX_DEVICES || def.links.len() > MAX_LINKS {
        return Err(ServiceError::bad(format!(
            "topologie trop grande (au plus {MAX_DEVICES} équipements et {MAX_LINKS} câbles)"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Requêtes
// ---------------------------------------------------------------------------

/// Corps de la création d'un lab : exactement une source.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLab {
    /// Réseau vide, à construire dans l'éditeur (valeur : son nom).
    pub empty: Option<String>,
    /// Topologie livrée avec NETIV (voir `builtin::TOPOLOGY_NAMES`).
    pub builtin: Option<String>,
    /// Topologie ou lab `.netiv` en texte.
    pub yaml: Option<String>,
    /// Topologie générée.
    pub generate: Option<GenerateTopology>,
    /// Challenge livré.
    pub challenge: Option<String>,
    /// Cyber range livré.
    pub range: Option<String>,
    /// Lab de la bibliothèque (par son nom).
    pub lab: Option<String>,
    /// Challenge généré.
    pub generate_challenge: Option<GenerateChallenge>,
}

/// Paramètres du générateur de topologies.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateTopology {
    /// Type (`lan`, `campus`...).
    #[serde(rename = "type")]
    pub kind: String,
    /// Nombre de routeurs.
    #[serde(default = "default_routers")]
    pub routers: usize,
    /// Nombre d'hôtes.
    #[serde(default = "default_hosts")]
    pub hosts: usize,
    /// Graine.
    #[serde(default = "default_seed")]
    pub seed: u64,
}

/// Paramètres du générateur de challenges.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateChallenge {
    /// Thème.
    pub topic: String,
    /// Difficulté.
    #[serde(default = "default_difficulty")]
    pub difficulty: String,
    /// Graine.
    #[serde(default = "default_seed")]
    pub seed: u64,
}

fn default_routers() -> usize {
    4
}
fn default_hosts() -> usize {
    10
}
fn default_seed() -> u64 {
    1
}
fn default_difficulty() -> String {
    "medium".to_string()
}

/// Corps de l'exécution d'une commande.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecBody {
    /// Ligne de commande (plusieurs commandes séparées par « ; »).
    pub line: String,
}

/// Paramètres de lecture de la trace.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TraceQuery {
    /// Premier événement (0 par défaut).
    pub since: Option<usize>,
    /// Nombre maximal d'événements (500 par défaut, 2 000 au plus).
    pub limit: Option<usize>,
}

/// Format demandé pour la topologie.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TopologyQuery {
    /// `json` (défaut), `yaml` ou `netiv`.
    pub format: Option<String>,
}

/// Format demandé pour la documentation.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DocsQuery {
    /// `md` (défaut), `json` ou `yaml`.
    pub format: Option<String>,
}

/// Une topologie envoyée par l'éditeur.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyBody {
    /// La topologie.
    pub topology: TopologyDef,
}

/// Positions des équipements sur le schéma.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionsBody {
    /// Équipement → `[x, y]`.
    pub positions: Layout,
}

/// Scénario « what-if ».
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhatIfBody {
    /// Par exemple « R1 goes down ».
    pub scenario: String,
}

// ---------------------------------------------------------------------------
// Réponses
// ---------------------------------------------------------------------------

/// Informations générales.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    /// « NETIV ».
    pub name: &'static str,
    /// Version de NETIV.
    pub version: &'static str,
    /// Nom du profil d'élève.
    pub profile: String,
    /// La progression est-elle sauvegardée ?
    pub persistent: bool,
    /// Les sessions sont en bac à sable (toujours vrai).
    pub sandbox: bool,
    /// Nombre maximal de labs ouverts.
    pub max_labs: usize,
}

/// Un challenge du catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct ChallengeEntry {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Thème.
    pub topic: String,
    /// Difficulté (`easy`, `medium`, `hard`).
    pub difficulty: &'static str,
    /// Durée conseillée.
    pub minutes: u32,
}

/// Un cyber range du catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct RangeEntry {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Difficulté.
    pub difficulty: &'static str,
    /// Durée conseillée.
    pub minutes: u32,
}

/// Un parcours d'apprentissage.
#[derive(Debug, Clone, Serialize)]
pub struct PathEntry {
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Nombre de leçons.
    pub lessons: usize,
}

/// Un lab de la bibliothèque.
#[derive(Debug, Clone, Serialize)]
pub struct LabFile {
    /// Nom (fichier sans extension).
    pub name: String,
    /// Titre.
    pub title: String,
    /// Auteur.
    pub author: String,
    /// `network` ou `challenge`.
    pub kind: &'static str,
}

/// Une cause de panne (réponses possibles d'un challenge).
#[derive(Debug, Clone, Serialize)]
pub struct CauseEntry {
    /// Identifiant (à donner à `answer`).
    pub slug: &'static str,
    /// Libellé.
    pub label: &'static str,
}

/// Paramètres connus des générateurs.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratorInfo {
    /// Types de topologies.
    pub types: Vec<&'static str>,
    /// Thèmes de challenges.
    pub topics: Vec<&'static str>,
}

/// Tout ce que l'on peut ouvrir.
#[derive(Debug, Clone, Serialize)]
pub struct Library {
    /// Topologies livrées.
    pub topologies: Vec<&'static str>,
    /// Challenges livrés.
    pub challenges: Vec<ChallengeEntry>,
    /// Cyber ranges livrés.
    pub ranges: Vec<RangeEntry>,
    /// Parcours d'apprentissage.
    pub paths: Vec<PathEntry>,
    /// Labs de la bibliothèque.
    pub labs: Vec<LabFile>,
    /// Causes de panne.
    pub causes: Vec<CauseEntry>,
    /// Comportements à identifier dans un cyber range.
    pub behaviors: Vec<&'static str>,
    /// Générateurs.
    pub generator: GeneratorInfo,
}

/// Un challenge réussi.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedEntry {
    /// Identifiant.
    pub id: String,
    /// Meilleur score.
    pub score: u32,
}

/// Un badge et son état.
#[derive(Debug, Clone, Serialize)]
pub struct BadgeEntry {
    /// Identifiant.
    pub id: &'static str,
    /// Nom.
    pub name: &'static str,
    /// Comment l'obtenir.
    pub how: &'static str,
    /// Obtenu ?
    pub earned: bool,
}

/// Le profil de l'élève, tel que l'affiche l'interface.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    /// Nom.
    pub name: String,
    /// XP total.
    pub xp: u32,
    /// Niveau.
    pub level: u32,
    /// Nom du niveau.
    pub level_name: &'static str,
    /// Seuil du niveau suivant.
    pub next_level_xp: Option<u32>,
    /// Nombre de challenges réussis.
    pub completed: usize,
    /// Challenges réussis.
    pub completed_ids: Vec<CompletedEntry>,
    /// Badges.
    pub badges: Vec<BadgeEntry>,
    /// Tableau de bord (texte).
    pub dashboard: String,
}

/// Une ligne du classement.
#[derive(Debug, Clone, Serialize)]
pub struct LeaderRow {
    /// Rang (à partir de 1).
    pub rank: usize,
    /// Nom.
    pub name: String,
    /// XP.
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

/// Classement des profils.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Leaderboard {
    /// Lignes, dans l'ordre.
    pub entries: Vec<LeaderRow>,
    /// Fichiers ignorés (profils illisibles).
    pub skipped: usize,
}

/// Un lab ouvert.
#[derive(Debug, Clone, Serialize)]
pub struct LabSummary {
    /// Identifiant.
    pub id: String,
    /// Nom du réseau.
    pub name: Option<String>,
    /// Challenge en cours.
    pub challenge: Option<String>,
}

/// Les labs ouverts.
#[derive(Debug, Clone, Serialize)]
pub struct LabList {
    /// Labs, par identifiant.
    pub labs: Vec<LabSummary>,
}

/// Lab créé.
#[derive(Debug, Clone, Serialize)]
pub struct Created {
    /// Identifiant.
    pub id: String,
    /// Sortie des commandes de création.
    pub output: String,
    /// État du lab.
    pub snapshot: Snapshot,
}

/// Lab fermé.
#[derive(Debug, Clone, Serialize)]
pub struct Deleted {
    /// Identifiant du lab fermé.
    pub deleted: String,
}

/// Résultat d'une commande. Une commande qui échoue n'est pas une erreur du
/// service : son message est dans `error`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecResult {
    /// Sortie.
    pub output: String,
    /// Message d'erreur de la commande.
    pub error: Option<String>,
    /// La commande demandait de quitter.
    pub quit: bool,
    /// Taille de la trace avant la commande (premiers événements nouveaux).
    pub new_events_from: usize,
    /// État du lab.
    pub snapshot: Snapshot,
}

/// Une catégorie du score de santé.
#[derive(Debug, Clone, Serialize)]
pub struct HealthCategory {
    /// Nom.
    pub name: &'static str,
    /// Score.
    pub score: u32,
    /// Poids.
    pub weight: u32,
    /// Remarques.
    pub notes: Vec<String>,
}

/// Score de santé du réseau.
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    /// Score global.
    pub global: u32,
    /// Détail.
    pub categories: Vec<HealthCategory>,
    /// Rapport texte.
    pub text: String,
}

/// Nombre de constats par gravité.
#[derive(Debug, Clone, Serialize)]
pub struct SeverityCounts {
    /// Graves.
    pub high: usize,
    /// Moyens.
    pub medium: usize,
    /// Faibles.
    pub low: usize,
}

/// Un constat de l'analyse de sécurité.
#[derive(Debug, Clone, Serialize)]
pub struct SecurityFinding {
    /// Identifiant (`SEC-001`...).
    pub id: &'static str,
    /// Gravité.
    pub severity: &'static str,
    /// Titre.
    pub title: String,
    /// Équipements concernés.
    pub devices: Vec<String>,
    /// Détail.
    pub detail: String,
    /// Recommandation.
    pub recommendation: String,
}

/// Analyse de sécurité.
#[derive(Debug, Clone, Serialize)]
pub struct Security {
    /// Score.
    pub score: u32,
    /// Constats par gravité.
    pub counts: SeverityCounts,
    /// Constats.
    pub findings: Vec<SecurityFinding>,
    /// Rapport texte.
    pub text: String,
}

/// Un texte dans un format donné (documentation, topologie exportée).
#[derive(Debug, Clone, Serialize)]
pub struct FormattedText {
    /// Format.
    pub format: String,
    /// Contenu.
    pub text: String,
}

/// La topologie d'un lab.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum TopologyView {
    /// Pour l'éditeur.
    Json {
        /// La topologie.
        topology: TopologyDef,
    },
    /// À télécharger (YAML ou `.netiv`).
    Text(FormattedText),
}

/// Topologie appliquée.
#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    /// Avertissements de validation.
    pub warnings: Vec<String>,
    /// État du lab.
    pub snapshot: Snapshot,
}

/// Validation d'une topologie.
#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    /// Erreurs bloquantes.
    pub errors: Vec<String>,
    /// Avertissements.
    pub warnings: Vec<String>,
}

/// Adressage complété.
#[derive(Debug, Clone, Serialize)]
pub struct Autoconfigured {
    /// Changements effectués.
    pub changes: Vec<String>,
    /// État du lab.
    pub snapshot: Snapshot,
}

/// Positions enregistrées.
#[derive(Debug, Clone, Serialize)]
pub struct Saved {
    /// Nombre de positions.
    pub saved: usize,
}

/// Impact d'un scénario « what-if ».
#[derive(Debug, Clone, Serialize)]
pub struct WhatIf {
    /// Scénario.
    pub scenario: String,
    /// Flux testés.
    pub flows: usize,
    /// Flux qui passaient.
    pub ok_before: usize,
    /// Flux qui passent encore.
    pub ok_after: usize,
    /// Flux cassés.
    pub broken: Vec<String>,
    /// Hôtes touchés.
    pub affected_hosts: Vec<String>,
    /// Services touchés.
    pub affected_services: Vec<String>,
    /// Hôtes isolés.
    pub isolated_hosts: Vec<String>,
    /// Routes cassées.
    pub broken_routes: usize,
    /// Segments isolés.
    pub isolated_segments: usize,
    /// Sans conséquence ?
    pub harmless: bool,
    /// Rapport texte.
    pub text: String,
}

// ---------------------------------------------------------------------------
// Le service
// ---------------------------------------------------------------------------

/// Les comportements que l'on peut désigner dans un cyber range.
const BEHAVIORS: [&str; 7] = [
    "port-scan",
    "host-sweep",
    "lateral-movement",
    "beaconing",
    "arp-spoofing",
    "repeated-denials",
    "rogue-dhcp",
];

/// Extensions reconnues dans la bibliothèque de labs.
const LAB_EXTENSIONS: [&str; 3] = ["netiv", "yaml", "yml"];

/// Un ensemble de labs et leur configuration.
pub struct Service {
    cfg: ServiceConfig,
    storage: Box<dyn Storage>,
    clock: Option<std::sync::Arc<dyn Fn() -> u64 + Send + Sync>>,
    labs: BTreeMap<String, Session>,
    next: u64,
}

impl Service {
    /// Service dont les fichiers (profils, bibliothèque) passent par
    /// `storage`.
    pub fn new(cfg: ServiceConfig, storage: Box<dyn Storage>) -> Self {
        Service {
            cfg,
            storage,
            clock: None,
            labs: BTreeMap::new(),
            next: 1,
        }
    }

    /// Service sur le stockage par défaut (le disque en natif).
    pub fn with_default_storage(cfg: ServiceConfig) -> Self {
        Self::new(cfg, default_storage())
    }

    /// Horloge (secondes) donnée à chaque nouvelle session : indispensable
    /// en WebAssembly, où il n'y a pas d'horloge système.
    pub fn set_clock(&mut self, clock: std::sync::Arc<dyn Fn() -> u64 + Send + Sync>) {
        self.clock = Some(clock);
    }

    /// Réglages.
    pub fn config(&self) -> &ServiceConfig {
        &self.cfg
    }

    /// Le stockage du service.
    pub fn storage(&self) -> &dyn Storage {
        self.storage.as_ref()
    }

    /// Emplacement du fichier de profil, s'il est sauvegardé.
    pub fn profile_path(&self) -> Option<PathBuf> {
        let dir = self.cfg.profile_dir.as_ref()?;
        Some(dir.join(format!("{}.json", safe_name(&self.cfg.profile_name))))
    }

    /// Une nouvelle session en bac à sable, rattachée au profil.
    fn new_session(&self) -> ServiceResult<Session> {
        let session = match self.profile_path() {
            Some(path) => {
                Session::with_profile_in(self.storage.share(), &self.cfg.profile_name, path)
                    .map_err(|e| ServiceError::bad(e.to_string()))?
            }
            None => Session::with_storage(self.storage.share()),
        };
        let mut session = session.sandboxed();
        session.set_labs_dir(self.cfg.labs_dir.clone());
        if let Some(clock) = &self.clock {
            let clock = clock.clone();
            session.set_clock(Box::new(move || clock()));
        }
        Ok(session)
    }

    fn lab(&mut self, id: &str) -> ServiceResult<&mut Session> {
        self.labs.get_mut(id).ok_or_else(|| unknown_lab(id))
    }

    fn sim<'a>(&'a mut self, id: &str) -> ServiceResult<(&'a Simulation, &'a Session)> {
        let s = self.lab(id)?;
        let sim = s.simulation().ok_or_else(no_network)?;
        Ok((sim, s))
    }

    /// Accès direct à la session d'un lab (lecture seule).
    pub fn session(&self, id: &str) -> Option<&Session> {
        self.labs.get(id)
    }

    // ----------------------------------------------------------- général

    /// Informations générales.
    pub fn info(&self) -> Info {
        Info {
            name: "NETIV",
            version: env!("CARGO_PKG_VERSION"),
            profile: self.cfg.profile_name.clone(),
            persistent: self.profile_path().is_some(),
            sandbox: true,
            max_labs: MAX_LABS,
        }
    }

    /// Labs de la bibliothèque, triés par nom.
    fn lab_files(&self) -> Vec<LabFile> {
        let Ok(entries) = self.storage.list(&self.cfg.labs_dir) else {
            return Vec::new();
        };
        let mut rows: Vec<LabFile> = entries
            .into_iter()
            .filter_map(|path| {
                let ext = path.extension()?.to_str()?;
                if !LAB_EXTENSIONS.contains(&ext) {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let text = self.storage.read(&path).ok()?;
                Some(match Bundle::from_yaml(&text) {
                    Ok(b) => LabFile {
                        name,
                        title: b.metadata.title,
                        author: b.metadata.author,
                        kind: if b.challenge.is_some() {
                            "challenge"
                        } else {
                            "network"
                        },
                    },
                    Err(_) => LabFile {
                        title: name.clone(),
                        name,
                        author: String::new(),
                        kind: "network",
                    },
                })
            })
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    /// Tout ce que l'on peut ouvrir.
    pub fn library(&self) -> Library {
        Library {
            topologies: builtin::TOPOLOGY_NAMES.to_vec(),
            challenges: builtin::challenges()
                .iter()
                .map(|c| ChallengeEntry {
                    id: c.id.clone(),
                    title: c.title.clone(),
                    topic: c.topic.clone(),
                    difficulty: c.difficulty.slug(),
                    minutes: c.time_limit_minutes,
                })
                .collect(),
            ranges: builtin::ranges()
                .iter()
                .map(|r| RangeEntry {
                    id: r.id.clone(),
                    title: r.title.clone(),
                    difficulty: r.difficulty.slug(),
                    minutes: r.time_limit_minutes,
                })
                .collect(),
            paths: learn::paths()
                .iter()
                .map(|p| PathEntry {
                    id: p.id.clone(),
                    title: p.title.clone(),
                    lessons: p.lessons.len(),
                })
                .collect(),
            labs: self.lab_files(),
            causes: netiv_diagnostics::CauseKind::ALL
                .iter()
                .map(|k| CauseEntry {
                    slug: k.slug(),
                    label: k.label(),
                })
                .collect(),
            behaviors: BEHAVIORS.to_vec(),
            generator: GeneratorInfo {
                types: netiv_topology::generate::TopoKind::ALL
                    .map(netiv_topology::generate::TopoKind::slug)
                    .to_vec(),
                topics: netiv_education::generate::TOPICS.to_vec(),
            },
        }
    }

    // ----------------------------------------------------------- profil

    /// Le profil actuel (relu depuis le stockage).
    pub fn current_profile(&self) -> ServiceResult<Profile> {
        Ok(self.new_session()?.profile().clone())
    }

    /// Le profil, tel que l'affiche l'interface.
    pub fn profile(&self) -> ServiceResult<ProfileView> {
        let p = self.current_profile()?;
        let xp = p.xp();
        let (level, level_name, next) = level_for(xp);
        Ok(ProfileView {
            xp,
            level,
            level_name,
            next_level_xp: next,
            completed: p.completed_count(),
            completed_ids: p
                .completed
                .iter()
                .map(|(id, c)| CompletedEntry {
                    id: id.clone(),
                    score: c.score,
                })
                .collect(),
            badges: BADGES
                .iter()
                .map(|b| BadgeEntry {
                    id: b.id,
                    name: b.name,
                    how: b.how,
                    earned: p.badges.contains(b.id),
                })
                .collect(),
            dashboard: p.dashboard(&builtin::catalog()),
            name: p.name,
        })
    }

    /// Le profil en JSON (format des fichiers de profil), pour le sauvegarder
    /// ailleurs (base de données d'une plateforme).
    pub fn export_profile(&self) -> ServiceResult<String> {
        self.current_profile()?
            .to_json()
            .map_err(|e| ServiceError::bad(e.to_string()))
    }

    /// Remplace le profil par `json` (format de [`Service::export_profile`]).
    /// Les labs ouverts le relisent avant leur prochaine commande.
    pub fn import_profile(&mut self, json: &str) -> ServiceResult<()> {
        let Some(path) = self.profile_path() else {
            return Err(ServiceError::bad(
                "profil en mémoire seulement : aucun emplacement où l'importer",
            ));
        };
        let profile = Profile::from_json(json, "profil importé")
            .map_err(|e| ServiceError::bad(e.to_string()))?;
        let text = profile
            .to_json()
            .map_err(|e| ServiceError::bad(e.to_string()))?;
        self.storage
            .write_atomic(&path, &text)
            .map_err(|e| ServiceError::bad(format!("profil non enregistré : {e}")))?;
        for s in self.labs.values_mut() {
            s.reload_profile();
        }
        Ok(())
    }

    /// Classement des profils du dossier de profils.
    pub fn leaderboard(&self) -> Leaderboard {
        let Some(dir) = &self.cfg.profile_dir else {
            return Leaderboard::default();
        };
        let Ok(entries) = self.storage.list(dir) else {
            return Leaderboard::default();
        };
        let profiles = entries.into_iter().filter_map(|path| {
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                return None;
            }
            let stem = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("?")
                .to_string();
            Some(load_profile(self.storage.as_ref(), &path, &stem))
        });
        let (board, skipped) = rank(profiles);
        Leaderboard {
            entries: board
                .into_iter()
                .enumerate()
                .map(|(i, e)| LeaderRow {
                    rank: i + 1,
                    name: e.name,
                    xp: e.xp,
                    level: e.level,
                    level_name: e.level_name,
                    completed: e.completed,
                    badges: e.badges,
                })
                .collect(),
            skipped,
        }
    }

    // ----------------------------------------------------------- labs

    /// Les labs ouverts.
    pub fn list_labs(&self) -> LabList {
        LabList {
            labs: self
                .labs
                .iter()
                .map(|(id, s)| LabSummary {
                    id: id.clone(),
                    name: s.network_name().map(str::to_string),
                    challenge: s.challenge_info().map(|c| c.id),
                })
                .collect(),
        }
    }

    /// Ouvre un lab. En cas d'échec, aucun lab n'est créé.
    pub fn create_lab(&mut self, body: &CreateLab) -> ServiceResult<Created> {
        let sources = [
            body.empty.is_some(),
            body.builtin.is_some(),
            body.yaml.is_some(),
            body.generate.is_some(),
            body.challenge.is_some(),
            body.range.is_some(),
            body.lab.is_some(),
            body.generate_challenge.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if sources != 1 {
            return Err(ServiceError::bad(
                "indique exactement une source : empty, builtin, yaml, generate, challenge, range, lab ou generate_challenge",
            ));
        }
        if self.labs.len() >= MAX_LABS {
            return Err(ServiceError::bad(format!(
                "trop de labs ouverts (maximum {MAX_LABS}) : ferme-en un"
            )));
        }
        let mut session = self.new_session()?;
        let mut out: Vec<u8> = Vec::new();
        let run = |session: &mut Session, line: &str, out: &mut Vec<u8>| -> ServiceResult<()> {
            session
                .exec(line, out)
                .map(|_| ())
                .map_err(|e| ServiceError::bad(e.to_string()))
        };

        if let Some(name) = &body.empty {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > MAX_NAME {
                return Err(ServiceError::bad(format!(
                    "nom de réseau invalide (1 à {MAX_NAME} caractères)"
                )));
            }
            let def = TopologyDef {
                schema_version: TOPOLOGY_SCHEMA_VERSION,
                network: NetworkDef {
                    name: name.to_string(),
                    seed: 1,
                },
                devices: Vec::new(),
                links: Vec::new(),
            };
            session.apply_topology(&def).map_err(edit_error)?;
            out.extend_from_slice(
                format!("Réseau vide « {name} » créé : ajoute des équipements depuis l'éditeur.\n")
                    .as_bytes(),
            );
        } else if let Some(name) = &body.builtin {
            let text = builtin::topology(name)
                .ok_or_else(|| ServiceError::bad(format!("réseau inconnu : « {name} »")))?;
            session
                .load_yaml(text, &mut out)
                .map_err(|e| ServiceError::bad(e.to_string()))?;
        } else if let Some(yaml) = &body.yaml {
            session
                .load_yaml(yaml, &mut out)
                .map_err(|e| ServiceError::bad(e.to_string()))?;
        } else if let Some(g) = &body.generate {
            let line = format!(
                "topology generate --type {} --routers {} --hosts {} --seed {}",
                token(&g.kind, "type")?,
                g.routers,
                g.hosts,
                g.seed
            );
            run(&mut session, &line, &mut out)?;
        } else if let Some(id) = &body.challenge {
            run(
                &mut session,
                &format!("challenge start {}", token(id, "challenge")?),
                &mut out,
            )?;
        } else if let Some(id) = &body.range {
            run(
                &mut session,
                &format!("range start {}", token(id, "scénario")?),
                &mut out,
            )?;
        } else if let Some(name) = &body.lab {
            let name = token(name, "lab")?;
            let dir = &self.cfg.labs_dir;
            let path = LAB_EXTENSIONS
                .iter()
                .map(|e| dir.join(format!("{name}.{e}")))
                .find(|p| self.storage.is_file(p))
                .ok_or_else(|| ServiceError::not_found(format!("lab inconnu : « {name} »")))?;
            let text = self
                .storage
                .read(&path)
                .map_err(|e| ServiceError::bad(e.to_string()))?;
            load_lab_text(&mut session, &text, &mut out)?;
        } else if let Some(g) = &body.generate_challenge {
            let line = format!(
                "challenge generate --topic {} --difficulty {} --seed {}",
                token(&g.topic, "thème")?,
                token(&g.difficulty, "difficulté")?,
                g.seed
            );
            run(&mut session, &line, &mut out)?;
        }

        let id = format!("lab-{}", self.next);
        self.next += 1;
        let snapshot = snapshot::snapshot(&id, &session);
        self.labs.insert(id.clone(), session);
        Ok(Created {
            id,
            output: String::from_utf8_lossy(&out).into_owned(),
            snapshot,
        })
    }

    /// Ouvre un lab à partir du texte d'un fichier `.netiv` (ou d'une
    /// topologie YAML) ; un lab qui porte un challenge le démarre.
    pub fn import_lab(&mut self, text: &str) -> ServiceResult<Created> {
        if text.len() > MAX_BODY {
            return Err(ServiceError::bad("lab trop volumineux"));
        }
        if self.labs.len() >= MAX_LABS {
            return Err(ServiceError::bad(format!(
                "trop de labs ouverts (maximum {MAX_LABS}) : ferme-en un"
            )));
        }
        let mut session = self.new_session()?;
        let mut out: Vec<u8> = Vec::new();
        load_lab_text(&mut session, text, &mut out)?;
        let id = format!("lab-{}", self.next);
        self.next += 1;
        let snapshot = snapshot::snapshot(&id, &session);
        self.labs.insert(id.clone(), session);
        Ok(Created {
            id,
            output: String::from_utf8_lossy(&out).into_owned(),
            snapshot,
        })
    }

    /// Le réseau du lab comme fichier `.netiv` partageable.
    pub fn export_lab(&mut self, id: &str) -> ServiceResult<String> {
        match self.get_topology(
            id,
            &TopologyQuery {
                format: Some("netiv".to_string()),
            },
        )? {
            TopologyView::Text(t) => Ok(t.text),
            TopologyView::Json { .. } => Err(ServiceError::bad("format inattendu")),
        }
    }

    /// État d'un lab.
    pub fn get_lab(&mut self, id: &str) -> ServiceResult<Snapshot> {
        let s = self.lab(id)?;
        Ok(snapshot::snapshot(id, s))
    }

    /// Ferme un lab.
    pub fn delete_lab(&mut self, id: &str) -> ServiceResult<Deleted> {
        self.labs
            .remove(id)
            .map(|_| Deleted {
                deleted: id.to_string(),
            })
            .ok_or_else(|| unknown_lab(id))
    }

    /// Exécute une ligne de commande dans un lab.
    pub fn exec(&mut self, id: &str, body: &ExecBody) -> ServiceResult<ExecResult> {
        if body.line.len() > MAX_LINE {
            return Err(ServiceError::bad(format!(
                "ligne trop longue (maximum {MAX_LINE} caractères)"
            )));
        }
        let session = self.lab(id)?;
        // Plusieurs labs peuvent partager le même profil : on relit avant d'agir.
        session.reload_profile();
        let before = session.simulation().map_or(0, |s| s.trace().len());
        let mut out: Vec<u8> = Vec::new();
        let result = session.exec(&body.line, &mut out);
        Ok(ExecResult {
            output: String::from_utf8_lossy(&out).into_owned(),
            error: result.as_ref().err().map(ToString::to_string),
            quit: matches!(result, Ok(Flow::Quit)),
            new_events_from: before,
            snapshot: snapshot::snapshot(id, session),
        })
    }

    // ----------------------------------------------------------- observer

    /// Événements de la trace.
    pub fn trace(&mut self, id: &str, q: &TraceQuery) -> ServiceResult<Value> {
        let (sim, _) = self.sim(id)?;
        Ok(snapshot::trace_json(
            sim,
            q.since.unwrap_or(0),
            q.limit.unwrap_or(500).min(2_000),
        ))
    }

    /// Dernières transactions.
    pub fn transactions(&mut self, id: &str) -> ServiceResult<Value> {
        let (sim, _) = self.sim(id)?;
        Ok(snapshot::transactions(sim, 50))
    }

    /// Parcours d'une transaction (Packet Journey).
    pub fn journey(&mut self, id: &str, tid: u64) -> ServiceResult<Value> {
        let (sim, _) = self.sim(id)?;
        snapshot::journey_json(sim, TraceId(tid))
            .ok_or_else(|| ServiceError::not_found(format!("aucune transaction #{tid}")))
    }

    /// Tables d'un équipement.
    pub fn device(&mut self, id: &str, name: &str) -> ServiceResult<Value> {
        let (sim, _) = self.sim(id)?;
        snapshot::device_json(sim, name)
            .ok_or_else(|| ServiceError::not_found(format!("équipement inconnu : « {name} »")))
    }

    // ----------------------------------------------------------- analyser

    /// Score de santé (verrouillé pendant un challenge).
    pub fn health(&mut self, id: &str) -> ServiceResult<Health> {
        let s = self.lab(id)?;
        not_during_challenge(s)?;
        let sim = s.simulation().ok_or_else(no_network)?;
        let h = health(sim);
        Ok(Health {
            global: h.global(),
            categories: h
                .categories
                .iter()
                .map(|c| HealthCategory {
                    name: c.name,
                    score: c.score,
                    weight: c.weight,
                    notes: c.notes.clone(),
                })
                .collect(),
            text: h.render(),
        })
    }

    /// Analyse de sécurité.
    pub fn security(&mut self, id: &str) -> ServiceResult<Security> {
        let (sim, _) = self.sim(id)?;
        let r = analyze(sim);
        let (high, medium, low) = r.counts();
        Ok(Security {
            score: r.score(),
            counts: SeverityCounts { high, medium, low },
            findings: r
                .findings
                .iter()
                .map(|f| SecurityFinding {
                    id: f.id,
                    severity: f.severity.label(),
                    title: f.title.clone(),
                    devices: f.devices.clone(),
                    detail: f.detail.clone(),
                    recommendation: f.recommendation.clone(),
                })
                .collect(),
            text: r.render(),
        })
    }

    /// Documentation générée du réseau.
    pub fn docs(&mut self, id: &str, q: &DocsQuery) -> ServiceResult<FormattedText> {
        let format: DocFormat = q
            .format
            .as_deref()
            .unwrap_or("md")
            .parse()
            .map_err(ServiceError::bad)?;
        let (sim, s) = self.sim(id)?;
        let name = s.network_name().unwrap_or("reseau").to_string();
        let text = generate_docs(sim, &name, format).map_err(ServiceError::bad)?;
        Ok(FormattedText {
            format: q.format.clone().unwrap_or_else(|| "md".into()),
            text,
        })
    }

    /// Impact d'un changement, sans modifier le lab.
    pub fn what_if(&mut self, id: &str, body: &WhatIfBody) -> ServiceResult<WhatIf> {
        if body.scenario.len() > MAX_LINE {
            return Err(ServiceError::bad("scénario trop long"));
        }
        let (sim, _) = self.sim(id)?;
        let i = what_if(sim, &body.scenario).map_err(ServiceError::bad)?;
        Ok(WhatIf {
            isolated_segments: i.isolated_segments(),
            harmless: i.is_harmless(),
            text: i.render(),
            scenario: i.scenario,
            flows: i.flows,
            ok_before: i.ok_before,
            ok_after: i.ok_after,
            broken: i.broken,
            affected_hosts: i.affected_hosts,
            affected_services: i.affected_services,
            isolated_hosts: i.isolated_hosts,
            broken_routes: i.broken_routes,
        })
    }

    // ----------------------------------------------------------- éditeur

    /// La topologie du lab : JSON (éditeur), ou texte YAML / `.netiv`.
    pub fn get_topology(&mut self, id: &str, q: &TopologyQuery) -> ServiceResult<TopologyView> {
        let author = self.cfg.profile_name.clone();
        let s = self.lab(id)?;
        let def = s.topology_def().ok_or_else(no_network)?;
        match q.format.as_deref().unwrap_or("json") {
            "json" => Ok(TopologyView::Json { topology: def }),
            "yaml" => {
                let text =
                    serde_yaml_ng::to_string(&def).map_err(|e| ServiceError::bad(e.to_string()))?;
                Ok(TopologyView::Text(FormattedText {
                    format: "yaml".to_string(),
                    text,
                }))
            }
            "netiv" => {
                let meta = BundleMeta {
                    title: def.network.name.clone(),
                    description: String::new(),
                    author,
                };
                let text = Bundle::new(meta, def)
                    .to_yaml()
                    .map_err(|e| ServiceError::bad(e.to_string()))?;
                Ok(TopologyView::Text(FormattedText {
                    format: "netiv".to_string(),
                    text,
                }))
            }
            other => Err(ServiceError::bad(format!(
                "format inconnu : « {other} » (json, yaml ou netiv)"
            ))),
        }
    }

    /// Remplace le réseau du lab (reconstruction : la trace repart de zéro).
    pub fn put_topology(&mut self, id: &str, body: &TopologyBody) -> ServiceResult<Applied> {
        check_size(&body.topology)?;
        let s = self.lab(id)?;
        let warnings = s.apply_topology(&body.topology).map_err(edit_error)?;
        Ok(Applied {
            warnings,
            snapshot: snapshot::snapshot(id, s),
        })
    }

    /// Vérifie une topologie sans rien modifier (le lab doit exister).
    pub fn validate_topology(
        &mut self,
        id: &str,
        body: &TopologyBody,
    ) -> ServiceResult<Validation> {
        check_size(&body.topology)?;
        self.lab(id)?;
        let (errors, warnings) = Session::validate_topology(&body.topology);
        Ok(Validation { errors, warnings })
    }

    /// Complète l'adressage (IP, passerelles, routes).
    pub fn autoconfig(&mut self, id: &str) -> ServiceResult<Autoconfigured> {
        let s = self.lab(id)?;
        let changes = s.autoconfigure().map_err(edit_error)?;
        Ok(Autoconfigured {
            changes,
            snapshot: snapshot::snapshot(id, s),
        })
    }

    /// Mémorise la position des équipements sur le schéma.
    pub fn put_positions(&mut self, id: &str, body: &PositionsBody) -> ServiceResult<Saved> {
        if body.positions.len() > MAX_DEVICES {
            return Err(ServiceError::bad("trop de positions"));
        }
        let s = self.lab(id)?;
        s.set_positions(&body.positions);
        Ok(Saved {
            saved: body.positions.len(),
        })
    }
}

/// Charge le texte d'un lab dans `session` ; un lab qui porte un challenge
/// le démarre (un lab de réseau pur garde simplement le réseau).
fn load_lab_text(session: &mut Session, text: &str, out: &mut Vec<u8>) -> ServiceResult<()> {
    session
        .load_yaml(text, out)
        .map_err(|e| ServiceError::bad(e.to_string()))?;
    if text.contains("challenge:") && text.contains("format_version") {
        // Un échec ici veut dire « pas de challenge » : on garde le réseau.
        let _ = session.start_challenge_text(text, out);
    }
    Ok(())
}

impl fmt::Debug for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Service")
            .field("cfg", &self.cfg)
            .field("labs", &self.labs.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}
