//! Cyber range : un incident s'est produit dans le simulateur ; l'élève mène
//! l'enquête puis corrige la configuration sans casser le trafic légitime.
//!
//! Tout est simulé : le « trafic d'attaque » est un script de commandes
//! exécutées par des équipements virtuels, rien ne sort de NETIV.

use std::net::Ipv4Addr;

use netiv_chaos::Difficulty;
use netiv_core::{Command, NetivError, SimTime, Simulation};
use netiv_diagnostics::{Probe, run_probe};
use netiv_security::Capture;
use netiv_security::forensics::{Behavior, analyze_capture};
use serde::{Deserialize, Serialize};

use crate::challenge::Objective;
use crate::faultspec::FaultSpec;
use crate::run::{AnswerOutcome, RunError, Score, ScoreInput, compute_score};

fn one() -> u32 {
    1
}

fn thousand() -> u64 {
    1_000
}

/// Un lot de trafic scripté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficStep {
    /// Date de départ (secondes simulées).
    pub at_s: u64,
    /// Équipement émetteur.
    pub from: String,
    /// Adresse visée.
    pub to: String,
    /// `tcp` ou `udp`.
    pub proto: String,
    /// Ports visés, dans l'ordre.
    pub ports: Vec<u16>,
    /// Nombre de passes sur la liste de ports.
    #[serde(default = "one")]
    pub repeat: u32,
    /// Délai entre deux envois.
    #[serde(default = "thousand")]
    pub every_ms: u64,
}

/// Le coupable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Culprit {
    /// Équipement à l'origine du trafic.
    pub device: String,
    /// Comportement (voir `forensics::Behavior::slug`).
    pub behavior: String,
}

/// Correction attendue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeSolution {
    /// Explication donnée à la fin.
    pub explanation: String,
    /// Commandes NETIV qui corrigent (vérifiées par les tests).
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Un scénario de cyber range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeScenario {
    /// Identifiant (`range-01`).
    pub id: String,
    /// Titre.
    pub title: String,
    /// Difficulté.
    pub difficulty: Difficulty,
    /// Énoncé.
    pub description: String,
    /// Durée conseillée.
    pub time_limit_minutes: u32,
    /// Réseau livré.
    pub topology_ref: String,
    /// Mauvaises configurations injectées (cachées).
    #[serde(with = "serde_yaml_ng::with::singleton_map_recursive")]
    pub faults: Vec<FaultSpec>,
    /// Trafic scripté.
    pub traffic: Vec<TrafficStep>,
    /// Le coupable.
    pub culprit: Culprit,
    /// Flux qui doivent être bloqués après correction.
    pub must_block: Vec<Objective>,
    /// Flux légitimes qui doivent continuer à fonctionner.
    pub must_work: Vec<Objective>,
    /// Solution.
    pub solution: RangeSolution,
}

/// Un scénario prêt à jouer.
#[derive(Debug, Clone)]
pub struct RangeBuilt {
    /// Le réseau, avec l'historique de l'incident dans la trace.
    pub sim: Simulation,
    /// Sondes qui doivent échouer après correction.
    pub blocked: Vec<Probe>,
    /// Sondes qui doivent réussir après correction.
    pub working: Vec<Probe>,
    /// Indices.
    pub hints: Vec<String>,
}

impl RangeScenario {
    /// Lit un scénario YAML.
    pub fn from_yaml(text: &str) -> Result<RangeScenario, String> {
        serde_yaml_ng::from_str(text).map_err(|e| format!("scénario illisible : {e}"))
    }

