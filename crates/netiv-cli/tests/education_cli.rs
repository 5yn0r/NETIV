#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 5 de bout en bout : challenges, score, progression, parcours, cours.

use std::path::Path;

use netiv_cli::{ManualClock, Session};
use netiv_education::builtin;

/// Session avec une horloge que le test contrôle.
fn session() -> (Session, ManualClock) {
    let clock = ManualClock::new(10_000);
    let mut s = Session::new();
    s.set_manual_clock(&clock);
    (s, clock)
}

fn persistent(dir: &Path, name: &str) -> (Session, ManualClock) {
    let clock = ManualClock::new(10_000);
    let mut s = Session::with_profile(name, dir.join(format!("{name}.json"))).unwrap();
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

#[test]
fn every_builtin_challenge_is_solved_by_its_documented_commands_for_a_perfect_score() {
    for c in builtin::challenges() {
        let (mut s, clock) = session();
        let start = run(&mut s, &format!("challenge start {}", c.id));
        assert!(start.contains(&c.title), "{}: {start}", c.id);
        assert!(start.contains("Barème"), "{start}");

        // Cassé au départ.
        assert!(
            run(&mut s, "challenge check").contains("Pas encore"),
            "[{}] le réseau devrait être cassé",
            c.id
        );
        // Le diagnostic automatique est verrouillé.
        assert!(fail(&mut s, "diagnose").contains("indisponible pendant un challenge"));

        clock.set(10_060);
        run(&mut s, &c.solution.commands.join(" ; "));
        let checked = run(&mut s, "challenge check");
        assert!(
            checked.contains("Objectif atteint"),
            "[{}] « {} » n'a pas réparé :\n{checked}",
            c.id,
            c.solution.commands.join(" ; ")
        );
        let end = run(
            &mut s,
            &format!("answer {} {}", c.solution.device, c.solution.cause),
        );
        assert!(end.contains("CHALLENGE TERMINÉ"), "[{}] {end}", c.id);
        assert!(end.contains("Score : 100/100"), "[{}] {end}", c.id);
        assert!(
            end.contains(c.solution.explanation.lines().next().unwrap().trim()),
            "{end}"
        );
        assert_eq!(s.profile().completed[&c.id].score, 100);
    }
}

#[test]
fn completing_every_challenge_unlocks_every_badge_and_the_top_level() {
    let (mut s, clock) = session();
    for c in builtin::challenges() {
        run(&mut s, &format!("challenge start {}", c.id));
        clock.set(clock.get() + 10);
        run(&mut s, &c.solution.commands.join(" ; "));
        run(&mut s, "challenge check");
        run(
            &mut s,
            &format!("answer {} {}", c.solution.device, c.solution.cause),
        );
    }
    assert_eq!(s.profile().badges.len(), netiv_education::BADGES.len());
    // 6×100 + 6×150 + 6×200 = 2700 XP → niveau 6.
    assert_eq!(s.profile().xp(), 2_700);
    let dash = run(&mut s, "dashboard");
    assert!(dash.contains("Niveau 6 « Expert »"), "{dash}");
    assert!(dash.contains("Challenges réussis : 18 / 21"), "{dash}");
    assert!(dash.contains("100%"), "{dash}");
    let badges = run(&mut s, "badges");
    assert_eq!(badges.matches('🏅').count(), 6, "{badges}");
}

#[test]
fn the_statement_shows_the_symptom_but_nothing_reveals_the_fault() {
    let (mut s, _) = session();
    let start = run(&mut s, "challenge start beginner-01");
    assert!(
        !start.contains("192.168.1.254") && !start.contains("wrong-gateway"),
        "{start}"
    );
    // Le journal est vide : aucune trace de l'injection.
    assert!(run(&mut s, "log all").contains("(trace vide)"));
    // La config visible donne bien le symptôme à observer.
    assert!(run(&mut s, "show route PC1").contains("192.168.1.254"));
}

#[test]
fn hints_are_progressive_and_cost_points() {
    let (mut s, clock) = session();
    run(&mut s, "challenge start beginner-01");
    let h1 = run(&mut s, "hint");
    assert!(h1.starts_with("Indice 1"), "{h1}");
    assert!(
        h1.contains("−10 points") && h1.contains("indices utilisés : 1"),
        "{h1}"
    );
    assert!(run(&mut s, "hint").starts_with("Indice 2"));
    clock.set(10_030);
    run(&mut s, "gateway PC1 192.168.1.1");
    run(&mut s, "challenge check");
    let end = run(&mut s, "answer PC1 wrong-gateway");
    assert!(end.contains("Score : 80/100"), "{end}");
    assert!(end.contains("indices 20"), "{end}");
}

#[test]
fn the_last_hint_is_the_solution_and_says_so() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    for _ in 0..3 {
        run(&mut s, "hint");
    }
    let last = run(&mut s, "hint");
    assert!(last.starts_with("Solution"), "{last}");
    assert!(last.contains("gateway PC1 192.168.1.1"), "{last}");
    assert!(last.contains("dernier niveau"), "{last}");
}

