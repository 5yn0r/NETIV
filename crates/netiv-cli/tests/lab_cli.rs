#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Partage de labs (`.netiv`), bibliothèque, classement local, mode bac à sable.

use std::path::PathBuf;

use netiv_cli::{ManualClock, Session};

fn labs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../labs")
}

fn example(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
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
fn the_shipped_library_lists_loads_and_plays() {
    let mut s = Session::new();
    s.set_labs_dir(labs_dir());
    let list = run(&mut s, "lab list");
    for name in [
        "bureau",
        "campus-vlan",
        "cyber-range",
        "triangle",
        "defi-boucle",
        "defi-pare-feu",
    ] {
        assert!(list.contains(name), "{name} manque :\n{list}");
    }
    assert!(
        list.contains("[challenge]") && list.contains("[réseau]"),
        "{list}"
    );

    // Tous les réseaux se chargent sans avertissement.
    for name in ["bureau", "campus-vlan", "cyber-range", "triangle"] {
        let out = run(&mut s, &format!("lab open {name}"));
        assert!(
            out.contains("chargé") && !out.contains('⚠'),
            "{name} : {out}"
        );
    }
    // Les challenges livrés se jouent et se résolvent.
    let path = labs_dir().join("defi-boucle.netiv");
    let start = run(&mut s, &format!("challenge start {}", path.display()));
    assert!(start.contains("Ça tourne en rond"), "{start}");
    run(&mut s, "route del R3 10.3.0.10/32");
    assert!(run(&mut s, "challenge check").contains("Objectif atteint"));
    let end = run(&mut s, "answer R3 routing-loop");
    assert!(end.contains("CHALLENGE TERMINÉ"), "{end}");
}

#[test]
fn a_network_exported_as_a_lab_can_be_reopened_with_its_changes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mon-lab.netiv");
    let mut s = Session::with_profile("prof", dir.path().join("prof.json")).unwrap();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    run(&mut s, "acl add R1 deny icmp");
    run(
        &mut s,
        &format!("lab export {} Mon TP de filtrage", file.display()),
    );

    let mut other = Session::new();
    let out = run(&mut other, &format!("load {}", file.display()));
    assert!(out.contains("Lab « Mon TP de filtrage » par prof"), "{out}");
    // La configuration modifiée a voyagé avec le lab.
    assert!(run(&mut other, "show acl R1").contains("deny icmp"));
    assert!(run(&mut other, "ping PC1 192.168.2.10").contains("100 % de perte"));
}

#[test]
fn quotes_around_an_exported_title_are_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("q.netiv");
    let mut s = Session::new();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    run(
        &mut s,
        &format!(r#"lab export {} "Mon titre""#, file.display()),
    );
    let out = run(&mut Session::new(), &format!("load {}", file.display()));
    assert!(out.contains("Lab « Mon titre »"), "{out}");
}

#[test]
fn lab_open_uses_the_configured_library_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    run(
        &mut s,
        &format!("lab export {}/a.netiv", dir.path().display()),
    );
    std::fs::write(dir.path().join("b.yaml"), example("dmz.yaml")).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "pas un lab").unwrap();

    let mut other = Session::new();
    other.set_labs_dir(dir.path().to_path_buf());
    let list = run(&mut other, "lab list");
    assert!(
        list.contains("a ") && list.contains("b ") && !list.contains("notes"),
        "{list}"
    );
    assert!(list.contains("(topologie simple)"), "{list}");
    assert!(run(&mut other, "lab open b").contains("Réseau « dmz »"));
    assert!(fail(&mut other, "lab open zzz").contains("lab inconnu"));
    assert!(fail(&mut other, "lab").contains("usage"));
    other.set_labs_dir(dir.path().join("absent"));
    assert!(fail(&mut other, "lab list").contains("inaccessible"));
}

#[test]
fn the_leaderboard_ranks_local_profiles_by_xp() {
    let dir = tempfile::tempdir().unwrap();
    let play = |name: &str, ids: &[(&str, &str, &str)]| {
        let clock = ManualClock::new(1_000);
        let mut s = Session::with_profile(name, dir.path().join(format!("{name}.json"))).unwrap();
        s.set_manual_clock(&clock);
        for (id, fix, answer) in ids {
            run(&mut s, &format!("challenge start {id}"));
            run(&mut s, fix);
            run(&mut s, "challenge check");
            run(&mut s, answer);
        }
        s
    };
    play(
        "bob",
        &[(
            "beginner-01",
            "gateway PC1 192.168.1.1",
            "answer PC1 wrong-gateway",
        )],
    );
    let mut alice = play(
        "alice",
        &[
            (
                "beginner-01",
                "gateway PC1 192.168.1.1",
                "answer PC1 wrong-gateway",
            ),
            (
                "beginner-02",
                "cable R1 eth1 restore",
                "answer R1 link-down",
            ),
        ],
    );
    play("zed", &[]);
    std::fs::write(dir.path().join("pas-un-profil.json"), "{ cassé").unwrap();
    std::fs::write(dir.path().join("note.txt"), "x").unwrap();

    let board = run(&mut alice, "leaderboard");
    let lines: Vec<&str> = board.lines().collect();
    assert!(
        lines[0].contains("RANG") && lines[0].contains("XP"),
        "{board}"
    );
    assert!(
        lines[1].starts_with("1 ") && lines[1].contains("alice") && lines[1].contains("200"),
        "{board}"
    );
    assert!(
        lines[2].starts_with("2 ") && lines[2].contains("bob") && lines[2].contains("100"),
        "{board}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("zed")),
        "un profil jamais sauvegardé n'existe pas : {board}"
    );
    assert!(board.contains("1 fichier(s) ignoré(s)"), "{board}");
    assert!(fail(&mut Session::new(), "leaderboard").contains("aucun dossier de profils"));
}

