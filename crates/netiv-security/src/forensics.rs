//! Forensique : une capture (`.nlv`) est un journal d'événements figé, que l'on
//! relit après coup pour reconstituer ce qui s'est passé et repérer les
//! comportements anormaux.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::net::Ipv4Addr;
use std::path::Path;

use netiv_core::{DeviceId, SimTime, Simulation, TraceEvent, TraceKind};
use serde::{Deserialize, Serialize};

/// Version du format `.nlv`.
pub const CAPTURE_FORMAT_VERSION: u32 = 1;

/// Description figée d'une interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfaceInfo {
    /// Nom.
    pub name: String,
    /// Adresse IP (CIDR).
    pub ip: Option<String>,
    /// Adresse MAC.
    pub mac: String,
}

/// Description figée d'un équipement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Identifiant.
    pub id: DeviceId,
    /// Nom.
    pub name: String,
    /// Type (`host`, `router`...).
    pub kind: String,
    /// Rôle effectif.
    pub role: String,
    /// Interfaces.
    pub ifaces: Vec<IfaceInfo>,
}

/// Une capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    /// Version du format.
    pub format_version: u32,
    /// Nom du réseau.
    pub network: String,
    /// Équipements au moment de la capture.
    pub devices: Vec<DeviceInfo>,
    /// Événements, dans l'ordre.
    pub events: Vec<TraceEvent>,
}

/// Erreurs de lecture / écriture d'une capture.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// Fichier inaccessible.
    #[error("capture « {path} » : {reason}")]
    Io {
        /// Chemin.
        path: String,
        /// Cause.
        reason: String,
    },
    /// Contenu invalide.
    #[error("capture « {path} » illisible : {reason}")]
    Corrupt {
        /// Chemin.
        path: String,
        /// Cause.
        reason: String,
    },
    /// Version trop récente.
    #[error(
        "capture au format version {0} : cette version de NETIV lit jusqu'à la version {CAPTURE_FORMAT_VERSION}"
    )]
    TooNew(u32),
}

impl Capture {
    /// Capture l'état de la trace de `sim`.
    pub fn from_sim(sim: &Simulation, network: &str) -> Capture {
        let devices = sim
            .devices()
            .iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: d.name.clone(),
                kind: d.kind.to_string(),
                role: d.effective_role().to_string(),
                ifaces: d
                    .ifaces
                    .iter()
                    .map(|i| IfaceInfo {
                        name: i.name.clone(),
                        ip: i.ip.map(|n| n.to_string()),
                        mac: i.mac.to_string(),
                    })
                    .collect(),
            })
            .collect();
        Capture {
            format_version: CAPTURE_FORMAT_VERSION,
            network: network.to_string(),
            devices,
            events: sim.trace().events().to_vec(),
        }
    }

    /// Écrit la capture (JSON).
    pub fn save(&self, path: &Path) -> Result<(), CaptureError> {
        let io = |e: std::io::Error| CaptureError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        };
        let text = serde_json::to_string(self).map_err(|e| CaptureError::Corrupt {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        std::fs::write(path, text).map_err(io)
    }

    /// Lit une capture.
    pub fn load(path: &Path) -> Result<Capture, CaptureError> {
        let text = std::fs::read_to_string(path).map_err(|e| CaptureError::Io {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        let cap: Capture = serde_json::from_str(&text).map_err(|e| CaptureError::Corrupt {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        if cap.format_version > CAPTURE_FORMAT_VERSION {
            return Err(CaptureError::TooNew(cap.format_version));
        }
        Ok(cap)
    }

    fn name(&self, id: DeviceId) -> &str {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map_or("?", |d| d.name.as_str())
    }

    fn role(&self, id: DeviceId) -> &str {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map_or("network", |d| d.role.as_str())
    }

    /// Équipement qui porte cette adresse.
    fn owner(&self, ip: Ipv4Addr) -> Option<&DeviceInfo> {
        let text = ip.to_string();
        self.devices.iter().find(|d| {
            d.ifaces
                .iter()
                .any(|i| i.ip.as_deref().and_then(|c| c.split('/').next()) == Some(text.as_str()))
        })
    }
}

/// Gravité d'un constat forensique.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Critique.
    High,
    /// Suspect.
    Medium,
    /// Information.
    Info,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::High => "HIGH",
            Level::Medium => "MEDIUM",
            Level::Info => "INFO",
        }
    }
}

/// Nature d'un comportement détecté.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Behavior {
    /// Un hôte teste de nombreux ports d'une même cible.
    PortScan,
    /// Un hôte teste de nombreuses cibles.
    HostSweep,
    /// Flux entre zones interdit par la politique.
    ZoneViolation,
    /// Connexions périodiques régulières vers une même destination.
    Beaconing,
    /// Paquets refusés en rafale par un équipement de filtrage.
    RepeatedDenials,
    /// Une adresse IP annoncée par plusieurs adresses MAC.
    ArpConflict,
    /// Modification de configuration.
    ConfigChange,
}