    /// Construit le scénario : réseau sain → pannes cachées → incident rejoué.
    pub fn build(&self) -> Result<RangeBuilt, String> {
        let text = crate::builtin::topology(&self.topology_ref)
            .ok_or_else(|| format!("réseau inconnu : « {} »", self.topology_ref))?;
        let mut loaded = netiv_topology::load_yaml(text).map_err(|e| e.to_string())?;
        loaded.sim.clear_trace();

        for (n, spec) in self.faults.iter().enumerate() {
            let f = spec
                .resolve(&loaded.sim)
                .map_err(|e| format!("panne n°{} : {e}", n + 1))?;
            loaded
                .sim
                .inject_fault(f, false)
                .map_err(|e| format!("panne n°{} : {e}", n + 1))?;
        }
        // Les clients DHCP redemandent leur adresse, pannes comprises.
        if loaded.sim.has_dhcp_clients() {
            loaded.sim.restart_dhcp().map_err(|e| e.to_string())?;
        }
        self.schedule_traffic(&mut loaded.sim)?;
        loaded.sim.run_until_idle().map_err(|e| e.to_string())?;

        let blocked = self
            .must_block
            .iter()
            .map(|o| o.to_probe(&loaded.sim))
            .collect::<Result<Vec<_>, _>>()?;
        let working = self
            .must_work
            .iter()
            .map(|o| o.to_probe(&loaded.sim))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RangeBuilt {
            sim: loaded.sim,
            blocked,
            working,
            hints: self.hints(),
        })
    }

    fn schedule_traffic(&self, sim: &mut Simulation) -> Result<(), String> {
        for step in &self.traffic {
            let dev = sim.device_id(&step.from).map_err(|e| e.to_string())?;
            let dst: Ipv4Addr = step
                .to
                .parse()
                .map_err(|_| format!("trafic : adresse invalide « {} »", step.to))?;
            let mut k: u64 = 0;
            for _ in 0..step.repeat.max(1) {
                for port in &step.ports {
                    let cmd = match step.proto.as_str() {
                        "tcp" => Command::Curl { dst, port: *port },
                        "udp" => Command::SendUdp {
                            dst,
                            port: *port,
                            data: "x".to_string(),
                        },
                        other => return Err(format!("trafic : protocole inconnu « {other} »")),
                    };
                    let at = SimTime::from_millis(step.at_s * 1_000 + k * step.every_ms);
                    sim.schedule_command(at, dev, cmd)
                        .map_err(|e: NetivError| e.to_string())?;
                    k += 1;
                }
            }
        }
        Ok(())
    }

    /// Indices génériques (4 niveaux), propres à l'enquête.
    pub fn hints(&self) -> Vec<String> {
        vec![
            "Indice 1 — Commence par établir la chronologie : « capture save incident.nlv » puis « forensic analyze incident.nlv » (ou « forensic analyze live »).".to_string(),
            "Indice 2 — Quelle machine initie des connexions qu'elle ne devrait jamais initier ? Compare son rôle (dmz, user, server) avec celui de ses cibles.".to_string(),
            "Indice 3 — Le pare-feu a laissé passer ce flux : lis ses règles avec « show acl FW1 ». Laquelle autorise la zone fautive vers la zone protégée ?".to_string(),
            format!(
                "Solution — {} Commande : {}",
                self.solution.explanation.trim(),
                self.solution.commands.join(" ; ")
            ),
        ]
    }

    /// Problèmes de définition (liste vide = valide). Vérifie que l'incident
    /// est réel, détectable, corrigeable, et que la correction ne casse rien.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        let built = match self.build() {
            Ok(b) => b,
            Err(e) => return vec![e],
        };
        let Some(behavior) = parse_behavior(&self.culprit.behavior) else {
            return vec![format!(
                "comportement inconnu : « {} »",
                self.culprit.behavior
            )];
        };
        if self.must_block.is_empty() || self.must_work.is_empty() {
            p.push("il faut au moins un flux à bloquer et un flux à préserver".to_string());
        }
        // 1. L'incident est réel : au moins un flux « à bloquer » réussit avant correction.
        let exposed = built
            .blocked
            .iter()
            .filter(|pr| run_probe(&built.sim, pr).is_ok_and(|r| r.success))
            .count();
        if exposed == 0 {
            p.push("avant correction, aucun flux « à bloquer » ne passe : l'incident n'a pas de brèche".to_string());
        }
        // 2. Les flux légitimes fonctionnent déjà.
        for (o, pr) in self.must_work.iter().zip(&built.working) {
            if !run_probe(&built.sim, pr).is_ok_and(|r| r.success) {
                p.push(format!(
                    "flux légitime {} → {} KO même avant correction",
                    o.from, o.to
                ));
            }
        }
        // 3. L'enquête aboutit : la forensique voit le coupable.
        let cap = Capture::from_sim(&built.sim, &self.id);
        let analysis = analyze_capture(&cap);
        if !analysis
            .findings
            .iter()
            .any(|f| f.behavior == behavior && f.source == self.culprit.device)
        {
            p.push(format!(
                "la forensique ne détecte pas « {} » depuis {}",
                self.culprit.behavior, self.culprit.device
            ));
        }
        // 4. La correction (réparer les pannes injectées) bloque et préserve.
        let mut fixed = built.sim.clone();
        fixed.repair_all_faults(false);
        for (o, pr) in self.must_block.iter().zip(&built.blocked) {
            if run_probe(&fixed, pr).is_ok_and(|r| r.success) {
                p.push(format!(
                    "flux {} → {} toujours ouvert après correction",
                    o.from, o.to
                ));
            }
        }
        for (o, pr) in self.must_work.iter().zip(&built.working) {
            if !run_probe(&fixed, pr).is_ok_and(|r| r.success) {
                p.push(format!(
                    "flux légitime {} → {} cassé par la correction",
                    o.from, o.to
                ));
            }
        }
        p
    }
}

