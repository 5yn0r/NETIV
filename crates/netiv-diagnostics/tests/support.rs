#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Diagnostics de support : un poste sans adresse n'est pas toujours un
//! problème DHCP (port éteint, VLAN vide), et un conflit d'adresses se
//! reconnaît au lieu d'être pris pour une panne de routage.

use std::net::Ipv4Addr;

use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, Ipv4Net, Simulation};
use netiv_diagnostics::{CauseKind, Request, diagnose};

fn pme() -> Simulation {
    let yaml = include_str!("../../netiv-education/content/topologies/pme.yaml");
    let mut sim = netiv_topology::load_yaml(yaml).unwrap().sim;
    sim.clear_trace();
    sim
}

fn dev(sim: &Simulation, n: &str) -> DeviceId {
    sim.device_id(n).unwrap()
}

/// « `from` joint-il le serveur de l'entreprise ? »
fn to_server(sim: &Simulation, from: &str) -> Request {
    Request {
        src: dev(sim, from),
        dst: "192.168.30.10".parse::<Ipv4Addr>().unwrap(),
        service: None,
    }
}

#[test]
fn a_disabled_switch_port_is_blamed_before_dhcp() {
    let healthy = pme();
    let mut sim = healthy.clone();
    let sw = dev(&sim, "SW1");
    let p2 = sim.iface_id(sw, "p2").unwrap();
    sim.set_iface_admin(sw, p2, false).unwrap();
    sim.restart_dhcp().unwrap();

    let c = diagnose(&sim, Some(&healthy), &to_server(&sim, "ACC2"))
        .unwrap()
        .cause
        .unwrap();
    assert_eq!(c.kind, CauseKind::InterfaceDown, "{}", c.detail);
    assert_eq!(c.device, "SW1");
    assert!(c.fix.unwrap().starts_with("iface SW1 p2 up"));
}

#[test]
fn a_port_alone_in_its_vlan_is_a_vlan_error_not_a_dhcp_one() {
    let healthy = pme();
    let mut sim = healthy.clone();
    let sw = dev(&sim, "SW1");
    let p5 = sim.iface_id(sw, "p5").unwrap();
    sim.set_port_mode(sw, p5, PortMode::Access { vlan: 99 })
        .unwrap();
    sim.restart_dhcp().unwrap();

    let c = diagnose(&sim, Some(&healthy), &to_server(&sim, "CPT2"))
        .unwrap()
        .cause
        .unwrap();
    assert_eq!(c.kind, CauseKind::WrongVlan, "{}", c.detail);
    assert_eq!(c.device, "SW1");
    assert!(c.detail.contains("VLAN 99"), "{}", c.detail);
}

#[test]
fn a_host_using_the_gateway_address_is_an_ip_conflict() {
    let healthy = pme();
    let mut sim = healthy.clone();
    let acc2 = dev(&sim, "ACC2");
    let eth0 = sim.iface_id(acc2, "eth0").unwrap();
    let gw: Ipv4Net = "192.168.10.1/24".parse().unwrap();
    sim.set_ip(acc2, eth0, Some(gw)).unwrap();

    // Avec le réseau sain comme référence, puis sans : le fautif est le poste
    // (celui dont l'adresse a changé, celui qui n'est pas le routeur).
    for base in [Some(&healthy), None] {
        let c = diagnose(&sim, base, &to_server(&sim, "ACC1"))
            .unwrap()
            .cause
            .unwrap();
        assert_eq!(c.kind, CauseKind::WrongIp, "{}", c.detail);
        assert_eq!(c.device, "ACC2", "{}", c.detail);
        assert!(c.detail.contains("conflit d'adresses"), "{}", c.detail);
        assert_eq!(c.fix.as_deref(), Some("ip ACC2 eth0 dhcp"));
    }
}

#[test]
fn a_healthy_network_has_no_conflict() {
    let sim = pme();
    for host in ["ACC1", "ACC2", "CPT1", "CPT2"] {
        let d = diagnose(&sim, None, &to_server(&sim, host)).unwrap();
        assert!(d.cause.is_none(), "{host} : {:?}", d.cause);
    }
}
