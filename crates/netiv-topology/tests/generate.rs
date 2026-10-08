#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Les topologies générées doivent être valides, entièrement joignables et reproductibles.

use netiv_core::{DeviceKind, Simulation};
use netiv_topology::generate::{GenSpec, MAX_HOSTS, TopoKind, generate};
use netiv_topology::{Loaded, build};

fn spec(kind: TopoKind, routers: usize, hosts: usize) -> GenSpec {
    GenSpec {
        kind,
        routers,
        hosts,
        seed: 1,
    }
}

fn load(s: &GenSpec) -> Loaded {
    let def = generate(s).unwrap_or_else(|e| panic!("{s:?} : {e}"));
    build(&def).unwrap_or_else(|e| panic!("{s:?} : {e}"))
}

fn end_devices(sim: &Simulation) -> Vec<netiv_core::DeviceId> {
    sim.devices()
        .iter()
        .filter(|d| matches!(d.kind, DeviceKind::Host | DeviceKind::Server))
        .map(|d| d.id)
        .collect()
}

/// Ping de chaque machine vers chaque autre : (réussites, total).
fn all_pairs(sim: &mut Simulation) -> (usize, usize) {
    let ends = end_devices(sim);
    let ips: Vec<_> = ends
        .iter()
        .map(|d| {
            sim.device(*d)
                .unwrap()
                .ifaces
                .iter()
                .find_map(|i| i.ip.map(|n| n.addr()))
                .unwrap()
        })
        .collect();
    let mut total = 0;
    for (i, src) in ends.iter().enumerate() {
        for (j, ip) in ips.iter().enumerate() {
            if i != j {
                sim.ping(*src, *ip, 1).unwrap();
                total += 1;
            }
        }
    }
    sim.run_until_idle().unwrap();
    let ok = sim
        .trace()
        .events()
        .iter()
        .filter(|e| matches!(e.kind, netiv_core::TraceKind::PingReply { .. }))
        .count();
    (ok, total)
}

#[test]
fn every_type_generates_a_valid_fully_reachable_network() {
    let cases = [
        spec(TopoKind::Lan, 0, 8),
        spec(TopoKind::Enterprise, 4, 12),
        spec(TopoKind::Campus, 5, 12),
        spec(TopoKind::Datacenter, 5, 9),
        spec(TopoKind::Isp, 5, 8),
        spec(TopoKind::Hybrid, 5, 10),
    ];
    for s in cases {
        let mut loaded = load(&s);
        assert!(loaded.warnings.is_empty(), "{s:?} : {:?}", loaded.warnings);
        let (ok, total) = all_pairs(&mut loaded.sim);
        assert!(total > 20, "{s:?}");
        assert_eq!(ok, total, "{s:?} : {ok}/{total} paires joignables");
    }
}

#[test]
fn minimal_sizes_work_for_every_type() {
    for kind in TopoKind::ALL {
        let routers = kind.min_routers();
        let mut loaded = load(&spec(kind, routers, 3));
        if kind == TopoKind::CyberRange {
            continue; // politique de pare-feu : testée à part
        }
        let (ok, total) = all_pairs(&mut loaded.sim);
        assert_eq!(ok, total, "{} avec {routers} routeurs", kind.slug());
    }
}

#[test]
fn counts_match_the_request() {
    let loaded = load(&spec(TopoKind::Enterprise, 4, 12));
    let sim = &loaded.sim;
    let n = |k| sim.devices().iter().filter(|d| d.kind == k).count();
    assert_eq!(n(DeviceKind::Router), 4);
    assert_eq!(n(DeviceKind::Host), 12);
    assert_eq!(
        n(DeviceKind::Server),
        1,
        "un serveur pour 20 postes, au moins un"
    );
    assert_eq!(n(DeviceKind::Switch), 4, "un switch par LAN");
    let big = load(&spec(TopoKind::Enterprise, 6, 100));
    assert_eq!(
        big.sim
            .devices()
            .iter()
            .filter(|d| d.kind == DeviceKind::Server)
            .count(),
        5
    );
}

