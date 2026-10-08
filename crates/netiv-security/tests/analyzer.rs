#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Le Security Analyzer sur des réseaux dont on connaît les défauts.

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::{Fault, Simulation};
use netiv_security::{Severity, analyze};

fn load(yaml: &str) -> Simulation {
    netiv_topology::load_yaml(yaml).unwrap().sim
}

fn cyber() -> Simulation {
    load(include_str!("../../../examples/cyber.yaml"))
}

fn dev(sim: &Simulation, n: &str) -> netiv_core::DeviceId {
    sim.device_id(n).unwrap()
}

fn ids(sim: &Simulation) -> Vec<&'static str> {
    analyze(sim).findings.iter().map(|f| f.id).collect()
}

#[test]
fn the_reference_architecture_only_has_the_unavoidable_redundancy_warning() {
    let sim = cyber();
    let report = analyze(&sim);
    assert_eq!(ids(&sim), ["SEC-011"], "{}", report.render());
    assert_eq!(report.get("SEC-011").unwrap().severity, Severity::Medium);
    assert_eq!(report.counts(), (0, 1, 0));
    assert_eq!(report.score(), 90);
}

#[test]
fn a_flat_network_is_flagged_for_missing_segmentation_and_mixed_servers() {
    let sim = load(
        r#"
network: { name: flat }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }] }
  - { name: PC2, type: host, interfaces: [{ name: eth0, ip: 10.0.0.2/24 }] }
  - { name: SRV, type: server, interfaces: [{ name: eth0, ip: 10.0.0.10/24 }] }
  - { name: SW, type: switch, ports: 3 }
links:
  - { a: "PC1:eth0", b: "SW:port1" }
  - { a: "PC2:eth0", b: "SW:port2" }
  - { a: "SRV:eth0", b: "SW:port3" }
"#,
    );
    let r = analyze(&sim);
    assert!(r.has("SEC-001"), "{}", r.render());
    assert_eq!(r.get("SEC-001").unwrap().severity, Severity::Medium);
    let isolation = r.get("SEC-002").expect("serveur mêlé aux postes");
    assert_eq!(isolation.severity, Severity::High);
    for who in ["SRV", "PC1", "PC2"] {
        assert!(
            isolation.devices.contains(&who.to_string()),
            "{isolation:?}"
        );
    }
}

#[test]
fn vlans_fix_the_mixed_network() {
    let mut sim = load(
        r#"
network: { name: seg }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 10.10.0.2/24 }], gateway: 10.10.0.1 }
  - { name: PC2, type: host, interfaces: [{ name: eth0, ip: 10.10.0.3/24 }], gateway: 10.10.0.1 }
  - { name: SRV, type: server, interfaces: [{ name: eth0, ip: 10.20.0.2/24 }], gateway: 10.20.0.1 }
  - name: SW
    type: switch
    interfaces:
      - { name: p1, vlan: 10 }
      - { name: p2, vlan: 10 }
      - { name: p3, vlan: 20 }
      - { name: r10, vlan: 10 }
      - { name: r20, vlan: 20 }
  - name: FW
    type: firewall
    interfaces: [{ name: eth0, ip: 10.10.0.1/24 }, { name: eth1, ip: 10.20.0.1/24 }]
    acl:
      rules: [{ action: allow, proto: tcp, port: "443", in: eth0 }]
links:
  - { a: "PC1:eth0", b: "SW:p1" }
  - { a: "PC2:eth0", b: "SW:p2" }
  - { a: "SRV:eth0", b: "SW:p3" }
  - { a: "FW:eth0", b: "SW:r10" }
  - { a: "FW:eth1", b: "SW:r20" }
"#,
    );
    let r = analyze(&sim);
    assert!(!r.has("SEC-001") && !r.has("SEC-002"), "{}", r.render());
    // Le pare-feu filtre : pas d'alerte de chemin ouvert non plus.
    assert!(!r.has("SEC-003"), "{}", r.render());

    // Sans filtrage entre les VLAN, l'alerte apparaît.
    sim.acl_set_default(dev(&sim, "FW"), AclAction::Allow)
        .unwrap();
    sim.acl_remove_rule(dev(&sim, "FW"), 0).unwrap();
    let srv = dev(&sim, "SRV");
    sim.add_service(srv, netiv_core::service::Service::tcp(80, "http"))
        .unwrap();
    assert!(analyze(&sim).has("SEC-003"));
}

#[test]
fn unfiltered_routing_between_users_and_servers_is_reported() {
    let sim = load(include_str!("../../../examples/mvp.yaml"));
    let r = analyze(&sim);
    let f = r.get("SEC-003").unwrap_or_else(|| panic!("{}", r.render()));
    assert_eq!(f.severity, Severity::Medium);
    assert!(f.devices.contains(&"SERVER".to_string()) && f.devices.contains(&"PC1".to_string()));
    assert!(!r.has("SEC-002"), "ils ne sont pas dans le même réseau");
}

