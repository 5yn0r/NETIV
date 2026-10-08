#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 7 de bout en bout : génération, what-if, santé, documentation, tuteur.

use std::path::PathBuf;

use netiv_chaos::Difficulty;
use netiv_cli::{ManualClock, Session};
use netiv_education::generate::{GenerateSpec, TOPICS, generate_challenge};

fn example(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

fn session() -> (Session, ManualClock) {
    let clock = ManualClock::new(70_000);
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

fn mvp() -> Session {
    let (mut s, _) = session();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    s
}

// ------------------------------------------------------------------ génération de réseaux

#[test]
fn a_generated_network_is_loaded_and_usable_right_away() {
    let (mut s, _) = session();
    let out = run(
        &mut s,
        "topology generate --type campus --routers 5 --hosts 12 --seed 3",
    );
    assert!(out.contains("Réseau « campus-5r-12h » chargé"), "{out}");
    assert!(!out.contains('⚠'), "{out}");
    let devices = run(&mut s, "devices");
    assert!(
        devices.contains("R5") && devices.contains("PC12") && devices.contains("SRV1"),
        "{devices}"
    );
    // PC1 joint le serveur à travers le cœur redondant.
    let ping = run(&mut s, "ping PC1 10.1.0.11");
    assert!(ping.contains("0 % de perte"), "{ping}");
    assert!(run(&mut s, "health").contains("GLOBAL SCORE"));
}

#[test]
fn a_generated_network_can_be_saved_and_reloaded() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("reseau.yaml");
    let (mut s, _) = session();
    let out = run(
        &mut s,
        &format!(
            "topology generate --type enterprise --routers 3 --hosts 6 --save {}",
            file.display()
        ),
    );
    assert!(out.contains("Topologie écrite"), "{out}");
    let (mut other, _) = session();
    let loaded = run(&mut other, &format!("load {}", file.display()));
    assert!(loaded.contains("enterprise-3r-6h"), "{loaded}");
}

#[test]
fn generator_errors_are_explained() {
    let (mut s, _) = session();
    assert!(fail(&mut s, "topology generate").contains("usage"));
    assert!(fail(&mut s, "topology generate --type nope").contains("type de réseau inconnu"));
    assert!(
        fail(&mut s, "topology generate --type campus --routers 2").contains("au moins 3 routeurs")
    );
    assert!(fail(&mut s, "topology generate --type campus --routers x").contains("invalide"));
    assert!(fail(&mut s, "topology generate --type campus --hosts").contains("attend une valeur"));
    assert!(fail(&mut s, "topology frobnicate").contains("usage"));
}

// ------------------------------------------------------------------ challenges générés

#[test]
fn every_generated_challenge_is_solved_by_its_own_commands() {
    let mut done = 0;
    for topic in TOPICS {
        for difficulty in [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard] {
            for seed in [1u64, 2] {
                let (mut s, clock) = session();
                let start = run(
                    &mut s,
                    &format!(
                        "challenge generate --topic {topic} --difficulty {} --seed {seed}",
                        difficulty.slug()
                    ),
                );
                assert!(start.contains("CHALLENGE gen-"), "{start}");
                // Le même tirage, par la bibliothèque, pour connaître la solution attendue.
                let c = generate_challenge(&GenerateSpec {
                    topic,
                    difficulty,
                    seed,
                })
                .unwrap();
                assert!(start.contains(&c.id), "{} : {start}", c.id);

                assert!(
                    run(&mut s, "challenge check").contains("Pas encore"),
                    "{} doit être cassé",
                    c.id
                );
                clock.set(clock.get() + 30);
                run(&mut s, &c.solution.commands.join(" ; "));
                let checked = run(&mut s, "challenge check");
                assert!(
                    checked.contains("Objectif atteint"),
                    "{} : « {} » n'a pas réparé\n{checked}",
                    c.id,
                    c.solution.commands.join(" ; ")
                );
                let end = run(
                    &mut s,
                    &format!("answer {} {}", c.solution.device, c.solution.cause),
                );
                assert!(end.contains("Score : 100/100"), "{} : {end}", c.id);
                done += 1;
            }
        }
    }
    assert_eq!(done, 6 * 3 * 2);
}

#[test]
fn a_generated_challenge_can_be_saved_and_played_elsewhere() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("defi.netiv");
    let (mut teacher, _) = session();
    let out = run(
        &mut teacher,
        &format!(
            "challenge generate --topic routing --difficulty hard --seed 4 --save {}",
            file.display()
        ),
    );
    assert!(out.contains("enregistré dans"), "{out}");

    let c = generate_challenge(&GenerateSpec {
        topic: "routing",
        difficulty: Difficulty::Hard,
        seed: 4,
    })
    .unwrap();
    let (mut student, _) = session();
    let start = run(&mut student, &format!("challenge start {}", file.display()));
    assert!(start.contains(&c.title), "{start}");
    run(&mut student, &c.solution.commands.join(" ; "));
    assert!(run(&mut student, "challenge check").contains("Objectif atteint"));
}

#[test]
fn challenge_generation_errors_are_explained() {
    let (mut s, _) = session();
    assert!(fail(&mut s, "challenge generate").contains("usage"));
    assert!(fail(&mut s, "challenge generate --topic cuisine").contains("thème inconnu"));
    assert!(
        fail(
            &mut s,
            "challenge generate --topic routing --difficulty ???"
        )
        .contains("difficulté inconnue")
    );
    assert!(
        fail(&mut s, "challenge generate --topic routing --seed x").contains("graine invalide")
    );
    assert!(fail(&mut s, "challenge generate routing").contains("usage"));
}

