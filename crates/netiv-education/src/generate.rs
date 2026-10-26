//! Générateur de challenges : un réseau, une panne, un énoncé, une solution et
//! un barème, produits automatiquement **puis vérifiés par le moteur**.
//!
//! Un challenge généré n'est jamais livré sans avoir passé les mêmes contrôles
//! qu'un challenge écrit à la main (`Challenge::problems`) : réseau sain
//! fonctionnel, panne réelle, diagnostic qui retrouve la cause annoncée. Si un
//! tirage échoue, on essaie la graine suivante.

use std::net::Ipv4Addr;

use netiv_chaos::{Chaos, Difficulty, Filter, Flow};
use netiv_core::Simulation;
use netiv_core::packet::L4Proto;
use netiv_diagnostics::diagnose;
use netiv_topology::generate::{GenSpec, TopoKind};

use crate::challenge::{Challenge, Objective, ServiceRef, Solution};
use crate::faultspec::FaultSpec;

/// Thèmes que le générateur sait produire.
pub const TOPICS: [&str; 6] = [
    "physical",
    "addressing",
    "switching",
    "routing",
    "services",
    "security",
];

/// Nombre de graines essayées avant d'abandonner.
const ATTEMPTS: u64 = 24;

/// Paramètres de génération.
#[derive(Debug, Clone, Copy)]
pub struct GenerateSpec<'a> {
    /// Thème (voir [`TOPICS`]).
    pub topic: &'a str,
    /// Difficulté.
    pub difficulty: Difficulty,
    /// Graine.
    pub seed: u64,
}

/// Gabarits de panne pertinents pour un thème. La latence élevée n'en fait
/// pas partie : elle ne casse pas le flux, donc n'a rien à « réparer ».
fn templates_for(topic: &str) -> Option<Vec<&'static str>> {
    Some(match topic {
        "physical" => vec!["link-down", "interface-down", "packet-loss"],
        "addressing" => vec!["wrong-ip", "wrong-mask", "wrong-gateway", "no-gateway"],
        "switching" => vec!["wrong-vlan", "static-arp"],
        "routing" => vec![
            "missing-route",
            "wrong-route",
            "routing-loop",
            "forwarding-off",
        ],
        "services" => vec!["service-down"],
        "security" => vec!["acl-deny", "acl-default-deny"],
        _ => return None,
    })
}

/// Le réseau et le flux d'un tirage.
struct Setup {
    sim: Simulation,
    name: String,
    flow: (String, Ipv4Addr, Option<(L4Proto, u16)>),
}

fn load(yaml: &str) -> Result<Simulation, String> {
    netiv_topology::load_yaml(yaml)
        .map(|l| l.sim)
        .map_err(|e| e.to_string())
}

fn generated(
    kind: TopoKind,
    routers: usize,
    hosts: usize,
    seed: u64,
) -> Result<Simulation, String> {
    let def = netiv_topology::generate::generate(&GenSpec {
        kind,
        routers,
        hosts,
        seed,
    })?;
    netiv_topology::build(&def)
        .map(|l| l.sim)
        .map_err(|e| e.to_string())
}

fn address_of(sim: &Simulation, name: &str) -> Result<Ipv4Addr, String> {
    let id = sim.device_id(name).map_err(|e| e.to_string())?;
    sim.device(id)
        .and_then(|d| d.ifaces.iter().find_map(|i| i.ip.map(|n| n.addr())))
        .ok_or_else(|| format!("{name} n'a pas d'adresse"))
}

