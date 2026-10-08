#![allow(clippy::unwrap_used, clippy::expect_used)]

//! La forensique relit des captures produites par le simulateur.

use std::net::Ipv4Addr;

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::{Command, Fault, SimTime, Simulation};
use netiv_security::forensics::{Behavior, Level, clock};
use netiv_security::{Analysis, Capture, analyze_capture};

fn cyber() -> Simulation {
    netiv_topology::load_yaml(include_str!("../../../examples/cyber.yaml"))
        .unwrap()
        .sim
}

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

/// Ouvre le DMZ → LAN, comme une erreur de configuration.
fn open_dmz_to_lan(sim: &mut Simulation) {
    let fw = sim.device_id("FW1").unwrap();
    let mut hole = AclRule::new(AclAction::Allow, RuleProto::Any);
    hole.src = Some("10.9.0.0/24".parse().unwrap());
    hole.dst = Some("192.168.10.0/24".parse().unwrap());
    hole.dst = Some("192.168.0.0/16".parse().unwrap());
    sim.inject_fault(
        Fault::AclInsert {
            device: fw,
            rule: hole,
        },
        false,
    )
    .unwrap();
}

fn curl_at(sim: &mut Simulation, at_ms: u64, from: &str, dst: &str, port: u16) {
    let d = sim.device_id(from).unwrap();
    sim.schedule_command(
        SimTime::from_millis(at_ms),
        d,
        Command::Curl { dst: ip(dst), port },
    )
    .unwrap();
}

fn capture(sim: &mut Simulation) -> Capture {
    sim.run_until_idle().unwrap();
    Capture::from_sim(sim, "cyber")
}

#[test]
fn a_port_scan_is_detected_and_attributed() {
    let mut sim = cyber();
    open_dmz_to_lan(&mut sim);
    for (i, port) in [21u16, 22, 23, 25, 80, 135, 139, 443, 445, 3389]
        .iter()
        .enumerate()
    {
        curl_at(
            &mut sim,
            5_000 + i as u64 * 500,
            "WEB01",
            "192.168.20.20",
            *port,
        );
    }
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let scan = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::PortScan)
        .expect("balayage non détecté");
    assert_eq!(scan.source, "WEB01");
    assert!(
        scan.detail.contains("10 ports de FILE01"),
        "{}",
        scan.detail
    );
    assert!(
        scan.detail.contains("2 connexion(s) abouties"),
        "22 et 445 répondent : {}",
        scan.detail
    );
    assert_eq!(scan.level, Level::High, "des connexions ont abouti");
    // La violation de zone est aussi relevée : une DMZ n'initie rien vers un serveur.
    let z = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::ZoneViolation)
        .unwrap();
    assert_eq!(z.level, Level::High);
    assert!(
        z.detail.contains("WEB01 (dmz)") && z.detail.contains("FILE01 (server)"),
        "{}",
        z.detail
    );
}

#[test]
fn a_blocked_scan_is_still_reported_but_at_a_lower_level() {
    let mut sim = cyber(); // aucun trou : le pare-feu bloque tout
    for (i, port) in [21u16, 22, 23, 25, 80, 135].iter().enumerate() {
        curl_at(
            &mut sim,
            1_000 + i as u64 * 200,
            "WEB01",
            "192.168.20.20",
            *port,
        );
    }
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let scan = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::PortScan)
        .unwrap();
    assert_eq!(scan.level, Level::Medium);
    assert!(scan.detail.contains("0 connexion(s) abouties"));
    let z = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::ZoneViolation)
        .unwrap();
    assert_eq!(z.level, Level::Medium, "tentative bloquée : {}", z.detail);
    // Le pare-feu a refusé des rafales de paquets.
    let d = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::RepeatedDenials)
        .unwrap();
    assert!(
        d.detail.contains("FW1 a refusé") && d.detail.contains("10.9.0.10"),
        "{}",
        d.detail
    );
}

#[test]
fn lateral_movement_is_a_forbidden_zone_crossing_even_with_a_single_flow() {
    let mut sim = cyber();
    open_dmz_to_lan(&mut sim);
    curl_at(&mut sim, 1_000, "WEB02", "192.168.20.20", 22);
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let z = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::ZoneViolation)
        .unwrap();
    assert_eq!(z.source, "WEB02");
    assert_eq!(z.level, Level::High);
    assert!(
        !a.has(Behavior::PortScan),
        "un seul flux n'est pas un balayage"
    );
}