#[test]
fn time_matters_and_is_measured_with_the_clock() {
    let (mut s, clock) = session();
    run(&mut s, "challenge start beginner-01"); // 15 min
    clock.set(10_000 + 600); // 10 minutes
    assert!(run(&mut s, "challenge show").contains("Temps restant : 5 min 00 s"));
    run(&mut s, "gateway PC1 192.168.1.1");
    run(&mut s, "challenge check");
    let end = run(&mut s, "answer PC1 wrong-gateway");
    // 10 min sur 15 : (900 − 600) × 20 / 675 = 8 points de temps.
    assert!(end.contains("temps 8"), "{end}");
    assert!(end.contains("Score : 88/100"), "{end}");
    assert!(end.contains("Temps : 10 min 00 s"), "{end}");
}

#[test]
fn wrong_answers_give_partial_feedback_and_cost_points() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    assert!(
        run(&mut s, "answer PC1 wrong-mask")
            .contains("Le bon équipement, mais pas la bonne nature")
    );
    assert!(
        run(&mut s, "answer R1 wrong-gateway")
            .contains("La bonne nature de panne, mais pas sur cet équipement")
    );
    assert!(run(&mut s, "answer R1 link-down").contains("Ce n'est pas ça"));
    let ok = run(&mut s, "answer PC1 mauvaise passerelle par défaut");
    assert!(ok.contains("Diagnostic correct"), "{ok}");
    run(&mut s, "gateway PC1 192.168.1.1");
    let end = run(&mut s, "challenge check");
    assert!(end.contains("diagnostic 10"), "40 − 3 × 10 : {end}");
}

#[test]
fn repair_first_then_answer_works_and_finish_forfeits_the_diagnosis() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    run(&mut s, "gateway PC1 192.168.1.1");
    let c = run(&mut s, "challenge check");
    assert!(
        c.contains("Objectif atteint") && c.contains("answer"),
        "{c}"
    );
    let end = run(&mut s, "challenge finish");
    assert!(end.contains("diagnostic 0"), "{end}");
    assert!(end.contains("Score : 60/100"), "{end}");
}

#[test]
fn giving_up_reveals_the_solution_without_points_and_unlocks_the_tools() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    let out = run(&mut s, "challenge giveup");
    assert!(
        out.contains("aucun point") && out.contains("192.168.1.254"),
        "{out}"
    );
    assert!(out.contains("gateway PC1 192.168.1.1"), "{out}");
    assert_eq!(s.profile().completed_count(), 0);
    // Le challenge est fini : le diagnostic automatique est de nouveau permis.
    assert!(run(&mut s, "diagnose PC1 192.168.2.10").contains("DIAGNOSTIC"));
    assert!(fail(&mut s, "challenge check").contains("terminé"));
}

