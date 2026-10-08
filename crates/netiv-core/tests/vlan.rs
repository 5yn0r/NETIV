#![allow(clippy::unwrap_used, clippy::expect_used)]

//! VLAN : isolation, trunk, routage inter-VLAN.

mod common;

use common::*;
use netiv_core::switch::PortMode;
use netiv_core::{DeviceKind, Simulation, TraceKind};

fn access(sim: &mut Simulation, endpoint: &str, vlan: u16) {
    let (d, i) = port(sim, endpoint);
    sim.set_port_mode(d, i, PortMode::Access { vlan }).unwrap();
}

fn trunk(sim: &mut Simulation, endpoint: &str, allowed: &[u16], native: u16) {
    let (d, i) = port(sim, endpoint);
    sim.set_port_mode(
        d,
        i,
        PortMode::Trunk {
            allowed: allowed.to_vec(),
            native,
        },
    )
    .unwrap();
}

/// Un switch, quatre PC dans le même sous-réseau, deux VLAN.
fn single_switch() -> Simulation {
    let mut sim = Simulation::new(1);
    for (n, ipaddr) in [("PC1", 1), ("PC2", 2), ("PC3", 3), ("PC4", 4)] {
        add(
            &mut sim,
            n,
            DeviceKind::Host,
            &[("eth0", Some(&format!("10.0.0.{ipaddr}/24")))],
        );
    }
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("p3", None), ("p4", None)],
    );
    for n in 1..=4 {
        link(&mut sim, &format!("PC{n}:eth0"), &format!("SW1:p{n}"));
    }
    access(&mut sim, "SW1:p1", 10);
    access(&mut sim, "SW1:p2", 10);
    access(&mut sim, "SW1:p3", 20);
    access(&mut sim, "SW1:p4", 20);
    sim.clear_trace();
    sim
}

#[test]
fn hosts_in_the_same_vlan_talk() {
    let mut sim = single_switch();
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    ping(&mut sim, "PC3", "10.0.0.4", 1);
    assert_eq!(ping_ok(&sim).len(), 2);
}

#[test]
fn vlans_isolate_hosts_sharing_a_subnet() {
    let mut sim = single_switch();
    ping(&mut sim, "PC1", "10.0.0.3", 1);
    assert!(ping_ok(&sim).is_empty());
    // Les broadcasts ARP de PC1 ne doivent jamais atteindre le VLAN 20.
    for name in ["PC3", "PC4"] {
        let d = dev(&sim, name);
        assert!(
            !sim.trace()
                .by_device(d)
                .any(|e| matches!(e.kind, TraceKind::FrameReceived { .. })),
            "{name} ne devrait rien recevoir du VLAN 10"
        );
    }
    // PC2, dans le même VLAN, a bien reçu la requête ARP.
    let pc2 = dev(&sim, "PC2");
    assert!(
        sim.trace()
            .by_device(pc2)
            .any(|e| matches!(e.kind, TraceKind::FrameReceived { .. }))
    );
}

#[test]
fn mac_table_is_kept_per_vlan() {
    let mut sim = single_switch();
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    ping(&mut sim, "PC3", "10.0.0.4", 1);
    let sw = match &sim.device(dev(&sim, "SW1")).unwrap().behavior {
        netiv_core::device::Behavior::Switch(s) => s.clone(),
        _ => unreachable!(),
    };
    let vlans: Vec<u16> = sw.mac_table.keys().map(|(v, _)| *v).collect();
    assert_eq!(vlans.iter().filter(|v| **v == 10).count(), 2);
    assert_eq!(vlans.iter().filter(|v| **v == 20).count(), 2);
}

#[test]
fn moving_a_port_to_the_wrong_vlan_cuts_communication() {
    let mut sim = single_switch();
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    assert_eq!(ping_ok(&sim).len(), 1);
    access(&mut sim, "SW1:p2", 20); // PC2 change de VLAN
    let before = ping_ok(&sim).len();
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    assert_eq!(ping_ok(&sim).len(), before, "le ping ne doit plus aboutir");
}

/// SW1 ── trunk ── SW2 ; PC1/PC3 sur SW1, PC2/PC4 sur SW2.
fn two_switches() -> Simulation {
    let mut sim = Simulation::new(1);
    for (n, ipaddr) in [("PC1", 1), ("PC2", 2), ("PC3", 3), ("PC4", 4)] {
        add(
            &mut sim,
            n,
            DeviceKind::Host,
            &[("eth0", Some(&format!("10.0.0.{ipaddr}/24")))],
        );
    }
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("tr", None)],
    );
    add(
        &mut sim,
        "SW2",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("tr", None)],
    );
    link(&mut sim, "PC1:eth0", "SW1:p1"); // VLAN 10
    link(&mut sim, "PC3:eth0", "SW1:p2"); // VLAN 20
    link(&mut sim, "PC2:eth0", "SW2:p1"); // VLAN 10
    link(&mut sim, "PC4:eth0", "SW2:p2"); // VLAN 20
    link(&mut sim, "SW1:tr", "SW2:tr");
    for sw in ["SW1", "SW2"] {
        access(&mut sim, &format!("{sw}:p1"), 10);
        access(&mut sim, &format!("{sw}:p2"), 20);
        trunk(&mut sim, &format!("{sw}:tr"), &[10, 20], 1);
    }
    sim.clear_trace();
    sim
}

