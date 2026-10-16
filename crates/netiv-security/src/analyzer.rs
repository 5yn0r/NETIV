//! Security Analyzer : détecte les défauts de **conception** d'une topologie.
//!
//! Chaque constat est établi par une mesure (une sonde dans une copie de la
//! simulation, un calcul sur le graphe), jamais par une simple heuristique de
//! nom : « le serveur X est joignable depuis les postes sans aucun filtrage »
//! veut dire qu'un `curl` y a réussi sur le chemin relevé.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::net::Ipv4Addr;

use netiv_core::acl::{AclAction, RuleProto};
use netiv_core::device::Behavior;
use netiv_core::packet::{EtherPayload, L4Proto};
use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, DeviceKind, IfaceId, Simulation, TraceKind};
use netiv_diagnostics::l2::{L2Options, l2_domain};
use netiv_diagnostics::{Probe, ProbeKind, parallel, run_probe};

/// Gravité d'un constat.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// À corriger en priorité.
    High,
    /// Important.
    Medium,
    /// Amélioration souhaitable.
    Low,
}

impl Severity {
    /// Libellé court.
    pub fn label(self) -> &'static str {
        match self {
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        }
    }

    /// Points retirés au score de sécurité.
    pub fn penalty(self) -> u32 {
        match self {
            Severity::High => 25,
            Severity::Medium => 10,
            Severity::Low => 3,
        }
    }
}

/// Un défaut de conception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Identifiant stable de la règle (`SEC-002`).
    pub id: &'static str,
    /// Gravité.
    pub severity: Severity,
    /// Titre.
    pub title: String,
    /// Équipements concernés.
    pub devices: Vec<String>,
    /// Ce qui a été constaté.
    pub detail: String,
    /// Comment corriger.
    pub recommendation: String,
}

/// Rapport d'analyse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Constats, triés par gravité puis identifiant.
    pub findings: Vec<Finding>,
}

impl Report {
    /// Nombre de constats par gravité : `(high, medium, low)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |s| self.findings.iter().filter(|f| f.severity == s).count();
        (n(Severity::High), n(Severity::Medium), n(Severity::Low))
    }

    /// Le rapport contient-il ce constat ?
    pub fn has(&self, id: &str) -> bool {
        self.findings.iter().any(|f| f.id == id)
    }

    /// Constat par identifiant.
    pub fn get(&self, id: &str) -> Option<&Finding> {
        self.findings.iter().find(|f| f.id == id)
    }

    /// Score de sécurité de 0 à 100 (100 = rien à signaler).
    pub fn score(&self) -> u32 {
        let lost: u32 = self.findings.iter().map(|f| f.severity.penalty()).sum();
        100u32.saturating_sub(lost)
    }

    /// Rendu texte (§17).
    pub fn render(&self) -> String {
        let mut s = String::new();
        if self.findings.is_empty() {
            let _ = writeln!(s, "ANALYSE DE SÉCURITÉ — rien à signaler.");
            return s;
        }
        let (h, m, l) = self.counts();
        let _ = writeln!(
            s,
            "ANALYSE DE SÉCURITÉ — {h} critique(s), {m} moyen(s), {l} faible(s) — score {}/100\n",
            self.score()
        );
        for f in &self.findings {
            let _ = writeln!(s, "[{}] {} — {}", f.severity.label(), f.id, f.title);
            let _ = writeln!(s, "        {}", f.detail);
            let _ = writeln!(s, "        → {}\n", f.recommendation);
        }
        s
    }
}

/// Analyse complète d'une topologie.
pub fn analyze(sim: &Simulation) -> Report {
    let mut a = Analyzer {
        sim,
        memo: BTreeMap::new(),
        pings: BTreeMap::new(),
        findings: Vec::new(),
    };
    a.prewarm_pings();
    a.segmentation();
    a.server_isolation();
    a.unfiltered_paths();
    a.management_exposure();
    a.cross_zone_access();
    a.permissive_rules();
    a.cleartext_services();
    a.trunks();
    a.single_points_of_failure();
    let mut findings = a.findings;
    findings.sort_by(|x, y| (x.severity, x.id, &x.devices).cmp(&(y.severity, y.id, &y.devices)));
    Report { findings }
}

