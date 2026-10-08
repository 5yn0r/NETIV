#![allow(clippy::unwrap_used, clippy::expect_used)]

//! What-if, score de santé et documentation sur des réseaux dont on connaît les défauts.

use netiv_analysis::docs::{DocFormat, build_doc};
use netiv_analysis::whatif::Change;
use netiv_analysis::{HealthReport, generate_docs, health, what_if};
use netiv_core::acl::RuleProto;
use netiv_core::packet::L4Proto;
use netiv_core::{Fault, LinkId, Simulation};

fn load(yaml: &str) -> Simulation {
    netiv_topology::load_yaml(yaml).unwrap().sim
}

fn office() -> Simulation {
    load(include_str!(
        "../../netiv-education/content/topologies/office.yaml"
    ))
}

fn mvp() -> Simulation {
    load(include_str!("../../../examples/mvp.yaml"))
}

fn cyber() -> Simulation {
    load(include_str!("../../../examples/cyber.yaml"))
}

fn dev(sim: &Simulation, n: &str) -> netiv_core::DeviceId {
    sim.device_id(n).unwrap()
}

// ================================================================ what-if

#[test]
fn what_if_phrases_are_understood() {
    assert_eq!(
        Change::parse("R1 goes down"),
        Ok(Change::DeviceDown("R1".into()))
    );
    assert_eq!(
        Change::parse("r1 DOWN"),
        Ok(Change::DeviceDown("r1".into()))
    );
    assert_eq!(
        Change::parse("disconnect SW2"),
        Ok(Change::Disconnect("SW2".into()))
    );
    assert_eq!(
        Change::parse("block TCP/443"),
        Ok(Change::Block {
            proto: RuleProto::Tcp,
            port: Some(443),
            on: None
        })
    );
    assert_eq!(
        Change::parse("block icmp on R1"),
        Ok(Change::Block {
            proto: RuleProto::Icmp,
            port: None,
            on: Some("R1".into())
        })
    );
    assert_eq!(
        Change::parse("remove route 10.10.0.0/16"),
        Ok(Change::RemoveRoute {
            dest: "10.10.0.0/16".parse().unwrap(),
            on: None
        })
    );
    assert_eq!(
        Change::parse("remove route 10.10.0.0/16 on R2"),
        Ok(Change::RemoveRoute {
            dest: "10.10.0.0/16".parse().unwrap(),
            on: Some("R2".into())
        })
    );
    assert_eq!(
        Change::parse("cut link R1:eth1"),
        Ok(Change::CutLink("R1:eth1".into()))
    );
    assert_eq!(
        Change::parse("stop tcp/80 on SERVER"),
        Ok(Change::StopService {
            proto: L4Proto::Tcp,
            port: 80,
            on: "SERVER".into()
        })
    );
    for bad in [
        "",
        "explode everything",
        "block",
        "block tcp/abc",
        "block sctp/1",
        "block icmp/5",
        "stop tcp on X",
        "remove route nope",
        "stop udp/53",
        "cut link",
    ] {
        assert!(Change::parse(bad).is_err(), "{bad:?} aurait dû être refusé");
    }
    assert!(Change::parse("???").unwrap_err().contains("Exemples"));
}

#[test]
fn a_router_failure_isolates_both_sides() {
    let sim = mvp();
    let i = what_if(&sim, "R1 goes down").unwrap();
    assert_eq!(i.flows, 2, "PC1 → SERVER et SERVER → PC1 (pings)");
    assert_eq!((i.ok_before, i.ok_after), (2, 0));
    assert_eq!(i.affected_hosts, ["PC1", "SERVER"]);
    assert_eq!(i.isolated_hosts, ["PC1", "SERVER"]);
    assert_eq!(
        i.broken_routes, 1,
        "la route statique de R1 devient inutilisable"
    );
    assert_eq!((i.segments_before, i.segments_after), (1, 2));
    assert_eq!(i.isolated_segments(), 1);
    assert!(!i.is_harmless());
    let text = i.render();
    assert!(
        text.starts_with("ANALYSE D'IMPACT — R1 goes down"),
        "{text}"
    );
    assert!(text.contains("PC1 → SERVER (ping)"), "{text}");
}