#[test]
fn an_exposed_management_interface_is_high_and_filtering_removes_it() {
    let yaml = r#"
network: { name: mgmt }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 10.0.0.2/24 }], gateway: 10.0.0.1 }
  - name: R1
    type: router
    interfaces:
      - { name: eth0, ip: 10.0.0.1/24 }
      - { name: mgmt, ip: 10.99.0.1/24, management: true }
    services: [{ proto: tcp, port: 22, name: ssh }]
links:
  - { a: "PC1:eth0", b: "R1:eth0" }
"#;
    let mut sim = load(yaml);
    // Les routes de retour existent (réseaux connectés) : PC1 joint 10.99.0.1 via R1.
    let r = analyze(&sim);
    let f = r.get("SEC-004").unwrap_or_else(|| panic!("{}", r.render()));
    assert_eq!(f.severity, Severity::High);
    assert!(f.devices.contains(&"R1".to_string()) && f.devices.contains(&"PC1".to_string()));

    // Une règle bloquant l'accès à l'administration depuis le réseau utilisateurs règle le problème.
    let mut deny = AclRule::new(AclAction::Deny, RuleProto::Any);
    deny.dst = Some("10.99.0.0/24".parse().unwrap());
    sim.acl_add_rule(dev(&sim, "R1"), deny).unwrap();
    assert!(!analyze(&sim).has("SEC-004"), "{}", analyze(&sim).render());
}

#[test]
fn dmz_to_lan_access_is_detected_then_fixed() {
    let mut sim = cyber();
    let fw = dev(&sim, "FW1");
    let mut hole = AclRule::new(AclAction::Allow, RuleProto::Any);
    hole.src = Some("10.9.0.0/24".parse().unwrap());
    hole.dst = Some("192.168.10.0/24".parse().unwrap());
    sim.inject_fault(
        Fault::AclInsert {
            device: fw,
            rule: hole,
        },
        false,
    )
    .unwrap();

    let r = analyze(&sim);
    let f = r.get("SEC-006").unwrap_or_else(|| panic!("{}", r.render()));
    assert_eq!(f.severity, Severity::High);
    for who in ["WEB01", "WEB02", "PC01", "PC02"] {
        assert!(f.devices.contains(&who.to_string()), "{f:?}");
    }
    sim.repair_all_faults(false);
    assert!(!analyze(&sim).has("SEC-006"));
}

#[test]
fn internet_to_internal_access_is_detected() {
    let mut sim = cyber();
    let fw = dev(&sim, "FW1");
    let mut hole = AclRule::new(AclAction::Allow, RuleProto::Any);
    hole.in_iface = Some(sim.iface_id(fw, "eth0").unwrap());
    sim.inject_fault(
        Fault::AclInsert {
            device: fw,
            rule: hole,
        },
        false,
    )
    .unwrap();
    let r = analyze(&sim);
    let f = r.get("SEC-005").unwrap_or_else(|| panic!("{}", r.render()));
    assert_eq!(f.severity, Severity::High);
    assert!(f.devices.contains(&"INET".to_string()) && f.devices.contains(&"FILE01".to_string()));
    // L'ouverture "any" est aussi signalée comme règle trop permissive... seulement si src/dst sont absents.
    assert!(r.has("SEC-007"), "{}", r.render());
}

#[test]
fn permissive_rules_and_open_firewalls_are_flagged() {
    let mut sim = cyber();
    let fw = dev(&sim, "FW1");
    sim.acl_set_default(fw, AclAction::Allow).unwrap();
    let r = analyze(&sim);
    let f = r.get("SEC-008").unwrap_or_else(|| panic!("{}", r.render()));
    assert_eq!(f.severity, Severity::High);
    assert_eq!(f.devices, ["FW1"]);
    assert!(f.recommendation.contains("acl default"));

    let sim2 = cyber();
    let mut sim2 = sim2;
    sim2.acl_add_rule(
        dev(&sim2, "FW1"),
        AclRule::new(AclAction::Allow, RuleProto::Any),
    )
    .unwrap();
    let r2 = analyze(&sim2);
    let f = r2
        .get("SEC-007")
        .unwrap_or_else(|| panic!("{}", r2.render()));
    assert!(f.detail.contains("FW1 (règle 6)"), "{}", f.detail);
}