#[test]
fn a_sandboxed_session_cannot_touch_the_users_files() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.yaml");
    std::fs::write(&secret, example("mvp.yaml")).unwrap();
    let mut s = Session::new().sandboxed();
    assert!(s.is_sandboxed());
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();

    let out = dir.path().join("out");
    for cmd in [
        format!("load {}", secret.display()),
        format!("capture save {}", out.display()),
        format!("forensic analyze {}", secret.display()),
        format!("docs md {}", out.display()),
        format!("lab export {}", out.display()),
        "lab list".to_string(),
        format!("lab open {}", secret.display()),
        format!(
            "topology generate --type lan --hosts 3 --save {}",
            out.display()
        ),
        format!("challenge export beginner-01 {}", out.display()),
        format!(
            "challenge generate --topic physical --save {}",
            out.display()
        ),
        format!("course new tp {}", out.display()),
        format!("course show {}", secret.display()),
        format!("challenge start {}", secret.display()),
        "leaderboard".to_string(),
    ] {
        let e = fail(&mut s, &cmd);
        assert!(
            e.contains("indisponible depuis l'interface web")
                || e.contains("challenge inconnu")
                || e.contains("aucun dossier"),
            "« {cmd} » devrait être refusé : {e}"
        );
    }
    assert!(!out.exists(), "rien ne doit avoir été écrit");
    // Les commandes sans fichier marchent normalement.
    assert!(run(&mut s, "ping PC1 192.168.2.10").contains("0 % de perte"));
    assert!(run(&mut s, "docs md").contains("# Documentation réseau"));
    assert!(run(&mut s, "forensic analyze live").contains("ANALYSE FORENSIQUE"));
    assert!(run(&mut s, "challenge start beginner-01").contains("CHALLENGE"));
}

#[test]
fn clock_is_shareable_between_threads() {
    // Une session doit pouvoir être déplacée dans un autre thread (serveur web).
    let mut s = Session::new();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    let handle = std::thread::spawn(move || {
        let mut out = Vec::new();
        s.exec("ping PC1 192.168.2.10", &mut out).unwrap();
        String::from_utf8(out).unwrap()
    });
    assert!(handle.join().unwrap().contains("0 % de perte"));
}

#[test]
fn positions_travel_with_an_exported_lab() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pos.netiv");
    let mut s = Session::new();
    s.load_yaml(&example("mvp.yaml"), &mut Vec::new()).unwrap();
    let layout = [("PC1".to_string(), [12, 34]), ("R1".to_string(), [56, 78])]
        .into_iter()
        .collect();
    s.set_positions(&layout);
    run(&mut s, &format!("lab export {}", file.display()));

    let mut other = Session::new();
    run(&mut other, &format!("load {}", file.display()));
    assert_eq!(other.positions().get("PC1"), Some(&[12, 34]));
    assert_eq!(other.positions().get("R1"), Some(&[56, 78]));
    assert_eq!(other.positions().get("PC2"), None);
}

#[test]
fn topology_autoconfig_addresses_a_drawn_network() {
    let yaml = r#"
network: { name: dessin }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0 }] }
  - { name: R1, type: router, interfaces: [{ name: eth0 }, { name: eth1 }] }
  - { name: SRV, type: server, interfaces: [{ name: eth0 }] }
links:
  - { a: "PC1:eth0", b: "R1:eth0" }
  - { a: "R1:eth1", b: "SRV:eth0" }
"#;
    let mut s = Session::new();
    s.load_yaml(yaml, &mut Vec::new()).unwrap();
    let out = run(&mut s, "topology autoconfig");
    assert!(out.contains("PC1 : passerelle"), "{out}");
    assert!(run(&mut s, "topology autoconfig").contains("Rien à compléter"));
    let srv = run(&mut s, "show ifaces SRV");
    let ip = srv
        .split_whitespace()
        .find(|w| w.contains('/') && w.starts_with("10."))
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_string();
    assert!(run(&mut s, &format!("ping PC1 {ip}")).contains("0 % de perte"));
}

#[test]
fn the_topology_is_locked_during_a_challenge() {
    let mut s = Session::new();
    run(&mut s, "challenge start beginner-01");
    assert!(s.edit_lock().is_some());
    assert!(fail(&mut s, "topology autoconfig").contains("challenge est en cours"));
    run(&mut s, "challenge giveup");
    assert!(s.edit_lock().is_none());
}
