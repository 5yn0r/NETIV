#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 4 de bout en bout, par la CLI : casser, diagnostiquer, réparer.

use std::collections::BTreeSet;
use std::path::PathBuf;

use netiv_cli::Session;

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn session(name: &str, setup: &[&str]) -> Session {
    let mut s = Session::new();
    s.load_yaml(&example(name), &mut Vec::new()).unwrap();
    for cmd in setup {
        s.exec(cmd, &mut Vec::new()).unwrap();
    }
    s
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

struct Case {
    name: &'static str,
    file: &'static str,
    setup: &'static [&'static str],
    flow: &'static str,
}

const CASES: [Case; 4] = [
    Case {
        name: "mvp/ping",
        file: "mvp.yaml",
        setup: &[],
        flow: "PC1 192.168.2.10",
    },
    Case {
        name: "mvp/web",
        file: "mvp.yaml",
        setup: &["service add SERVER tcp 80 http"],
        flow: "PC1 192.168.2.10 tcp 80",
    },
    Case {
        name: "dmz/web",
        file: "dmz.yaml",
        setup: &[],
        flow: "PC1 10.9.0.10 tcp 80",
    },
    Case {
        name: "dmz/ping",
        file: "dmz.yaml",
        setup: &[],
        flow: "PC1 10.9.0.10",
    },
];

#[test]
fn every_suggested_fix_really_repairs_the_network() {
    let mut templates: BTreeSet<String> = BTreeSet::new();
    let mut repaired = 0;
    for case in &CASES {
        for seed in 0..20u64 {
            let mut s = session(case.file, case.setup);
            run(&mut s, &format!("chaos start {} --seed {seed}", case.flow));
            let reveal = run(&mut s, "chaos reveal");
            let template = reveal
                .lines()
                .find(|l| l.trim_start().starts_with("1."))
                .and_then(|l| l.split_whitespace().nth(2))
                .unwrap_or_default()
                .to_string();
            let report = run(&mut s, "diagnose");
            let fix = report
                .lines()
                .find_map(|l| l.strip_prefix("Pour réparer : "))
                .unwrap_or_else(|| {
                    panic!("[{}] graine {seed} ({template}) : aucun correctif proposé\n{reveal}\n{report}", case.name)
                })
                .to_string();
            assert!(
                !fix.contains('<'),
                "[{}] graine {seed} ({template}) : correctif incomplet « {fix} »\n{report}",
                case.name
            );
            run(&mut s, &fix);
            let after = run(&mut s, "diagnose");
            assert!(
                after.contains("Résultat : tout répond"),
                "[{}] graine {seed} ({template}) : « {fix} » n'a pas réparé\n{reveal}\n{report}\n{after}",
                case.name
            );
            templates.insert(template);
            repaired += 1;
        }
    }
    assert_eq!(repaired, 80);
    assert!(
        templates.len() >= 12,
        "le tirage doit couvrir presque tout le catalogue : {templates:?}"
    );
}

#[test]
fn chaos_start_hides_the_fault_but_reveal_shows_it() {
    let mut s = session("mvp.yaml", &[]);
    let out = run(&mut s, "chaos start PC1 192.168.2.10 --seed 5");
    assert!(out.contains("Une panne a été injectée"), "{out}");
    assert!(
        !out.contains("passerelle") && !out.contains("câble"),
        "rien ne doit fuiter : {out}"
    );
    // Le journal ne dit rien non plus.
    assert!(run(&mut s, "log all").contains("(trace vide)"));
    let reveal = run(&mut s, "chaos reveal");
    assert!(reveal.contains("Pannes injectées :"), "{reveal}");
    assert_eq!(run(&mut s, "chaos undo").trim(), "1 panne(s) réparée(s).");
    assert!(run(&mut s, "ping PC1 192.168.2.10").contains("0 % de perte"));
    assert!(run(&mut s, "chaos reveal").contains("(réparée)"));
}

#[test]
fn visible_chaos_is_announced_in_the_trace() {
    let mut s = session("mvp.yaml", &[]);
    let out = run(&mut s, "chaos start PC1 192.168.2.10 --seed 5 --visible");
    assert!(out.contains("Panne injectée :"), "{out}");
    assert!(run(&mut s, "log all").contains("PANNE :"));
}