#[test]
fn the_answer_giving_tools_are_locked_during_a_challenge() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    for cmd in [
        "diagnose PC1 192.168.2.10",
        "chaos reveal",
        "chaos start PC1 192.168.2.10",
        "baseline save",
    ] {
        assert!(
            fail(&mut s, cmd).contains("indisponible pendant un challenge"),
            "{cmd}"
        );
    }
    // Les outils d'observation, eux, restent disponibles.
    for cmd in [
        "ping PC1 192.168.2.10",
        "show route PC1",
        "show arp PC1",
        "log",
        "devices",
    ] {
        run(&mut s, cmd);
    }
}

#[test]
fn progress_is_saved_between_sessions() {
    let dir = tempfile::tempdir().unwrap();
    {
        let (mut s, _) = persistent(dir.path(), "alice");
        run(&mut s, "challenge start beginner-01");
        run(&mut s, "gateway PC1 192.168.1.1");
        run(&mut s, "challenge check");
        run(&mut s, "answer PC1 wrong-gateway");
        assert_eq!(s.profile().xp(), 100);
    }
    let (mut s, _) = persistent(dir.path(), "alice");
    assert_eq!(s.profile().xp(), 100);
    let list = run(&mut s, "challenge list");
    let line = list.lines().find(|l| l.starts_with("beginner-01")).unwrap();
    assert!(line.contains("100"), "{line}");
    let dash = run(&mut s, "dashboard");
    assert!(
        dash.contains("TABLEAU DE BORD — alice") && dash.contains("Network Beginner"),
        "{dash}"
    );
    // Un autre élève repart de zéro.
    let (other, _) = persistent(dir.path(), "bob");
    assert_eq!(other.profile().xp(), 0);
}

#[test]
fn a_better_second_attempt_only_adds_the_difference() {
    let (mut s, clock) = session();
    for hints in [2, 0] {
        run(&mut s, "challenge start beginner-01");
        for _ in 0..hints {
            run(&mut s, "hint");
        }
        clock.set(clock.get() + 1);
        run(&mut s, "gateway PC1 192.168.1.1");
        run(&mut s, "challenge check");
        run(&mut s, "answer PC1 wrong-gateway");
    }
    assert_eq!(s.profile().xp(), 100, "le meilleur résultat seulement");
    assert_eq!(s.profile().completed["beginner-01"].times, 2);
}

#[test]
fn challenge_list_filters_by_topic_and_difficulty() {
    let (mut s, _) = session();
    let all = run(&mut s, "challenge list");
    assert_eq!(all.lines().count(), 1 + 18);
    let routing = run(&mut s, "challenge list routing");
    assert_eq!(routing.lines().count(), 1 + 4, "{routing}");
    assert!(routing.lines().skip(1).all(|l| l.contains("Routage")));
    let hard = run(&mut s, "challenge list difficile");
    assert_eq!(hard.lines().count(), 1 + 6, "{hard}");
    assert!(run(&mut s, "challenge list zzz").contains("aucun challenge"));
}

#[test]
fn usage_errors_are_explained() {
    let (mut s, _) = session();
    assert!(fail(&mut s, "challenge start nope").contains("challenge inconnu"));
    assert!(fail(&mut s, "challenge start").contains("usage"));
    assert!(fail(&mut s, "challenge check").contains("aucun challenge en cours"));
    assert!(fail(&mut s, "challenge show").contains("aucun challenge en cours"));
    assert!(
        fail(&mut s, "answer PC1 wrong-gateway").contains("aucun challenge ou scénario en cours")
    );
    assert!(fail(&mut s, "answer").contains("usage"));
    assert!(fail(&mut s, "answer PC1").contains("usage"));
    assert!(fail(&mut s, "answer PC1 n-importe-quoi").contains("cause inconnue"));
    assert!(fail(&mut s, "challenge finish").contains("aucun challenge en cours"));
    assert!(fail(&mut s, "hint").contains("lance d'abord"));
    run(&mut s, "challenge start beginner-01");
    assert!(fail(&mut s, "challenge finish").contains("pas encore atteint"));
    assert!(run(&mut s, "challenge causes").contains("wrong-gateway"));
}

