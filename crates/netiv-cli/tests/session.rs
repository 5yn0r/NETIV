#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Tests de la session CLI sur le réseau du MVP (`examples/mvp.yaml`).

use std::path::PathBuf;

use netiv_cli::{Flow, Session};

fn example() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp.yaml");
    std::fs::read_to_string(&path).expect("examples/mvp.yaml doit exister")
}

fn session() -> Session {
    let mut s = Session::new();
    s.load_yaml(&example(), &mut Vec::new()).unwrap();
    s
}

/// Exécute des commandes et retourne la sortie complète.
fn run(s: &mut Session, script: &[&str]) -> String {
    let mut out = Vec::new();
    for line in script {
        out.extend_from_slice(format!("netiv> {line}\n").as_bytes());
        s.exec(line, &mut out)
            .unwrap_or_else(|e| panic!("« {line} » a échoué : {e}"));
        out.push(b'\n');
    }
    String::from_utf8(out).unwrap()
}

fn fail(s: &mut Session, line: &str) -> String {
    s.exec(line, &mut Vec::new()).unwrap_err().to_string()
}

#[test]
fn the_example_topology_loads_without_warnings() {
    let mut out = Vec::new();
    Session::new().load_yaml(&example(), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("5 équipement(s), 4 câble(s)"), "{text}");
    assert!(
        !text.contains('⚠'),
        "le réseau de référence ne doit rien signaler :\n{text}"
    );
}

/// Trace de référence : toute modification de la sortie doit être voulue.
/// Pour la régénérer : `UPDATE_GOLDEN=1 cargo test -p netiv-cli`.
#[test]
fn mvp_session_matches_golden_output() {
    let mut s = session();
    let actual = run(
        &mut s,
        &[
            "ping PC1 192.168.2.10 2",
            "trace 1",
            "inspect 19",
            "explain 25",
            "show route R1",
            "show arp PC1",
            "show mac SW1",
        ],
    );
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/mvp_session.txt");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&golden)
        .expect("fichier de référence manquant : lancer avec UPDATE_GOLDEN=1");
    assert_eq!(
        actual, expected,
        "la sortie diffère de tests/golden/mvp_session.txt"
    );
}

#[test]
fn mvp_session_is_deterministic() {
    let script = ["ping PC1 192.168.2.10 3", "log all"];
    assert_eq!(run(&mut session(), &script), run(&mut session(), &script));
}

#[test]
fn ping_reports_ttl_and_rtt() {
    let out = run(&mut session(), &["ping PC1 192.168.2.10 2"]);
    assert!(
        out.contains("seq 1  réponse de 192.168.2.10  ttl=62  temps=16.000 ms"),
        "{out}"
    );
    // Le second ping profite des caches ARP : 8 sauts de 1 ms.
    assert!(
        out.contains("seq 2  réponse de 192.168.2.10  ttl=62  temps=8.000 ms"),
        "{out}"
    );
    assert!(
        out.contains("2 envoyé(s), 2 reçu(s), 0 % de perte"),
        "{out}"
    );
}

#[test]
fn journey_shows_the_path_with_ttl() {
    let mut s = session();
    let out = run(&mut s, &["ping PC1 192.168.2.10", "trace 1"]);
    assert!(
        out.contains("PC1 → SW1 → R1 (ttl 64) → R2 (ttl 63) → SERVER (ttl 62)"),
        "{out}"
    );
}

#[test]
fn breaking_the_gateway_and_reading_the_trace_explains_why() {
    let mut s = session();
    let out = run(
        &mut s,
        &[
            "gateway PC1 192.168.1.254",
            "ping PC1 192.168.2.10",
            "trace 1",
        ],
    );
    assert!(out.contains("100 % de perte"), "{out}");
    assert!(out.contains("tentative 3"), "{out}");
    assert!(out.contains("aucune réponse pour 192.168.1.254"), "{out}");
}

#[test]
fn removing_a_route_gives_an_icmp_error_naming_the_router() {
    let mut s = session();
    let out = run(
        &mut s,
        &["route del R1 192.168.2.0/24", "ping PC1 192.168.2.10"],
    );
    assert!(
        out.contains("erreur de 192.168.1.1 : réseau inaccessible"),
        "{out}"
    );
}

#[test]
fn cutting_and_restoring_a_cable() {
    let mut s = session();
    let out = run(
        &mut s,
        &[
            "cable R1 eth1 cut",
            "ping PC1 192.168.2.10",
            "cable R1 eth1 restore",
            "ping PC1 192.168.2.10",
        ],
    );
    assert!(out.contains("coupé"), "{out}");
    assert!(out.contains("pas de réponse"), "{out}");
    assert!(out.contains("rétabli"), "{out}");
    assert!(
        out.contains("1 envoyé(s), 1 reçu(s), 0 % de perte"),
        "{out}"
    );
}