/// Comprend un nom de comportement : nom court ou libellé.
pub fn parse_behavior(text: &str) -> Option<Behavior> {
    let t = text.trim().to_lowercase().replace(['_', ' '], "-");
    [
        (
            Behavior::PortScan,
            ["port-scan", "balayage-de-ports", "scan"],
        ),
        (
            Behavior::HostSweep,
            ["host-sweep", "balayage-de-reseau", "sweep"],
        ),
        (
            Behavior::ZoneViolation,
            ["lateral-movement", "mouvement-lateral", "rebond"],
        ),
        (Behavior::Beaconing, ["beaconing", "balise", "beacon"]),
        (
            Behavior::RepeatedDenials,
            ["repeated-denials", "refus-repetes", "denials"],
        ),
        (
            Behavior::ArpConflict,
            ["arp-spoofing", "usurpation-arp", "arp-conflict"],
        ),
        (
            Behavior::RogueDhcp,
            ["rogue-dhcp", "dhcp-pirate", "serveur-dhcp-pirate"],
        ),
    ]
    .iter()
    .find(|(_, names)| names.contains(&t.as_str()))
    .map(|(b, _)| *b)
}

/// Résultat de `range check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeCheck {
    /// La correction est insuffisante ou casse du trafic légitime.
    Failing {
        /// Flux « à bloquer » encore ouverts.
        still_open: Vec<String>,
        /// Flux légitimes cassés.
        broken: Vec<String>,
    },
    /// Réglé, mais le coupable n'a pas été désigné.
    NeedsAnswer,
    /// Terminé.
    Finished(Score),
}

/// Un scénario en cours.
#[derive(Debug, Clone)]
pub struct RangeRun {
    scenario: RangeScenario,
    blocked: Vec<Probe>,
    working: Vec<Probe>,
    hints: Vec<String>,
    started_secs: u64,
    hints_used: u32,
    wrong_answers: u32,
    answered: bool,
    fixed: bool,
    finished: Option<Score>,
    gave_up: bool,
}

impl RangeRun {
    /// Démarre un scénario.
    pub fn start(scenario: RangeScenario, now_secs: u64) -> Result<(RangeRun, Simulation), String> {
        let built = scenario.build()?;
        Ok((
            RangeRun {
                scenario,
                blocked: built.blocked,
                working: built.working,
                hints: built.hints,
                started_secs: now_secs,
                hints_used: 0,
                wrong_answers: 0,
                answered: false,
                fixed: false,
                finished: None,
                gave_up: false,
            },
            built.sim,
        ))
    }

