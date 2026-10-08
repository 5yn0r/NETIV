//! Network Health Score : six notes de 0 à 100 et une note globale.
//!
//! Les formules sont volontairement simples, documentées, et chaque point perdu
//! s'accompagne d'une explication.
//!
//! | Catégorie | Mesure | Formule |
//! |---|---|---|
//! | Connectivity | paires de machines joignables **sans filtrage** (ce que le routage permet) | `100 × joignables / paires` |
//! | Addressing | avertissements d'adressage (IP en double, passerelle ou route hors réseau, câble entre réseaux différents) | `100 − 20 × avertissements` |
//! | Routing | routes utilisables, symétrie des échanges | `100 × utilisables / configurées − 10 × paires asymétriques` |
//! | Segmentation | défauts SEC-001/002/003/005/006 de l'analyseur | `100 − (40, 35, 20, 25, 25)` selon les défauts présents |
//! | Availability | points uniques de défaillance, composants en panne | `100 − 20 × routeur/pare-feu unique − 8 × switch unique − 10 × composant en panne` |
//! | Security | score de l'analyseur de sécurité | `100 − 25 × HIGH − 10 × MEDIUM − 3 × LOW` |
//!
//! Note globale : moyenne pondérée (Connectivity 25 %, Segmentation 20 %,
//! Security 20 %, Routing 15 %, Addressing 10 %, Availability 10 %).

use std::fmt::Write as _;

use netiv_core::device::Behavior;
use netiv_core::{DeviceKind, Simulation};
use netiv_security::analyze;

use crate::reach::{self, FlowId};

/// Une catégorie de la note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    /// Nom (Connectivity, Addressing...).
    pub name: &'static str,
    /// Note de 0 à 100.
    pub score: u32,
    /// Poids dans la note globale (en pourcentage).
    pub weight: u32,
    /// Pourquoi des points ont été perdus.
    pub notes: Vec<String>,
}

/// Rapport de santé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Les six catégories.
    pub categories: Vec<Category>,
}

impl HealthReport {
    /// Note globale (moyenne pondérée, arrondie).
    pub fn global(&self) -> u32 {
        let total: u32 = self.categories.iter().map(|c| c.weight).sum();
        if total == 0 {
            return 0;
        }
        let sum: u32 = self.categories.iter().map(|c| c.score * c.weight).sum();
        (sum + total / 2) / total
    }

    /// Note d'une catégorie.
    pub fn score_of(&self, name: &str) -> Option<u32> {
        self.categories
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.score)
    }

    /// Rendu texte (§16).
    pub fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "NETWORK HEALTH\n");
        for c in &self.categories {
            let _ = writeln!(s, "{:<14}{:>3}/100", c.name, c.score);
        }
        let _ = writeln!(s, "\n{:<14}{:>3}/100", "GLOBAL SCORE", self.global());
        let notes: Vec<&Category> = self
            .categories
            .iter()
            .filter(|c| !c.notes.is_empty())
            .collect();
        if !notes.is_empty() {
            let _ = writeln!(s, "\nDétail");
            for c in notes {
                for n in &c.notes {
                    let _ = writeln!(s, "  {:<14} {n}", c.name);
                }
            }
        }
        s
    }
}

fn clamp(v: i64) -> u32 {
    u32::try_from(v.clamp(0, 100)).unwrap_or(0)
}