#[test]
fn step_by_step_execution() {
    let mut s = session();
    let out = run(
        &mut s,
        &["ping PC1 192.168.2.10 1 --step", "time", "step", "time"],
    );
    assert!(out.contains("Ping programmé"), "{out}");
    assert!(out.contains("Événements en attente : 1"), "{out}");
    assert!(out.contains("commande : ping 192.168.2.10"), "{out}");
    // Après le premier pas, la requête ARP et les deux timers sont en file.
    assert!(!out.contains("Événements en attente : 0"), "{out}");

    let out = run(&mut s, &["run", "step"]);
    assert!(out.contains("Plus rien à exécuter."), "{out}");
}

#[test]
fn changing_ip_to_a_wrong_mask_is_reported() {
    let mut s = session();
    let out = run(&mut s, &["ip PC1 eth0 192.168.1.10/16", "show ifaces PC1"]);
    assert!(out.contains("192.168.1.10/16"), "{out}");
}

#[test]
fn user_errors_are_explained() {
    let mut s = session();
    assert!(fail(&mut s, "ping NOPE 1.1.1.1").contains("équipements : PC1, SW1, R1, R2, SERVER"));
    assert!(fail(&mut s, "ping PC1 pas_une_ip").contains("ni une adresse IPv4 ni un nom"));
    assert!(fail(&mut s, "frobnicate").contains("commande inconnue"));
    assert!(fail(&mut s, "trace 999").contains("aucune transaction #999"));
    assert!(fail(&mut s, "trace abc").contains("numéro invalide"));
    assert!(fail(&mut s, "inspect 0").contains("aucun événement"));
    assert!(fail(&mut s, "show mac PC1").contains("n'est pas un switch"));
    assert!(fail(&mut s, "ip PC1 eth9 1.2.3.4/24").contains("eth9"));
    assert!(fail(&mut s, "ping SW1 1.1.1.1").contains("switch"));
    assert!(fail(&mut s, "cable SW1 port3 cut").contains("aucun câble"));
    assert!(fail(&mut s, "loss PC1 eth0 2").contains("taux de perte invalide"));
    // Un nom valide sans serveur DNS : expliqué, pas une erreur de saisie.
    let mut out = Vec::new();
    s.exec("ping PC1 pas-une-ip", &mut out).unwrap();
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("aucun serveur DNS configuré")
    );
}

#[test]
fn inspect_and_explain_refuse_the_wrong_kind_of_event() {
    let mut s = session();
    run(&mut s, &["ping PC1 192.168.2.10"]);
    // L'événement 0 est « CommandIssued » : ni trame, ni décision de route.
    assert!(fail(&mut s, "inspect 0").contains("n'est pas une trame"));
    assert!(fail(&mut s, "explain 0").contains("n'est pas une décision de routage"));
}

#[test]
fn commands_need_a_topology() {
    let mut s = Session::new();
    assert!(fail(&mut s, "devices").contains("aucune topologie chargée"));
    assert!(fail(&mut s, "ping A 1.1.1.1").contains("aucune topologie chargée"));
}

#[test]
fn quit_and_blank_lines() {
    let mut s = Session::new();
    let mut out = Vec::new();
    assert_eq!(s.exec("", &mut out).unwrap(), Flow::Continue);
    assert_eq!(s.exec("quit", &mut out).unwrap(), Flow::Quit);
    assert_eq!(s.exec("exit", &mut out).unwrap(), Flow::Quit);
}

#[test]
fn lossy_cable_is_reproducible_with_the_topology_seed() {
    let script = ["loss PC1 eth0 0.3", "ping PC1 192.168.2.10 20"];
    let first = run(&mut session(), &script);
    let second = run(&mut session(), &script);
    assert_eq!(first, second);
    // Avec 30 % de perte, il y a des succès ET des échecs.
    assert!(first.contains("réponse de"), "{first}");
    assert!(first.contains("pas de réponse"), "{first}");
}

// ---------------------------------------------------------------- M1 : VLAN, TCP/UDP, filtrage

fn dmz() -> Session {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/dmz.yaml");
    let mut s = Session::new();
    let mut out = Vec::new();
    s.load_yaml(&std::fs::read_to_string(path).unwrap(), &mut out)
        .unwrap();
    assert!(
        !String::from_utf8(out).unwrap().contains('⚠'),
        "l'exemple DMZ ne doit rien signaler"
    );
    s
}

#[test]
fn dmz_lets_the_web_through_and_blocks_ssh() {
    let mut s = dmz();
    let out = run(&mut s, &["curl PC1 10.9.0.10 80", "curl PC1 10.9.0.10 22"]);
    assert!(out.contains("« HTTP/1.1 200 OK »"), "{out}");
    assert!(out.contains("délai dépassé"), "{out}");
}

#[test]
fn the_dmz_cannot_open_connections_to_the_lan() {
    let mut s = dmz();
    let out = run(&mut s, &["curl SRV 192.168.1.10 22"]);
    assert!(out.contains("délai dépassé"), "{out}");
}