impl Behavior {
    /// Nom court (utilisé pour répondre dans un scénario).
    pub fn slug(self) -> &'static str {
        match self {
            Behavior::PortScan => "port-scan",
            Behavior::HostSweep => "host-sweep",
            Behavior::ZoneViolation => "lateral-movement",
            Behavior::Beaconing => "beaconing",
            Behavior::RepeatedDenials => "repeated-denials",
            Behavior::ArpConflict => "arp-spoofing",
            Behavior::ConfigChange => "config-change",
        }
    }
}

/// Un comportement anormal repéré dans la capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicFinding {
    /// Nature.
    pub behavior: Behavior,
    /// Gravité.
    pub level: Level,
    /// Équipement à l'origine (nom).
    pub source: String,
    /// Description.
    pub detail: String,
    /// Premier événement.
    pub first: SimTime,
    /// Dernier événement.
    pub last: SimTime,
}

/// Résultat d'une analyse.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Lignes de la chronologie (une par échange).
    pub timeline: Vec<(SimTime, String)>,
    /// Comportements détectés.
    pub findings: Vec<ForensicFinding>,
}

/// Un échange initié par une machine.
#[derive(Clone, Debug)]
struct Flow {
    time: SimTime,
    src: DeviceId,
    dst: Ipv4Addr,
    proto: &'static str,
    port: u16,
    trace_id: Option<netiv_core::TraceId>,
    /// `Some(true)` réussi, `Some(false)` échoué, `None` sans résultat.
    ok: Option<bool>,
}

fn flows(cap: &Capture) -> Vec<Flow> {
    let mut out: Vec<Flow> = Vec::new();
    for e in &cap.events {
        let start = match &e.kind {
            TraceKind::CurlStarted { dst, port } => Some((*dst, "tcp", *port)),
            TraceKind::UdpSent { dst, port } => Some((*dst, "udp", *port)),
            TraceKind::PingSent { dst, .. } => Some((*dst, "icmp", 0)),
            _ => None,
        };
        if let Some((dst, proto, port)) = start {
            out.push(Flow {
                time: e.time,
                src: e.device,
                dst,
                proto,
                port,
                trace_id: e.trace_id,
                ok: None,
            });
        }
    }
    for e in &cap.events {
        let result = match &e.kind {
            TraceKind::CurlResult { outcome, .. } => {
                Some(matches!(outcome, netiv_core::trace::CurlOutcome::Ok { .. }))
            }
            TraceKind::UdpResult { outcome, .. } => Some(matches!(
                outcome,
                netiv_core::trace::UdpOutcome::Reply { .. }
            )),
            TraceKind::PingReply { .. } => Some(true),
            TraceKind::PingError { .. } | TraceKind::PingTimeout { .. } => Some(false),
            _ => None,
        };
        if let (Some(ok), Some(tid)) = (result, e.trace_id) {
            if let Some(f) = out.iter_mut().find(|f| f.trace_id == Some(tid)) {
                f.ok = Some(ok);
            }
        }
    }
    out
}

/// Politique de zones : qui n'a pas à initier de flux vers qui.
fn forbidden(src_role: &str, dst_role: &str) -> bool {
    matches!(src_role, "internet" | "dmz" | "guest" | "iot")
        && matches!(dst_role, "user" | "server")
}

/// Fenêtre de détection d'un balayage.
const SCAN_WINDOW: SimTime = SimTime::from_millis(30_000);
/// Nombre de cibles / ports distincts à partir duquel on parle de balayage.
const SCAN_THRESHOLD: usize = 5;

/// Formate une durée en `HH:MM:SS.mmm`.
pub fn clock(t: SimTime) -> String {
    let ms = t.as_micros() / 1_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1_000) % 60,
        ms % 1_000
    )
}

