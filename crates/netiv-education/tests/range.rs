#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Les scénarios de cyber range : incident réel, enquête possible, correction vérifiable.

use netiv_core::Simulation;
use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_education::builtin;
use netiv_education::range::{RangeCheck, RangeRun, RangeScenario, parse_behavior};
use netiv_education::run::AnswerOutcome;
use netiv_security::forensics::Behavior;
use netiv_security::{Capture, analyze, analyze_capture};

fn start(id: &str) -> (RangeRun, Simulation) {
    RangeRun::start(builtin::range(id).unwrap().clone(), 1_000).unwrap()
}

/// La bonne correction : supprimer la règle fautive (première de la liste).
fn fix(sim: &mut Simulation) {
    let fw = sim.device_id("FW1").unwrap();
    sim.acl_remove_rule(fw, 0).unwrap();
}

#[test]
fn all_shipped_scenarios_parse_and_are_consistent() {
    assert_eq!(builtin::ranges().len(), builtin::range_sources().len());
    assert_eq!(builtin::ranges().len(), 3);
    for r in builtin::ranges() {
        let problems = r.problems();
        assert!(problems.is_empty(), "{} : {problems:?}", r.id);
    }
}

#[test]
fn the_incident_is_already_in_the_history_when_the_student_arrives() {
    let (_, sim) = start("range-01");
    assert!(
        sim.trace().len() > 100,
        "l'historique doit contenir l'incident"
    );
    let cap = Capture::from_sim(&sim, "cyber");
    let a = analyze_capture(&cap);
    let scan = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::PortScan)
        .unwrap();
    assert_eq!(scan.source, "WEB01");
    // Les pannes injectées ne sont annoncées nulle part dans le journal.
    assert!(!sim.trace().events().iter().any(|e| matches!(
        e.kind,
        netiv_core::TraceKind::FaultApplied { .. } | netiv_core::TraceKind::ConfigChanged { .. }
    )));
}

#[test]
fn the_dmz_to_lan_scenarios_are_also_revealed_by_the_security_analyzer() {
    // Le comportement (balayage, rebond) se lit dans la forensique ; la brèche de
    // conception, elle, se lit dans l'analyseur.
    for id in ["range-01", "range-02"] {
        let (_, sim) = start(id);
        let report = analyze(&sim);
        assert!(report.has("SEC-006"), "{id} : {}", report.render());
    }
    // Une fois la règle fautive retirée, la brèche disparaît.
    let (_, mut sim) = start("range-01");
    fix(&mut sim);
    assert!(!analyze(&sim).has("SEC-006"));
}

#[test]
fn a_correct_fix_then_the_right_culprit_scores_full_marks() {
    let (mut run, mut sim) = start("range-01");
    fix(&mut sim);
    assert_eq!(run.check(&sim, 1_100).unwrap(), RangeCheck::NeedsAnswer);
    let AnswerOutcome::Correct(Some(score)) = run.answer("WEB01", "port-scan", 1_110).unwrap()
    else {
        panic!("le scénario devrait être terminé");
    };
    assert_eq!(
        (score.diagnosis, score.fix, score.time, score.total),
        (40, 40, 20, 100)
    );
    assert_eq!(score.xp, 150, "difficulté moyenne : ×1,5");
    assert!(run.is_finished());
}

#[test]
fn the_culprit_can_be_named_first() {
    let (mut run, mut sim) = start("range-02");
    assert_eq!(
        run.answer("WEB02", "lateral-movement", 1_010).unwrap(),
        AnswerOutcome::Correct(None)
    );
    fix(&mut sim);
    let RangeCheck::Finished(s) = run.check(&sim, 1_020).unwrap() else {
        panic!()
    };
    assert_eq!(s.total, 100);
    assert_eq!(s.xp, 200, "difficile : ×2");
}

#[test]
fn an_incomplete_fix_names_the_flows_still_open() {
    let (mut run, mut sim) = start("range-01");
    // On ferme seulement le SSH : le SMB et le ping restent ouverts.
    let fw = sim.device_id("FW1").unwrap();
    let mut deny = AclRule::new(AclAction::Deny, RuleProto::Tcp);
    deny.port_from = Some(22);
    deny.port_to = Some(22);
    sim.acl_insert_rule(fw, 0, deny).unwrap();
    let RangeCheck::Failing { still_open, broken } = run.check(&sim, 1_050).unwrap() else {
        panic!("la correction est incomplète");
    };
    assert_eq!(still_open.len(), 2, "{still_open:?}");
    assert!(still_open.iter().any(|f| f.contains("tcp/445")));
    assert!(still_open.iter().any(|f| f.contains("(ping)")));
    assert!(broken.is_empty());
}

#[test]
fn a_heavy_handed_fix_that_breaks_legitimate_traffic_is_refused() {
    let (mut run, mut sim) = start("range-01");
    // « Tout couper » : plus aucune règle, donc plus de web ni de partage de fichiers.
    let fw = sim.device_id("FW1").unwrap();
    while sim.acl_remove_rule(fw, 0).is_ok() {}
    let RangeCheck::Failing { still_open, broken } = run.check(&sim, 1_050).unwrap() else {
        panic!("supprimer toutes les règles casse le trafic légitime");
    };
    assert!(
        still_open.is_empty(),
        "l'attaque est bien bloquée : {still_open:?}"
    );
    assert_eq!(
        broken.len(),
        4,
        "tous les flux légitimes sont coupés : {broken:?}"
    );
    assert!(!run.is_finished());
}

