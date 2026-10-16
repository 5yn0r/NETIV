#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Serveur DHCP pirate : constat de conception et détection forensique.

use netiv_core::Fault;
use netiv_core::dhcp::DhcpPool;
use netiv_security::{Capture, analyze, analyze_capture};

fn lab() -> netiv_core::Simulation {
    netiv_topology::load_yaml(include_str!(
        "../../netiv-education/content/topologies/dhcp.yaml"
    ))
    .unwrap()
    .sim
}

fn rogue(sim: &mut netiv_core::Simulation) {
    let imp = sim.device_id("IMP").unwrap();
    sim.inject_fault(
        Fault::DhcpPools {
            device: imp,
            pools: vec![DhcpPool {
                network: "10.10.1.0/24".parse().unwrap(),
                first: "10.10.1.200".parse().unwrap(),
                last: "10.10.1.210".parse().unwrap(),
                gateway: Some("10.10.1.254".parse().unwrap()),
                dns: None,
                lease_s: 3_600,
            }],
        },
        false,
    )
    .unwrap();
    sim.restart_dhcp().unwrap();
}

#[test]
fn a_dhcp_server_on_a_user_device_is_a_high_finding() {
    let mut sim = lab();
    assert!(!analyze(&sim).render().contains("SEC-012"), "réseau sain");
    rogue(&mut sim);
    let report = analyze(&sim).render();
    assert!(report.contains("[HIGH] SEC-012"), "{report}");
    assert!(report.contains("IMP"), "{report}");
}

#[test]
fn forensics_spots_the_rogue_server_in_a_capture() {
    let mut sim = lab();
    rogue(&mut sim);
    let cap = Capture::from_sim(&sim, "dhcp");
    let text = analyze_capture(&cap).render(&cap);
    assert!(text.contains("serveur DHCP pirate"), "{text}");
    assert!(text.contains("en concurrence avec SRV"), "{text}");
    // Sans pirate : rien.
    let clean = lab();
    let cap = Capture::from_sim(&clean, "dhcp");
    assert!(!analyze_capture(&cap).render(&cap).contains("pirate"));
}