/// Analyse une capture.
pub fn analyze_capture(cap: &Capture) -> Analysis {
    let flows = flows(cap);
    let mut findings = Vec::new();
    detect_scans(cap, &flows, &mut findings);
    detect_zone_violations(cap, &flows, &mut findings);
    detect_beaconing(cap, &flows, &mut findings);
    detect_denials(cap, &mut findings);
    detect_arp_conflicts(cap, &mut findings);
    detect_config_changes(cap, &mut findings);
    findings.sort_by_key(|a| (a.level, a.first, a.behavior));

    let mut timeline: Vec<(SimTime, String)> = Vec::new();
    for f in &flows {
        let what = match f.proto {
            "icmp" => "ping".to_string(),
            p => format!("{p}/{}", f.port),
        };
        let dst = cap
            .owner(f.dst)
            .map_or_else(|| f.dst.to_string(), |d| format!("{} ({})", d.name, f.dst));
        let res = match f.ok {
            Some(true) => "réussi",
            Some(false) => "échec",
            None => "sans résultat",
        };
        timeline.push((
            f.time,
            format!("{} → {dst} {what} : {res}", cap.name(f.src)),
        ));
    }
    for e in &cap.events {
        match &e.kind {
            TraceKind::ConfigChanged { description } | TraceKind::FaultApplied { description } => {
                timeline.push((e.time, format!("CONFIG {description}")));
            }
            TraceKind::AclDecision { text, .. } => {
                timeline.push((
                    e.time,
                    format!("{} refuse un paquet : {text}", cap.name(e.device)),
                ));
            }
            _ => {}
        }
    }
    timeline.sort_by_key(|(t, _)| *t);
    Analysis { timeline, findings }
}

fn detect_scans(cap: &Capture, flows: &[Flow], out: &mut Vec<ForensicFinding>) {
    // Un port par cible.
    let mut by_pair: BTreeMap<(DeviceId, Ipv4Addr), Vec<&Flow>> = BTreeMap::new();
    for f in flows.iter().filter(|f| f.proto != "icmp") {
        by_pair.entry((f.src, f.dst)).or_default().push(f);
    }
    for ((src, dst), list) in &by_pair {
        if let Some(window) = densest_window(list, |f| f.port) {
            let ok = window.flows.iter().filter(|f| f.ok == Some(true)).count();
            out.push(ForensicFinding {
                behavior: Behavior::PortScan,
                level: if ok > 0 { Level::High } else { Level::Medium },
                source: cap.name(*src).to_string(),
                detail: format!(
                    "{} a sondé {} ports de {} en {} ({} connexion(s) abouties)",
                    cap.name(*src),
                    window.distinct,
                    cap.owner(*dst).map_or(dst.to_string(), |d| d.name.clone()),
                    window.span_text(),
                    ok
                ),
                first: window.first,
                last: window.last,
            });
        }
    }
    // Une cible par source.
    let mut by_src: BTreeMap<DeviceId, Vec<&Flow>> = BTreeMap::new();
    for f in flows {
        by_src.entry(f.src).or_default().push(f);
    }
    for (src, list) in &by_src {
        if let Some(window) = densest_window(list, |f| u32::from(f.dst)) {
            let alive = window.flows.iter().filter(|f| f.ok == Some(true)).count();
            out.push(ForensicFinding {
                behavior: Behavior::HostSweep,
                level: Level::Medium,
                source: cap.name(*src).to_string(),
                detail: format!(
                    "{} a contacté {} adresses différentes en {} ({} réponse(s))",
                    cap.name(*src),
                    window.distinct,
                    window.span_text(),
                    alive
                ),
                first: window.first,
                last: window.last,
            });
        }
    }
}

struct Window<'a> {
    flows: Vec<&'a Flow>,
    distinct: usize,
    first: SimTime,
    last: SimTime,
}

impl Window<'_> {
    fn span_text(&self) -> String {
        let s = self.last.saturating_since(self.first).as_micros() as f64 / 1_000_000.0;
        format!("{s:.1} s")
    }
}

/// Fenêtre glissante contenant le plus de valeurs distinctes de `key`, si elle
/// atteint le seuil de détection.
fn densest_window<'a, K: Ord + Copy>(
    list: &[&'a Flow],
    key: impl Fn(&Flow) -> K,
) -> Option<Window<'a>> {
    let mut sorted: Vec<&Flow> = list.to_vec();
    sorted.sort_by_key(|f| f.time);
    let mut best: Option<Window<'a>> = None;
    for i in 0..sorted.len() {
        let start = sorted[i].time;
        let in_window: Vec<&Flow> = sorted[i..]
            .iter()
            .copied()
            .take_while(|f| f.time.saturating_since(start) <= SCAN_WINDOW)
            .collect();
        let distinct: BTreeSet<K> = in_window.iter().map(|f| key(f)).collect();
        if distinct.len() >= SCAN_THRESHOLD
            && best.as_ref().is_none_or(|b| distinct.len() > b.distinct)
        {
            let last = in_window.last().map_or(start, |f| f.time);
            best = Some(Window {
                distinct: distinct.len(),
                first: start,
                last,
                flows: in_window,
            });
        }
    }
    best
}