#[test]
fn the_trace_shows_which_policy_blocked_the_flow() {
    let mut s = dmz();
    let out = run(&mut s, &["curl PC1 10.9.0.10 22", "trace 1"]);
    assert!(
        out.contains("filtrage : politique par défaut : deny"),
        "{out}"
    );
    assert!(out.contains("TCP [SYN] (retransmission 2)"), "{out}");
}

#[test]
fn opening_a_port_from_the_cli_fixes_the_flow() {
    let mut s = dmz();
    let out = run(
        &mut s,
        &[
            "acl add FW1 allow tcp dst 10.9.0.0/24 port 22 in eth0",
            "show acl FW1",
            "curl PC1 10.9.0.10 22",
        ],
    );
    assert!(out.contains("règle 3 ajoutée"), "{out}");
    assert!(
        out.contains("3. allow tcp any → 10.9.0.0/24 port 22 depuis eth0"),
        "{out}"
    );
    assert!(out.contains("« ssh prêt »"), "{out}");
}

#[test]
fn acl_rules_can_be_inserted_and_deleted() {
    let mut s = dmz();
    let out = run(
        &mut s,
        &[
            "acl insert FW1 1 deny tcp port 80",
            "curl PC1 10.9.0.10 80",
            "acl del FW1 1",
            "curl PC1 10.9.0.10 80",
        ],
    );
    assert!(out.contains("règle 1 insérée"), "{out}");
    let first = out
        .find("délai dépassé")
        .expect("le deny doit bloquer le web");
    let ok = out.find("« HTTP/1.1 200 OK »").expect("web rétabli");
    assert!(first < ok, "{out}");
}

#[test]
fn service_commands() {
    let mut s = dmz();
    let out = run(
        &mut s,
        &[
            "service del SRV tcp 80",
            "curl PC1 10.9.0.10 80",
            "service add SRV tcp 80 http",
            "curl PC1 10.9.0.10 80",
            "show services SRV",
        ],
    );
    assert!(out.contains("connexion refusée (RST)"), "{out}");
    assert!(out.contains("« HTTP/1.1 200 OK »"), "{out}");
    assert!(out.contains("tcp/80     http"), "{out}");
}

#[test]
fn udp_command_reports_blocked_or_closed_ports() {
    let mut s = dmz();
    s.exec(
        "acl add FW1 allow udp dst 10.9.0.0/24 in eth0",
        &mut Vec::new(),
    )
    .unwrap();
    s.exec("service add SRV udp 7 echo", &mut Vec::new())
        .unwrap();
    let out = run(
        &mut s,
        &["udp PC1 10.9.0.10 7 coucou", "udp PC1 10.9.0.10 9"],
    );
    assert!(out.contains("réponse « coucou »"), "{out}");
    assert!(out.contains("port inaccessible (ICMP)"), "{out}");
}

#[test]
fn vlan_port_commands() {
    let mut s = dmz();
    let out = run(
        &mut s,
        &[
            "port SW1 p2 access 20",
            "port SW1 p1 trunk 10,20 native 10",
            "show vlan SW1",
        ],
    );
    assert!(out.contains("p2       access   VLAN 20"), "{out}");
    assert!(
        out.contains("p1       trunk    VLAN 10,20, natif 10"),
        "{out}"
    );
}

#[test]
fn moving_a_host_to_another_vlan_isolates_it() {
    let mut s = dmz();
    let out = run(&mut s, &["port SW1 p1 access 20", "ping PC1 192.168.1.11"]);
    assert!(out.contains("100 % de perte"), "{out}");
}

#[test]
fn m1_commands_report_usage_errors() {
    let mut s = dmz();
    assert!(fail(&mut s, "acl add FW1 allow tcp port").contains("attend une valeur"));
    assert!(fail(&mut s, "acl add FW1 allow tcp color red").contains("option inconnue"));
    assert!(fail(&mut s, "acl add FW1 explode").contains("action inconnue"));
    assert!(fail(&mut s, "acl del FW1 0").contains("à partir de 1"));
    assert!(fail(&mut s, "acl del FW1 42").contains("n'existe pas"));
    assert!(fail(&mut s, "acl add FW1 allow in eth9").contains("eth9"));
    assert!(fail(&mut s, "acl add FW1 allow port 90-80").contains("plage"));
    assert!(fail(&mut s, "port SW1 p1 access 5000").contains("VLAN invalide"));
    assert!(fail(&mut s, "port PC1 eth0 access 10").contains("pas un switch"));
    assert!(fail(&mut s, "service add SRV sctp 80 x").contains("protocole inconnu"));
    assert!(fail(&mut s, "curl PC1 10.9.0.10 0").contains("port invalide"));
    assert!(fail(&mut s, "show vlan PC1").contains("pas un switch"));
    assert!(fail(&mut s, "show acl SW1").contains("switch"));
}

#[test]
fn m1_sessions_are_deterministic() {
    let script = ["curl PC1 10.9.0.10 80", "curl PC1 10.9.0.10 22", "log all"];
    let a = run(&mut dmz(), &script);
    let b = run(&mut dmz(), &script);
    assert_eq!(a, b);
}
