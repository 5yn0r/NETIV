#![allow(clippy::unwrap_used, clippy::expect_used)]

//! DHCP et DNS dans le format de topologie : lecture, export, validation.

use netiv_topology::{TopologyDef, build, export, load_yaml};

const DHCP: &str = include_str!("../../netiv-education/content/topologies/dhcp.yaml");

#[test]
fn the_dhcp_network_boots_and_every_client_gets_an_address() {
    let loaded = load_yaml(DHCP).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let sim = &loaded.sim;
    for pc in ["PC1", "PC2", "PC3"] {
        let d = sim.device(sim.device_id(pc).unwrap()).unwrap();
        assert!(d.ifaces[0].ip.is_some(), "{pc} sans adresse");
    }
    // Le démarrage DHCP est visible dans la trace.
    assert!(
        sim.trace()
            .events()
            .iter()
            .any(|e| matches!(e.kind, netiv_core::TraceKind::Dhcp { .. }))
    );
}

#[test]
fn export_keeps_dhcp_and_dns_configuration_not_the_leases() {
    let loaded = load_yaml(DHCP).unwrap();
    let def = export(&loaded.sim, "dhcp");
    let dev = |n: &str| def.devices.iter().find(|d| d.name == n).unwrap();
    // Le poste est « dhcp », pas l'adresse obtenue, et sans passerelle écrite.
    assert_eq!(dev("PC1").interfaces[0].ip.as_deref(), Some("dhcp"));
    assert_eq!(dev("PC1").gateway, None);
    assert_eq!(
        dev("IMP").interfaces[0].ip.as_deref(),
        Some("10.10.1.50/24")
    );
    assert_eq!(
        dev("R1").interfaces[0].dhcp_relay.as_deref(),
        Some("10.10.2.10")
    );
    let srv = dev("SRV");
    assert_eq!(srv.dhcp.as_ref().unwrap().pools.len(), 2);
    assert_eq!(
        srv.dhcp.as_ref().unwrap().pools[0].range,
        "10.10.1.100-10.10.1.150"
    );
    assert_eq!(
        srv.dns_server
            .as_ref()
            .unwrap()
            .records
            .get("web.lan")
            .map(String::as_str),
        Some("10.10.2.10")
    );
    // Aller-retour YAML : même réseau, même configuration.
    let yaml = serde_yaml_ng::to_string(&def).unwrap();
    let back: TopologyDef = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(back, def);
    let rebuilt = build(&back).unwrap();
    assert_eq!(
        rebuilt.sim.config_fingerprint(),
        loaded.sim.config_fingerprint()
    );
}

#[test]
fn dhcp_and_dns_mistakes_are_reported() {
    let bad = r#"
network: { name: x }
devices:
  - name: SW
    type: switch
    ports: 2
    dhcp: { pools: [{ network: 10.0.0.0/24, range: 10.0.0.10-10.0.0.20 }] }
  - name: H
    type: host
    interfaces: [{ name: eth0, ip: 10.0.0.1/24, dhcp_relay: 10.0.0.9 }]
  - name: S
    type: server
    interfaces: [{ name: eth0, ip: 10.0.0.2/24 }]
    dhcp: { pools: [{ network: 10.0.0.0/24, range: 10.0.1.10-10.0.1.20 }] }
    dns_server: { records: { "bad_name": 10.0.0.2 } }
"#;
    let errors = load_yaml(bad).unwrap_err().errors;
    let all = errors.join("\n");
    assert!(all.contains("un switch n'a ni DHCP ni DNS"), "{all}");
    assert!(
        all.contains("seul un routeur ou un pare-feu relaie"),
        "{all}"
    );
    assert!(all.contains("hors du réseau"), "{all}");
    assert!(all.contains("nom de domaine invalide"), "{all}");
}

#[test]
fn a_client_without_server_is_a_warning_not_an_error() {
    let lonely = r#"
network: { name: seul }
devices:
  - { name: PC, type: host, interfaces: [{ name: eth0, ip: dhcp }] }
  - { name: SW, type: switch, ports: 2 }
links:
  - { a: "PC:eth0", b: "SW:port1" }
"#;
    let loaded = load_yaml(lonely).unwrap();
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("aucune adresse obtenue par DHCP")),
        "{:?}",
        loaded.warnings
    );
}
