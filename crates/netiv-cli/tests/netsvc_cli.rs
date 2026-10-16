#![allow(clippy::unwrap_used, clippy::expect_used)]

//! DHCP et DNS depuis la session : commandes, pannes, diagnostic, défis.

use netiv_cli::{ManualClock, Session};

const DHCP: &str = include_str!("../../netiv-education/content/topologies/dhcp.yaml");

fn session() -> Session {
    let mut s = Session::new();
    s.load_yaml(DHCP, &mut Vec::new()).unwrap();
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

#[test]
fn show_dhcp_and_names() {
    let mut s = session();
    let srv = run(&mut s, "show dhcp SRV");
    assert!(srv.contains("Serveur DHCP : démarré"), "{srv}");
    assert!(srv.contains("2/51 attribuée(s)"), "{srv}");
    assert!(run(&mut s, "show ifaces PC1").contains("(DHCP)"));
    assert!(run(&mut s, "show dns PC1").contains("10.10.2.10 (reçu par DHCP)"));

    let ping = run(&mut s, "ping PC1 web.lan");
    assert!(ping.contains("DNS : web.lan = 10.10.2.10"), "{ping}");
    assert!(ping.contains("0 % de perte"), "{ping}");
    assert!(run(&mut s, "ping PC1 web.lan").contains("(cache)"));
    assert!(run(&mut s, "curl PC3 srv.lan").contains("HTTP/1.1 200 OK"));
    assert!(run(&mut s, "nslookup PC2 inconnu.lan").contains("NXDOMAIN"));
    assert!(fail(&mut s, "ping PC1 pas_un_nom").contains("ni une adresse IPv4 ni un nom"));
}

#[test]
fn a_stopped_dhcp_server_is_diagnosed_then_repaired() {
    let mut s = session();
    run(&mut s, "dhcp server SRV off");
    run(&mut s, "dhcp PC1 release");
    let renew = run(&mut s, "dhcp PC1 renew");
    assert!(renew.contains("aucun serveur n'a répondu"), "{renew}");
    let d = run(&mut s, "diagnose PC1 10.10.2.10");
    assert!(d.contains("serveur DHCP arrêté ou absent sur SRV"), "{d}");
    assert!(d.contains("dhcp server SRV on"), "{d}");
    run(&mut s, "dhcp server SRV on");
    assert!(run(&mut s, "dhcp PC1").contains("obtient 10.10.1.100/24"));
    assert!(run(&mut s, "ping PC1 10.10.2.10").contains("0 % de perte"));
}

#[test]
fn a_missing_relay_and_a_static_address() {
    let mut s = session();
    run(&mut s, "dhcp relay R1 eth0 none");
    run(&mut s, "dhcp PC2 release");
    run(&mut s, "dhcp PC2");
    let d = run(&mut s, "diagnose PC2 10.10.2.10");
    assert!(d.contains("relais DHCP absent ou erroné sur R1"), "{d}");
    assert!(d.contains("dhcp relay R1 eth0 10.10.2.10"), "{d}");
    // Adresse fixe, puis retour au DHCP.
    run(&mut s, "ip PC3 eth0 10.10.2.77/24");
    assert!(run(&mut s, "show ifaces PC3").contains("10.10.2.77/24 "));
    assert!(fail(&mut s, "dhcp PC3").contains("n'a pas d'interface configurée par DHCP"));
    assert!(run(&mut s, "ip PC3 eth0 dhcp").contains("obtient"));
}

#[test]
fn pools_and_records_are_managed_from_the_command_line() {
    let mut s = session();
    run(
        &mut s,
        "dhcp pool add SRV 10.10.1.0/24 10.10.1.200-10.10.1.200 gw 10.10.1.1 dns 10.10.2.10",
    );
    let show = run(&mut s, "show dhcp SRV");
    assert!(show.contains("10.10.1.200-10.10.1.200"), "{show}");
    assert!(
        fail(&mut s, "dhcp pool add SRV 10.10.1.0/24 10.10.9.1-10.10.9.2").contains("invalide")
    );
    run(&mut s, "dhcp pool del SRV 10.10.2.0/24");
    assert!(!run(&mut s, "show dhcp SRV").contains("10.10.2.0/24"));

    run(&mut s, "dns record add SRV mail.lan 10.10.2.10");
    assert!(run(&mut s, "nslookup PC1 mail.lan").contains("mail.lan = 10.10.2.10"));
    run(&mut s, "dns record del SRV mail.lan");
    let d = run(&mut s, "diagnose PC1 mail.lan");
    assert!(
        d.contains("enregistrement DNS erroné ou absent sur SRV"),
        "{d}"
    );
}

#[test]
fn dns_failures_are_diagnosed_by_name() {
    let mut s = session();
    run(&mut s, "dns PC2 10.10.2.99");
    let d = run(&mut s, "diagnose PC2 srv.lan");
    assert!(d.contains("mauvais serveur DNS sur PC2"), "{d}");
    assert!(d.contains("dns PC2 10.10.2.10"), "{d}");
    run(&mut s, "dns PC2 none");
    run(&mut s, "dns server SRV off");
    let d = run(&mut s, "diagnose PC2 srv.lan");
    assert!(d.contains("serveur DNS arrêté sur SRV"), "{d}");
    run(&mut s, "dns server SRV on");
    let d = run(&mut s, "diagnose PC2 srv.lan");
    assert!(d.contains("[✓] Résolution DNS"), "{d}");
}

#[test]
fn the_dhcp_challenges_are_solved_by_their_commands() {
    for (id, answer, fix) in [
        (
            "beginner-06",
            "answer SRV dhcp-server-down",
            &["dhcp server SRV on", "dhcp PC1 renew"][..],
        ),
        (
            "intermediate-06",
            "answer R1 dhcp-relay-missing",
            &["dhcp relay R1 eth0 10.10.2.10", "dhcp PC1 renew"][..],
        ),
        (
            "advanced-06",
            "answer IMP rogue-dhcp",
            &["dhcp server IMP off", "dhcp PC1 renew"][..],
        ),
    ] {
        let clock = ManualClock::new(1_000);
        let mut s = Session::new();
        s.set_manual_clock(&clock);
        run(&mut s, &format!("challenge start {id}"));
        assert!(run(&mut s, answer).contains("correct"), "{id}");
        for c in fix {
            run(&mut s, c);
        }
        let end = run(&mut s, "challenge check");
        assert!(end.contains("100/100"), "{id} : {end}");
    }
}
