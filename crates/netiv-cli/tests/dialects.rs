#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Consoles Cisco IOS et terminal de poste (Windows, Linux) : de vrais défis
//! sont résolus pour 100 points sans aucune commande NETIV de configuration.
//! La syntaxe NETIV, elle, ne change pas.

use netiv_cli::{ManualClock, Session};

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

/// Lance le défi, tape les lignes données, vérifie et répond : 100 points attendus.
fn solve(id: &str, lines: &[&str], device: &str, cause: &str) {
    let (mut s, _clock) = session();
    run(&mut s, &format!("challenge start {id}"));
    assert!(
        run(&mut s, "challenge check").contains("Pas encore"),
        "[{id}] devrait être cassé"
    );
    for line in lines {
        run(&mut s, line);
    }
    let checked = run(&mut s, "challenge check");
    assert!(
        checked.contains("Objectif atteint"),
        "[{id}] pas réparé par {lines:?} :\n{checked}"
    );
    let end = run(&mut s, &format!("answer {device} {cause}"));
    assert!(end.contains("Score : 100/100"), "[{id}] {end}");
}

#[test]
fn cisco_cli_solves_router_and_switch_labs() {
    // Interface éteinte sur un routeur (Gi0/1 = eth1 sur un routeur).
    solve(
        "beginner-03",
        &[
            "console R1",
            "en",
            "conf t",
            "int gi0/1",
            "no shut",
            "end",
            "exit",
        ],
        "R1",
        "interface-down",
    );
    // Route manquante.
    solve(
        "intermediate-02",
        &[
            "console R2",
            "enable",
            "configure terminal",
            "ip route 10.3.0.0 255.255.255.0 10.23.0.2",
            "end",
        ],
        "R2",
        "missing-route",
    );
    // Trunk qui ne porte pas les bons VLAN.
    solve(
        "intermediate-04",
        &[
            "console SW1",
            "en",
            "conf t",
            "interface tr",
            "switchport mode trunk",
            "switchport trunk allowed vlan 10,20",
            "switchport trunk native vlan 1",
            "end",
        ],
        "SW1",
        "wrong-vlan",
    );
    // Relais DHCP absent : ip helper-address, puis le poste redemande son adresse.
    solve(
        "intermediate-06",
        &[
            "console R1",
            "en",
            "conf t",
            "int gi0/0",
            "ip helper-address 10.10.2.10",
            "end",
            "exit",
            "console PC1",
            "ipconfig /renew",
        ],
        "R1",
        "dhcp-relay-missing",
    );
}