#[test]
fn campus_and_datacenter_are_redundant_by_construction() {
    // campus : cœur R1–R2, chaque distribution reliée aux deux cœurs.
    let def = generate(&spec(TopoKind::Campus, 5, 6)).unwrap();
    let r2r = def
        .links
        .iter()
        .filter(|l| l.a.starts_with('R') && l.b.starts_with('R'))
        .count();
    assert_eq!(r2r, 1 + 2 * 3);
    let def = generate(&spec(TopoKind::Datacenter, 6, 6)).unwrap();
    let r2r = def
        .links
        .iter()
        .filter(|l| l.a.starts_with('R') && l.b.starts_with('R'))
        .count();
    assert_eq!(r2r, 2 * 4, "chaque leaf est reliée aux deux spines");
}

#[test]
fn addressing_plan_is_clean_and_unique() {
    let def = generate(&spec(TopoKind::Hybrid, 7, 40)).unwrap();
    let mut ips = std::collections::BTreeSet::new();
    for d in &def.devices {
        for i in &d.interfaces {
            if let Some(ip) = &i.ip {
                let addr = ip.split('/').next().unwrap().to_string();
                assert!(ips.insert(addr.clone()), "adresse {addr} en double");
            }
        }
    }
    assert!(ips.len() > 40);
    // Les LAN sont des /24 en 10.x, les liens des /30 en 172.16.
    for d in def.devices.iter().filter(|d| d.kind == "router") {
        for i in &d.interfaces {
            let ip = i.ip.as_deref().unwrap();
            assert!(
                ip.starts_with("10.") && ip.ends_with("/24")
                    || ip.starts_with("172.16.") && ip.ends_with("/30"),
                "{ip}"
            );
        }
    }
}

#[test]
fn generation_is_deterministic_and_the_seed_changes_the_placement_only() {
    let a =
        serde_yaml_ng::to_string(&generate(&spec(TopoKind::Enterprise, 4, 10)).unwrap()).unwrap();
    let b =
        serde_yaml_ng::to_string(&generate(&spec(TopoKind::Enterprise, 4, 10)).unwrap()).unwrap();
    assert_eq!(a, b);
    let distinct: std::collections::BTreeSet<String> = (0..6u64)
        .map(|seed| {
            let def = generate(&GenSpec {
                seed,
                ..spec(TopoKind::Enterprise, 4, 10)
            })
            .unwrap();
            // Nombre de postes par LAN.
            let per_lan: Vec<usize> = (2..=4)
                .map(|r| {
                    def.links
                        .iter()
                        .filter(|l| l.b.starts_with(&format!("SW{r}:")) && l.a.starts_with("PC"))
                        .count()
                })
                .collect();
            format!("{per_lan:?}")
        })
        .collect();
    assert!(
        distinct.len() >= 2,
        "la graine doit varier la répartition : {distinct:?}"
    );
    // La structure (nombre d'équipements) ne dépend pas de la graine.
    let counts: std::collections::BTreeSet<usize> = (0..6u64)
        .map(|seed| {
            generate(&GenSpec {
                seed,
                ..spec(TopoKind::Enterprise, 4, 10)
            })
            .unwrap()
            .devices
            .len()
        })
        .collect();
    assert_eq!(counts.len(), 1);
}

#[test]
fn a_generated_network_survives_an_export_roundtrip() {
    let def = generate(&spec(TopoKind::Campus, 4, 10)).unwrap();
    let mut a = build(&def).unwrap();
    let exported = netiv_topology::export(&a.sim, "x");
    let mut b = build(&exported).unwrap();
    assert_eq!(all_pairs(&mut a.sim), all_pairs(&mut b.sim));
}