#[test]
fn regular_callbacks_are_flagged_as_beaconing_but_irregular_traffic_is_not() {
    let mut sim = cyber();
    // Ouvre la sortie de la DMZ (erreur de configuration).
    let fw = sim.device_id("FW1").unwrap();
    let mut hole = AclRule::new(AclAction::Allow, RuleProto::Any);
    hole.src = Some("10.9.0.0/24".parse().unwrap());
    hole.dst = Some("203.0.113.0/24".parse().unwrap());
    sim.inject_fault(
        Fault::AclInsert {
            device: fw,
            rule: hole,
        },
        false,
    )
    .unwrap();
    for k in 0..6 {
        curl_at(&mut sim, 3_000 + k * 3_000, "WEB02", "203.0.113.10", 4444);
    }
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let b = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::Beaconing)
        .expect("balise non détectée");
    assert_eq!(b.source, "WEB02");
    assert!(
        b.detail.contains("toutes les 3.0 s") && b.detail.contains("6 fois"),
        "{}",
        b.detail
    );
    assert!(!a.has(Behavior::PortScan));

    // Mêmes flux, mais espacés de façon irrégulière : pas une balise.
    let mut sim = cyber();
    let fw = sim.device_id("FW1").unwrap();
    let mut hole = AclRule::new(AclAction::Allow, RuleProto::Any);
    hole.src = Some("10.9.0.0/24".parse().unwrap());
    hole.dst = Some("203.0.113.0/24".parse().unwrap());
    sim.inject_fault(
        Fault::AclInsert {
            device: fw,
            rule: hole,
        },
        false,
    )
    .unwrap();
    for t in [1_000u64, 1_500, 9_000, 9_400, 30_000, 31_000] {
        curl_at(&mut sim, t, "WEB02", "203.0.113.10", 4444);
    }
    let cap = capture(&mut sim);
    assert!(!analyze_capture(&cap).has(Behavior::Beaconing));
}

#[test]
fn a_host_sweep_is_detected() {
    let mut sim = cyber();
    for (i, host) in [
        "10.9.0.10",
        "10.9.0.11",
        "192.168.10.11",
        "192.168.10.12",
        "192.168.20.20",
        "203.0.113.10",
    ]
    .iter()
    .enumerate()
    {
        curl_at(&mut sim, 1_000 + i as u64 * 300, "PC01", host, 80);
    }
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let s = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::HostSweep)
        .unwrap();
    assert_eq!(s.source, "PC01");
    assert!(s.detail.contains("6 adresses"), "{}", s.detail);
    assert!(!a.has(Behavior::PortScan), "même port, cibles différentes");
}

#[test]
fn an_ip_claimed_by_two_macs_is_an_arp_conflict() {
    let mut sim = netiv_topology::load_yaml(
        r#"
network: { name: dup }
devices:
  - { name: A, type: host, interfaces: [{ name: eth0, ip: 10.0.0.50/24 }] }
  - { name: B, type: host, interfaces: [{ name: eth0, ip: 10.0.0.50/24 }] }
  - { name: C, type: host, interfaces: [{ name: eth0, ip: 10.0.0.2/24 }] }
  - { name: SW, type: switch, ports: 3 }
links:
  - { a: "A:eth0", b: "SW:port1" }
  - { a: "B:eth0", b: "SW:port2" }
  - { a: "C:eth0", b: "SW:port3" }
"#,
    )
    .unwrap()
    .sim;
    let c = sim.device_id("C").unwrap();
    sim.ping(c, ip("10.0.0.50"), 1).unwrap();
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let f = a
        .findings
        .iter()
        .find(|f| f.behavior == Behavior::ArpConflict)
        .expect("conflit ARP");
    assert_eq!(f.level, Level::High);
    assert!(
        f.detail.contains("10.0.0.50") && f.detail.contains("2 adresses MAC"),
        "{}",
        f.detail
    );
}