#[test]
fn chaos_filters_by_layer_and_difficulty() {
    for seed in 0..8 {
        let mut s = session("mvp.yaml", &[]);
        run(
            &mut s,
            &format!("chaos start PC1 192.168.2.10 --layer l1 --seed {seed}"),
        );
        let reveal = run(&mut s, "chaos reveal");
        assert!(
            ["link-down", "interface-down", "packet-loss", "high-latency"]
                .iter()
                .any(|t| reveal.contains(t)),
            "{reveal}"
        );
    }
    let mut s = session("mvp.yaml", &[]);
    run(
        &mut s,
        "chaos start PC1 192.168.2.10 --difficulty hard --seed 1",
    );
    assert!(run(&mut s, "chaos reveal").contains("[hard]"));
}

#[test]
fn diagnose_compares_with_a_saved_baseline() {
    let mut s = session("mvp.yaml", &[]);
    run(&mut s, "gateway PC1 192.168.1.254");
    let without = run(&mut s, "diagnose PC1 192.168.2.10");
    assert!(
        without.contains("mauvaise passerelle par défaut"),
        "{without}"
    );
    assert!(
        !without.contains("Attendu"),
        "sans référence, on ne sait pas ce qui était attendu\n{without}"
    );

    // Même panne, mais cette fois on avait mémorisé l'état sain.
    let mut s = session("mvp.yaml", &["baseline save", "gateway PC1 192.168.1.254"]);
    let with = run(&mut s, "diagnose PC1 192.168.2.10");
    assert!(with.contains("Attendu  : 192.168.1.1"), "{with}");
    assert!(
        with.contains("Pour réparer : gateway PC1 192.168.1.1"),
        "{with}"
    );
    run(&mut s, "gateway PC1 192.168.1.1");
    assert!(run(&mut s, "diagnose PC1 192.168.2.10").contains("tout répond"));
    run(&mut s, "baseline clear");
}

#[test]
fn hints_go_from_vague_to_the_solution() {
    let mut s = session("mvp.yaml", &["baseline save", "gateway PC1 192.168.1.254"]);
    assert!(fail(&mut s, "hint").contains("lance d'abord « diagnose"));
    run(&mut s, "diagnose PC1 192.168.2.10");
    let h: Vec<String> = (0..4).map(|_| run(&mut s, "hint")).collect();
    assert!(h[0].starts_with("Indice 1"), "{}", h[0]);
    assert!(h[1].starts_with("Indice 2"));
    assert!(h[2].starts_with("Indice 3"));
    assert!(h[3].starts_with("Solution"), "{}", h[3]);
    assert!(h[3].contains("gateway PC1 192.168.1.1"));
    for early in &h[..3] {
        assert!(
            !early.contains("gateway PC1 192.168.1.1"),
            "fuite : {early}"
        );
    }
    // Au-delà, on répète la solution.
    assert!(run(&mut s, "hint").starts_with("Solution"));
    // Un nouveau diagnostic repart au premier indice.
    run(&mut s, "diagnose PC1 192.168.2.10");
    assert!(run(&mut s, "hint").starts_with("Indice 1"));
}

#[test]
fn hint_on_a_chaos_session_works_without_a_prior_diagnose() {
    let mut s = session("mvp.yaml", &[]);
    run(&mut s, "chaos start PC1 192.168.2.10 --seed 9");
    assert!(run(&mut s, "hint").starts_with("Indice 1"));
}