#[test]
fn invalid_requests_are_refused_with_a_clear_message() {
    let e = |s: GenSpec| generate(&s).unwrap_err();
    assert!(e(spec(TopoKind::Campus, 2, 5)).contains("au moins 3 routeurs"));
    assert!(e(spec(TopoKind::Enterprise, 1, 5)).contains("au moins 2 routeurs"));
    assert!(e(spec(TopoKind::Enterprise, 500, 5)).contains("trop de routeurs"));
    assert!(e(spec(TopoKind::Enterprise, 4, MAX_HOSTS + 1)).contains("trop de machines"));
    assert!(e(spec(TopoKind::Enterprise, 2, 600)).contains("sur un même LAN"));
    assert!(e(spec(TopoKind::Lan, 0, 300)).contains("un seul LAN"));
    assert!(
        "pas-un-type"
            .parse::<TopoKind>()
            .unwrap_err()
            .contains("types :")
    );
    assert_eq!("Cyber_Range".parse::<TopoKind>(), Ok(TopoKind::CyberRange));
    assert_eq!("entreprise".parse::<TopoKind>(), Ok(TopoKind::Enterprise));
}

#[test]
fn a_larger_network_still_builds_and_routes() {
    let mut loaded = load(&spec(TopoKind::Campus, 12, 200));
    let sim = &mut loaded.sim;
    assert_eq!(
        sim.devices()
            .iter()
            .filter(|d| d.kind == DeviceKind::Router)
            .count(),
        12
    );
    let ends = end_devices(sim);
    assert!(ends.len() >= 210);
    // Quelques paires choisies aux deux bouts du réseau.
    let ip_of = |sim: &Simulation, name: &str| {
        let d = sim.device(sim.device_id(name).unwrap()).unwrap();
        d.ifaces[0].ip.unwrap().addr()
    };
    let (first, last, srv) = (ip_of(sim, "PC1"), ip_of(sim, "PC200"), ip_of(sim, "SRV1"));
    let pc1 = sim.device_id("PC1").unwrap();
    let pc200 = sim.device_id("PC200").unwrap();
    sim.ping(pc1, last, 1).unwrap();
    sim.ping(pc200, first, 1).unwrap();
    sim.ping(pc1, srv, 1).unwrap();
    sim.run_until_idle().unwrap();
    let ok = sim
        .trace()
        .events()
        .iter()
        .filter(|e| matches!(e.kind, netiv_core::TraceKind::PingReply { .. }))
        .count();
    assert_eq!(ok, 3);
}

#[test]
fn the_cyber_range_enforces_its_zone_policy() {
    use netiv_core::TraceKind;
    let mut loaded = load(&spec(TopoKind::CyberRange, 0, 20));
    let sim = &mut loaded.sim;
    let dev = |n: &str| sim.device_id(n).unwrap();
    let (inet, web, pc, file) = (dev("INET"), dev("WEB01"), dev("PC01"), dev("FILE01"));
    let ip = |s: &str| -> std::net::Ipv4Addr { s.parse().unwrap() };
    // Autorisés : LAN → web DMZ, LAN → Internet, Internet → web DMZ, LAN → fichiers.
    for (src, dst, port) in [
        (pc, "10.9.0.10", 80),
        (pc, "203.0.113.10", 80),
        (inet, "10.9.0.10", 80),
        (pc, "192.168.20.20", 445),
    ] {
        sim.command(src, netiv_core::Command::Curl { dst: ip(dst), port })
            .unwrap();
    }
    // Interdits : DMZ → LAN, DMZ → fichiers, Internet → LAN.
    for (src, dst, port) in [
        (web, "192.168.10.10", 80),
        (web, "192.168.20.20", 445),
        (inet, "192.168.10.10", 80),
    ] {
        sim.command(src, netiv_core::Command::Curl { dst: ip(dst), port })
            .unwrap();
    }
    let _ = file;
    sim.run_until_idle().unwrap();
    let results: Vec<bool> = sim
        .trace()
        .events()
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::CurlResult { outcome, .. } => {
                Some(matches!(outcome, netiv_core::trace::CurlOutcome::Ok { .. }))
            }
            _ => None,
        })
        .collect();
    // L'ordre d'achèvement dépend des délais : on compte seulement.
    assert_eq!(results.iter().filter(|r| **r).count(), 4, "{results:?}");
    assert_eq!(results.iter().filter(|r| !**r).count(), 3, "{results:?}");
    // 20 postes ⇒ 2 serveurs web.
    assert!(sim.device_id("WEB02").is_ok() && sim.device_id("PC20").is_ok());
}