#[test]
fn loading_another_network_cancels_the_challenge() {
    let (mut s, _) = session();
    run(&mut s, "challenge start beginner-01");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp.yaml");
    run(&mut s, &format!("load {}", path.display()));
    assert!(fail(&mut s, "challenge check").contains("aucun challenge en cours"));
    // Et les outils sont déverrouillés.
    run(&mut s, "baseline save");
}

#[test]
fn challenges_travel_as_netiv_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mon-defi.netiv");
    let (mut teacher, _) = session();
    let out = run(
        &mut teacher,
        &format!("challenge export intermediate-02 {}", file.display()),
    );
    assert!(out.contains("exporté"), "{out}");

    // Un autre poste (aucune connaissance du réseau livré) le joue tel quel.
    let (mut student, clock) = session();
    let start = run(&mut student, &format!("challenge start {}", file.display()));
    assert!(start.contains("Il manque une route"), "{start}");
    assert!(run(&mut student, "challenge check").contains("Pas encore"));
    clock.set(10_020);
    run(&mut student, "route add R2 10.3.0.0/24 via 10.23.0.2");
    assert!(run(&mut student, "challenge check").contains("Objectif atteint"));
    let end = run(&mut student, "answer R2 missing-route");
    assert!(end.contains("Score : 100/100"), "{end}");
}

#[test]
fn a_hand_written_challenge_file_is_validated_when_started() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("casse.yaml");
    std::fs::write(
        &file,
        "id: x\ntitle: T\ntopic: routing\ndifficulty: easy\ndescription: d\ntopology_ref: office\nobjective: { from: PC1, to: 192.168.2.10 }\nfaults:\n  - set_gateway: { device: PC1, gateway: 192.168.1.254 }\nsolution: { cause: link-down, device: PC1, explanation: e }\n",
    )
    .unwrap();
    let (mut s, _) = session();
    let e = fail(&mut s, &format!("challenge start {}", file.display()));
    assert!(e.contains("le diagnostic trouve « wrong-gateway »"), "{e}");
    let bad = dir.path().join("illisible.yaml");
    std::fs::write(&bad, "pas: du challenge").unwrap();
    assert!(fail(&mut s, &format!("challenge start {}", bad.display())).contains("illisible"));
}

#[test]
fn challenge_sessions_are_deterministic() {
    let play = || {
        let (mut s, clock) = session();
        let mut all = String::new();
        for line in [
            "challenge start intermediate-04",
            "ping PC1 10.10.0.13",
            "show vlan SW1",
            "hint",
            "port SW1 tr trunk 10,20 native 1",
            "challenge check",
            "answer SW1 wrong-vlan",
        ] {
            clock.set(clock.get() + 7);
            all += &run(&mut s, line);
        }
        all
    };
    assert_eq!(play(), play());
}

// ------------------------------------------------------------------ parcours

#[test]
fn learning_path_progress_follows_reading_and_challenges() {
    let (mut s, _) = session();
    let list = run(&mut s, "learn");
    assert!(
        list.contains("fundamentals") && list.contains("0/11"),
        "{list}"
    );
    let path = run(&mut s, "learn fundamentals");
    assert!(path.contains("[ ]  1. Ethernet et adresses MAC"), "{path}");

    let lesson = run(&mut s, "learn read 1");
    assert!(
        lesson.contains("LEÇON 1/11") && lesson.contains("Leçon validée"),
        "{lesson}"
    );
    assert!(run(&mut s, "learn read arp").contains("ARP"));
    assert!(run(&mut s, "learn fundamentals").contains("Progression : 2/11"));

    // Une leçon à challenges ne se valide pas en la lisant.
    let l = run(&mut s, "learn read ipv4");
    assert!(
        !l.contains("Leçon validée") && l.contains("beginner-04"),
        "{l}"
    );
    assert!(run(&mut s, "learn next").contains("LEÇON 3/11"));
    run(&mut s, "challenge start beginner-04");
    run(&mut s, "ip PC1 eth0 192.168.1.10/24");
    run(&mut s, "challenge check");
    run(&mut s, "answer PC1 wrong-ip");
    assert!(run(&mut s, "learn fundamentals").contains("[✓]  3. Adresses IPv4"));
    assert!(fail(&mut s, "learn read 99").contains("leçon inconnue"));
    assert!(fail(&mut s, "learn nope").contains("parcours inconnu"));
}