#[test]
fn chaos_usage_errors() {
    let mut s = session("mvp.yaml", &[]);
    assert!(fail(&mut s, "chaos undo").contains("aucune panne"));
    assert!(fail(&mut s, "chaos reveal").contains("aucune panne"));
    assert!(fail(&mut s, "chaos start").contains("attendu"));
    assert!(fail(&mut s, "chaos start PC1 1.1.1.1 --layer l9").contains("couche inconnue"));
    assert!(
        fail(&mut s, "chaos start PC1 192.168.2.10 --difficulty ??")
            .contains("difficulté inconnue")
    );
    assert!(fail(&mut s, "chaos start PC1 192.168.2.10 --seed x").contains("graine invalide"));
    assert!(fail(&mut s, "chaos start PC1 192.168.2.10 --layer").contains("attend une valeur"));
    assert!(
        fail(
            &mut s,
            "chaos start PC1 192.168.2.10 --layer services --seed 1"
        )
        .contains("aucune panne du catalogue")
    );
    assert!(fail(&mut s, "diagnose").contains("usage"));
    assert!(fail(&mut s, "chaos explode").contains("usage"));
    // Flux qui ne fonctionne pas déjà : impossible de choisir une panne pertinente.
    run(&mut s, "gateway PC1 none");
    assert!(fail(&mut s, "chaos start PC1 192.168.2.10").contains("ne fonctionne pas"));
}

#[test]
fn a_second_chaos_start_must_watch_the_same_flow() {
    let mut s = session("mvp.yaml", &[]);
    run(&mut s, "chaos start PC1 192.168.2.10 --seed 1");
    assert!(fail(&mut s, "chaos start SERVER 192.168.1.10").contains("autre flux"));
    run(&mut s, "chaos start PC1 192.168.2.10 --seed 2");
    assert_eq!(run(&mut s, "chaos reveal").matches("[").count(), 2);
    assert_eq!(run(&mut s, "chaos undo").trim(), "2 panne(s) réparée(s).");
}

#[test]
fn chaos_list_shows_the_catalog() {
    let mut s = session("mvp.yaml", &[]);
    let out = run(&mut s, "chaos list");
    assert!(
        out.contains("wrong-gateway") && out.contains("routing-loop") && out.contains("acl-deny")
    );
    assert_eq!(out.lines().count(), 1 + netiv_chaos::catalog().len());
}

#[test]
fn several_commands_can_be_chained_with_semicolons() {
    let mut s = session("mvp.yaml", &[]);
    let out = run(
        &mut s,
        "gateway PC1 none ; gateway PC1 192.168.1.1 ; ping PC1 192.168.2.10",
    );
    assert!(out.contains("0 % de perte"), "{out}");
    // La chaîne s'arrête à la première erreur.
    let e = fail(
        &mut s,
        "gateway PC1 none ; frobnicate ; gateway PC1 192.168.1.1",
    );
    assert!(e.contains("commande inconnue"));
    assert!(run(&mut s, "show route PC1").contains("connectée"));
    assert!(
        !run(&mut s, "show route PC1").contains("par défaut"),
        "la 3e commande ne doit pas s'être exécutée"
    );
}

#[test]
fn small_config_commands() {
    let mut s = session("mvp.yaml", &[]);
    assert!(run(&mut s, "forwarding R1 off").contains("transfert de paquets désactivé"));
    assert!(run(&mut s, "ping PC1 192.168.2.10").contains("100 % de perte"));
    run(&mut s, "forwarding R1 on");
    assert!(run(&mut s, "latency R1 eth1 150").contains("latence 150.000 ms"));
    assert!(
        run(&mut s, "arp PC1 static 192.168.1.1 02:00:00:00:be:ef").contains("entrée ARP statique")
    );
    assert!(run(&mut s, "ping PC1 192.168.2.10").contains("100 % de perte"));
    assert!(run(&mut s, "arp PC1 del 192.168.1.1").contains("supprimée"));
    assert!(fail(&mut s, "forwarding R1 maybe").contains("on ou off"));
    assert!(fail(&mut s, "latency R1 eth1 vite").contains("durée invalide"));
    assert!(fail(&mut s, "arp PC1 static 1.1.1.1 zz").contains("MAC invalide"));
    assert!(fail(&mut s, "forwarding SW1 on").contains("switch"));
}

#[test]
fn chaos_sessions_are_deterministic() {
    let script = |seed: u64| {
        let mut s = session("dmz.yaml", &[]);
        let mut all = String::new();
        for line in [
            format!("chaos start PC1 10.9.0.10 tcp 80 --seed {seed}"),
            "diagnose".to_string(),
            "hint".to_string(),
            "chaos reveal".to_string(),
        ] {
            all += &run(&mut s, &line);
        }
        all
    };
    assert_eq!(script(11), script(11));
}