#[test]
fn blocking_a_port_breaks_only_that_service() {
    let sim = office();
    let i = what_if(&sim, "block tcp/80").unwrap();
    assert_eq!(i.affected_services, ["tcp/80 sur SERVER"]);
    assert!(
        i.broken.iter().all(|f| f.contains("tcp/80")),
        "{:?}",
        i.broken
    );
    assert_eq!(i.broken.len(), 2, "PC1 et PC2 perdent le web");
    assert!(
        i.isolated_hosts.is_empty(),
        "le ping et le ssh fonctionnent encore"
    );
    assert_eq!(i.broken_routes, 0);
    assert_eq!(i.isolated_segments(), 0);
}

#[test]
fn blocking_on_one_device_and_a_harmless_change() {
    let sim = office();
    // R1 bloque le ssh : les deux postes perdent tcp/22 mais pas tcp/80.
    let i = what_if(&sim, "block tcp/22 on R1").unwrap();
    assert!(i.broken.iter().all(|f| f.contains("tcp/22")));
    // Un port que personne n'utilise : aucun impact.
    let h = what_if(&sim, "block udp/9999").unwrap();
    assert!(h.is_harmless(), "{}", h.render());
    assert!(h.render().contains("Aucun impact"));
}

#[test]
fn disconnecting_a_switch_creates_isolated_segments() {
    let sim = office();
    let i = what_if(&sim, "disconnect SW1").unwrap();
    assert_eq!(i.segments_before, 1);
    assert_eq!(i.segments_after, 3, "PC1, PC2 et le reste");
    assert_eq!(i.isolated_segments(), 2);
    assert!(
        i.isolated_hosts.contains(&"PC1".to_string())
            && i.isolated_hosts.contains(&"PC2".to_string())
    );
    assert!(!i.isolated_hosts.contains(&"SERVER".to_string()) || i.ok_after == 0);
}

#[test]
fn route_and_link_and_service_scenarios() {
    let sim = mvp();
    let r = what_if(&sim, "remove route 192.168.2.0/24").unwrap();
    assert_eq!(r.broken_routes, 1);
    assert!(!r.broken.is_empty());
    let c = what_if(&sim, "cut link R1:eth1").unwrap();
    assert_eq!((c.ok_before, c.ok_after), (2, 0));
    let s = what_if(&office(), "stop tcp/80 on SERVER").unwrap();
    assert_eq!(s.affected_services, ["tcp/80 sur SERVER"]);
    assert_eq!(s.broken.len(), 2);
}

#[test]
fn what_if_errors_are_clear() {
    let sim = mvp();
    assert!(
        what_if(&sim, "NOPE goes down")
            .unwrap_err()
            .contains("équipement inconnu")
    );
    assert!(
        what_if(&sim, "remove route 10.77.0.0/16")
            .unwrap_err()
            .contains("aucun équipement")
    );
    assert!(
        what_if(&sim, "cut link R1:eth9")
            .unwrap_err()
            .contains("eth9")
    );
    assert!(
        what_if(&sim, "cut link PC1")
            .unwrap_err()
            .contains("Équipement:interface")
    );
}

#[test]
fn what_if_never_modifies_the_network_and_is_deterministic() {
    let mut sim = office();
    sim.add_service(
        dev(&sim, "SERVER"),
        netiv_core::service::Service::udp(53, "dns"),
    )
    .unwrap();
    let fp = sim.config_fingerprint();
    let (trace, pending) = (sim.trace().len(), sim.pending());
    let a = what_if(&sim, "R1 goes down").unwrap();
    let b = what_if(&sim, "R1 goes down").unwrap();
    assert_eq!(a, b);
    assert_eq!(sim.config_fingerprint(), fp);
    assert_eq!((sim.trace().len(), sim.pending()), (trace, pending));
}

// ================================================================ santé

fn weighted(h: &HealthReport) -> u32 {
    let total: u32 = h.categories.iter().map(|c| c.weight).sum();
    let sum: u32 = h.categories.iter().map(|c| c.score * c.weight).sum();
    (sum + total / 2) / total
}

#[test]
fn a_well_designed_network_scores_high_with_availability_as_the_weak_point() {
    let h = health(&cyber());
    for name in ["Connectivity", "Addressing", "Routing", "Segmentation"] {
        assert_eq!(h.score_of(name), Some(100), "{name} : {}", h.render());
    }
    assert_eq!(h.score_of("Security"), Some(90));
    assert_eq!(
        h.score_of("Availability"),
        Some(64),
        "FW1 (−20), SWD (−8), SWL (−8)"
    );
    assert_eq!(h.global(), weighted(&h));
    assert_eq!(h.global(), 94);
    let text = h.render();
    assert!(text.starts_with("NETWORK HEALTH"), "{text}");
    assert!(text.contains("GLOBAL SCORE   94/100"), "{text}");
    assert!(
        text.contains("FW1 est un point unique de défaillance"),
        "{text}"
    );
}

