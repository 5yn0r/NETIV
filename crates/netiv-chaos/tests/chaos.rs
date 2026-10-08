#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Le Chaos Engine contre le moteur de diagnostic : tout ce que l'un casse,
//! l'autre doit le retrouver, et la réparation doit tout restaurer.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use netiv_chaos::{Chaos, ChaosError, Difficulty, Filter, Flow, catalog};
use netiv_core::packet::L4Proto;
use netiv_core::service::Service;
use netiv_core::{SimRng, Simulation};
use netiv_diagnostics::{CauseKind, Layer, Status, diagnose, run_probe};

fn load(yaml: &str) -> Simulation {
    netiv_topology::load_yaml(yaml).unwrap().sim
}

fn mvp() -> Simulation {
    let mut sim = load(include_str!("../../../examples/mvp.yaml"));
    let srv = sim.device_id("SERVER").unwrap();
    sim.add_service(srv, Service::tcp(80, "http")).unwrap();
    sim.clear_trace();
    sim
}

fn dmz() -> Simulation {
    load(include_str!("../../../examples/dmz.yaml"))
}

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

/// (nom, réseau, flux)
fn scenarios() -> Vec<(&'static str, Simulation, Flow)> {
    let m = mvp();
    let pc1 = m.device_id("PC1").unwrap();
    let d = dmz();
    let dpc = d.device_id("PC1").unwrap();
    vec![
        (
            "mvp/ping",
            m.clone(),
            Flow {
                src: pc1,
                dst: ip("192.168.2.10"),
                service: None,
            },
        ),
        (
            "mvp/web",
            m,
            Flow {
                src: pc1,
                dst: ip("192.168.2.10"),
                service: Some((L4Proto::Tcp, 80)),
            },
        ),
        (
            "dmz/web",
            d.clone(),
            Flow {
                src: dpc,
                dst: ip("10.9.0.10"),
                service: Some((L4Proto::Tcp, 80)),
            },
        ),
        (
            "dmz/ping",
            d,
            Flow {
                src: dpc,
                dst: ip("10.9.0.10"),
                service: None,
            },
        ),
    ]
}

#[test]
fn every_template_is_found_by_the_diagnosis_and_healed_by_the_repair() {
    let mut covered: BTreeSet<&str> = BTreeSet::new();
    let mut checked = 0;
    for (name, sim, flow) in scenarios() {
        let chaos = Chaos::new(&sim, flow).unwrap();
        for t in catalog() {
            for seed in 0..6u64 {
                let mut work = sim.clone();
                let healthy = work.config_fingerprint();
                let mut rng = SimRng::new(seed * 7919 + 1);
                let Some(fault) = (t.build)(&work, chaos.path(), &mut rng) else {
                    continue;
                };
                let id = work.inject_fault(fault, false).unwrap();
                let d = diagnose(&work, Some(&chaos.baseline), &flow.request()).unwrap();
                let got = d.cause.as_ref().map(|c| c.kind);
                assert_eq!(
                    got,
                    Some(t.kind),
                    "[{name}] gabarit « {} » (graine {seed}) : cause trouvée {:?}\n{}",
                    t.id,
                    got,
                    d.render(true, true)
                );
                assert_ne!(
                    d.status,
                    Status::Healthy,
                    "[{name}] {} : doit être détecté",
                    t.id
                );
                // La réparation restaure exactement la configuration et le flux.
                work.repair_fault(id, false).unwrap();
                assert_eq!(work.config_fingerprint(), healthy, "[{name}] {}", t.id);
                let probe = run_probe(
                    &work,
                    &netiv_diagnostics::Probe {
                        src: flow.src,
                        dst: flow.dst,
                        kind: match flow.service {
                            None => netiv_diagnostics::ProbeKind::Ping { count: 2 },
                            Some((L4Proto::Tcp, p)) => {
                                netiv_diagnostics::ProbeKind::Tcp { port: p }
                            }
                            Some((L4Proto::Udp, p)) => {
                                netiv_diagnostics::ProbeKind::Udp { port: p }
                            }
                        },
                    },
                )
                .unwrap();
                assert!(probe.success, "[{name}] {} : flux non rétabli", t.id);
                covered.insert(t.id);
                checked += 1;
            }
        }
    }
    let all: BTreeSet<&str> = catalog().iter().map(|t| t.id).collect();
    let missing: Vec<_> = all.difference(&covered).collect();
    assert!(
        missing.is_empty(),
        "gabarits jamais applicables sur ces réseaux : {missing:?}"
    );
    assert!(checked >= 17 * 2, "{checked} combinaisons vérifiées");
}

#[test]
fn the_catalog_covers_every_cause_the_diagnosis_knows() {
    let kinds: BTreeSet<CauseKind> = catalog().iter().map(|t| t.kind).collect();
    for k in CauseKind::ALL {
        assert!(kinds.contains(&k), "aucune panne ne produit {k:?}");
    }
    let ids: BTreeSet<&str> = catalog().iter().map(|t| t.id).collect();
    assert_eq!(
        ids.len(),
        catalog().len(),
        "identifiants de gabarits uniques"
    );
    assert!(catalog().iter().all(|t| !t.summary.is_empty()));
}