// ------------------------------------------------------------------ enseignant

#[test]
fn a_teacher_builds_validates_and_exports_a_course() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("tp04.yaml");
    let (mut s, _) = session();
    let made = run(&mut s, &format!("course new tp04 {}", file.display()));
    assert!(made.contains("Modèle de cours écrit"), "{made}");
    assert!(fail(&mut s, &format!("course new tp04 {}", file.display())).contains("existe déjà"));
    assert!(run(&mut s, &format!("course validate {}", file.display())).contains("valide"));

    let sheet = run(&mut s, &format!("course show {}", file.display()));
    assert!(
        sheet.contains("TP tp04 — Routage") && sheet.contains("└── Examen"),
        "{sheet}"
    );

    let md = dir.path().join("tp04.md");
    run(
        &mut s,
        &format!("course export {} {}", file.display(), md.display()),
    );
    let text = std::fs::read_to_string(&md).unwrap();
    assert!(
        text.starts_with("# TP tp04 — Routage") && text.contains("Indices : interdits"),
        "{text}"
    );

    // Une erreur de l'enseignant est signalée avec précision.
    let broken = std::fs::read_to_string(&file)
        .unwrap()
        .replace("intermediate-02", "intermediate-99");
    std::fs::write(&file, broken).unwrap();
    let e = fail(&mut s, &format!("course validate {}", file.display()));
    assert!(
        e.contains("invalide") && e.contains("intermediate-99"),
        "{e}"
    );
}

#[test]
fn an_exam_forbids_hints_enforces_its_time_limit_and_is_graded() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("tp.yaml");
    let (mut s, clock) = session();
    run(&mut s, &format!("course new tp {}", file.display()));

    // Élément 1 : un TP sans challenge, consignes seulement.
    assert!(
        run(&mut s, &format!("course start {} 1", file.display())).contains("Découvrir le réseau")
    );

    // Élément 3 : l'examen (advanced-01, 25 min, sans indices).
    let start = run(&mut s, &format!("course start {} 3", file.display()));
    assert!(
        start.contains("indices interdits") && start.contains("25 min imparties"),
        "{start}"
    );
    assert!(fail(&mut s, "hint").contains("indices sont désactivés"));
    clock.set(10_060);
    run(&mut s, "route add R3 10.1.0.0/24 via 10.23.0.1");
    run(&mut s, "challenge check");
    let end = run(&mut s, "answer R3 missing-route");
    assert!(end.contains("Score : 100/100"), "{end}");

    // Élément 2 : le challenge classique, réussi avec 1 indice → 90/100.
    run(&mut s, &format!("course start {} 2", file.display()));
    run(&mut s, "hint");
    run(&mut s, "route add R2 10.3.0.0/24 via 10.23.0.2");
    run(&mut s, "challenge check");
    run(&mut s, "answer R2 missing-route");

    let grade = run(&mut s, &format!("course grade {}", file.display()));
    assert!(grade.contains("RELEVÉ — invité"), "{grade}");
    assert!(grade.contains("  9 / 10  Il manque une route"), "{grade}");
    assert!(grade.contains(" 10 / 10  Contrôle"), "{grade}");
    assert!(grade.contains(" 19 / 20  TOTAL"), "{grade}");

    assert!(fail(&mut s, &format!("course start {} 9", file.display())).contains("inexistant"));
    assert!(fail(&mut s, "course frobnicate").contains("usage"));
}
