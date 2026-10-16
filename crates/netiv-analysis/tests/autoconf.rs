#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Adressage automatique : un réseau dessiné sans aucune adresse devient
//! entièrement joignable, sans jamais écraser ce qui était déjà configuré.

use netiv_analysis::reach;
use netiv_topology::autoconf::autoconfigure;
use netiv_topology::generate::{GenSpec, TopoKind, generate};
use netiv_topology::{TopologyDef, build};

/// Retire toute la configuration IP (comme un réseau qu'on vient de dessiner).
fn strip(def: &mut TopologyDef) {
    for d in &mut def.devices {
        d.gateway = None;
        d.routes.clear();
        for i in &mut d.interfaces {
            i.ip = None;
        }
    }
}

fn fully_reachable(def: &TopologyDef) -> (usize, usize) {
    let sim = build(def).unwrap().sim;
    let m = reach::routable(&sim);
    (m.ok_count(), m.len())
}

#[test]
fn every_generated_network_is_reachable_after_stripping_and_autoconfig() {
    for kind in [
        TopoKind::Lan,
        TopoKind::Enterprise,
        TopoKind::Campus,
        TopoKind::Datacenter,
        TopoKind::Isp,
        TopoKind::Hybrid,
    ] {
        for seed in [1, 2] {
            let mut def = generate(&GenSpec {
                kind,
                routers: 5,
                hosts: 14,
                seed,
            })
            .unwrap();
            strip(&mut def);
            let report = autoconfigure(&mut def);
            assert!(!report.is_empty(), "{kind:?} : rien n'a été configuré");
            let (ok, total) = fully_reachable(&def);
            assert!(total > 0, "{kind:?} : aucun flux mesuré");
            assert_eq!(
                ok, total,
                "{kind:?} graine {seed} : {ok}/{total} joignables\n{report:#?}"
            );
        }
    }
}

#[test]
fn autoconfig_is_idempotent_and_deterministic() {
    let mut a = generate(&GenSpec {
        kind: TopoKind::Campus,
        routers: 4,
        hosts: 10,
        seed: 3,
    })
    .unwrap();
    strip(&mut a);
    let mut b = a.clone();
    let ra = autoconfigure(&mut a);
    let rb = autoconfigure(&mut b);
    assert_eq!(a, b, "même entrée → même résultat");
    assert_eq!(ra, rb);
    let again = autoconfigure(&mut a);
    assert!(again.is_empty(), "seconde passe : {again:?}");
}

#[test]
fn existing_values_are_never_overwritten() {
    let yaml = r#"
network: { name: dessin }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 172.16.5.50/24 }], gateway: 172.16.5.254 }
  - { name: PC2, type: host, interfaces: [{ name: eth0 }] }
  - { name: SW1, type: switch, ports: 4 }
  - { name: R1, type: router, interfaces: [{ name: eth0 }, { name: eth1 }] }
  - { name: R2, type: router, interfaces: [{ name: eth0 }, { name: eth1 }] }
  - { name: SRV, type: server, interfaces: [{ name: eth0 }] }
links:
  - { a: "PC1:eth0", b: "SW1:port1" }
  - { a: "PC2:eth0", b: "SW1:port2" }
  - { a: "SW1:port3", b: "R1:eth0" }
  - { a: "R1:eth1", b: "R2:eth0" }
  - { a: "R2:eth1", b: "SRV:eth0" }
"#;
    let mut def: TopologyDef = serde_yaml_ng::from_str(yaml).unwrap();
    let report = autoconfigure(&mut def);
    let dev = |n: &str| def.devices.iter().find(|d| d.name == n).unwrap();

    // Valeurs déjà écrites : intactes.
    assert_eq!(
        dev("PC1").interfaces[0].ip.as_deref(),
        Some("172.16.5.50/24")
    );
    assert_eq!(dev("PC1").gateway.as_deref(), Some("172.16.5.254"));
    // Le segment réutilise le réseau de PC1 ; le routeur prend .1.
    assert_eq!(dev("R1").interfaces[0].ip.as_deref(), Some("172.16.5.1/24"));
    assert_eq!(
        dev("PC2").interfaces[0].ip.as_deref(),
        Some("172.16.5.10/24")
    );
    assert_eq!(dev("PC2").gateway.as_deref(), Some("172.16.5.1"));
    // Lien entre routeurs : un /30.
    assert!(
        dev("R1").interfaces[1]
            .ip
            .as_deref()
            .unwrap()
            .ends_with("/30")
    );
    // Routes dans les deux sens.
    assert_eq!(dev("R1").routes.len(), 1);
    assert_eq!(dev("R2").routes.len(), 1);
    assert!(
        report.iter().any(|l| l.contains("SRV : passerelle")),
        "{report:#?}"
    );

    // PC2 → SRV fonctionne ; PC1 garde sa passerelle (fausse) : à l'élève de voir.
    let sim = build(&def).unwrap().sim;
    let pc2 = sim.device_id("PC2").unwrap();
    let srv_ip = reach::address_of(&sim, sim.device_id("SRV").unwrap()).unwrap();
    let mut s = sim.clone();
    s.ping(pc2, srv_ip, 1).unwrap();
    s.run_until_idle().unwrap();
    assert!(
        s.trace()
            .events()
            .iter()
            .any(|e| matches!(e.kind, netiv_core::TraceKind::PingReply { .. }))
    );
}

#[test]
fn vlans_split_segments() {
    let yaml = r#"
network: { name: vlans }
devices:
  - { name: A, type: host, interfaces: [{ name: eth0 }] }
  - { name: B, type: host, interfaces: [{ name: eth0 }] }
  - name: SW1
    type: switch
    interfaces: [{ name: p1, vlan: 10 }, { name: p2, vlan: 20 }, { name: r10, vlan: 10 }, { name: r20, vlan: 20 }]
  - { name: R1, type: router, interfaces: [{ name: v10 }, { name: v20 }] }
links:
  - { a: "A:eth0", b: "SW1:p1" }
  - { a: "B:eth0", b: "SW1:p2" }
  - { a: "SW1:r10", b: "R1:v10" }
  - { a: "SW1:r20", b: "R1:v20" }
"#;
    let mut def: TopologyDef = serde_yaml_ng::from_str(yaml).unwrap();
    autoconfigure(&mut def);
    let ip = |n: &str| {
        def.devices.iter().find(|d| d.name == n).unwrap().interfaces[0]
            .ip
            .clone()
            .unwrap()
    };
    // Deux VLAN → deux réseaux différents.
    let (a, b) = (ip("A"), ip("B"));
    assert_ne!(
        a.rsplit_once('.').unwrap().0,
        b.rsplit_once('.').unwrap().0,
        "{a} / {b}"
    );
    let (ok, total) = fully_reachable(&def);
    assert_eq!((ok, total), (2, 2));
}
