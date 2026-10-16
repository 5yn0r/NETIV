#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 6 de bout en bout : analyse, forensique, cyber range.

use std::path::PathBuf;

use netiv_cli::{ManualClock, Session};
use netiv_education::builtin;

fn example(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

fn session() -> (Session, ManualClock) {
    let clock = ManualClock::new(50_000);
    let mut s = Session::new();
    s.set_manual_clock(&clock);
    (s, clock)
}

fn run(s: &mut Session, line: &str) -> String {
    let mut out = Vec::new();
    s.exec(line, &mut out)
        .unwrap_or_else(|e| panic!("« {line} » a échoué : {e}"));
    String::from_utf8(out).unwrap()
}

fn fail(s: &mut Session, line: &str) -> String {
    s.exec(line, &mut Vec::new()).unwrap_err().to_string()
}

fn cyber() -> Session {
    let (mut s, _) = session();
    s.load_yaml(&example("cyber.yaml"), &mut Vec::new())
        .unwrap();
    s
}

#[test]
fn security_analyze_reports_the_reference_architecture_as_clean_apart_from_redundancy() {
    let mut s = cyber();
    let out = run(&mut s, "security analyze");
    assert!(
        out.starts_with(
            "ANALYSE DE SÉCURITÉ — 0 critique(s), 1 moyen(s), 0 faible(s) — score 90/100"
        ),
        "{out}"
    );
    assert!(out.contains("[MEDIUM] SEC-011"), "{out}");
    assert!(fail(&mut Session::new(), "security analyze").contains("aucune topologie chargée"));
    assert!(fail(&mut s, "security").contains("usage"));
}

#[test]
fn breaking_the_policy_makes_the_analyzer_complain_and_fixing_it_clears_the_alert() {
    let mut s = cyber();
    run(
        &mut s,
        "acl insert FW1 1 allow src 10.9.0.0/24 dst 192.168.10.0/24 in eth1",
    );
    let bad = run(&mut s, "security analyze");
    assert!(
        bad.contains("[HIGH] SEC-006") && bad.contains("WEB01"),
        "{bad}"
    );
    assert!(
        bad.contains("→ Ajouter des règles « deny » de la DMZ vers le LAN"),
        "{bad}"
    );
    run(&mut s, "acl del FW1 1");
    assert!(!run(&mut s, "security analyze").contains("SEC-006"));
}

#[test]
fn a_capture_can_be_saved_then_analyzed_from_a_file_or_live() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nuit.nlv");
    let mut s = cyber();
    run(
        &mut s,
        "acl insert FW1 1 allow src 10.9.0.0/24 dst 192.168.0.0/16 in eth1",
    );
    for port in [21, 22, 23, 25, 80, 135] {
        run(&mut s, &format!("curl WEB01 192.168.20.20 {port}"));
    }
    let saved = run(&mut s, &format!("capture save {}", file.display()));
    assert!(
        saved.contains("Capture enregistrée") && saved.contains("événements"),
        "{saved}"
    );

    let from_file = run(&mut s, &format!("forensic analyze {}", file.display()));
    assert!(
        from_file.contains("ANALYSE FORENSIQUE — réseau « cyber »"),
        "{from_file}"
    );
    assert!(
        from_file.contains("a sondé 6 ports de FILE01"),
        "{from_file}"
    );
    assert!(from_file.contains("WEB01 (dmz) a initié"), "{from_file}");

    // Une autre session, sans aucune topologie, lit la même capture à l'identique.
    let mut other = Session::new();
    assert_eq!(
        run(&mut other, &format!("forensic analyze {}", file.display())),
        from_file
    );

    assert!(run(&mut s, "forensic analyze live").contains("a sondé 6 ports"));
    let packets = run(
        &mut s,
        &format!("forensic analyze {} --packets", file.display()),
    );
    assert!(packets.contains("WEB01/eth0  TCP [SYN]"), "{packets}");
    assert!(fail(&mut s, "forensic analyze").contains("usage"));
    assert!(fail(&mut s, "forensic analyze /nope.nlv").contains("capture"));
    assert!(fail(&mut s, "capture save").contains("usage"));
}

#[test]
fn every_range_scenario_is_solved_by_its_documented_commands_for_a_perfect_score() {
    for r in builtin::ranges() {
        let (mut s, clock) = session();
        let start = run(&mut s, &format!("range start {}", r.id));
        assert!(
            start.contains("CYBER RANGE") && start.contains(&r.title),
            "{start}"
        );

        // La brèche est ouverte au départ.
        let before = run(&mut s, "range check");
        assert!(
            before.contains("Pas encore") && before.contains("ce flux illégitime passe toujours"),
            "[{}] {before}",
            r.id
        );
        // Les outils qui donnent la réponse sont verrouillés ; les outils d'enquête, non.
        assert!(
            fail(&mut s, "diagnose PC01 10.9.0.10").contains("indisponible pendant un challenge")
        );
        run(&mut s, "forensic analyze live");
        run(&mut s, "security analyze");
        run(&mut s, "show acl FW1");

        clock.set(50_090);
        run(&mut s, &r.solution.commands.join(" ; "));
        let checked = run(&mut s, "range check");
        assert!(checked.contains("Brèche fermée"), "[{}] {checked}", r.id);
        let end = run(
            &mut s,
            &format!("answer {} {}", r.culprit.device, r.culprit.behavior),
        );
        assert!(
            end.contains("SCÉNARIO TERMINÉ") && end.contains("Score : 100/100"),
            "[{}] {end}",
            r.id
        );
        assert_eq!(s.profile().completed[&r.id].score, 100);
    }
}