/// Choisit le réseau et le flux selon le thème, de façon déterministe.
fn setup(topic: &str, seed: u64) -> Result<Setup, String> {
    let variant = (seed % 3) as usize;
    let (sim, name, src, dst, service): (Simulation, String, &str, &str, Option<(L4Proto, u16)>) =
        match topic {
            "physical" | "addressing" => {
                let sim = generated(TopoKind::Enterprise, 3 + variant, 4 + 2 * variant, seed)?;
                (sim, "enterprise".into(), "PC1", "SRV1", None)
            }
            "services" => {
                let sim = generated(TopoKind::Enterprise, 3 + variant, 6, seed)?;
                (
                    sim,
                    "enterprise".into(),
                    "PC1",
                    "SRV1",
                    Some((L4Proto::Tcp, 80)),
                )
            }
            "routing" => {
                let kind = [TopoKind::Isp, TopoKind::Hybrid, TopoKind::Enterprise][variant];
                let sim = generated(kind, 4 + variant, 6, seed)?;
                (sim, kind.slug().into(), "PC1", "SRV1", None)
            }
            "switching" => (
                load(crate::builtin::topology("campus").ok_or("réseau « campus » absent")?)?,
                "campus".into(),
                "PC1",
                if variant == 0 { "PC3" } else { "SRV" },
                None,
            ),
            "security" => {
                if variant == 0 {
                    (
                        load(crate::builtin::topology("dmz").ok_or("réseau « dmz » absent")?)?,
                        "dmz".into(),
                        "PC1",
                        "SRV",
                        Some((L4Proto::Tcp, 80)),
                    )
                } else {
                    (
                        load(crate::builtin::topology("cyber").ok_or("réseau « cyber » absent")?)?,
                        "cyber".into(),
                        "PC01",
                        "WEB01",
                        Some((L4Proto::Tcp, 80)),
                    )
                }
            }
            other => {
                return Err(format!(
                    "thème inconnu : « {other} » (thèmes : {})",
                    TOPICS.join(", ")
                ));
            }
        };
    let dst_ip = address_of(&sim, dst)?;
    Ok(Setup {
        sim,
        name,
        flow: (src.to_string(), dst_ip, service),
    })
}

/// Difficultés à essayer, de la demandée vers les voisines.
fn difficulty_ladder(d: Difficulty) -> Vec<Difficulty> {
    match d {
        Difficulty::Easy => vec![Difficulty::Easy, Difficulty::Medium, Difficulty::Hard],
        Difficulty::Medium => vec![Difficulty::Medium, Difficulty::Easy, Difficulty::Hard],
        Difficulty::Hard => vec![Difficulty::Hard, Difficulty::Medium, Difficulty::Easy],
    }
}