/// Évalue la santé d'un réseau.
pub fn health(sim: &Simulation) -> HealthReport {
    let report = analyze(sim);

    // --- Connectivity & Routing (pings sans filtrage) -----------------------
    let routable = reach::routable(sim);
    let total = routable.len();
    let ok = routable.ok_count();
    let mut conn_notes = Vec::new();
    let connectivity = if total == 0 {
        100
    } else {
        let lost = total - ok;
        if lost > 0 {
            let mut broken: Vec<String> = routable
                .results
                .iter()
                .filter(|(_, v)| !**v)
                .map(|(f, _)| reach::label(sim, f))
                .collect();
            let more = broken.len().saturating_sub(3);
            broken.truncate(3);
            conn_notes.push(format!(
                "{lost} paire(s) sur {total} ne peuvent pas communiquer, par ex. {}{}",
                broken.join(", "),
                if more > 0 {
                    format!(" (+{more})")
                } else {
                    String::new()
                }
            ));
        }
        clamp((100 * ok as i64) / total as i64)
    };

    // --- Addressing ----------------------------------------------------------
    let warnings: Vec<String> = netiv_topology::lint(sim)
        .into_iter()
        .filter(|w| {
            w.contains("utilisée plusieurs fois")
                || w.contains("n'est dans aucun réseau connecté")
                || w.contains("inutilisable")
                || w.contains("réseaux différents")
        })
        .collect();
    let addressing = clamp(100 - 20 * warnings.len() as i64);

    // --- Routing -------------------------------------------------------------
    let mut configured = 0usize;
    let mut usable = 0usize;
    for d in sim.devices() {
        if let Behavior::Ip(n) = &d.behavior {
            let (active, inactive) = n.active_routes(&d.ifaces);
            let active_via = active
                .iter()
                .filter(|r| r.origin != netiv_core::route::RouteOrigin::Connected)
                .count();
            configured += active_via + inactive.len();
            usable += active_via;
        }
    }
    let mut asym = 0usize;
    for (f, v) in &routable.results {
        if *v {
            let back = FlowId {
                src: f.dst,
                dst: f.src,
                service: None,
            };
            if routable.results.get(&back) == Some(&false) {
                asym += 1;
            }
        }
    }
    let mut routing_notes = Vec::new();
    let route_pct = if configured == 0 {
        100
    } else {
        (100 * usable as i64) / configured as i64
    };
    if usable < configured {
        routing_notes.push(format!(
            "{} route(s) sur {configured} inutilisable(s)",
            configured - usable
        ));
    }
    if asym > 0 {
        routing_notes.push(format!(
            "{asym} échange(s) fonctionnent dans un sens seulement"
        ));
    }
    let routing = clamp(route_pct - 10 * asym as i64);

    // --- Segmentation --------------------------------------------------------
    let mut seg_loss = 0i64;
    let mut seg_notes = Vec::new();
    for (id, loss) in [
        ("SEC-001", 40),
        ("SEC-002", 35),
        ("SEC-003", 20),
        ("SEC-005", 25),
        ("SEC-006", 25),
    ] {
        if let Some(f) = report.get(id) {
            seg_loss += loss;
            seg_notes.push(format!("{} (−{loss})", f.title));
        }
    }
    let segmentation = clamp(100 - seg_loss);

    // --- Availability --------------------------------------------------------
    let mut avail_loss = 0i64;
    let mut avail_notes = Vec::new();
    if let Some(f) = report.get("SEC-011") {
        for name in &f.devices {
            let kind = sim
                .device_id(name)
                .ok()
                .and_then(|id| sim.device(id))
                .map(|d| d.kind);
            let loss = match kind {
                Some(DeviceKind::Router | DeviceKind::Firewall) => 20,
                _ => 8,
            };
            avail_loss += loss;
            avail_notes.push(format!(
                "{name} est un point unique de défaillance (−{loss})"
            ));
        }
    }
    for l in sim.links().iter().filter(|l| !l.up) {
        avail_loss += 10;
        avail_notes.push(format!(
            "câble {} ↔ {} coupé (−10)",
            netiv_core::render::device_name(sim, l.a.0),
            netiv_core::render::device_name(sim, l.b.0)
        ));
    }
    for d in sim.devices() {
        for i in d.ifaces.iter().filter(|i| !i.admin_up && i.link.is_some()) {
            avail_loss += 10;
            avail_notes.push(format!("interface {}/{} désactivée (−10)", d.name, i.name));
        }
    }
    let availability = clamp(100 - avail_loss);

    // --- Security ------------------------------------------------------------
    let security = report.score();
    let sec_notes: Vec<String> = report
        .findings
        .iter()
        .map(|f| {
            format!(
                "[{}] {} (−{})",
                f.severity.label(),
                f.title,
                f.severity.penalty()
            )
        })
        .collect();

    HealthReport {
        categories: vec![
            Category {
                name: "Connectivity",
                score: connectivity,
                weight: 25,
                notes: conn_notes,
            },
            Category {
                name: "Addressing",
                score: addressing,
                weight: 10,
                notes: warnings,
            },
            Category {
                name: "Routing",
                score: routing,
                weight: 15,
                notes: routing_notes,
            },
            Category {
                name: "Segmentation",
                score: segmentation,
                weight: 20,
                notes: seg_notes,
            },
            Category {
                name: "Availability",
                score: availability,
                weight: 10,
                notes: avail_notes,
            },
            Category {
                name: "Security",
                score: security,
                weight: 20,
                notes: sec_notes,
            },
        ],
    }
}