#[test]
fn categories_and_weights_follow_the_documented_model() {
    let h = health(&mvp());
    let names: Vec<&str> = h.categories.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "Connectivity",
            "Addressing",
            "Routing",
            "Segmentation",
            "Availability",
            "Security"
        ]
    );
    assert_eq!(h.categories.iter().map(|c| c.weight).sum::<u32>(), 100);
    assert!(h.categories.iter().all(|c| c.score <= 100));
    assert_eq!(h.global(), weighted(&h));
    // mvp : filtrage absent entre postes et serveurs, aucune redondance.
    assert_eq!(h.score_of("Segmentation"), Some(80));
    assert_eq!(h.score_of("Availability"), Some(52));
}

#[test]
fn a_cut_cable_hurts_connectivity_and_availability() {
    let mut sim = mvp();
    let good = health(&sim);
    sim.inject_fault(Fault::LinkDown { link: LinkId(2) }, false)
        .unwrap();
    let bad = health(&sim);
    assert_eq!(
        bad.score_of("Connectivity"),
        Some(0),
        "aucune des 2 paires ne passe"
    );
    assert!(bad.score_of("Availability") < good.score_of("Availability"));
    assert!(bad.global() < good.global());
    let c = bad
        .categories
        .iter()
        .find(|c| c.name == "Connectivity")
        .unwrap();
    assert!(c.notes[0].contains("2 paire(s) sur 2"), "{:?}", c.notes);
    let a = bad
        .categories
        .iter()
        .find(|c| c.name == "Availability")
        .unwrap();
    assert!(a.notes.iter().any(|n| n.contains("coupé")), "{:?}", a.notes);
    // Réparer ramène le score d'avant.
    sim.repair_all_faults(false);
    assert_eq!(health(&sim), good);
}

#[test]
fn addressing_problems_are_counted() {
    let sim = load(
        r#"
network: { name: dup }
devices:
  - { name: A, type: host, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }], gateway: 10.9.9.9 }
  - { name: B, type: host, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }] }
links: [{ a: "A:eth0", b: "B:eth0" }]
"#,
    );
    let h = health(&sim);
    assert_eq!(
        h.score_of("Addressing"),
        Some(60),
        "IP en double (−20) et passerelle hors réseau (−20)"
    );
    let a = h
        .categories
        .iter()
        .find(|c| c.name == "Addressing")
        .unwrap();
    assert_eq!(a.notes.len(), 2, "{:?}", a.notes);
}

#[test]
fn a_missing_return_route_breaks_connectivity_in_both_directions() {
    // Un ping exige l'aller ET le retour : sans route de retour, les deux
    // sens échouent. Les routes configurées restent, elles, utilisables.
    let mut sim = mvp();
    sim.inject_fault(
        Fault::RemoveRoutes {
            device: dev(&sim, "R2"),
            dest: "192.168.1.0/24".parse().unwrap(),
        },
        false,
    )
    .unwrap();
    let h = health(&sim);
    assert_eq!(h.score_of("Connectivity"), Some(0), "{}", h.render());
    assert_eq!(
        h.score_of("Routing"),
        Some(100),
        "toutes les routes restantes sont utilisables"
    );
}

#[test]
fn an_unusable_static_route_lowers_the_routing_score() {
    let mut sim = mvp();
    sim.inject_fault(
        Fault::Multi(vec![
            Fault::RemoveRoutes {
                device: dev(&sim, "R1"),
                dest: "192.168.2.0/24".parse().unwrap(),
            },
            Fault::AddRoute {
                device: dev(&sim, "R1"),
                route: netiv_core::StaticRoute {
                    dest: "192.168.2.0/24".parse().unwrap(),
                    via: "172.31.0.1".parse().unwrap(),
                },
            },
        ]),
        false,
    )
    .unwrap();
    let h = health(&sim);
    let r = h.categories.iter().find(|c| c.name == "Routing").unwrap();
    // PC1 et R2 gardent des routes valides ; celle de R1 est inutilisable : 1 sur 4.
    assert!(r.score < 100, "{}", h.render());
    assert!(r.notes[0].contains("inutilisable"), "{:?}", r.notes);
}