fn symptom(src: &str, dst_name: &str, dst_ip: Ipv4Addr, service: Option<(L4Proto, u16)>) -> String {
    match service {
        None => format!(
            "{src} n'arrive plus à joindre {dst_name} ({dst_ip}) : le ping reste sans réponse."
        ),
        Some((p, port)) => format!(
            "{src} n'arrive plus à utiliser le service {p}/{port} de {dst_name} ({dst_ip})."
        ),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

/// Génère un challenge valide.
pub fn generate_challenge(spec: &GenerateSpec) -> Result<Challenge, String> {
    let templates = templates_for(spec.topic).ok_or_else(|| {
        format!(
            "thème inconnu : « {} » (thèmes : {})",
            spec.topic,
            TOPICS.join(", ")
        )
    })?;
    let mut last_error = String::from("aucun tirage valide");
    for attempt in 0..ATTEMPTS {
        let seed = spec.seed.wrapping_add(attempt);
        match try_once(spec, &templates, seed) {
            Ok(c) => return Ok(c),
            Err(e) => last_error = e,
        }
    }
    Err(format!(
        "impossible de générer un challenge « {} » ({}) après {ATTEMPTS} essais : {last_error}",
        spec.topic,
        spec.difficulty.slug()
    ))
}

fn try_once(
    spec: &GenerateSpec,
    templates: &[&'static str],
    seed: u64,
) -> Result<Challenge, String> {
    let setup = setup(spec.topic, seed)?;
    let src = setup
        .sim
        .device_id(&setup.flow.0)
        .map_err(|e| e.to_string())?;
    let (_, dst_ip, service) = setup.flow.clone();
    let flow = Flow {
        src,
        dst: dst_ip,
        service,
    };
    let healthy = setup.sim.clone();
    let dst_name = healthy
        .devices()
        .iter()
        .find(|d| {
            d.ifaces
                .iter()
                .any(|i| i.ip.is_some_and(|n| n.addr() == dst_ip))
        })
        .map(|d| d.name.clone())
        .ok_or("destination introuvable")?;

    let mut chaos = Chaos::new(&healthy, flow).map_err(|e| e.to_string())?;
    let mut failure = String::from("aucune panne applicable");
    for difficulty in difficulty_ladder(spec.difficulty) {
        let filter = Filter {
            difficulty: Some(difficulty),
            templates: Some(templates.to_vec()),
            ..Filter::default()
        };
        let mut work = healthy.clone();
        let inj = match chaos.start(&mut work, &filter, seed, false) {
            Ok(i) => i,
            Err(e) => {
                failure = e.to_string();
                continue;
            }
        };
        let fault = work
            .faults()
            .iter()
            .find(|f| f.id == inj.fault_id)
            .map(|f| f.fault.clone())
            .ok_or("panne introuvable")?;
        let request = flow.request();
        let diagnosis = diagnose(&work, Some(&healthy), &request).map_err(|e| e.to_string())?;
        let Some(cause) = diagnosis.cause else {
            failure = "le diagnostic ne trouve rien".to_string();
            continue;
        };
        let commands: Vec<String> = cause
            .fix
            .as_deref()
            .map(|f| {
                f.split(';')
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if commands.is_empty() || commands.iter().any(|c| c.contains('<')) {
            failure = "pas de correctif exécutable".to_string();
            continue;
        }
        let template = netiv_chaos::catalog()
            .iter()
            .find(|t| t.id == inj.template)
            .ok_or("gabarit introuvable")?;

        let objective = Objective {
            from: setup.flow.0.clone(),
            to: dst_ip.to_string(),
            name: None,
            service: service.map(|(p, port)| ServiceRef {
                proto: p.to_string(),
                port,
            }),
        };
        let topo = netiv_topology::export(&healthy, &setup.name);
        let challenge = Challenge {
            id: format!("gen-{}-{}-{}", spec.topic, difficulty.slug(), seed),
            title: format!(
                "Mystère {} n°{seed}",
                crate::profile::topic_label(spec.topic).to_lowercase()
            ),
            topic: spec.topic.to_string(),
            difficulty,
            track: None,
            ticket: None,
            description: format!(
                "{} Trouve la cause et corrige-la.\n\nRéseau : {} équipements, dont {} routeur(s) ou pare-feu.",
                symptom(&setup.flow.0, &dst_name, dst_ip, service),
                healthy.devices().len(),
                healthy
                    .devices()
                    .iter()
                    .filter(|d| matches!(
                        d.kind,
                        netiv_core::DeviceKind::Router | netiv_core::DeviceKind::Firewall
                    ))
                    .count()
            ),
            time_limit_minutes: match difficulty {
                Difficulty::Easy => 15,
                Difficulty::Medium => 20,
                Difficulty::Hard => 30,
            },
            topology: Some(topo),
            topology_ref: None,
            objective,
            faults: vec![FaultSpec::from_fault(&healthy, &fault)],
            solution: Solution {
                cause: cause.kind.slug().to_string(),
                device: cause.device.clone(),
                explanation: format!("{} : {}.", capitalize(template.summary), cause.detail),
                commands,
            },
            hints: Vec::new(),
        };
        let problems = challenge.problems();
        if problems.is_empty() {
            return Ok(challenge);
        }
        failure = problems.join(" ; ");
    }
    Err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(topic: &str, d: Difficulty, seed: u64) -> GenerateSpec<'_> {
        GenerateSpec {
            topic,
            difficulty: d,
            seed,
        }
    }

    #[test]
    fn every_topic_and_difficulty_yields_a_valid_challenge() {
        for topic in TOPICS {
            for d in [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard] {
                for seed in 0..3 {
                    let c = generate_challenge(&spec(topic, d, seed))
                        .unwrap_or_else(|e| panic!("{topic}/{}/{seed} : {e}", d.slug()));
                    assert_eq!(c.topic, topic);
                    let problems = c.problems();
                    assert!(problems.is_empty(), "{} : {problems:?}", c.id);
                    assert!(!c.solution.commands.is_empty());
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let a = generate_challenge(&spec("routing", Difficulty::Hard, 5)).unwrap();
        let b = generate_challenge(&spec("routing", Difficulty::Hard, 5)).unwrap();
        assert_eq!(a.to_yaml().unwrap(), b.to_yaml().unwrap());
    }

    #[test]
    fn different_seeds_give_different_challenges() {
        let ids: std::collections::BTreeSet<String> = (0..12)
            .map(|s| {
                let c = generate_challenge(&spec("routing", Difficulty::Medium, s)).unwrap();
                format!("{}|{}|{:?}", c.solution.cause, c.solution.device, c.faults)
            })
            .collect();
        assert!(ids.len() >= 4, "trop peu de variété : {ids:?}");
    }

    #[test]
    fn the_requested_difficulty_is_honoured_when_the_topic_offers_it() {
        let hard = generate_challenge(&spec("routing", Difficulty::Hard, 1)).unwrap();
        assert_eq!(hard.difficulty, Difficulty::Hard);
        assert_eq!(hard.time_limit_minutes, 30);
        let easy = generate_challenge(&spec("services", Difficulty::Easy, 1)).unwrap();
        assert_eq!(easy.difficulty, Difficulty::Easy);
        assert_eq!(easy.solution.cause, "service-down");
    }

    #[test]
    fn a_topic_without_the_requested_difficulty_falls_back_to_the_closest_one() {
        // « services » n'a qu'un gabarit facile : un défi « hard » retombe dessus.
        let c = generate_challenge(&spec("services", Difficulty::Hard, 1)).unwrap();
        assert_eq!(c.difficulty, Difficulty::Easy);
        assert_eq!(c.solution.cause, "service-down");
    }

    #[test]
    fn the_statement_describes_the_symptom_without_leaking_the_cause() {
        for topic in TOPICS {
            let c = generate_challenge(&spec(topic, Difficulty::Medium, 2)).unwrap();
            let text = c.description.to_lowercase();
            assert!(text.contains("trouve la cause"), "{text}");
            assert!(!text.contains(&c.solution.cause), "{} : {text}", c.id);
            for cmd in &c.solution.commands {
                assert!(!c.description.contains(cmd.as_str()));
            }
        }
    }

    #[test]
    fn generated_challenges_are_self_contained_and_survive_a_file_roundtrip() {
        let c = generate_challenge(&spec("addressing", Difficulty::Easy, 4)).unwrap();
        assert!(c.topology.is_some() && c.topology_ref.is_none());
        let back = Challenge::from_yaml(&c.to_yaml().unwrap()).unwrap();
        assert_eq!(back, c);
        let bundle = c.to_bundle("generateur").unwrap();
        let again = Challenge::from_bundle(
            &netiv_topology::Bundle::from_yaml(&bundle.to_yaml().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(again.problems().is_empty());
    }

    #[test]
    fn unknown_topic_is_reported() {
        let e = generate_challenge(&spec("cuisine", Difficulty::Easy, 0)).unwrap_err();
        assert!(e.contains("thème inconnu") && e.contains("routing"), "{e}");
    }

    #[test]
    fn identifiers_encode_topic_difficulty_and_seed() {
        let c = generate_challenge(&spec("physical", Difficulty::Easy, 7)).unwrap();
        assert!(c.id.starts_with("gen-physical-easy-"), "{}", c.id);
    }
}
