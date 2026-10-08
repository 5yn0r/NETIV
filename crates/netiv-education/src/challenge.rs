//! Définition d'un challenge : un réseau, des pannes cachées, un objectif.

use std::net::Ipv4Addr;

use netiv_chaos::Difficulty;
use netiv_core::packet::L4Proto;
use netiv_core::{NetivError, Simulation};
use netiv_diagnostics::{
    CauseKind, Probe, ProbeKind, Request, RootCause, diagnose, hints_for, run_probe,
};
use netiv_topology::TopologyDef;
use serde::{Deserialize, Serialize};

use crate::faultspec::FaultSpec;

/// Le service que l'objectif doit atteindre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRef {
    /// `tcp` ou `udp`.
    pub proto: String,
    /// Port.
    pub port: u16,
}

/// Ce qui doit fonctionner pour réussir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// Équipement émetteur.
    pub from: String,
    /// Adresse visée.
    pub to: String,
    /// Service visé ; absent = ping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceRef>,
}

impl Objective {
    /// La sonde correspondante, avec les noms résolus sur `sim`.
    pub fn to_probe(&self, sim: &Simulation) -> Result<Probe, String> {
        let src = sim
            .device_id(&self.from)
            .map_err(|e| format!("objectif : {e}"))?;
        let dst: Ipv4Addr = self
            .to
            .parse()
            .map_err(|_| format!("objectif : adresse invalide « {} »", self.to))?;
        let kind = match &self.service {
            None => ProbeKind::Ping { count: 2 },
            Some(s) => match s.proto.as_str() {
                "tcp" => ProbeKind::Tcp { port: s.port },
                "udp" => ProbeKind::Udp { port: s.port },
                other => return Err(format!("objectif : protocole inconnu « {other} »")),
            },
        };
        Ok(Probe { src, dst, kind })
    }
}

/// La réponse attendue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Solution {
    /// Nature de la panne (voir `CauseKind::slug`).
    pub cause: String,
    /// Équipement fautif.
    pub device: String,
    /// Explication donnée à la fin.
    pub explanation: String,
    /// Commandes NETIV qui réparent (documentation, vérifiées par les tests).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

fn default_minutes() -> u32 {
    20
}

/// Un challenge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    /// Identifiant stable (`beginner-01`).
    pub id: String,
    /// Titre.
    pub title: String,
    /// Thème : `physical`, `addressing`, `switching`, `routing`, `services`, `security`.
    pub topic: String,
    /// Difficulté.
    pub difficulty: Difficulty,
    /// Énoncé.
    pub description: String,
    /// Durée conseillée.
    #[serde(default = "default_minutes")]
    pub time_limit_minutes: u32,
    /// Réseau décrit en ligne.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<TopologyDef>,
    /// Ou nom d'un réseau livré avec NETIV (`office`, `dmz`, `campus`, `triangle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology_ref: Option<String>,
    /// Objectif.
    pub objective: Objective,
    /// Pannes injectées (cachées).
    #[serde(with = "serde_yaml_ng::with::singleton_map_recursive")]
    pub faults: Vec<FaultSpec>,
    /// Solution.
    pub solution: Solution,
    /// Indices écrits à la main ; sinon calculés depuis la panne.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
}

/// Erreurs de construction d'un challenge.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChallengeError {
    /// Le challenge est mal défini.
    #[error("challenge « {id} » invalide :\n{problems}")]
    Invalid {
        /// Identifiant.
        id: String,
        /// Problèmes, un par ligne.
        problems: String,
    },
    /// Erreur du moteur.
    #[error(transparent)]
    Engine(#[from] NetivError),
}

/// Un challenge prêt à jouer.
#[derive(Debug, Clone)]
pub struct Built {
    /// Le réseau, pannes injectées (silencieusement).
    pub sim: Simulation,
    /// Le réseau sain (référence privée : jamais montrée à l'élève).
    pub baseline: Simulation,
    /// La sonde de l'objectif.
    pub probe: Probe,
    /// La requête de diagnostic correspondante.
    pub request: Request,
    /// La vraie cause racine (calculée par le diagnostic).
    pub cause: RootCause,
    /// Les indices, du plus vague à la solution.
    pub hints: Vec<String>,
}

impl Challenge {
    /// Lit un challenge YAML.
    pub fn from_yaml(text: &str) -> Result<Challenge, String> {
        serde_yaml_ng::from_str(text).map_err(|e| format!("challenge illisible : {e}"))
    }