#[test]
fn normal_traffic_raises_no_alarm() {
    let mut sim = cyber();
    let ok =
        |sim: &mut Simulation, at, from: &str, dst: &str, port| curl_at(sim, at, from, dst, port);
    ok(&mut sim, 1_000, "PC01", "10.9.0.10", 80);
    ok(&mut sim, 2_000, "PC02", "192.168.20.20", 445);
    ok(&mut sim, 3_000, "INET", "10.9.0.10", 80);
    ok(&mut sim, 4_000, "PC01", "203.0.113.10", 80);
    let pc = sim.device_id("PC01").unwrap();
    sim.schedule_command(
        SimTime::from_millis(5_000),
        pc,
        Command::Ping {
            dst: ip("192.168.20.20"),
            count: 3,
        },
    )
    .unwrap();
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let suspicious: Vec<_> = a.suspicious().collect();
    assert!(suspicious.is_empty(), "faux positifs : {suspicious:?}");
    assert!(a.render(&cap).contains("rien de suspect détecté"));
}

#[test]
fn configuration_changes_appear_on_the_timeline_but_not_as_suspicious() {
    let mut sim = cyber();
    sim.clear_trace();
    let pc = sim.device_id("PC01").unwrap();
    sim.set_gateway(pc, Some(ip("192.168.10.1"))).unwrap();
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    assert!(a.has(Behavior::ConfigChange));
    assert_eq!(a.suspicious().count(), 0);
    assert!(
        a.timeline
            .iter()
            .any(|(_, l)| l.starts_with("CONFIG PC01 : passerelle"))
    );
}

#[test]
fn rendering_has_a_timeline_and_an_anomaly_section() {
    let mut sim = cyber();
    open_dmz_to_lan(&mut sim);
    for (i, port) in [22u16, 23, 25, 80, 445, 3389].iter().enumerate() {
        curl_at(
            &mut sim,
            5_000 + i as u64 * 500,
            "WEB01",
            "192.168.20.20",
            *port,
        );
    }
    let cap = capture(&mut sim);
    let a = analyze_capture(&cap);
    let text = a.render(&cap);
    assert!(
        text.contains("CHRONOLOGIE") && text.contains("ANOMALIES"),
        "{text}"
    );
    assert!(
        text.contains("00:00:05.000  WEB01 → FILE01 (192.168.20.20) tcp/22 : réussi"),
        "{text}"
    );
    assert!(text.contains("tcp/23 : échec"), "{text}");
    assert!(text.contains("[HIGH] 00:00:05.000–00:00:07.500"), "{text}");
    let packets = Analysis::render_packets(&cap);
    assert!(packets.contains("WEB01/eth0  TCP [SYN]"), "{packets}");
    assert!(packets.contains("ARP Request"), "{packets}");
}

#[test]
fn clock_formatting() {
    assert_eq!(clock(SimTime::ZERO), "00:00:00.000");
    assert_eq!(clock(SimTime::from_millis(61_250)), "00:01:01.250");
    assert_eq!(clock(SimTime::from_millis(3_600_000 + 5)), "01:00:00.005");
}

#[test]
fn captures_roundtrip_through_files_and_bad_files_are_refused() {
    let mut sim = cyber();
    curl_at(&mut sim, 1_000, "PC01", "10.9.0.10", 80);
    let cap = capture(&mut sim);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.nlv");
    cap.save(&path).unwrap();
    let back = Capture::load(&path).unwrap();
    assert_eq!(back, cap);
    // L'analyse d'une capture rechargée est identique.
    assert_eq!(
        analyze_capture(&back).render(&back),
        analyze_capture(&cap).render(&cap)
    );

    let bad = dir.path().join("bad.nlv");
    std::fs::write(&bad, "pas du json").unwrap();
    assert!(
        Capture::load(&bad)
            .unwrap_err()
            .to_string()
            .contains("illisible")
    );
    assert!(Capture::load(&dir.path().join("absent.nlv")).is_err());

    let mut future = cap.clone();
    future.format_version = 99;
    let p = dir.path().join("future.nlv");
    future.save(&p).unwrap();
    assert!(
        Capture::load(&p)
            .unwrap_err()
            .to_string()
            .contains("version 99")
    );
}

#[test]
fn forensics_is_deterministic() {
    let run = || {
        let mut sim = cyber();
        open_dmz_to_lan(&mut sim);
        for (i, port) in [21u16, 22, 23, 25, 80, 135].iter().enumerate() {
            curl_at(
                &mut sim,
                1_000 + i as u64 * 200,
                "WEB01",
                "192.168.20.20",
                *port,
            );
        }
        let cap = capture(&mut sim);
        analyze_capture(&cap).render(&cap)
    };
    assert_eq!(run(), run());
}