#[test]
fn blocking_by_a_deny_rule_instead_of_deleting_also_works() {
    let (mut run, mut sim) = start("range-03");
    let fw = sim.device_id("FW1").unwrap();
    let mut deny = AclRule::new(AclAction::Deny, RuleProto::Any);
    deny.src = Some("10.9.0.0/24".parse().unwrap());
    deny.dst = Some("203.0.113.0/24".parse().unwrap());
    deny.in_iface = Some(sim.iface_id(fw, "eth1").unwrap());
    sim.acl_insert_rule(fw, 0, deny).unwrap();
    assert_eq!(run.check(&sim, 1_020).unwrap(), RangeCheck::NeedsAnswer);
}

#[test]
fn wrong_culprits_cost_points_and_give_partial_feedback() {
    let (mut run, mut sim) = start("range-01");
    assert_eq!(
        run.answer("WEB02", "port-scan", 1_001).unwrap(),
        AnswerOutcome::Wrong {
            device_ok: false,
            cause_ok: true
        }
    );
    assert_eq!(
        run.answer("WEB01", "beaconing", 1_002).unwrap(),
        AnswerOutcome::Wrong {
            device_ok: true,
            cause_ok: false
        }
    );
    run.answer("WEB01", "balayage de ports", 1_003).unwrap();
    fix(&mut sim);
    let RangeCheck::Finished(s) = run.check(&sim, 1_004).unwrap() else {
        panic!()
    };
    assert_eq!(s.diagnosis, 20);
}

#[test]
fn hints_walk_from_method_to_solution() {
    let (mut run, _) = start("range-01");
    let hints: Vec<String> = (0..4).map(|_| run.next_hint().unwrap()).collect();
    assert!(hints[0].contains("forensic analyze"), "{}", hints[0]);
    assert!(hints[2].contains("show acl FW1"));
    assert!(hints[3].starts_with("Solution") && hints[3].contains("acl del FW1 1"));
    assert!(
        !hints[..3].iter().any(|h| h.contains("acl del FW1 1")),
        "pas de fuite"
    );
    assert_eq!(run.hints_used(), 4);
}

#[test]
fn giving_up_and_finishing_early() {
    let (mut run, mut sim) = start("range-01");
    assert!(run.finish(1_010).is_err());
    fix(&mut sim);
    run.check(&sim, 1_020).unwrap();
    let s = run.finish(1_030).unwrap();
    assert_eq!((s.diagnosis, s.fix), (0, 40));
    let (mut run, _) = start("range-02");
    run.give_up().unwrap();
    assert!(run.is_finished() && run.check(&Simulation::new(1), 5).is_err());
}

#[test]
fn behaviors_are_understood_by_slug_or_label() {
    assert_eq!(parse_behavior("port-scan"), Some(Behavior::PortScan));
    assert_eq!(
        parse_behavior("Balayage de ports"),
        Some(Behavior::PortScan)
    );
    assert_eq!(parse_behavior("rebond"), Some(Behavior::ZoneViolation));
    assert_eq!(parse_behavior("Balise"), Some(Behavior::Beaconing));
    assert_eq!(
        parse_behavior("usurpation_arp"),
        Some(Behavior::ArpConflict)
    );
    assert_eq!(parse_behavior("???"), None);
}

#[test]
fn inconsistent_scenarios_are_rejected_with_a_reason() {
    let base = builtin::range("range-01").unwrap().clone();
    let problems = |f: &dyn Fn(&mut RangeScenario)| {
        let mut r = base.clone();
        f(&mut r);
        r.problems()
    };
    let p = problems(&|r| r.faults.clear());
    assert!(
        p.iter()
            .any(|x| x.contains("aucun flux « à bloquer » ne passe")),
        "{p:?}"
    );
    let p = problems(&|r| r.culprit.device = "PC01".into());
    assert!(p.iter().any(|x| x.contains("ne détecte pas")), "{p:?}");
    let p = problems(&|r| r.culprit.behavior = "nonsense".into());
    assert!(
        p.iter().any(|x| x.contains("comportement inconnu")),
        "{p:?}"
    );
    let p = problems(&|r| r.topology_ref = "nope".into());
    assert!(p.iter().any(|x| x.contains("réseau inconnu")), "{p:?}");
    let p = problems(&|r| r.must_work.clear());
    assert!(p.iter().any(|x| x.contains("au moins un flux")), "{p:?}");
    let p = problems(&|r| r.traffic[0].proto = "icmp".into());
    assert!(p.iter().any(|x| x.contains("protocole inconnu")), "{p:?}");
    let p = problems(&|r| r.must_work[0].to = "1.1.1.1".into());
    assert!(
        p.iter().any(|x| x.contains("KO même avant correction")),
        "{p:?}"
    );
}

#[test]
fn the_catalog_counts_cyber_ranges_under_security() {
    let cat = builtin::catalog();
    assert_eq!(
        cat.len(),
        builtin::challenges().len() + builtin::ranges().len()
    );
    let security_challenges = builtin::challenges()
        .iter()
        .filter(|c| c.topic == "security")
        .count();
    assert_eq!(
        cat.iter().filter(|(_, t)| t == "security").count(),
        security_challenges + builtin::ranges().len()
    );
}

#[test]
fn replaying_a_scenario_is_deterministic() {
    let (_, a) = start("range-03");
    let (_, b) = start("range-03");
    assert_eq!(a.trace().events(), b.trace().events());
}