    /// Écrit le challenge en YAML.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(self).map_err(|e| e.to_string())
    }

    /// Extrait le challenge d'un fichier `.netiv`.
    pub fn from_bundle(bundle: &netiv_topology::Bundle) -> Result<Challenge, String> {
        let value = bundle
            .challenge
            .clone()
            .ok_or("ce fichier .netiv ne contient pas de challenge")?;
        let mut c: Challenge = serde_yaml_ng::from_value(value)
            .map_err(|e| format!("challenge illisible dans le .netiv : {e}"))?;
        if c.topology.is_none() && c.topology_ref.is_none() {
            c.topology = Some(bundle.topology.clone());
        }
        Ok(c)
    }

    /// Empaquette le challenge (réseau compris) dans un fichier `.netiv`
    /// autonome, partageable.
    pub fn to_bundle(&self, author: &str) -> Result<netiv_topology::Bundle, String> {
        let topology = self.topology_def()?;
        let mut portable = self.clone();
        portable.topology = None;
        portable.topology_ref = None;
        let mut bundle = netiv_topology::Bundle::new(
            netiv_topology::BundleMeta {
                title: self.title.clone(),
                description: self.description.trim().to_string(),
                author: author.to_string(),
            },
            topology,
        );
        bundle.challenge = Some(serde_yaml_ng::to_value(&portable).map_err(|e| e.to_string())?);
        Ok(bundle)
    }

    /// Le réseau du challenge.
    pub fn topology_def(&self) -> Result<TopologyDef, String> {
        match (&self.topology, &self.topology_ref) {
            (Some(t), None) => Ok(t.clone()),
            (None, Some(name)) => {
                let text = crate::builtin::topology(name).ok_or_else(|| {
                    format!(
                        "réseau inconnu : « {name} » (réseaux livrés : {})",
                        crate::builtin::TOPOLOGY_NAMES.join(", ")
                    )
                })?;
                serde_yaml_ng::from_str(text)
                    .map_err(|e| format!("réseau « {name} » illisible : {e}"))
            }
            (Some(_), Some(_)) => {
                Err("« topology » et « topology_ref » sont exclusifs".to_string())
            }
            (None, None) => Err("il manque « topology » ou « topology_ref »".to_string()),
        }
    }

    fn probe_kind(&self) -> Result<ProbeKind, String> {
        Ok(match &self.objective.service {
            None => ProbeKind::Ping { count: 4 },
            Some(s) => match s.proto.as_str() {
                "tcp" => ProbeKind::Tcp { port: s.port },
                "udp" => ProbeKind::Udp { port: s.port },
                other => return Err(format!("objectif : protocole inconnu « {other} »")),
            },
        })
    }

    /// Construit le challenge : réseau sain, pannes cachées, sonde, vraie cause.
    ///
    /// Vérifie au passage sa cohérence (voir [`Challenge::problems`]).
    pub fn build(&self) -> Result<Built, ChallengeError> {
        let invalid = |problems: Vec<String>| ChallengeError::Invalid {
            id: self.id.clone(),
            problems: problems
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n"),
        };

        let def = self.topology_def().map_err(|e| invalid(vec![e]))?;
        let loaded = netiv_topology::build(&def).map_err(|e| invalid(e.errors))?;
        let healthy = loaded.sim;

        let src = healthy
            .device_id(&self.objective.from)
            .map_err(|e| invalid(vec![format!("objectif : {e}")]))?;
        let dst: Ipv4Addr = self.objective.to.parse().map_err(|_| {
            invalid(vec![format!(
                "objectif : adresse invalide « {} »",
                self.objective.to
            )])
        })?;
        let kind = self.probe_kind().map_err(|e| invalid(vec![e]))?;
        let probe = Probe { src, dst, kind };
        let request = Request {
            src,
            dst,
            service: self.objective.service.as_ref().map(|s| {
                let proto = if s.proto == "udp" {
                    L4Proto::Udp
                } else {
                    L4Proto::Tcp
                };
                (proto, s.port)
            }),
        };

        let mut problems = Vec::new();
        if !run_probe(&healthy, &probe)?.success {
            problems.push("l'objectif ne fonctionne pas sur le réseau sain".to_string());
        }
        let Some(solution_kind) = CauseKind::from_slug(&self.solution.cause) else {
            problems.push(format!(
                "solution : cause inconnue « {} » (causes : {})",
                self.solution.cause,
                CauseKind::ALL
                    .iter()
                    .map(|k| k.slug())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return Err(invalid(problems));
        };
        if self.faults.is_empty() {
            problems.push("aucune panne à injecter".to_string());
        }

        let mut sim = healthy.clone();
        for (n, spec) in self.faults.iter().enumerate() {
            match spec.resolve(&sim) {
                Ok(f) => {
                    if let Err(e) = sim.inject_fault(f, false) {
                        problems.push(format!("panne n°{} : {e}", n + 1));
                    }
                }
                Err(e) => problems.push(format!("panne n°{} : {e}", n + 1)),
            }
        }
        if !problems.is_empty() {
            return Err(invalid(problems));
        }

        if run_probe(&sim, &probe)?.success {
            problems.push(
                "l'objectif fonctionne malgré les pannes : le challenge n'a rien à réparer"
                    .to_string(),
            );
        }
        let diagnosis = diagnose(&sim, Some(&healthy), &request)?;
        let cause = diagnosis.cause.clone();
        match &cause {
            Some(c) => {
                if c.kind != solution_kind {
                    problems.push(format!(
                        "la solution annonce « {} » mais le diagnostic trouve « {} » ({})",
                        solution_kind.slug(),
                        c.kind.slug(),
                        c.detail
                    ));
                }
                if c.device != self.solution.device {
                    problems.push(format!(
                        "la solution désigne {} mais le diagnostic désigne {}",
                        self.solution.device, c.device
                    ));
                }
            }
            None => problems.push("le diagnostic ne trouve aucune cause".to_string()),
        }
        if self.time_limit_minutes == 0 {
            problems.push("durée nulle".to_string());
        }
        for (field, value) in [
            ("id", &self.id),
            ("titre", &self.title),
            ("énoncé", &self.description),
            ("thème", &self.topic),
        ] {
            if value.trim().is_empty() {
                problems.push(format!("{field} vide"));
            }
        }
        let Some(cause) = cause.filter(|_| problems.is_empty()) else {
            return Err(invalid(problems));
        };

        let hints = if self.hints.is_empty() {
            hints_for(&cause).to_vec()
        } else {
            self.hints.clone()
        };
        Ok(Built {
            sim,
            baseline: healthy,
            probe,
            request,
            cause,
            hints,
        })
    }

    /// Problèmes de définition (liste vide = challenge valide).
    pub fn problems(&self) -> Vec<String> {
        match self.build() {
            Ok(_) => Vec::new(),
            Err(ChallengeError::Invalid { problems, .. }) => problems
                .lines()
                .map(|l| l.trim_start_matches("  - ").to_string())
                .collect(),
            Err(e) => vec![e.to_string()],
        }
    }

    /// Multiplicateur d'XP selon la difficulté (en centièmes).
    pub fn xp_percent(&self) -> u32 {
        match self.difficulty {
            Difficulty::Easy => 100,
            Difficulty::Medium => 150,
            Difficulty::Hard => 200,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const GATEWAY: &str = r#"
id: test-gateway
title: Mauvaise passerelle
topic: addressing
difficulty: easy
description: PC1 ne joint plus SERVER.
time_limit_minutes: 10
topology_ref: office
objective: { from: PC1, to: 192.168.2.10 }
faults:
  - set_gateway: { device: PC1, gateway: 192.168.1.254 }
solution:
  cause: wrong-gateway
  device: PC1
  explanation: La passerelle doit être 192.168.1.1.
  commands: ["gateway PC1 192.168.1.1"]
"#;

    #[test]
    fn a_valid_challenge_builds() {
        let c = Challenge::from_yaml(GATEWAY).unwrap();
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        let b = c.build().unwrap();
        assert_eq!(b.cause.kind, CauseKind::WrongGateway);
        assert_eq!(b.hints.len(), 4);
        assert!(b.sim.trace().is_empty(), "les pannes sont cachées");
        assert!(!b.sim.faults().is_empty());
        assert!(b.baseline.faults().is_empty());
    }

    #[test]
    fn yaml_roundtrip() {
        let c = Challenge::from_yaml(GATEWAY).unwrap();
        let again = Challenge::from_yaml(&c.to_yaml().unwrap()).unwrap();
        assert_eq!(c, again);
    }

    fn problems_of(mutate: impl FnOnce(&mut Challenge)) -> Vec<String> {
        let mut c = Challenge::from_yaml(GATEWAY).unwrap();
        mutate(&mut c);
        c.problems()
    }

    fn any(p: &[String], needle: &str) -> bool {
        p.iter().any(|x| x.contains(needle))
    }

    #[test]
    fn inconsistent_challenges_are_rejected_with_a_reason() {
        let p = problems_of(|c| c.faults.clear());
        assert!(any(&p, "aucune panne"), "{p:?}");

        let p = problems_of(|c| c.solution.cause = "wrong-mask".into());
        assert!(any(&p, "diagnostic trouve « wrong-gateway »"), "{p:?}");

        let p = problems_of(|c| c.solution.device = "R1".into());
        assert!(any(&p, "diagnostic désigne PC1"), "{p:?}");

        let p = problems_of(|c| c.solution.cause = "nonsense".into());
        assert!(any(&p, "cause inconnue"), "{p:?}");

        let p = problems_of(|c| c.objective.from = "NOPE".into());
        assert!(any(&p, "équipement inconnu"), "{p:?}");

        let p = problems_of(|c| c.objective.to = "pas-une-ip".into());
        assert!(any(&p, "adresse invalide"), "{p:?}");

        let p = problems_of(|c| c.topology_ref = Some("inconnu".into()));
        assert!(any(&p, "réseau inconnu"), "{p:?}");

        let p = problems_of(|c| c.topology_ref = None);
        assert!(any(&p, "il manque"), "{p:?}");

        let p = problems_of(|c| {
            c.faults = vec![FaultSpec::LinkDown {
                a: "PC1:eth9".into(),
            }]
        });
        assert!(any(&p, "panne n°1"), "{p:?}");

        let p = problems_of(|c| c.time_limit_minutes = 0);
        assert!(any(&p, "durée nulle"), "{p:?}");

        let p = problems_of(|c| c.title = " ".into());
        assert!(any(&p, "titre vide"), "{p:?}");
    }

    #[test]
    fn a_fault_that_breaks_nothing_is_rejected() {
        // Couper le câble d'un PC sans importance ne gêne pas PC1 → SERVER.
        let p = problems_of(|c| {
            c.faults = vec![FaultSpec::AclDefault {
                device: "R1".into(),
                action: "allow".into(),
            }]
        });
        assert!(any(&p, "fonctionne malgré les pannes"), "{p:?}");
    }

    #[test]
    fn objective_must_work_on_the_healthy_network() {
        let p = problems_of(|c| c.objective.to = "10.77.0.1".into());
        assert!(any(&p, "ne fonctionne pas sur le réseau sain"), "{p:?}");
    }

    #[test]
    fn unknown_yaml_fields_are_refused() {
        assert!(Challenge::from_yaml(&GATEWAY.replace("topic:", "topik:")).is_err());
        assert!(Challenge::from_yaml("id: x").is_err());
    }

    #[test]
    fn hand_written_hints_replace_the_generated_ones() {
        let mut c = Challenge::from_yaml(GATEWAY).unwrap();
        c.hints = vec!["Regarde la passerelle".into(), "Elle est fausse".into()];
        assert_eq!(c.build().unwrap().hints.len(), 2);
    }

    #[test]
    fn a_challenge_travels_inside_a_netiv_file() {
        let c = Challenge::from_yaml(GATEWAY).unwrap();
        let bundle = c.to_bundle("prof").unwrap();
        assert_eq!(bundle.metadata.author, "prof");
        // Le fichier est autonome : plus de référence à un réseau livré.
        let text = bundle.to_yaml().unwrap();
        assert!(!text.contains("topology_ref"), "{text}");
        let again = netiv_topology::Bundle::from_yaml(&text).unwrap();
        let back = Challenge::from_bundle(&again).unwrap();
        assert!(back.topology.is_some() && back.topology_ref.is_none());
        assert!(back.problems().is_empty(), "{:?}", back.problems());
        assert_eq!(back.solution, c.solution);
        // Un .netiv sans challenge est refusé clairement.
        let mut plain = again.clone();
        plain.challenge = None;
        assert!(
            Challenge::from_bundle(&plain)
                .unwrap_err()
                .contains("ne contient pas de challenge")
        );
    }

    #[test]
    fn xp_multiplier_follows_difficulty() {
        let mut c = Challenge::from_yaml(GATEWAY).unwrap();
        assert_eq!(c.xp_percent(), 100);
        c.difficulty = Difficulty::Hard;
        assert_eq!(c.xp_percent(), 200);
    }
}