#[test]
fn cleartext_services_and_wide_trunks_are_flagged() {
    let sim = load(
        r#"
network: { name: x }
devices:
  - name: SRV
    type: server
    interfaces: [{ name: eth0, ip: 10.0.0.2/24 }]
    services: [{ proto: tcp, port: 23, name: telnet }, { proto: udp, port: 161, name: snmp }, { proto: tcp, port: 22, name: ssh }]
  - name: S1
    type: switch
    interfaces: [{ name: a, vlan: 10 }, { name: tr, trunk: [] }, { name: ok, trunk: [10] }]
links:
  - { a: "SRV:eth0", b: "S1:a" }
"#,
    );
    let r = analyze(&sim);
    let c = r.get("SEC-009").unwrap_or_else(|| panic!("{}", r.render()));
    assert!(
        c.detail.contains("telnet") && c.detail.contains("snmp") && !c.detail.contains("ssh"),
        "{}",
        c.detail
    );
    let t = r.get("SEC-010").unwrap();
    assert_eq!(t.severity, Severity::Low);
    assert_eq!(t.devices, ["S1/tr"]);
}

#[test]
fn redundancy_removes_a_device_from_the_single_points_of_failure() {
    let yaml = |with_backup: bool| {
        let backup = if with_backup {
            r#"
  - name: R3
    type: router
    interfaces: [{ name: eth0, ip: 10.13.0.2/30 }, { name: eth1, ip: 10.23.0.2/30 }]
"#
        } else {
            ""
        };
        let links = if with_backup {
            r#"
  - { a: "R1:eth2", b: "R3:eth0" }
  - { a: "R2:eth2", b: "R3:eth1" }
"#
        } else {
            ""
        };
        format!(
            r#"
network: {{ name: ring }}
devices:
  - {{ name: PC1, type: host, interfaces: [{{ name: eth0, ip: 10.1.0.2/24 }}], gateway: 10.1.0.1 }}
  - {{ name: SRV, type: server, interfaces: [{{ name: eth0, ip: 10.2.0.2/24 }}], gateway: 10.2.0.1 }}
  - name: R1
    type: router
    interfaces: [{{ name: eth0, ip: 10.1.0.1/24 }}, {{ name: eth1, ip: 10.12.0.1/30 }}, {{ name: eth2, ip: 10.13.0.1/30 }}]
  - name: R2
    type: router
    interfaces: [{{ name: eth0, ip: 10.2.0.1/24 }}, {{ name: eth1, ip: 10.12.0.2/30 }}, {{ name: eth2, ip: 10.23.0.1/30 }}]
{backup}
links:
  - {{ a: "PC1:eth0", b: "R1:eth0" }}
  - {{ a: "SRV:eth0", b: "R2:eth0" }}
  - {{ a: "R1:eth1", b: "R2:eth1" }}
{links}
"#
        )
    };
    let single = load(&yaml(false));
    let f = analyze(&single);
    let spof = f.get("SEC-011").unwrap();
    assert!(spof.devices.contains(&"R1".to_string()) && spof.devices.contains(&"R2".to_string()));

    let ring = load(&yaml(true));
    let f = analyze(&ring);
    let spof = f
        .get("SEC-011")
        .expect("R1 et R2 restent uniques pour PC1 et SRV");
    assert!(
        !spof.devices.contains(&"R3".to_string()),
        "R3 n'est pas un point unique : {:?}",
        spof.devices
    );
}

#[test]
fn a_cleaner_network_scores_higher() {
    let mut sim = cyber();
    let clean = analyze(&sim).score();
    sim.acl_set_default(dev(&sim, "FW1"), AclAction::Allow)
        .unwrap();
    let dirty = analyze(&sim);
    assert!(dirty.score() < clean, "{} vs {clean}", dirty.score());
    let (h, _, _) = dirty.counts();
    assert!(h >= 1);
    // High d'abord dans le rapport.
    assert_eq!(dirty.findings[0].severity, Severity::High);
}

#[test]
fn the_report_is_readable_and_the_analysis_deterministic() {
    let mut sim = cyber();
    sim.acl_set_default(dev(&sim, "FW1"), AclAction::Allow)
        .unwrap();
    let a = analyze(&sim);
    assert_eq!(a, analyze(&sim));
    let text = a.render();
    assert!(text.starts_with("ANALYSE DE SÉCURITÉ —"), "{text}");
    assert!(text.contains("[HIGH] SEC-008"), "{text}");
    assert!(text.contains("→ "), "{text}");
    let clean = Report::default_for_tests();
    assert!(clean.render().contains("rien à signaler"));
}

// Petit utilitaire pour tester le rapport vide sans construire un réseau.
trait Empty {
    fn default_for_tests() -> netiv_security::Report;
}
struct Report;
impl Empty for Report {
    fn default_for_tests() -> netiv_security::Report {
        netiv_security::Report { findings: vec![] }
    }
}

#[test]
fn analysis_does_not_modify_the_network() {
    let mut sim = cyber();
    sim.acl_set_default(dev(&sim, "FW1"), AclAction::Allow)
        .unwrap();
    let fp = sim.config_fingerprint();
    let (trace, pending) = (sim.trace().len(), sim.pending());
    let _ = analyze(&sim);
    assert_eq!(sim.config_fingerprint(), fp);
    assert_eq!((sim.trace().len(), sim.pending()), (trace, pending));
}