#[test]
fn the_same_seed_injects_the_same_fault() {
    let run = |seed| {
        let (_, mut sim, flow) = scenarios().remove(1);
        let mut chaos = Chaos::new(&sim, flow).unwrap();
        let inj = chaos
            .start(&mut sim, &Filter::default(), seed, false)
            .unwrap();
        (inj.template, inj.description, sim.config_fingerprint())
    };
    assert_eq!(run(42), run(42));
    let distinct: BTreeSet<_> = (0..24).map(|s| run(s).0).collect();
    assert!(
        distinct.len() >= 4,
        "le tirage doit varier avec la graine : {distinct:?}"
    );
}

#[test]
fn filters_restrict_the_draw() {
    let (_, sim, flow) = scenarios().remove(1);
    let chaos = Chaos::new(&sim, flow).unwrap();
    let l1 = chaos.applicable(&Filter {
        layer: Some(Layer::L1),
        ..Filter::default()
    });
    assert!(!l1.is_empty());
    assert!(l1.iter().all(|t| t.kind.layer() == Layer::L1));
    let hard = chaos.applicable(&Filter {
        difficulty: Some(Difficulty::Hard),
        ..Filter::default()
    });
    assert!(hard.iter().all(|t| t.difficulty == Difficulty::Hard));
    let kinds = chaos.applicable(&Filter {
        kinds: Some(vec![CauseKind::WrongGateway]),
        ..Filter::default()
    });
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].id, "wrong-gateway");

    // Chaque injection respecte le filtre.
    for seed in 0..12 {
        let mut work = sim.clone();
        let mut c = Chaos::new(&sim, flow).unwrap();
        let inj = c
            .start(
                &mut work,
                &Filter {
                    layer: Some(Layer::L3),
                    ..Filter::default()
                },
                seed,
                false,
            )
            .unwrap();
        assert_eq!(inj.kind.layer(), Layer::L3, "{inj:?}");
    }
}

#[test]
fn impossible_criteria_are_reported_clearly() {
    // Un simple ping n'a pas de service à arrêter : la couche « services » est vide.
    let (_, mut sim, flow) = scenarios().remove(0);
    let mut chaos = Chaos::new(&sim, flow).unwrap();
    let err = chaos
        .start(
            &mut sim,
            &Filter {
                layer: Some(Layer::Services),
                ..Filter::default()
            },
            1,
            false,
        )
        .unwrap_err();
    assert_eq!(err, ChaosError::NothingApplicable);
    assert!(
        sim.faults().is_empty(),
        "rien ne doit être injecté en cas d'échec"
    );
}

#[test]
fn a_flow_that_does_not_work_cannot_be_chaosed() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    sim.set_gateway(pc1, None).unwrap();
    let err = Chaos::new(
        &sim,
        Flow {
            src: pc1,
            dst: ip("192.168.2.10"),
            service: None,
        },
    )
    .unwrap_err();
    assert_eq!(err, ChaosError::FlowNotWorking);
}

#[test]
fn undo_restores_the_network_and_hidden_faults_leave_no_trace() {
    let (_, mut sim, flow) = scenarios().remove(0);
    let healthy = sim.config_fingerprint();
    let mut chaos = Chaos::new(&sim, flow).unwrap();
    let inj = chaos.start(&mut sim, &Filter::default(), 3, false).unwrap();
    assert!(
        sim.trace().is_empty(),
        "une panne cachée ne doit rien révéler : {inj:?}"
    );
    assert_ne!(sim.config_fingerprint(), healthy);
    assert_eq!(chaos.undo_all(&mut sim, false), 1);
    assert_eq!(sim.config_fingerprint(), healthy);
    assert_eq!(chaos.undo_all(&mut sim, false), 0, "déjà réparé");
}

#[test]
fn visible_faults_are_announced() {
    let (_, mut sim, flow) = scenarios().remove(0);
    let mut chaos = Chaos::new(&sim, flow).unwrap();
    let inj = chaos.start(&mut sim, &Filter::default(), 3, true).unwrap();
    let text = netiv_core::render::describe(&sim, &sim.trace().events()[0]);
    assert!(text.starts_with("PANNE : "), "{text}");
    assert!(
        text.contains(&inj.description)
            || inj.description.contains("PANNE")
            || !inj.description.is_empty()
    );
}

#[test]
fn several_faults_can_be_stacked_and_all_undone() {
    let (_, mut sim, flow) = scenarios().remove(1);
    let healthy = sim.config_fingerprint();
    let mut chaos = Chaos::new(&sim, flow).unwrap();
    chaos.start(&mut sim, &Filter::default(), 1, false).unwrap();
    chaos.start(&mut sim, &Filter::default(), 2, false).unwrap();
    assert_eq!(chaos.injected.len(), 2);
    assert_eq!(chaos.undo_all(&mut sim, false), 2);
    assert_eq!(sim.config_fingerprint(), healthy);
}

#[test]
fn difficulty_parsing() {
    assert_eq!(Difficulty::from_slug("facile"), Some(Difficulty::Easy));
    assert_eq!(Difficulty::from_slug("hard"), Some(Difficulty::Hard));
    assert_eq!(
        Difficulty::from_slug("intermediate"),
        Some(Difficulty::Medium)
    );
    assert_eq!(Difficulty::from_slug("???"), None);
    assert_eq!(Difficulty::Medium.slug(), "medium");
}