fn detect_zone_violations(cap: &Capture, flows: &[Flow], out: &mut Vec<ForensicFinding>) {
    let mut groups: BTreeMap<(DeviceId, Ipv4Addr, &str, u16), Vec<&Flow>> = BTreeMap::new();
    for f in flows {
        let Some(dst) = cap.owner(f.dst) else {
            continue;
        };
        if forbidden(cap.role(f.src), &dst.role) {
            groups
                .entry((f.src, f.dst, f.proto, f.port))
                .or_default()
                .push(f);
        }
    }
    // On regroupe par (source, cible) : un balayage ne doit pas produire un
    // constat par port.
    let mut by_pair: BTreeMap<(DeviceId, Ipv4Addr), Vec<&Flow>> = BTreeMap::new();
    for ((src, dst, _, _), list) in groups {
        by_pair.entry((src, dst)).or_default().extend(list);
    }
    for ((src, dst), list) in by_pair {
        let ok = list.iter().filter(|f| f.ok == Some(true)).count();
        let ports: BTreeSet<String> = list
            .iter()
            .map(|f| {
                if f.proto == "icmp" {
                    "ping".to_string()
                } else {
                    format!("{}/{}", f.proto, f.port)
                }
            })
            .collect();
        let first = list.iter().map(|f| f.time).min().unwrap_or_default();
        let last = list.iter().map(|f| f.time).max().unwrap_or_default();
        let target = cap
            .owner(dst)
            .map_or(dst.to_string(), |d| format!("{} ({})", d.name, d.role));
        out.push(ForensicFinding {
            behavior: Behavior::ZoneViolation,
            level: if ok > 0 { Level::High } else { Level::Medium },
            source: cap.name(src).to_string(),
            detail: format!(
                "{} ({}) a initié {} flux vers {target} ({}) : {} réussi(s) — flux interdit entre ces zones",
                cap.name(src),
                cap.role(src),
                list.len(),
                ports.iter().cloned().collect::<Vec<_>>().join(", "),
                ok
            ),
            first,
            last,
        });
    }
}

fn detect_beaconing(cap: &Capture, flows: &[Flow], out: &mut Vec<ForensicFinding>) {
    let mut groups: BTreeMap<(DeviceId, Ipv4Addr, &str, u16), Vec<SimTime>> = BTreeMap::new();
    for f in flows {
        groups
            .entry((f.src, f.dst, f.proto, f.port))
            .or_default()
            .push(f.time);
    }
    for ((src, dst, proto, port), mut times) in groups {
        times.sort();
        if times.len() < 5 {
            continue;
        }
        let gaps: Vec<f64> = times
            .windows(2)
            .map(|w| w[1].saturating_since(w[0]).as_micros() as f64)
            .collect();
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        if mean <= 0.0 {
            continue;
        }
        let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64;
        let cv = var.sqrt() / mean;
        // Régulier (peu de dispersion) et assez lent pour n'être ni une rafale
        // ni un balayage (qui est déjà traité à part).
        if cv <= 0.25 && mean >= 1_000_000.0 {
            out.push(ForensicFinding {
                behavior: Behavior::Beaconing,
                level: Level::Medium,
                source: cap.name(src).to_string(),
                detail: format!(
                    "{} contacte {} sur {} toutes les {:.1} s ({} fois) : comportement de balise",
                    cap.name(src),
                    cap.owner(dst).map_or(dst.to_string(), |d| d.name.clone()),
                    if proto == "icmp" {
                        "ping".to_string()
                    } else {
                        format!("{proto}/{port}")
                    },
                    mean / 1_000_000.0,
                    times.len()
                ),
                first: times[0],
                last: times[times.len() - 1],
            });
        }
    }
}

fn detect_denials(cap: &Capture, out: &mut Vec<ForensicFinding>) {
    let mut groups: BTreeMap<(DeviceId, Ipv4Addr), Vec<SimTime>> = BTreeMap::new();
    for e in &cap.events {
        if !matches!(e.kind, TraceKind::AclDecision { .. }) {
            continue;
        }
        let src = e
            .caused_by
            .and_then(|s| usize::try_from(s).ok())
            .and_then(|i| cap.events.get(i))
            .and_then(|rx| match &rx.kind {
                TraceKind::FrameReceived { frame, .. } => match &frame.payload {
                    netiv_core::packet::EtherPayload::Ipv4(p) => Some(p.src),
                    netiv_core::packet::EtherPayload::Arp(_) => None,
                },
                _ => None,
            });
        if let Some(src) = src {
            groups.entry((e.device, src)).or_default().push(e.time);
        }
    }
    for ((dev, src), times) in groups {
        if times.len() >= SCAN_THRESHOLD {
            out.push(ForensicFinding {
                behavior: Behavior::RepeatedDenials,
                level: Level::Medium,
                source: cap.owner(src).map_or(src.to_string(), |d| d.name.clone()),
                detail: format!(
                    "{} a refusé {} paquets en provenance de {src}",
                    cap.name(dev),
                    times.len()
                ),
                first: times[0],
                last: times[times.len() - 1],
            });
        }
    }
}