#[test]
fn investigating_with_the_forensic_tool_finds_the_culprit_the_scenario_expects() {
    for r in builtin::ranges() {
        let (mut s, _) = session();
        run(&mut s, &format!("range start {}", r.id));
        let report = run(&mut s, "forensic analyze live");
        assert!(report.contains("ANOMALIES"), "{report}");
        assert!(
            report.contains(&r.culprit.device),
            "[{}] le coupable doit apparaître :\n{report}",
            r.id
        );
    }
}

#[test]
fn a_clumsy_fix_that_cuts_legitimate_traffic_is_refused_with_the_broken_flows() {
    let (mut s, _) = session();
    run(&mut s, "range start range-01");
    // Maladroit : on supprime toutes les règles du pare-feu.
    for _ in 0..8 {
        let _ = s.exec("acl del FW1 1", &mut Vec::new());
    }
    let out = run(&mut s, "range check");
    assert!(out.contains("ce flux légitime est cassé"), "{out}");
    assert!(
        out.contains("sans couper le trafic légitime") || out.contains("SANS couper"),
        "{out}"
    );
    assert!(
        !out.contains("ce flux illégitime passe toujours"),
        "l'attaque est bien bloquée : {out}"
    );
}

#[test]
fn range_hints_wrong_answers_giveup_and_errors() {
    let (mut s, _) = session();
    assert!(fail(&mut s, "range check").contains("aucun scénario en cours"));
    assert!(fail(&mut s, "range start nope").contains("scénario inconnu"));
    assert!(run(&mut s, "range list").contains("range-03"));

    run(&mut s, "range start range-02");
    assert!(run(&mut s, "hint").starts_with("Indice 1"));
    assert!(run(&mut s, "hint").contains("rôle"));
    assert!(
        run(&mut s, "answer WEB01 lateral-movement")
            .contains("Le bon comportement, mais pas cette machine")
    );
    assert!(
        run(&mut s, "answer WEB02 beaconing")
            .contains("La bonne machine, mais pas ce comportement")
    );
    assert!(fail(&mut s, "answer WEB02 nimportequoi").contains("comportement inconnu"));
    assert!(run(&mut s, "range show").contains("Indices utilisés : 2"));

    let out = run(&mut s, "range giveup");
    assert!(
        out.contains("Coupable : WEB02") && out.contains("acl del FW1 1"),
        "{out}"
    );
    assert_eq!(s.profile().completed_count(), 0);
    // Terminé : les outils réservés sont de nouveau disponibles.
    assert!(run(&mut s, "diagnose PC01 10.9.0.10").contains("DIAGNOSTIC"));
}

#[test]
fn naming_the_culprit_first_and_a_late_fix_cost_time_points() {
    let (mut s, clock) = session();
    run(&mut s, "range start range-01"); // 30 min
    assert!(run(&mut s, "answer WEB01 port-scan").contains("Coupable identifié ✓ (+40)"));
    clock.set(50_000 + 1_800); // pile à la limite : plus aucun point de temps
    run(&mut s, "acl del FW1 1");
    let end = run(&mut s, "range check");
    assert!(end.contains("Score : 80/100"), "{end}");
    assert!(end.contains("temps 0"), "{end}");
}

#[test]
fn security_scenarios_unlock_the_network_defender_badge_and_count_in_the_dashboard() {
    let (mut s, _) = session();
    for r in builtin::ranges().iter().take(2) {
        run(&mut s, &format!("range start {}", r.id));
        run(&mut s, &r.solution.commands.join(" ; "));
        run(&mut s, "range check");
        run(
            &mut s,
            &format!("answer {} {}", r.culprit.device, r.culprit.behavior),
        );
    }
    assert!(
        s.profile().badges.contains("network-defender"),
        "{:?}",
        s.profile().badges
    );
    let dash = run(&mut s, "dashboard");
    assert!(
        dash.contains("Sécurité") && dash.contains("(2/6)"),
        "{dash}"
    );
    assert!(dash.contains("Network Defender"), "{dash}");
}

#[test]
fn range_sessions_are_deterministic() {
    let play = || {
        let (mut s, clock) = session();
        let mut all = String::new();
        for line in [
            "range start range-03",
            "forensic analyze live",
            "security analyze",
            "show acl FW1",
            "acl del FW1 1",
            "range check",
            "answer WEB02 beaconing",
        ] {
            clock.set(clock.get() + 11);
            all += &run(&mut s, line);
        }
        all
    };
    assert_eq!(play(), play());
}

#[test]
fn loading_another_network_ends_the_scenario() {
    let (mut s, _) = session();
    run(&mut s, "range start range-01");
    assert!(fail(&mut s, "diagnose PC01 10.9.0.10").contains("indisponible"));
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    assert!(fail(&mut s, "range check").contains("aucun scénario en cours"));
    run(&mut s, "baseline save");
}
