//! Garde-fous des optimisations : l'index de la trace et les copies légères
//! doivent donner exactement les mêmes résultats que les versions naïves.

mod common;

use std::collections::BTreeSet;

use common::*;
use netiv_core::{TraceEvent, TraceId};

fn linear(events: &[TraceEvent], id: TraceId) -> Vec<u64> {
    events
        .iter()
        .filter(|e| e.trace_id == Some(id))
        .map(|e| e.seq)
        .collect()
}

#[test]
fn indexed_by_trace_matches_linear_filter() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    sim.ping(pc1, ip("192.168.2.10"), 5).unwrap();
    sim.run_until_idle().unwrap();
    sim.ping(pc1, ip("10.99.0.1"), 2).unwrap(); // échec : erreur ICMP
    sim.run_until_idle().unwrap();

    let all = events(&sim);
    let ids: BTreeSet<TraceId> = all.iter().filter_map(|e| e.trace_id).collect();
    assert!(ids.len() >= 7);
    let mut total = 0;
    for id in ids {
        let indexed: Vec<u64> = sim.trace().by_trace(id).map(|e| e.seq).collect();
        assert_eq!(indexed, linear(all, id), "transaction {id}");
        total += indexed.len();
    }
    assert_eq!(total, all.iter().filter(|e| e.trace_id.is_some()).count());
}

#[test]
fn index_survives_clear_and_serde_round_trip() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    sim.ping(pc1, ip("192.168.2.10"), 2).unwrap();
    sim.run_until_idle().unwrap();

    let json = serde_json::to_string(sim.trace()).unwrap();
    let back: netiv_core::TraceLog = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, sim.trace());
    let id = TraceId(1);
    assert_eq!(
        back.by_trace(id).map(|e| e.seq).collect::<Vec<_>>(),
        linear(sim.trace().events(), id)
    );

    sim.clear_trace();
    assert_eq!(sim.trace().by_trace(id).count(), 0);
}

#[test]
fn fresh_clone_drops_history_but_keeps_configuration() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    sim.ping(pc1, ip("192.168.2.10"), 50).unwrap();
    sim.run_until_idle().unwrap();
    assert!(sim.trace().len() > 1000);

    let fresh = sim.fresh_clone();
    assert!(fresh.trace().is_empty());
    assert_eq!(fresh.pending(), 0);
    assert_eq!(fresh.config_fingerprint(), sim.config_fingerprint());

    // Même comportement qu'une copie complète remise à zéro.
    let mut full = sim.clone();
    full.reset_runtime();
    let mut a = fresh;
    a.ping(pc1, ip("192.168.2.10"), 2).unwrap();
    a.run_until_idle().unwrap();
    full.ping(pc1, ip("192.168.2.10"), 2).unwrap();
    full.run_until_idle().unwrap();
    assert_eq!(a.trace().events(), full.trace().events());
}