// ---------------------------------------------------------------------------

struct Analyzer<'a> {
    sim: &'a Simulation,
    /// Résultats des sondes TCP/UDP : (source, destination, 1 = tcp / 2 = udp, port).
    memo: BTreeMap<(DeviceId, Ipv4Addr, u8, u16), Option<bool>>,
    /// Résultats des pings, avec les routeurs traversés à l'aller.
    pings: BTreeMap<(DeviceId, Ipv4Addr), Option<PingInfo>>,
    findings: Vec<Finding>,
}

/// Ce qu'un ping de sonde a appris.
#[derive(Clone, Debug)]
struct PingInfo {
    success: bool,
    /// Routeurs et pare-feu qui ont reçu la requête, dans l'ordre.
    routers: Vec<DeviceId>,
}

/// Lance un ping de sonde et relève les équipements de filtrage traversés à
/// l'aller (un routeur / pare-feu qui reçoit un paquet destiné à `dst`).
fn probe_ping(sim: &Simulation, src: DeviceId, dst: Ipv4Addr) -> Option<PingInfo> {
    let kind = ProbeKind::Ping { count: 1 };
    let r = run_probe(sim, &Probe { src, dst, kind }).ok()?;
    let mut routers = Vec::new();
    for e in &r.events {
        if let TraceKind::FrameReceived { frame, .. } = &e.kind {
            if let EtherPayload::Ipv4(p) = &frame.payload {
                let router = sim
                    .device(e.device)
                    .is_some_and(|d| matches!(d.kind, DeviceKind::Router | DeviceKind::Firewall));
                if p.dst == dst && router && !routers.contains(&e.device) {
                    routers.push(e.device);
                }
            }
        }
    }
    Some(PingInfo {
        success: r.success,
        routers,
    })
}

/// Une première adresse IP de l'équipement.
fn first_ip(sim: &Simulation, d: DeviceId) -> Option<Ipv4Addr> {
    sim.device(d)?
        .ifaces
        .iter()
        .find_map(|i| i.ip.map(|n| n.addr()))
}