    /// Le scénario joué.
    pub fn scenario(&self) -> &RangeScenario {
        &self.scenario
    }

    /// Terminé ?
    pub fn is_finished(&self) -> bool {
        self.finished.is_some() || self.gave_up
    }

    /// Secondes restantes sur le temps conseillé.
    pub fn remaining_secs(&self, now: u64) -> i64 {
        i64::from(self.scenario.time_limit_minutes) * 60
            - i64::try_from(now.saturating_sub(self.started_secs)).unwrap_or(i64::MAX)
    }

    /// Indices utilisés.
    pub fn hints_used(&self) -> u32 {
        self.hints_used
    }

    /// Indice suivant (−10 points chacun).
    pub fn next_hint(&mut self) -> Result<String, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        let idx = (self.hints_used as usize).min(self.hints.len().saturating_sub(1));
        self.hints_used += 1;
        Ok(self.hints.get(idx).cloned().unwrap_or_default())
    }

    /// Désigne le coupable et son comportement.
    pub fn answer(
        &mut self,
        device: &str,
        behavior: &str,
        now: u64,
    ) -> Result<AnswerOutcome, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        if self.answered {
            return Ok(AnswerOutcome::Correct(None));
        }
        let device_ok = device.eq_ignore_ascii_case(&self.scenario.culprit.device);
        let cause_ok = parse_behavior(behavior)
            .is_some_and(|b| Some(b) == parse_behavior(&self.scenario.culprit.behavior));
        if device_ok && cause_ok {
            self.answered = true;
            let score = self.fixed.then(|| self.finalize(now));
            return Ok(AnswerOutcome::Correct(score));
        }
        self.wrong_answers += 1;
        Ok(AnswerOutcome::Wrong {
            device_ok,
            cause_ok,
        })
    }

    /// Vérifie la correction sur une copie propre du réseau.
    pub fn check(&mut self, sim: &Simulation, now: u64) -> Result<RangeCheck, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        let label = |o: &Objective| match &o.service {
            None => format!("{} → {} (ping)", o.from, o.to),
            Some(s) => format!("{} → {} ({}/{})", o.from, o.to, s.proto, s.port),
        };
        let mut still_open = Vec::new();
        for (o, p) in self.scenario.must_block.iter().zip(&self.blocked) {
            if run_probe(sim, p).is_ok_and(|r| r.success) {
                still_open.push(label(o));
            }
        }
        let mut broken = Vec::new();
        for (o, p) in self.scenario.must_work.iter().zip(&self.working) {
            if !run_probe(sim, p).is_ok_and(|r| r.success) {
                broken.push(label(o));
            }
        }
        if !still_open.is_empty() || !broken.is_empty() {
            return Ok(RangeCheck::Failing { still_open, broken });
        }
        self.fixed = true;
        if self.answered {
            Ok(RangeCheck::Finished(self.finalize(now)))
        } else {
            Ok(RangeCheck::NeedsAnswer)
        }
    }

    /// Termine sans désigner le coupable.
    pub fn finish(&mut self, now: u64) -> Result<Score, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        if !self.fixed {
            return Err(RunError::NotFixed);
        }
        Ok(self.finalize(now))
    }

    /// Abandonne.
    pub fn give_up(&mut self) -> Result<(), RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        self.gave_up = true;
        Ok(())
    }

    fn finalize(&mut self, now: u64) -> Score {
        let xp_percent = match self.scenario.difficulty {
            Difficulty::Easy => 100,
            Difficulty::Medium => 150,
            Difficulty::Hard => 200,
        };
        let score = compute_score(&ScoreInput {
            answered: self.answered,
            wrong_answers: self.wrong_answers,
            fixed: self.fixed,
            hints: self.hints_used,
            elapsed_secs: now.saturating_sub(self.started_secs),
            limit_minutes: self.scenario.time_limit_minutes,
            xp_percent,
        });
        self.finished = Some(score.clone());
        score
    }
}