#[test]
fn trunk_carries_both_vlans_but_keeps_them_apart() {
    let mut sim = two_switches();
    ping(&mut sim, "PC1", "10.0.0.2", 1); // VLAN 10 à travers le trunk
    ping(&mut sim, "PC3", "10.0.0.4", 1); // VLAN 20 à travers le trunk
    assert_eq!(ping_ok(&sim).len(), 2);
    let before = ping_ok(&sim).len();
    ping(&mut sim, "PC1", "10.0.0.4", 1); // VLAN 10 → VLAN 20 : impossible
    assert_eq!(ping_ok(&sim).len(), before);
}

#[test]
fn trunk_tags_frames_and_access_ports_do_not() {
    let mut sim = two_switches();
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    let sw1 = dev(&sim, "SW1");
    let tr = sim.iface_id(sw1, "tr").unwrap();
    let on_trunk: Vec<Option<u16>> = sim
        .trace()
        .by_device(sw1)
        .filter_map(|e| match &e.kind {
            TraceKind::FrameSent { iface, frame } if *iface == tr => Some(frame.vlan),
            _ => None,
        })
        .collect();
    assert!(!on_trunk.is_empty());
    assert!(on_trunk.iter().all(|v| *v == Some(10)), "{on_trunk:?}");
    // Aucune trame étiquetée n'est jamais remise à un hôte.
    for e in sim.trace().events() {
        if let TraceKind::FrameReceived { frame, .. } = &e.kind {
            let is_host = sim.device(e.device).unwrap().kind == DeviceKind::Host;
            assert!(!is_host || frame.vlan.is_none());
        }
    }
}

#[test]
fn native_vlan_travels_untagged() {
    let mut sim = two_switches();
    for sw in ["SW1", "SW2"] {
        trunk(&mut sim, &format!("{sw}:tr"), &[10, 20], 10);
    }
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    let tagged_10 =
        sim.trace().events().iter().any(
            |e| matches!(&e.kind, TraceKind::FrameSent { frame, .. } if frame.vlan == Some(10)),
        );
    assert!(!tagged_10, "le VLAN natif ne doit pas être étiqueté");
    assert_eq!(ping_ok(&sim).len(), 1);
}

#[test]
fn trunk_allowed_list_blocks_other_vlans() {
    let mut sim = two_switches();
    for sw in ["SW1", "SW2"] {
        trunk(&mut sim, &format!("{sw}:tr"), &[10], 1);
    }
    ping(&mut sim, "PC3", "10.0.0.4", 1);
    assert!(
        ping_ok(&sim).is_empty(),
        "le VLAN 20 n'est plus autorisé sur le trunk"
    );
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    assert_eq!(ping_ok(&sim).len(), 1, "le VLAN 10 passe toujours");
}

#[test]
fn mismatched_native_vlans_leak_nothing_between_subnets() {
    let mut sim = two_switches();
    trunk(&mut sim, "SW1:tr", &[], 10);
    trunk(&mut sim, "SW2:tr", &[], 20);
    // Un trunk dont les extrémités n'ont pas le même VLAN natif fait basculer
    // les trames non étiquetées d'un VLAN dans l'autre : PC1 (VLAN 10) joint
    // PC4 (VLAN 20) alors qu'aucun routeur n'existe.
    ping(&mut sim, "PC1", "10.0.0.4", 1);
    assert_eq!(
        ping_ok(&sim).len(),
        1,
        "fuite entre VLAN due au natif discordant"
    );
}

#[test]
fn inter_vlan_routing_through_a_router() {
    let mut sim = Simulation::new(1);
    let pc1 = add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.10.10/24"))],
    );
    let pc2 = add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.20.10/24"))],
    );
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("r10", None), ("r20", None)],
    );
    add(
        &mut sim,
        "R1",
        DeviceKind::Router,
        &[
            ("eth0", Some("10.0.10.1/24")),
            ("eth1", Some("10.0.20.1/24")),
        ],
    );
    gw(&mut sim, pc1, "10.0.10.1");
    gw(&mut sim, pc2, "10.0.20.1");
    link(&mut sim, "PC1:eth0", "SW1:p1");
    link(&mut sim, "PC2:eth0", "SW1:p2");
    link(&mut sim, "R1:eth0", "SW1:r10");
    link(&mut sim, "R1:eth1", "SW1:r20");
    access(&mut sim, "SW1:p1", 10);
    access(&mut sim, "SW1:r10", 10);
    access(&mut sim, "SW1:p2", 20);
    access(&mut sim, "SW1:r20", 20);

    ping(&mut sim, "PC1", "10.0.20.10", 1);
    let ok = ping_ok(&sim);
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].1, 63, "un seul routeur traversé");
}

#[test]
fn vlan_runs_are_deterministic() {
    let run = || {
        let mut sim = two_switches();
        ping(&mut sim, "PC1", "10.0.0.2", 3);
        ping(&mut sim, "PC3", "10.0.0.4", 3);
        sim.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}