#[test]
fn support_labs_with_switch_cli_and_windows_or_linux_terminal() {
    // Port de switch éteint (switch : Gi0/2 = p2), puis renouvellement DHCP Windows.
    solve(
        "support-01",
        &[
            "console SW1",
            "en",
            "conf t",
            "int gi0/2",
            "no shutdown",
            "end",
            "exit",
            "console ACC2",
            "ipconfig /renew",
        ],
        "SW1",
        "interface-down",
    );
    // Port dans le mauvais VLAN, puis dhclient (Linux).
    solve(
        "support-09",
        &[
            "console SW1",
            "en",
            "conf t",
            "int p5",
            "switchport access vlan 20",
            "end",
            "console CPT2",
            "dhclient eth0",
        ],
        "SW1",
        "wrong-vlan",
    );
    // APIPA : la carte repasse en DHCP (Windows, puis Linux).
    solve(
        "support-02",
        &[
            "console ACC1",
            r#"netsh interface ip set address "eth0" dhcp"#,
        ],
        "ACC1",
        "wrong-ip",
    );
    solve(
        "support-02",
        &["console ACC1", "dhclient eth0"],
        "ACC1",
        "wrong-ip",
    );
    // Mauvais serveur DNS sur le poste.
    solve(
        "support-05",
        &["console ACC1", r#"netsh interface ip set dns "eth0" dhcp"#],
        "ACC1",
        "dns-wrong-server",
    );
    solve(
        "support-05",
        &["console ACC1", "resolvectl dns eth0 192.168.30.10"],
        "ACC1",
        "dns-wrong-server",
    );
}

#[test]
fn host_terminal_solves_addressing_labs() {
    solve(
        "beginner-01",
        &["console PC1", "route add 0.0.0.0 mask 0.0.0.0 192.168.1.1"],
        "PC1",
        "wrong-gateway",
    );
    solve(
        "beginner-01",
        &["console PC1", "ip route add default via 192.168.1.1"],
        "PC1",
        "wrong-gateway",
    );
    solve(
        "beginner-04",
        &[
            "console PC1",
            "netsh interface ip set address name=eth0 static 192.168.1.10 255.255.255.0",
        ],
        "PC1",
        "wrong-ip",
    );
    solve(
        "beginner-04",
        &["console PC1", "ip addr add 192.168.1.10/24 dev eth0"],
        "PC1",
        "wrong-ip",
    );
}

#[test]
fn consoles_change_prompt_and_keep_netiv_pilot_commands() {
    let (mut s, _) = session();
    run(&mut s, "challenge start support-01");
    assert!(s.console_info().is_none());
    assert!(run(&mut s, "console SW1").contains("Console Cisco IOS de SW1"));
    assert_eq!(s.console_info().unwrap().prompt, "SW1>");
    run(&mut s, "en");
    assert_eq!(s.console_info().unwrap().prompt, "SW1#");
    run(&mut s, "conf t");
    run(&mut s, "int fa0/2");
    let info = s.console_info().unwrap();
    assert_eq!(
        (info.kind, info.prompt.as_str()),
        ("ios", "SW1(config-if)#")
    );
    // Pilotage NETIV toujours disponible (boutons Indice / Vérifier de l'interface).
    assert!(
        run(&mut s, "hint").contains("ndice"),
        "hint pendant une console"
    );
    assert!(run(&mut s, "challenge check").contains("Pas encore"));
    // « netiv <commande> » : toute commande NETIV.
    assert!(run(&mut s, "netiv devices").contains("SW1"));
    // Erreurs au format Cisco.
    assert!(fail(&mut s, "frobnicate").contains("% Invalid input"));
    assert!(fail(&mut s, "sw access vlan abc").contains("% Invalid input"));
    // Remonter les modes, puis quitter.
    run(&mut s, "exit");
    assert_eq!(s.console_info().unwrap().prompt, "SW1(config)#");
    run(&mut s, "end");
    run(&mut s, "exit");
    assert!(
        s.console_info().is_none(),
        "exit au niveau privilégié : retour à NETIV"
    );
    // Hors console, « ip » garde son sens NETIV.
    assert!(
        fail(&mut s, "ip a").contains("usage")
            || fail(&mut s, "ip a").contains("équipement inconnu")
    );
}

#[test]
fn cisco_and_terminal_displays() {
    let (mut s, _) = session();
    run(&mut s, "challenge start support-01");
    run(&mut s, "console SW1");
    let brief = run(&mut s, "sh ip int br");
    assert!(brief.contains("GigabitEthernet0/2 (p2)"), "{brief}");
    assert!(
        brief.contains("administratively down"),
        "le port p2 est éteint par la panne :\n{brief}"
    );
    let vlans = run(&mut s, "show vlan brief");
    assert!(
        vlans.contains("VLAN0010") && vlans.contains("VLAN0020"),
        "{vlans}"
    );
    let run_cfg = run(&mut s, "show run");
    assert!(
        run_cfg.contains("hostname SW1")
            && run_cfg.contains("switchport access vlan 20")
            && run_cfg.contains(" shutdown"),
        "{run_cfg}"
    );
    run(&mut s, "exit");

    run(&mut s, "console R1");
    let routes = run(&mut s, "show ip route");
    assert!(
        routes.contains("Gateway of last resort is 203.0.113.1"),
        "{routes}"
    );
    assert!(
        routes.contains("C        192.168.30.0/24 is directly connected"),
        "{routes}"
    );
    run(&mut s, "exit");

    run(&mut s, "console ACC1");
    assert_eq!(s.console_info().unwrap().prompt, "ACC1:~$");
    let ipconfig = run(&mut s, "ipconfig /all");
    assert!(
        ipconfig.contains("DHCP activé")
            && ipconfig.contains("Oui")
            && ipconfig.contains("Serveurs DNS"),
        "{ipconfig}"
    );
    let ipa = run(&mut s, "ip a");
    assert!(
        ipa.contains("inet 192.168.10.") && ipa.contains("dynamic eth0"),
        "{ipa}"
    );
    assert!(run(&mut s, "cat /etc/resolv.conf").contains("nameserver 192.168.30.10"));
    assert!(run(&mut s, "ip route").contains("default via 192.168.10.1 dev eth0"));
    assert!(run(&mut s, "ping intranet.pme").contains("192.168.30.10"));
    assert!(fail(&mut s, "sudo rm -rf /").contains("commande introuvable"));
}