#[test]
fn a_flat_network_has_poor_segmentation() {
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
    let h = health(&sim);
    assert_eq!(
        h.score_of("Segmentation"),
        Some(25),
        "SEC-001 (−40) et SEC-002 (−35)"
    );
    assert_eq!(h.score_of("Connectivity"), Some(100));
}

#[test]
fn health_is_deterministic_and_does_not_touch_the_network() {
    let sim = cyber();
    let fp = sim.config_fingerprint();
    assert_eq!(health(&sim), health(&sim));
    assert_eq!(sim.config_fingerprint(), fp);
    assert!(sim.trace().is_empty());
}

// ================================================================ documentation

#[test]
fn markdown_documentation_covers_every_section() {
    let md = generate_docs(&cyber(), "cyber", DocFormat::Markdown).unwrap();
    for section in [
        "# Documentation réseau — cyber",
        "## Topologie",
        "## Plan d'adressage",
        "## Routage",
        "## Équipements",
        "## Filtrage",
        "## Recommandations de sécurité",
    ] {
        assert!(md.contains(section), "section manquante : {section}\n{md}");
    }
    assert!(
        md.contains("| FW1 | eth1 | 10.9.0.1/24 | 10.9.0.0/24 | SWD:port1 |"),
        "{md}"
    );
    assert!(md.contains("### FW1 (politique par défaut : deny)"), "{md}");
    assert!(
        md.contains("1. allow tcp any → 10.9.0.0/24 port 80 depuis eth0 — Internet vers web DMZ"),
        "{md}"
    );
    assert!(md.contains("| WEB01 | server | dmz |"), "{md}");
    assert!(md.contains("SEC-011"), "{md}");
    // Les ports de switch sont toujours listés ; sans configuration, ils sont en VLAN 1.
    assert!(
        md.contains("| SWD | port2 | accès VLAN 1 | WEB01:eth0 |"),
        "{md}"
    );
}

#[test]
fn a_vlan_table_appears_when_switches_have_vlans() {
    let sim = load(include_str!(
        "../../netiv-education/content/topologies/campus.yaml"
    ));
    let md = generate_docs(&sim, "campus", DocFormat::Markdown).unwrap();
    assert!(md.contains("## VLAN et ports de switch"), "{md}");
    assert!(
        md.contains("| SW1 | p1 | accès VLAN 10 | PC1:eth0 |"),
        "{md}"
    );
    assert!(
        md.contains("| SW1 | tr | trunk (VLAN 10,20) natif 1 | SW2:tr |"),
        "{md}"
    );
}

#[test]
fn json_and_yaml_documentation_parse_back_to_the_same_document() {
    let sim = mvp();
    let doc = build_doc(&sim, "mvp");
    let json = generate_docs(&sim, "mvp", DocFormat::Json).unwrap();
    let back: netiv_analysis::NetworkDoc = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
    let yaml = generate_docs(&sim, "mvp", DocFormat::Yaml).unwrap();
    let back: netiv_analysis::NetworkDoc = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(back, doc);
    assert_eq!(doc.devices.len(), 5);
    assert_eq!(doc.links.len(), 4);
    let r1 = doc.devices.iter().find(|d| d.name == "R1").unwrap();
    assert_eq!(
        r1.routes.len(),
        3,
        "deux réseaux connectés + une route statique"
    );
}

#[test]
fn formats_are_parsed_and_pdf_is_explained() {
    assert_eq!("md".parse::<DocFormat>(), Ok(DocFormat::Markdown));
    assert_eq!("JSON".parse::<DocFormat>(), Ok(DocFormat::Json));
    assert_eq!("yml".parse::<DocFormat>(), Ok(DocFormat::Yaml));
    let e = "pdf".parse::<DocFormat>().unwrap_err();
    assert!(e.contains("pandoc"), "{e}");
    assert!(
        "docx"
            .parse::<DocFormat>()
            .unwrap_err()
            .contains("md, json, yaml")
    );
}

#[test]
fn documentation_is_deterministic() {
    assert_eq!(
        generate_docs(&cyber(), "c", DocFormat::Markdown).unwrap(),
        generate_docs(&cyber(), "c", DocFormat::Markdown).unwrap()
    );
}