impl Analyzer<'_> {
    fn name(&self, d: DeviceId) -> String {
        self.sim.device(d).map_or("?".into(), |x| x.name.clone())
    }

    fn role(&self, d: DeviceId) -> &str {
        self.sim.device(d).map_or("network", |x| x.effective_role())
    }

    fn end_devices(&self) -> Vec<DeviceId> {
        self.sim
            .devices()
            .iter()
            .filter(|d| matches!(d.kind, DeviceKind::Host | DeviceKind::Server))
            .map(|d| d.id)
            .collect()
    }

    fn with_role(&self, roles: &[&str]) -> Vec<DeviceId> {
        self.end_devices()
            .into_iter()
            .filter(|d| roles.contains(&self.role(*d)))
            .collect()
    }

    fn push(
        &mut self,
        id: &'static str,
        severity: Severity,
        title: &str,
        devices: Vec<String>,
        detail: String,
        recommendation: &str,
    ) {
        self.findings.push(Finding {
            id,
            severity,
            title: title.to_string(),
            devices,
            detail,
            recommendation: recommendation.to_string(),
        });
    }

    // -------------------------------------------------------------- mesures

    /// `src` joint-il `dst` ? Essaie le ping puis chaque service TCP/UDP
    /// connu de la destination ; `Some(true)` si au moins un aboutit.
    fn reaches(&mut self, src: DeviceId, dst: Ipv4Addr) -> bool {
        self.reach_kind(src, dst, 0, 0) == Some(true) || self.reaches_service(src, dst)
    }

    fn reaches_service(&mut self, src: DeviceId, dst: Ipv4Addr) -> bool {
        let services: Vec<(L4Proto, u16)> = self
            .sim
            .devices()
            .iter()
            .filter(|d| {
                d.ifaces
                    .iter()
                    .any(|i| i.ip.is_some_and(|n| n.addr() == dst))
            })
            .flat_map(|d| match &d.behavior {
                Behavior::Ip(n) => n
                    .services
                    .iter()
                    .map(|s| (s.proto, s.port))
                    .collect::<Vec<_>>(),
                Behavior::Switch(_) => Vec::new(),
            })
            .collect();
        services.into_iter().any(|(proto, port)| {
            let k = if proto == L4Proto::Tcp { 1 } else { 2 };
            self.reach_kind(src, dst, k, port) == Some(true)
        })
    }

    /// Résultat (mémorisé) d'un ping de sonde.
    fn ping_info(&mut self, src: DeviceId, dst: Ipv4Addr) -> Option<&PingInfo> {
        let sim = self.sim;
        self.pings
            .entry((src, dst))
            .or_insert_with(|| probe_ping(sim, src, dst))
            .as_ref()
    }

    /// Lance en parallèle les pings dont les contrôles auront besoin
    /// (postes → serveurs, postes → administration, zones externes →
    /// interne). Les contrôles lisent ensuite le cache : le résultat est le
    /// même qu'en séquentiel, en une fraction du temps.
    fn prewarm_pings(&mut self) {
        let mut pairs: BTreeSet<(DeviceId, Ipv4Addr)> = BTreeSet::new();
        let ip_of = |d: &DeviceId| first_ip(self.sim, *d);
        let servers: Vec<Ipv4Addr> = self
            .with_role(&["server"])
            .iter()
            .filter_map(ip_of)
            .collect();
        for u in self.with_role(&["user"]) {
            pairs.extend(servers.iter().map(|ip| (u, *ip)));
        }
        let mgmt: Vec<(DeviceId, Ipv4Addr)> = self
            .sim
            .devices()
            .iter()
            .flat_map(|d| {
                d.ifaces
                    .iter()
                    .filter(|i| i.management)
                    .filter_map(move |i| i.ip.map(|n| (d.id, n.addr())))
            })
            .collect();
        for u in self.with_role(&["user", "guest", "iot"]) {
            pairs.extend(mgmt.iter().filter(|(d, _)| *d != u).map(|(_, ip)| (u, *ip)));
        }
        let internal: Vec<(DeviceId, Ipv4Addr)> = self
            .with_role(&["user", "server"])
            .into_iter()
            .filter_map(|d| first_ip(self.sim, d).map(|ip| (d, ip)))
            .collect();
        for s in self.with_role(&["internet", "dmz"]) {
            pairs.extend(
                internal
                    .iter()
                    .filter(|(d, _)| *d != s)
                    .map(|(_, ip)| (s, *ip)),
            );
        }
        let todo: Vec<(DeviceId, Ipv4Addr)> = pairs
            .into_iter()
            .filter(|k| !self.pings.contains_key(k))
            .collect();
        // Une seule copie sans historique, partagée par toutes les sondes.
        let base = self.sim.fresh_clone();
        let results = parallel::par_map(&todo, |(src, dst)| probe_ping(&base, *src, *dst));
        self.pings.extend(todo.into_iter().zip(results));
    }

    /// kind : 0 = ping, 1 = tcp, 2 = udp.
    fn reach_kind(&mut self, src: DeviceId, dst: Ipv4Addr, kind: u8, port: u16) -> Option<bool> {
        if kind == 0 {
            return self.ping_info(src, dst).map(|p| p.success);
        }
        let key = (src, dst, kind, port);
        if let Some(v) = self.memo.get(&key) {
            return *v;
        }
        let pk = match kind {
            1 => ProbeKind::Tcp { port },
            _ => ProbeKind::Udp { port },
        };
        let r = run_probe(self.sim, &Probe { src, dst, kind: pk })
            .ok()
            .map(|r| r.success);
        self.memo.insert(key, r);
        r
    }

    /// Équipements de filtrage traversés à l'aller par un flux `src → dst`.
    fn routers_on_path(&mut self, src: DeviceId, dst: Ipv4Addr) -> Vec<DeviceId> {
        self.ping_info(src, dst)
            .map(|p| p.routers.clone())
            .unwrap_or_default()
    }

    /// Domaines de diffusion : groupes d'interfaces IP qui communiquent en
    /// couche 2 (à travers les switches, VLAN compris).
    fn broadcast_domains(&self) -> Vec<BTreeSet<(DeviceId, IfaceId)>> {
        let opts = L2Options {
            vlan_aware: true,
            honor_state: false,
        };
        let mut nodes: Vec<(DeviceId, IfaceId)> = Vec::new();
        for d in self.sim.devices() {
            if d.kind == DeviceKind::Switch {
                continue;
            }
            for i in &d.ifaces {
                if i.link.is_some() {
                    nodes.push((d.id, i.id));
                }
            }
        }
        let mut seen: BTreeSet<(DeviceId, IfaceId)> = BTreeSet::new();
        let mut out = Vec::new();
        for n in &nodes {
            if seen.contains(n) {
                continue;
            }
            let mut group: BTreeSet<(DeviceId, IfaceId)> = l2_domain(self.sim, n.0, n.1, opts);
            group.insert(*n);
            seen.extend(group.iter().copied());
            out.push(group);
        }
        out
    }

    // --------------------------------------------------------------- règles

    /// SEC-001 : réseau « plat ».
    fn segmentation(&mut self) {
        let ends = self.end_devices();
        if ends.len() < 3 {
            return;
        }
        let domains = self.broadcast_domains();
        let with_ends: Vec<_> = domains
            .iter()
            .filter(|g| g.iter().any(|(d, _)| ends.contains(d)))
            .collect();
        if with_ends.len() == 1 {
            let names: Vec<String> = ends.iter().map(|d| self.name(*d)).collect();
            self.push(
                "SEC-001",
                Severity::Medium,
                "Aucune segmentation du réseau",
                names.clone(),
                format!(
                    "Les {} machines ({}) partagent un même domaine de diffusion : n'importe laquelle peut atteindre les autres sans traverser le moindre équipement de filtrage.",
                    names.len(),
                    names.join(", ")
                ),
                "Découper en VLAN / sous-réseaux par fonction (postes, serveurs, invités) et les relier par un routeur ou un pare-feu.",
            );
        }
    }

    /// SEC-002 : serveurs mêlés aux postes utilisateurs.
    fn server_isolation(&mut self) {
        let domains = self.broadcast_domains();
        let mut shared: BTreeSet<String> = BTreeSet::new();
        let mut servers: BTreeSet<String> = BTreeSet::new();
        for g in &domains {
            let devs: BTreeSet<DeviceId> = g.iter().map(|(d, _)| *d).collect();
            let s: Vec<DeviceId> = devs
                .iter()
                .copied()
                .filter(|d| self.role(*d) == "server" && self.is_end(*d))
                .collect();
            let u: Vec<DeviceId> = devs
                .iter()
                .copied()
                .filter(|d| self.role(*d) == "user" && self.is_end(*d))
                .collect();
            if !s.is_empty() && !u.is_empty() {
                servers.extend(s.iter().map(|d| self.name(*d)));
                shared.extend(u.iter().map(|d| self.name(*d)));
            }
        }
        if !servers.is_empty() {
            let mut all: Vec<String> = servers.iter().cloned().collect();
            all.extend(shared.iter().cloned());
            self.push(
                "SEC-002",
                Severity::High,
                "Les serveurs ne sont pas isolés des postes utilisateurs",
                all,
                format!(
                    "{} partage(nt) le même réseau local que {} : un poste compromis voit directement les serveurs (ARP, broadcast, aucun filtrage possible).",
                    servers.iter().cloned().collect::<Vec<_>>().join(", "),
                    shared.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
                "Placer les serveurs dans un VLAN / réseau dédié, derrière un pare-feu qui n'ouvre que les services nécessaires.",
            );
        }
    }

    fn is_end(&self, d: DeviceId) -> bool {
        self.sim
            .device(d)
            .is_some_and(|x| matches!(x.kind, DeviceKind::Host | DeviceKind::Server))
    }

    /// SEC-003 : un poste atteint un serveur à travers des routeurs sans filtrage.
    fn unfiltered_paths(&mut self) {
        let users = self.with_role(&["user"]);
        let servers = self.with_role(&["server"]);
        let mut open: BTreeSet<String> = BTreeSet::new();
        let mut sources: BTreeSet<String> = BTreeSet::new();
        for s in &servers {
            let Some(ip) = first_ip(self.sim, *s) else {
                continue;
            };
            for u in &users {
                if !self.reaches(*u, ip) {
                    continue;
                }
                let routers = self.routers_on_path(*u, ip);
                if routers.is_empty() {
                    continue; // même réseau : relève de SEC-002
                }
                let filtered = routers.iter().any(|r| self.filters(*r));
                if !filtered {
                    open.insert(self.name(*s));
                    sources.insert(self.name(*u));
                }
            }
        }
        if !open.is_empty() {
            let mut all: Vec<String> = open.iter().cloned().collect();
            all.extend(sources.iter().cloned());
            self.push(
                "SEC-003",
                Severity::Medium,
                "Aucun filtrage entre les postes et les serveurs",
                all,
                format!(
                    "{} atteint(atteignent) {} à travers des routeurs qui laissent tout passer : tous les services des serveurs sont exposés aux postes.",
                    sources.iter().cloned().collect::<Vec<_>>().join(", "),
                    open.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
                "Insérer un pare-feu (ou des ACL) entre les postes et les serveurs, avec une politique « deny » par défaut et des règles limitées aux services utilisés.",
            );
        }
    }

    /// Un équipement applique-t-il un filtrage ?
    fn filters(&self, d: DeviceId) -> bool {
        match self.sim.device(d).map(|x| &x.behavior) {
            Some(Behavior::Ip(n)) => !n.acl.rules.is_empty() || n.acl.default == AclAction::Deny,
            _ => false,
        }
    }

    /// SEC-004 : interface d'administration joignable depuis les postes.
    fn management_exposure(&mut self) {
        let users = self.with_role(&["user", "guest", "iot"]);
        let mut exposed: Vec<(String, String)> = Vec::new();
        for dev in self.sim.devices() {
            for i in dev.ifaces.iter().filter(|i| i.management) {
                let Some(net) = i.ip else { continue };
                for u in &users {
                    if *u == dev.id {
                        continue;
                    }
                    if self.reaches(*u, net.addr()) {
                        exposed.push((dev.name.clone(), self.name(*u)));
                    }
                }
            }
        }
        if exposed.is_empty() {
            return;
        }
        let targets: BTreeSet<String> = exposed.iter().map(|(t, _)| t.clone()).collect();
        let from: BTreeSet<String> = exposed.iter().map(|(_, u)| u.clone()).collect();
        let mut all: Vec<String> = targets.iter().cloned().collect();
        all.extend(from.iter().cloned());
        self.push(
            "SEC-004",
            Severity::High,
            "Interface d'administration accessible depuis le réseau utilisateurs",
            all,
            format!(
                "L'interface de management de {} répond à {} : un poste utilisateur peut tenter de se connecter à l'administration de l'équipement.",
                targets.iter().cloned().collect::<Vec<_>>().join(", "),
                from.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            "Isoler l'administration dans un réseau dédié (out-of-band ou VLAN de management) et n'y autoriser que les postes d'administration.",
        );
    }

    /// SEC-005 (Internet → interne) et SEC-006 (DMZ → LAN).
    fn cross_zone_access(&mut self) {
        let internal = self.with_role(&["user", "server"]);
        let zones = [
            (
                "internet",
                "SEC-005",
                "Un hôte externe atteint le réseau interne",
                "Depuis Internet, {src} atteint {dst} sans que le pare-feu ne s'y oppose : le réseau interne est exposé.",
                "Interdire tout flux entrant initié depuis Internet vers l'interne ; ne publier que les services de la DMZ.",
            ),
            (
                "dmz",
                "SEC-006",
                "La DMZ peut initier des connexions vers le réseau interne",
                "{src} (DMZ) atteint {dst} (réseau interne) : un serveur exposé compromis donne accès direct au LAN.",
                "Ajouter des règles « deny » de la DMZ vers le LAN, et ne laisser passer que les réponses aux flux initiés par le LAN (pare-feu à suivi de connexion).",
            ),
        ];
        for (role, id, title, detail, advice) in zones {
            let sources = self.with_role(&[role]);
            let mut pairs: Vec<(String, String)> = Vec::new();
            for s in &sources {
                for t in &internal {
                    if s == t {
                        continue;
                    }
                    let Some(ip) = first_ip(self.sim, *t) else {
                        continue;
                    };
                    if self.reaches(*s, ip) {
                        pairs.push((self.name(*s), self.name(*t)));
                    }
                }
            }
            if pairs.is_empty() {
                continue;
            }
            let srcs: BTreeSet<String> = pairs.iter().map(|p| p.0.clone()).collect();
            let dsts: BTreeSet<String> = pairs.iter().map(|p| p.1.clone()).collect();
            let mut all: Vec<String> = srcs.iter().cloned().collect();
            all.extend(dsts.iter().cloned());
            let text = detail
                .replace(
                    "{src}",
                    &srcs.iter().cloned().collect::<Vec<_>>().join(", "),
                )
                .replace(
                    "{dst}",
                    &dsts.iter().cloned().collect::<Vec<_>>().join(", "),
                );
            self.push(id, Severity::High, title, all, text, advice);
        }
    }

    /// SEC-007 (règle allow any any) et SEC-008 (pare-feu « allow » par défaut).
    fn permissive_rules(&mut self) {
        let mut any_any: Vec<(String, usize)> = Vec::new();
        let mut open_fw: Vec<String> = Vec::new();
        for d in self.sim.devices() {
            let Behavior::Ip(n) = &d.behavior else {
                continue;
            };
            for (i, r) in n.acl.rules.iter().enumerate() {
                if r.action == AclAction::Allow
                    && r.proto == RuleProto::Any
                    && r.src.is_none_or(|n| n.prefix() == 0)
                    && r.dst.is_none_or(|n| n.prefix() == 0)
                    && r.port_from.is_none()
                {
                    any_any.push((d.name.clone(), i + 1));
                }
            }
            if d.kind == DeviceKind::Firewall && n.acl.default == AclAction::Allow {
                open_fw.push(d.name.clone());
            }
        }
        if !any_any.is_empty() {
            let devs: Vec<String> = any_any.iter().map(|(d, _)| d.clone()).collect();
            let list = any_any
                .iter()
                .map(|(d, n)| format!("{d} (règle {n})"))
                .collect::<Vec<_>>()
                .join(", ");
            self.push(
                "SEC-007",
                Severity::Medium,
                "Règle de filtrage trop permissive (allow any any)",
                devs,
                format!("{list} autorise(nt) tout trafic de n'importe quelle source vers n'importe quelle destination : la règle annule l'intérêt du filtrage."),
                "Remplacer par des règles précises (protocole, réseaux, ports) et laisser la politique par défaut à « deny ».",
            );
        }
        if !open_fw.is_empty() {
            self.push(
                "SEC-008",
                Severity::High,
                "Pare-feu en politique « allow » par défaut",
                open_fw.clone(),
                format!(
                    "{} laisse passer tout ce qu'aucune règle n'interdit explicitement : le moindre oubli ouvre le réseau.",
                    open_fw.join(", ")
                ),
                "Passer la politique par défaut à « deny » (acl default <équipement> deny) et autoriser explicitement les flux légitimes.",
            );
        }
    }

    /// SEC-009 : protocoles non chiffrés.
    fn cleartext_services(&mut self) {
        const CLEARTEXT: [&str; 5] = ["telnet", "ftp", "tftp", "rsh", "snmp"];
        let mut found: Vec<(String, String)> = Vec::new();
        for d in self.sim.devices() {
            if let Behavior::Ip(n) = &d.behavior {
                for s in &n.services {
                    if CLEARTEXT.contains(&s.name.as_str()) {
                        found.push((
                            d.name.clone(),
                            format!("{} ({}/{})", s.name, s.proto, s.port),
                        ));
                    }
                }
            }
        }
        if !found.is_empty() {
            let devs: BTreeSet<String> = found.iter().map(|(d, _)| d.clone()).collect();
            self.push(
                "SEC-009",
                Severity::Medium,
                "Service non chiffré exposé",
                devs.iter().cloned().collect(),
                found
                    .iter()
                    .map(|(d, s)| format!("{d} : {s}"))
                    .collect::<Vec<_>>()
                    .join(" ; ")
                    + " — identifiants et données circulent en clair.",
                "Remplacer par une variante chiffrée (SSH, SFTP, SNMPv3) ou restreindre l'accès à un réseau d'administration.",
            );
        }
    }

    /// SEC-010 : trunk qui transporte tous les VLAN.
    fn trunks(&mut self) {
        let mut wide: Vec<String> = Vec::new();
        for d in self.sim.devices() {
            let Behavior::Switch(sw) = &d.behavior else {
                continue;
            };
            for i in &d.ifaces {
                if let PortMode::Trunk { allowed, .. } = sw.port_mode(i.id) {
                    if allowed.is_empty() {
                        wide.push(format!("{}/{}", d.name, i.name));
                    }
                }
            }
        }
        if !wide.is_empty() {
            self.push(
                "SEC-010",
                Severity::Low,
                "Trunk ouvert à tous les VLAN",
                wide.clone(),
                format!("{} transporte(nt) tous les VLAN, y compris ceux qui n'ont pas à traverser ce lien.", wide.join(", ")),
                "Limiter chaque trunk aux VLAN nécessaires (port <switch> <port> trunk 10,20).",
            );
        }
    }

    /// SEC-011 : points uniques de défaillance.
    fn single_points_of_failure(&mut self) {
        let ends = self.end_devices();
        if ends.len() < 2 {
            return;
        }
        let n = self.sim.devices().len();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for l in self.sim.links() {
            let (a, b) = (l.a.0.0 as usize, l.b.0.0 as usize);
            if a < n && b < n {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        let reachable_ends = |removed: Option<usize>| -> usize {
            // Nombre de paires de machines qui communiquent encore.
            let mut seen = vec![false; n];
            let mut pairs = 0usize;
            for e in &ends {
                let s = e.0 as usize;
                if seen[s] || Some(s) == removed {
                    continue;
                }
                let mut comp = 0usize;
                let mut q = VecDeque::from([s]);
                seen[s] = true;
                while let Some(x) = q.pop_front() {
                    if ends.iter().any(|e| e.0 as usize == x) {
                        comp += 1;
                    }
                    for &y in &adj[x] {
                        if !seen[y] && Some(y) != removed {
                            seen[y] = true;
                            q.push_back(y);
                        }
                    }
                }
                pairs += comp * comp.saturating_sub(1) / 2;
            }
            pairs
        };
        let baseline = reachable_ends(None);
        let mut spof: Vec<String> = Vec::new();
        let mut critical = false;
        for d in self.sim.devices() {
            if matches!(d.kind, DeviceKind::Host | DeviceKind::Server) {
                continue;
            }
            if reachable_ends(Some(d.id.0 as usize)) < baseline {
                spof.push(d.name.clone());
                critical |= matches!(d.kind, DeviceKind::Router | DeviceKind::Firewall);
            }
        }
        if !spof.is_empty() {
            self.push(
                "SEC-011",
                if critical { Severity::Medium } else { Severity::Low },
                "Pas de redondance : points uniques de défaillance",
                spof.clone(),
                format!(
                    "La panne de {} coupe des machines entre elles : aucun chemin de secours n'existe.",
                    spof.join(", ")
                ),
                "Doubler les équipements et les liens critiques (deuxième routeur, liens agrégés, topologie maillée).",
            );
        }
    }
}
