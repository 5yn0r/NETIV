//! `netiv bench` : mesures de performance reproductibles.
//!
//! Le réseau est généré à partir d'une graine : deux exécutions sur la même
//! machine mesurent exactement le même travail. Seuls les temps varient.

use std::time::{Duration, Instant};

use netiv_analysis::reach;
use netiv_core::Simulation;
use netiv_topology::generate::{GenSpec, TopoKind, generate};

/// Paramètres d'une mesure.
pub struct BenchSpec {
    /// Nombre de routeurs du réseau généré.
    pub routers: usize,
    /// Nombre de postes.
    pub hosts: usize,
    /// Nombre de pings envoyés pour mesurer le moteur.
    pub pings: u16,
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

/// Exécute les mesures et rend un tableau texte.
pub fn run(spec: &BenchSpec) -> Result<String, String> {
    let def = generate(&GenSpec {
        kind: TopoKind::Datacenter,
        routers: spec.routers,
        hosts: spec.hosts,
        seed: 1,
    })?;
    let (loaded, t_load) = timed(|| netiv_topology::build(&def));
    let loaded = loaded.map_err(|e| e.to_string())?;
    let mut sim = loaded.sim;

    let ends = reach::end_devices(&sim);
    let (Some(&src), Some(&dst)) = (ends.first(), ends.last()) else {
        return Err("réseau généré sans machine".into());
    };
    let dst_ip = reach::address_of(&sim, dst).ok_or("destination sans adresse")?;

    // 1. Moteur : une longue série de pings à travers le cœur du réseau.
    sim.ping(src, dst_ip, spec.pings)
        .map_err(|e| e.to_string())?;
    let (events, t_engine) = timed(|| sim.run_until_idle());
    let events = events.map_err(|e| e.to_string())?;
    let rate = events as f64 / t_engine.as_secs_f64().max(1e-9);

    // 2. Requêtes sur la trace accumulée.
    let tids: std::collections::BTreeSet<_> = sim
        .trace()
        .events()
        .iter()
        .filter_map(|e| e.trace_id)
        .collect();
    let (found, t_index) = timed(|| {
        tids.iter()
            .take(1000)
            .map(|t| sim.trace().by_trace(*t).count())
            .sum::<usize>()
    });

    // 3. Analyses, sur la simulation chargée de toute sa trace (cas réel
    //    d'une session longue).
    let flows = reach::flows(&sim);
    let (_, t_seq) = timed(|| reach::measure_all_with(&sim, &flows, 1));
    let (_, t_par) = timed(|| reach::measure_all(&sim, &flows));
    let (_, t_security) = timed(|| netiv_security::analyze(&sim));
    let (_, t_health) = timed(|| netiv_analysis::health(&sim));
    let (whatif, t_whatif) = timed(|| netiv_analysis::what_if(&sim, "R1 goes down"));
    whatif?;
    let (_, t_clone) = timed(|| clone_many(&sim, 100));

    let threads = std::thread::available_parallelism().map_or(1, |c| c.get());
    let mut s = String::new();
    s.push_str(&format!(
        "NETIV BENCH — datacenter, {} équipements, {} câbles, {threads} cœur(s)\n\n",
        sim.devices().len(),
        sim.links().len()
    ));
    let rows = [
        ("Chargement de la topologie", ms(t_load)),
        (
            "Moteur",
            format!(
                "{} pings : {events} événements traités ({} entrées de trace) en {} → {:.0} év./s",
                spec.pings,
                sim.trace().len(),
                ms(t_engine),
                rate
            ),
        ),
        (
            "Trace indexée",
            format!(
                "{} transactions ({found} événements) en {}",
                tids.len().min(1000),
                ms(t_index)
            ),
        ),
        (
            "Copie neuve ×100",
            format!(
                "{} (trace de {} événements ignorée)",
                ms(t_clone),
                sim.trace().len()
            ),
        ),
        (
            "Matrice d'accessibilité",
            format!(
                "{} flux : {} séquentiel, {} parallèle (×{:.1})",
                flows.len(),
                ms(t_seq),
                ms(t_par),
                t_seq.as_secs_f64() / t_par.as_secs_f64().max(1e-9)
            ),
        ),
        ("Analyse de sécurité", ms(t_security)),
        ("Score de santé", ms(t_health)),
        ("What-if « R1 goes down »", ms(t_whatif)),
    ];
    for (k, v) in rows {
        s.push_str(&format!("{k:<28} {v}\n"));
    }
    Ok(s)
}

fn clone_many(sim: &Simulation, n: usize) -> usize {
    (0..n).map(|_| sim.fresh_clone().devices().len()).sum()
}