fn detect_arp_conflicts(cap: &Capture, out: &mut Vec<ForensicFinding>) {
    let mut claims: BTreeMap<Ipv4Addr, BTreeMap<String, (DeviceId, SimTime)>> = BTreeMap::new();
    for e in &cap.events {
        if let TraceKind::FrameSent { frame, .. } = &e.kind {
            if let netiv_core::packet::EtherPayload::Arp(a) = &frame.payload {
                claims
                    .entry(a.sender_ip)
                    .or_default()
                    .entry(a.sender_mac.to_string())
                    .or_insert((e.device, e.time));
            }
        }
    }
    for (ip, macs) in claims {
        if macs.len() > 1 {
            let who: Vec<String> = macs
                .iter()
                .map(|(mac, (d, _))| format!("{} ({mac})", cap.name(*d)))
                .collect();
            let first = macs.values().map(|(_, t)| *t).min().unwrap_or_default();
            let last = macs.values().map(|(_, t)| *t).max().unwrap_or_default();
            out.push(ForensicFinding {
                behavior: Behavior::ArpConflict,
                level: Level::High,
                source: macs.values().next().map_or("?".into(), |(d, _)| cap.name(*d).to_string()),
                detail: format!(
                    "l'adresse {ip} est annoncée par {} adresses MAC différentes : {} — usurpation ARP possible",
                    macs.len(),
                    who.join(", ")
                ),
                first,
                last,
            });
        }
    }
}

fn detect_config_changes(cap: &Capture, out: &mut Vec<ForensicFinding>) {
    for e in &cap.events {
        if let TraceKind::ConfigChanged { description } | TraceKind::FaultApplied { description } =
            &e.kind
        {
            out.push(ForensicFinding {
                behavior: Behavior::ConfigChange,
                level: Level::Info,
                source: cap.name(e.device).to_string(),
                detail: description.clone(),
                first: e.time,
                last: e.time,
            });
        }
    }
}

impl Analysis {
    /// Contient-il ce comportement ?
    pub fn has(&self, b: Behavior) -> bool {
        self.findings.iter().any(|f| f.behavior == b)
    }

    /// Comportements détectés, hors changements de configuration.
    pub fn suspicious(&self) -> impl Iterator<Item = &ForensicFinding> {
        self.findings
            .iter()
            .filter(|f| f.behavior != Behavior::ConfigChange)
    }

    /// Rendu texte : chronologie puis anomalies.
    pub fn render(&self, cap: &Capture) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "ANALYSE FORENSIQUE — réseau « {} », {} événements",
            cap.network,
            cap.events.len()
        );
        let _ = writeln!(s, "\nCHRONOLOGIE");
        if self.timeline.is_empty() {
            let _ = writeln!(s, "  (aucun échange dans cette capture)");
        }
        for (t, line) in &self.timeline {
            let _ = writeln!(s, "  {}  {line}", clock(*t));
        }
        let _ = writeln!(s, "\nANOMALIES");
        let mut any = false;
        for f in self.suspicious() {
            any = true;
            let span = if f.first == f.last {
                clock(f.first)
            } else {
                format!("{}–{}", clock(f.first), clock(f.last))
            };
            let _ = writeln!(s, "  [{}] {span}  {}", f.level.label(), f.detail);
        }
        if !any {
            let _ = writeln!(s, "  rien de suspect détecté");
        }
        s
    }

    /// Chronologie détaillée, trame par trame (`--packets`).
    pub fn render_packets(cap: &Capture) -> String {
        let mut s = String::new();
        for e in &cap.events {
            if let TraceKind::FrameSent { iface, frame } = &e.kind {
                let ifname = cap
                    .devices
                    .iter()
                    .find(|d| d.id == e.device)
                    .and_then(|d| d.ifaces.get(iface.0 as usize))
                    .map_or("?", |i| i.name.as_str());
                let _ = writeln!(
                    s,
                    "{}  {}/{ifname}  {frame}",
                    clock(e.time),
                    cap.name(e.device)
                );
            }
        }
        s
    }
}