#[test]
fn generation_is_reproducible_from_the_seed() {
    let go = |seed: u64| {
        let (mut s, _) = session();
        run(
            &mut s,
            &format!("challenge generate --topic physical --difficulty medium --seed {seed}"),
        )
    };
    assert_eq!(go(9), go(9));
}

// ------------------------------------------------------------------ what-if / santé / docs

#[test]
fn what_if_reports_the_impact_without_touching_the_network() {
    let mut s = mvp();
    let out = run(&mut s, "what-if R1 goes down");
    assert!(out.contains("ANALYSE D'IMPACT — R1 goes down"), "{out}");
    assert!(
        out.contains("Machines affectées : 2") && out.contains("Segments isolés    : 1"),
        "{out}"
    );
    assert!(run(&mut s, "what-if \"block tcp/443\"").contains("Aucun impact"));
    // Le réseau réel n'a pas bougé : le ping fonctionne encore.
    assert!(run(&mut s, "ping PC1 192.168.2.10").contains("0 % de perte"));
    assert!(run(&mut s, "log all").matches("PANNE").count() == 0);
    assert!(fail(&mut s, "what-if").contains("usage"));
    assert!(fail(&mut s, "what-if explode everything").contains("scénario non compris"));
    assert!(fail(&mut Session::new(), "what-if R1 goes down").contains("aucune topologie chargée"));
}

#[test]
fn health_reflects_the_state_of_the_network() {
    let mut s = mvp();
    let healthy = run(&mut s, "health");
    assert!(healthy.starts_with("NETWORK HEALTH"), "{healthy}");
    assert!(healthy.contains("Connectivity  100/100"), "{healthy}");
    run(&mut s, "cable R1 eth1 cut");
    let broken = run(&mut s, "health");
    assert!(broken.contains("Connectivity    0/100"), "{broken}");
    assert!(broken.contains("câble R1 ↔ R2 coupé"), "{broken}");
}

#[test]
fn health_and_the_tutor_do_not_give_the_answer_during_a_challenge() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-02"); // câble coupé
    assert!(fail(&mut s, "health").contains("indisponible pendant un challenge"));
    // what-if et docs ne désignent pas la panne : ils restent autorisés.
    run(&mut s, "what-if R1 goes down");
    run(&mut s, "docs md");
}

#[test]
fn docs_can_be_printed_or_written_in_three_formats() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = mvp();
    let md = run(&mut s, "docs md");
    assert!(
        md.starts_with("# Documentation réseau — mvp") && md.contains("## Plan d'adressage"),
        "{md}"
    );
    let file = dir.path().join("doc.json");
    let out = run(&mut s, &format!("docs json {}", file.display()));
    assert!(out.contains("Documentation écrite"), "{out}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(json["devices"].as_array().unwrap().len(), 5);
    assert!(run(&mut s, "docs yaml").contains("recommendations:"));
    assert!(fail(&mut s, "docs pdf").contains("pandoc"));
    assert!(fail(&mut s, "docs").contains("usage"));
    assert!(fail(&mut s, "docs docx").contains("format inconnu"));
}

// ------------------------------------------------------------------ tuteur

#[test]
fn the_tutor_explains_concepts_for_free_anywhere() {
    let (mut s, _) = session();
    let out = run(&mut s, "tutor c'est quoi l'ARP ?");
    assert!(out.contains("ARP associe une adresse IP"), "{out}");
    assert!(
        !out.contains("points"),
        "une explication est gratuite : {out}"
    );
    run(&mut s, "challenge start beginner-01");
    let during = run(&mut s, "tutor explique le TTL");
    assert!(
        during.contains("compteur") && !during.contains("−10"),
        "{during}"
    );
    assert_eq!(s.profile().completed_count(), 0);
}

#[test]
fn the_tutor_guides_during_a_challenge_and_charges_like_a_hint() {
    let (mut s, clock) = session();
    run(&mut s, "challenge start beginner-01");
    let a = run(&mut s, "tutor pourquoi le ping échoue ?");
    assert!(
        a.contains("Indice 1") && a.contains("−10 points — indices utilisés : 1"),
        "{a}"
    );
    let refuse = run(&mut s, "tutor donne-moi la solution");
    assert!(refuse.contains("Je ne donne pas la réponse"), "{refuse}");
    assert!(refuse.contains("Indice 2"), "{refuse}");
    clock.set(clock.get() + 10);
    run(&mut s, "gateway PC1 192.168.1.1");
    run(&mut s, "challenge check");
    let end = run(&mut s, "answer PC1 wrong-gateway");
    assert!(
        end.contains("Score : 80/100") && end.contains("indices 20"),
        "{end}"
    );
}

#[test]
fn the_tutor_needs_a_question_and_a_challenge_for_guidance() {
    let (mut s, _) = session();
    assert!(fail(&mut s, "tutor").contains("pose une question"));
    assert!(fail(&mut s, "tutor pourquoi ça échoue").contains("aucun challenge en cours"));
}

#[test]
fn intelligence_sessions_are_deterministic() {
    let play = || {
        let (mut s, clock) = session();
        let mut all = String::new();
        for line in [
            "topology generate --type hybrid --routers 4 --hosts 8 --seed 2",
            "health",
            "what-if R2 goes down",
            "docs md",
            "challenge generate --topic security --difficulty medium --seed 5",
            "tutor c'est quoi un pare-feu ?",
            "hint",
        ] {
            clock.set(clock.get() + 3);
            all += &run(&mut s, line);
        }
        all
    };
    assert_eq!(play(), play());
}
