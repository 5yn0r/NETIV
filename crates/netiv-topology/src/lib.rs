//! Description, validation et chargement de topologies NETIV (YAML).
//!
//! Pipeline : `YAML → TopologyDef (serde) → validation → Simulation`.
//!
//! La validation distingue deux niveaux :
//! * les **erreurs** (structure impossible : nom dupliqué, interface
//!   inexistante...) empêchent la construction ;
//! * les **avertissements** (IP en double, passerelle hors sous-réseau...)
//!   n'empêchent rien : un laboratoire de dépannage contient volontairement
//!   des réseaux mal configurés.

pub mod generate;

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::packet::L4Proto;
use netiv_core::service::Service;
use netiv_core::switch::PortMode;
use netiv_core::{
    DeviceId, DeviceKind, IfaceId, Ipv4Net, LinkParams, SimTime, Simulation, StaticRoute,
};
use serde::{Deserialize, Serialize};

/// Description complète d'un réseau.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyDef {
    /// Métadonnées.
    pub network: NetworkDef,
    /// Équipements.
    #[serde(default)]
    pub devices: Vec<DeviceDef>,
    /// Câbles.
    #[serde(default)]
    pub links: Vec<LinkDef>,
}

/// Métadonnées du réseau.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkDef {
    /// Nom du réseau.
    pub name: String,
    /// Graine du générateur aléatoire (0 par défaut).
    #[serde(default)]
    pub seed: u64,
}

/// Un équipement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDef {
    /// Nom unique.
    pub name: String,
    /// `host`, `server`, `router`, `firewall` ou `switch`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Rôle : `user`, `server`, `dmz`, `internet`, `network`, `guest`, `iot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Interfaces explicites.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<InterfaceDef>,
    /// Switch : nombre de ports `port1..portN` à créer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<u32>,
    /// Passerelle par défaut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    /// Routes statiques.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RouteDef>,
    /// Services à l'écoute.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceDef>,
    /// Entrées ARP statiques.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arp: Vec<ArpDef>,
    /// Filtrage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acl: Option<AclDef>,
}

/// Une interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceDef {
    /// Nom (`eth0`).
    pub name: String,
    /// Adresse en notation CIDR (`192.168.1.10/24`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// Interface réservée à l'administration de l'équipement.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub management: bool,
    /// Switch : port d'accès dans ce VLAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// Switch : port trunk transportant ces VLAN (liste vide = tous).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk: Option<Vec<u16>>,
    /// Switch : VLAN natif d'un port trunk (1 par défaut).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_vlan: Option<u16>,
}

/// Une entrée ARP statique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArpDef {
    /// Adresse IP.
    pub ip: String,
    /// Adresse MAC associée.
    pub mac: String,
}

/// Un service à l'écoute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDef {
    /// `tcp` ou `udp`.
    pub proto: String,
    /// Port d'écoute.
    pub port: u16,
    /// Nom : `http`, `ssh`, `echo`...
    pub name: String,
}

/// Règles de filtrage d'un équipement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AclDef {
    /// Politique par défaut : `allow` ou `deny`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Suivi de connexion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stateful: Option<bool>,
    /// Règles, évaluées dans l'ordre.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleDefAcl>,
}

/// Une règle de filtrage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDefAcl {
    /// `allow` ou `deny`.
    pub action: String,
    /// `any` (défaut), `icmp`, `tcp` ou `udp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<String>,
    /// Réseau source (CIDR).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    /// Réseau destination (CIDR).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<String>,
    /// Port destination : `80` ou `1000-2000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Interface d'entrée (nom).
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub in_iface: Option<String>,
    /// Commentaire.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// Une route statique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDef {
    /// Réseau de destination (CIDR).
    pub dest: String,
    /// Prochain saut.
    pub via: String,
}

/// Un câble entre deux interfaces (`Équipement:interface`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkDef {
    /// Première extrémité.
    pub a: String,
    /// Seconde extrémité.
    pub b: String,
    /// Latence en microsecondes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_us: Option<u64>,
    /// Probabilité de perte (0 à 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss: Option<f64>,
}

/// Erreurs de chargement, toutes regroupées pour que l'utilisateur les
/// corrige en une seule passe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyErrors {
    /// Messages, un par problème.
    pub errors: Vec<String>,
}

impl fmt::Display for TopologyErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "- {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TopologyErrors {}

/// Résultat d'un chargement réussi.
#[derive(Debug)]
pub struct Loaded {
    /// Nom du réseau.
    pub name: String,
    /// Simulation prête à l'emploi (trace vide, `t = 0`).
    pub sim: Simulation,
    /// Problèmes de configuration non bloquants.
    pub warnings: Vec<String>,
}

/// Charge une topologie depuis un texte YAML.
pub fn load_yaml(text: &str) -> Result<Loaded, TopologyErrors> {
    let def: TopologyDef = serde_yaml_ng::from_str(text).map_err(|e| TopologyErrors {
        errors: vec![format!("YAML invalide : {e}")],
    })?;
    build(&def)
}

/// Construit la simulation décrite par `def`.
pub fn build(def: &TopologyDef) -> Result<Loaded, TopologyErrors> {
    let mut errors: Vec<String> = Vec::new();
    let mut sim = Simulation::new(def.network.seed);
    let mut ids: BTreeMap<&str, DeviceId> = BTreeMap::new();

    // 1. Équipements, interfaces, configuration IP.
    for d in &def.devices {
        let ctx = format!("équipement « {} »", d.name);
        let kind: DeviceKind = match d.kind.parse() {
            Ok(k) => k,
            Err(e) => {
                errors.push(format!("{ctx} : {e}"));
                continue;
            }
        };
        let id = match sim.add_device(&d.name, kind) {
            Ok(id) => id,
            Err(e) => {
                errors.push(format!("{ctx} : {e}"));
                continue;
            }
        };
        ids.insert(&d.name, id);
        if let Some(role) = &d.role {
            if let Err(e) = sim.set_role(id, Some(role)) {
                errors.push(format!("{ctx} : {e}"));
            }
        }

        if d.ports.is_some() && kind != DeviceKind::Switch {
            errors.push(format!("{ctx} : « ports » n'a de sens que pour un switch"));
        }
        if kind == DeviceKind::Switch {
            if d.gateway.is_some() || !d.routes.is_empty() {
                errors.push(format!("{ctx} : un switch n'a ni passerelle ni routes"));
            }
            for n in 1..=d.ports.unwrap_or(0) {
                if let Err(e) = sim.add_interface(id, &format!("port{n}"), None) {
                    errors.push(format!("{ctx} : {e}"));
                }
            }
        }
        for i in &d.interfaces {
            let ip = match i.ip.as_deref().map(str::parse::<Ipv4Net>) {
                None => None,
                Some(Ok(n)) => Some(n),
                Some(Err(e)) => {
                    errors.push(format!("{ctx}, interface « {} » : {e}", i.name));
                    continue;
                }
            };
            match sim.add_interface(id, &i.name, ip) {
                Ok(iface) => {
                    if i.management {
                        if kind == DeviceKind::Switch {
                            errors.push(format!(
                                "{ctx}, interface « {} » : « management » n'a de sens que sur un équipement IP",
                                i.name
                            ));
                        } else if let Err(e) = sim.set_management(id, iface, true) {
                            errors.push(format!("{ctx} : {e}"));
                        }
                    }
                }
                Err(e) => errors.push(format!("{ctx} : {e}")),
            }
        }
        apply_port_modes(&mut sim, id, d, kind, &ctx, &mut errors);
        apply_services(&mut sim, id, d, kind, &ctx, &mut errors);
        apply_acl(&mut sim, id, d, kind, &ctx, &mut errors);
        apply_static_arp(&mut sim, id, d, kind, &ctx, &mut errors);
        if let Some(gw) = &d.gateway {
            match gw.parse::<Ipv4Addr>() {
                Ok(g) if kind != DeviceKind::Switch => {
                    if let Err(e) = sim.set_gateway(id, Some(g)) {
                        errors.push(format!("{ctx} : {e}"));
                    }
                }
                Ok(_) => {}
                Err(_) => errors.push(format!("{ctx} : passerelle invalide : « {gw} »")),
            }
        }
        for r in &d.routes {
            let (dest, via) = (r.dest.parse::<Ipv4Net>(), r.via.parse::<Ipv4Addr>());
            match (dest, via) {
                (Ok(dest), Ok(via)) if kind != DeviceKind::Switch => {
                    if let Err(e) = sim.add_route(id, StaticRoute { dest, via }) {
                        errors.push(format!("{ctx} : {e}"));
                    }
                }
                (Ok(_), Ok(_)) => {}
                (Err(e), _) => errors.push(format!("{ctx}, route : {e}")),
                (_, Err(_)) => errors.push(format!(
                    "{ctx}, route : prochain saut invalide : « {} »",
                    r.via
                )),
            }
        }
    }

    // 2. Câbles.
    for (n, l) in def.links.iter().enumerate() {
        let ctx = format!("lien n°{} ({} ↔ {})", n + 1, l.a, l.b);
        let a = resolve_end(&sim, &ids, &l.a);
        let b = resolve_end(&sim, &ids, &l.b);
        let (a, b) = match (a, b) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                for e in [a.err(), b.err()].into_iter().flatten() {
                    errors.push(format!("{ctx} : {e}"));
                }
                continue;
            }
        };
        let params = LinkParams {
            latency: l
                .latency_us
                .map_or(LinkParams::default().latency, SimTime::from_micros),
            loss: l.loss.unwrap_or(0.0),
        };
        if let Err(e) = sim.connect(a, b, params) {
            errors.push(format!("{ctx} : {e}"));
        }
    }

    if !errors.is_empty() {
        return Err(TopologyErrors { errors });
    }

    let warnings = lint(&sim);
    sim.clear_trace();
    Ok(Loaded {
        name: def.network.name.clone(),
        sim,
        warnings,
    })
}

fn valid_vlan(v: u16) -> bool {
    (1..=4094).contains(&v)
}

fn apply_port_modes(
    sim: &mut Simulation,
    id: DeviceId,
    d: &DeviceDef,
    kind: DeviceKind,
    ctx: &str,
    errors: &mut Vec<String>,
) {
    for i in &d.interfaces {
        if i.vlan.is_none() && i.trunk.is_none() && i.native_vlan.is_none() {
            continue;
        }
        let where_ = format!("{ctx}, interface « {} »", i.name);
        if kind != DeviceKind::Switch {
            errors.push(format!(
                "{where_} : « vlan » / « trunk » ne s'appliquent qu'aux ports de switch"
            ));
            continue;
        }
        let mode = match (i.vlan, &i.trunk) {
            (Some(_), Some(_)) => {
                errors.push(format!(
                    "{where_} : « vlan » (accès) et « trunk » sont exclusifs"
                ));
                continue;
            }
            (Some(v), None) => {
                if i.native_vlan.is_some() {
                    errors.push(format!(
                        "{where_} : « native_vlan » n'a de sens que sur un trunk"
                    ));
                    continue;
                }
                PortMode::Access { vlan: v }
            }
            (None, Some(list)) => PortMode::Trunk {
                allowed: list.clone(),
                native: i.native_vlan.unwrap_or(netiv_core::switch::DEFAULT_VLAN),
            },
            (None, None) => {
                errors.push(format!(
                    "{where_} : « native_vlan » n'a de sens que sur un trunk"
                ));
                continue;
            }
        };
        let vlans: Vec<u16> = match &mode {
            PortMode::Access { vlan } => vec![*vlan],
            PortMode::Trunk { allowed, native } => allowed
                .iter()
                .copied()
                .chain(std::iter::once(*native))
                .collect(),
        };
        if let Some(bad) = vlans.iter().find(|v| !valid_vlan(**v)) {
            errors.push(format!("{where_} : VLAN {bad} invalide (attendu 1 à 4094)"));
            continue;
        }
        match sim.iface_id(id, &i.name) {
            Ok(iface) => {
                if let Err(e) = sim.set_port_mode(id, iface, mode) {
                    errors.push(format!("{where_} : {e}"));
                }
            }
            Err(e) => errors.push(format!("{where_} : {e}")),
        }
    }
}

fn apply_static_arp(
    sim: &mut Simulation,
    id: DeviceId,
    d: &DeviceDef,
    kind: DeviceKind,
    ctx: &str,
    errors: &mut Vec<String>,
) {
    if d.arp.is_empty() {
        return;
    }
    if kind == DeviceKind::Switch {
        errors.push(format!("{ctx} : un switch n'a pas de table ARP"));
        return;
    }
    for a in &d.arp {
        match (
            a.ip.parse::<Ipv4Addr>(),
            a.mac.parse::<netiv_core::MacAddr>(),
        ) {
            (Ok(ip), Ok(mac)) => {
                if let Err(e) = sim.set_static_arp(id, ip, Some(mac)) {
                    errors.push(format!("{ctx} : {e}"));
                }
            }
            (Err(_), _) => errors.push(format!("{ctx}, arp : adresse IPv4 invalide « {} »", a.ip)),
            (_, Err(e)) => errors.push(format!("{ctx}, arp : {e}")),
        }
    }
}

fn parse_proto(s: &str) -> Option<L4Proto> {
    match s.to_ascii_lowercase().as_str() {
        "tcp" => Some(L4Proto::Tcp),
        "udp" => Some(L4Proto::Udp),
        _ => None,
    }
}

fn apply_services(
    sim: &mut Simulation,
    id: DeviceId,
    d: &DeviceDef,
    kind: DeviceKind,
    ctx: &str,
    errors: &mut Vec<String>,
) {
    if d.services.is_empty() {
        return;
    }
    if kind == DeviceKind::Switch {
        errors.push(format!("{ctx} : un switch n'héberge pas de services"));
        return;
    }
    for sv in &d.services {
        let Some(proto) = parse_proto(&sv.proto) else {
            errors.push(format!(
                "{ctx}, service « {} » : protocole inconnu « {} » (tcp ou udp)",
                sv.name, sv.proto
            ));
            continue;
        };
        if sv.port == 0 {
            errors.push(format!(
                "{ctx}, service « {} » : le port 0 est invalide",
                sv.name
            ));
            continue;
        }
        let service = Service {
            proto,
            port: sv.port,
            name: sv.name.clone(),
        };
        if let Err(e) = sim.add_service(id, service) {
            errors.push(format!("{ctx} : {e}"));
        }
    }
}

/// `"80"` → `(80, 80)` ; `"1000-2000"` → `(1000, 2000)`.
fn parse_ports(s: &str) -> Option<(u16, u16)> {
    match s.split_once('-') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
            (a <= b).then_some((a, b))
        }
        None => {
            let p = s.trim().parse().ok()?;
            Some((p, p))
        }
    }
}

fn parse_action(s: &str) -> Option<AclAction> {
    match s.to_ascii_lowercase().as_str() {
        "allow" | "permit" => Some(AclAction::Allow),
        "deny" | "drop" => Some(AclAction::Deny),
        _ => None,
    }
}

fn apply_acl(
    sim: &mut Simulation,
    id: DeviceId,
    d: &DeviceDef,
    kind: DeviceKind,
    ctx: &str,
    errors: &mut Vec<String>,
) {
    let Some(acl) = &d.acl else {
        return;
    };
    if kind == DeviceKind::Switch {
        errors.push(format!(
            "{ctx} : un switch n'a pas de règles de filtrage IP"
        ));
        return;
    }
    if let Some(def) = &acl.default {
        match parse_action(def) {
            Some(a) => {
                let _ = sim.acl_set_default(id, a);
            }
            None => errors.push(format!(
                "{ctx}, acl : politique par défaut inconnue « {def} » (allow ou deny)"
            )),
        }
    }
    if let Some(st) = acl.stateful {
        let _ = sim.acl_set_stateful(id, st);
    }
    for (n, r) in acl.rules.iter().enumerate() {
        let where_ = format!("{ctx}, règle n°{}", n + 1);
        match rule_from_def(sim, id, r) {
            Ok(rule) => {
                if let Err(e) = sim.acl_add_rule(id, rule) {
                    errors.push(format!("{where_} : {e}"));
                }
            }
            Err(problems) => errors.extend(problems.into_iter().map(|p| format!("{where_} : {p}"))),
        }
    }
}

/// Convertit une règle de fichier en règle du moteur. `device` sert à résoudre
/// le nom de l'interface d'entrée. Retourne tous les problèmes trouvés.
pub fn rule_from_def(
    sim: &Simulation,
    device: DeviceId,
    r: &RuleDefAcl,
) -> Result<AclRule, Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    let Some(action) = parse_action(&r.action) else {
        return Err(vec![format!(
            "action inconnue « {} » (allow ou deny)",
            r.action
        )]);
    };
    let proto = match r.proto.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("any") => RuleProto::Any,
        Some("icmp") => RuleProto::Icmp,
        Some("tcp") => RuleProto::Tcp,
        Some("udp") => RuleProto::Udp,
        Some(other) => return Err(vec![format!("protocole inconnu « {other} »")]),
    };
    let mut rule = AclRule::new(action, proto);
    rule.comment = r.comment.clone();
    for (field, slot) in [(&r.src, &mut rule.src), (&r.dst, &mut rule.dst)] {
        if let Some(text) = field {
            match text.parse::<Ipv4Net>() {
                Ok(n) => *slot = Some(n),
                Err(e) => problems.push(e.to_string()),
            }
        }
    }
    if let Some(text) = &r.port {
        match parse_ports(text) {
            Some((a, b)) => {
                if !matches!(proto, RuleProto::Tcp | RuleProto::Udp | RuleProto::Any) {
                    problems.push("un port n'a de sens que pour tcp, udp ou any".to_string());
                }
                rule.port_from = Some(a);
                rule.port_to = Some(b);
            }
            None => problems.push(format!("port invalide « {text} » (80 ou 1000-2000)")),
        }
    }
    if let Some(name) = &r.in_iface {
        match sim.iface_id(device, name) {
            Ok(i) => rule.in_iface = Some(i),
            Err(e) => problems.push(e.to_string()),
        }
    }
    if problems.is_empty() {
        Ok(rule)
    } else {
        Err(problems)
    }
}

/// Convertit une règle du moteur en règle de fichier.
pub fn rule_to_def(sim: &Simulation, device: DeviceId, r: &AclRule) -> RuleDefAcl {
    let dev = sim.device(device);
    RuleDefAcl {
        action: r.action.to_string(),
        proto: (r.proto != RuleProto::Any).then(|| r.proto.to_string()),
        src: r.src.map(|n| n.trunc().to_string()),
        dst: r.dst.map(|n| n.trunc().to_string()),
        port: match (r.port_from, r.port_to) {
            (Some(a), Some(b)) if a != b => Some(format!("{a}-{b}")),
            (Some(a), _) => Some(a.to_string()),
            _ => None,
        },
        in_iface: r.in_iface.map(|i| {
            dev.and_then(|d| d.iface(i))
                .map_or_else(|| "?".to_string(), |x| x.name.clone())
        }),
        comment: r.comment.clone(),
    }
}

fn resolve_end(
    sim: &Simulation,
    ids: &BTreeMap<&str, DeviceId>,
    text: &str,
) -> Result<(DeviceId, IfaceId), String> {
    let (dev, iface) = text
        .split_once(':')
        .ok_or_else(|| format!("extrémité « {text} » invalide (attendu Équipement:interface)"))?;
    let dev_id = ids
        .get(dev)
        .copied()
        .ok_or_else(|| format!("équipement inconnu : « {dev} »"))?;
    let iface_id = sim.iface_id(dev_id, iface).map_err(|e| e.to_string())?;
    Ok((dev_id, iface_id))
}

/// Détecte les configurations suspectes (sans les refuser).
pub fn lint(sim: &Simulation) -> Vec<String> {
    use netiv_core::device::Behavior;
    let mut w = Vec::new();

    // Adresses IP en double.
    let mut owners: BTreeMap<Ipv4Addr, Vec<String>> = BTreeMap::new();
    for d in sim.devices() {
        for i in &d.ifaces {
            if let Some(n) = i.ip {
                owners
                    .entry(n.addr())
                    .or_default()
                    .push(format!("{}/{}", d.name, i.name));
            }
        }
    }
    for (ip, who) in owners {
        if who.len() > 1 {
            w.push(format!(
                "adresse IP {ip} utilisée plusieurs fois : {}",
                who.join(", ")
            ));
        }
    }

    for d in sim.devices() {
        let on_link = |ip: Ipv4Addr| {
            d.ifaces
                .iter()
                .any(|i| i.ip.is_some_and(|n| n.contains(ip)))
        };
        if let Behavior::Ip(node) = &d.behavior {
            if let Some(gw) = node.gateway {
                if !on_link(gw) {
                    w.push(format!(
                        "{} : la passerelle {gw} n'est dans aucun réseau connecté de l'équipement",
                        d.name
                    ));
                }
            }
            for r in &node.static_routes {
                if !on_link(r.via) {
                    w.push(format!(
                        "{} : la route {} via {} est inutilisable (prochain saut hors réseau connecté)",
                        d.name,
                        r.dest.trunc(),
                        r.via
                    ));
                }
            }
        }
        for i in &d.ifaces {
            if i.ip.is_some() && i.link.is_none() {
                w.push(format!(
                    "{}/{} a une adresse IP mais aucun câble",
                    d.name, i.name
                ));
            }
        }
    }

    // Câble entre deux interfaces IP de réseaux différents.
    for l in sim.links() {
        let ip_of = |(d, i): (DeviceId, IfaceId)| {
            sim.device(d)
                .and_then(|dev| dev.iface(i))
                .and_then(|x| x.ip)
        };
        if let (Some(a), Some(b)) = (ip_of(l.a), ip_of(l.b)) {
            if a.trunc() != b.trunc() {
                w.push(format!(
                    "câble {} ↔ {} : les deux extrémités sont dans des réseaux différents ({} et {})",
                    end_name(sim, l.a),
                    end_name(sim, l.b),
                    a.trunc(),
                    b.trunc()
                ));
            }
        }
    }

    // Cohérence VLAN entre deux switches reliés.
    for l in sim.links() {
        let mode = |(d, i): (DeviceId, IfaceId)| match sim.device(d).map(|dev| &dev.behavior) {
            Some(Behavior::Switch(sw)) => Some(sw.port_mode(i)),
            _ => None,
        };
        let (Some(ma), Some(mb)) = (mode(l.a), mode(l.b)) else {
            continue;
        };
        let (ea, eb) = (end_name(sim, l.a), end_name(sim, l.b));
        match (ma, mb) {
            (PortMode::Trunk { native: x, .. }, PortMode::Trunk { native: y, .. }) if x != y => {
                w.push(format!(
                    "trunk {ea} ↔ {eb} : VLAN natif différent aux deux extrémités ({x} et {y}) : les trames non étiquetées changeront de VLAN"
                ));
            }
            (PortMode::Access { vlan: x }, PortMode::Access { vlan: y }) if x != y => {
                w.push(format!(
                    "câble {ea} ↔ {eb} : ports d'accès dans des VLAN différents ({x} et {y})"
                ));
            }
            (PortMode::Access { .. }, PortMode::Trunk { .. })
            | (PortMode::Trunk { .. }, PortMode::Access { .. }) => {
                w.push(format!(
                    "câble {ea} ↔ {eb} : un port d'accès est relié à un trunk"
                ));
            }
            _ => {}
        }
    }

    // Pare-feu qui bloque tout.
    for d in sim.devices() {
        if let Behavior::Ip(node) = &d.behavior {
            if node.acl.default == AclAction::Deny && node.acl.rules.is_empty() {
                w.push(format!(
                    "{} : politique « deny » sans aucune règle : tout le trafic entrant est bloqué",
                    d.name
                ));
            }
        }
    }
    w
}

/// Extrémité d'un câble au format fichier : `Équipement:interface`.
fn endpoint(sim: &Simulation, (d, i): (DeviceId, IfaceId)) -> String {
    format!(
        "{}:{}",
        netiv_core::render::device_name(sim, d),
        netiv_core::render::iface_name(sim, d, i)
    )
}

fn end_name(sim: &Simulation, (d, i): (DeviceId, IfaceId)) -> String {
    format!(
        "{}/{}",
        netiv_core::render::device_name(sim, d),
        netiv_core::render::iface_name(sim, d, i)
    )
}

// ---------------------------------------------------------------------------
// Export et format `.netiv`
// ---------------------------------------------------------------------------

/// Version courante du format `.netiv`.
pub const NETIV_FORMAT_VERSION: u32 = 1;

/// Décrit l'état **courant** d'une simulation (y compris les pannes déjà
/// injectées) sous forme de topologie. Les équipements et interfaces sont
/// émis dans l'ordre de création : recharger le résultat redonne les mêmes
/// adresses MAC.
pub fn export(sim: &Simulation, name: &str) -> TopologyDef {
    use netiv_core::device::Behavior;
    let devices = sim
        .devices()
        .iter()
        .map(|d| {
            let interfaces = d
                .ifaces
                .iter()
                .map(|i| {
                    let mode = match &d.behavior {
                        Behavior::Switch(sw) => sw.ports.get(&i.id).cloned(),
                        Behavior::Ip(_) => None,
                    };
                    let (vlan, trunk, native_vlan) = match mode {
                        Some(PortMode::Access { vlan }) => (Some(vlan), None, None),
                        Some(PortMode::Trunk { allowed, native }) => {
                            (None, Some(allowed), Some(native))
                        }
                        None => (None, None, None),
                    };
                    InterfaceDef {
                        name: i.name.clone(),
                        ip: i.ip.map(|n| n.to_string()),
                        management: i.management,
                        vlan,
                        trunk,
                        native_vlan,
                    }
                })
                .collect();
            let (gateway, routes, services, acl) = match &d.behavior {
                Behavior::Ip(n) => (
                    n.gateway.map(|g| g.to_string()),
                    n.static_routes
                        .iter()
                        .map(|r| RouteDef {
                            dest: r.dest.trunc().to_string(),
                            via: r.via.to_string(),
                        })
                        .collect(),
                    n.services
                        .iter()
                        .map(|sv| ServiceDef {
                            proto: sv.proto.to_string(),
                            port: sv.port,
                            name: sv.name.clone(),
                        })
                        .collect(),
                    export_acl(d, &n.acl),
                ),
                Behavior::Switch(_) => (None, Vec::new(), Vec::new(), None),
            };
            let arp = match &d.behavior {
                Behavior::Ip(n) => n
                    .static_arp
                    .iter()
                    .map(|(ip, mac)| ArpDef {
                        ip: ip.to_string(),
                        mac: mac.to_string(),
                    })
                    .collect(),
                Behavior::Switch(_) => Vec::new(),
            };
            DeviceDef {
                name: d.name.clone(),
                kind: d.kind.to_string(),
                role: d.role.clone(),
                interfaces,
                ports: None,
                gateway,
                routes,
                services,
                arp,
                acl,
            }
        })
        .collect();
    let links = sim
        .links()
        .iter()
        .map(|l| LinkDef {
            a: endpoint(sim, l.a),
            b: endpoint(sim, l.b),
            latency_us: Some(l.latency.as_micros()),
            loss: (l.loss > 0.0).then_some(l.loss),
        })
        .collect();
    TopologyDef {
        network: NetworkDef {
            name: name.to_string(),
            seed: sim.seed(),
        },
        devices,
        links,
    }
}

/// Règles d'un équipement ; `None` si elles sont identiques aux valeurs par
/// défaut de son type (rien à écrire dans le fichier).
fn export_acl(d: &netiv_core::Device, acl: &netiv_core::acl::Acl) -> Option<AclDef> {
    let base = if d.kind == DeviceKind::Firewall {
        netiv_core::acl::Acl::firewall()
    } else {
        netiv_core::acl::Acl::default()
    };
    if acl.rules.is_empty() && acl.default == base.default && acl.stateful == base.stateful {
        return None;
    }
    let iface_name = |id: IfaceId| {
        d.iface(id)
            .map_or_else(|| "?".to_string(), |i| i.name.clone())
    };
    Some(AclDef {
        default: (acl.default != base.default).then(|| acl.default.to_string()),
        stateful: (acl.stateful != base.stateful).then_some(acl.stateful),
        rules: acl
            .rules
            .iter()
            .map(|r| RuleDefAcl {
                action: r.action.to_string(),
                proto: (r.proto != RuleProto::Any).then(|| r.proto.to_string()),
                src: r.src.map(|n| n.trunc().to_string()),
                dst: r.dst.map(|n| n.trunc().to_string()),
                port: match (r.port_from, r.port_to) {
                    (Some(a), Some(b)) if a != b => Some(format!("{a}-{b}")),
                    (Some(a), _) => Some(a.to_string()),
                    _ => None,
                },
                in_iface: r.in_iface.map(iface_name),
                comment: r.comment.clone(),
            })
            .collect(),
    })
}

/// Informations descriptives d'un lab partagé.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BundleMeta {
    /// Titre.
    pub title: String,
    /// Description libre.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Auteur.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
}

/// Fichier `.netiv` : une topologie et, en option, un scénario ou un
/// challenge (laissés en YAML brut : leurs formats appartiennent aux crates
/// qui les définissent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// Version du format.
    pub format_version: u32,
    /// Métadonnées.
    pub metadata: BundleMeta,
    /// Le réseau.
    pub topology: TopologyDef,
    /// Scénario (trafic, pannes...) optionnel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<serde_yaml_ng::Value>,
    /// Challenge optionnel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge: Option<serde_yaml_ng::Value>,
}

impl Bundle {
    /// Empaquette une topologie.
    pub fn new(meta: BundleMeta, topology: TopologyDef) -> Self {
        Bundle {
            format_version: NETIV_FORMAT_VERSION,
            metadata: meta,
            topology,
            scenario: None,
            challenge: None,
        }
    }

    /// Sérialise en YAML.
    pub fn to_yaml(&self) -> Result<String, TopologyErrors> {
        serde_yaml_ng::to_string(self).map_err(|e| TopologyErrors {
            errors: vec![format!("sérialisation impossible : {e}")],
        })
    }

    /// Lit un bundle `.netiv` (YAML ou JSON).
    pub fn from_yaml(text: &str) -> Result<Self, TopologyErrors> {
        let b: Bundle = serde_yaml_ng::from_str(text).map_err(|e| TopologyErrors {
            errors: vec![format!("fichier .netiv invalide : {e}")],
        })?;
        if b.format_version > NETIV_FORMAT_VERSION {
            return Err(TopologyErrors {
                errors: vec![format!(
                    "format .netiv version {} non supporté (cette version de NETIV lit jusqu'à la version {NETIV_FORMAT_VERSION})",
                    b.format_version
                )],
            });
        }
        Ok(b)
    }
}

/// Charge n'importe quel fichier de lab : un bundle `.netiv` (reconnu à sa
/// clé `format_version`) ou une topologie simple.
pub fn load_any(text: &str) -> Result<(Option<Bundle>, Loaded), TopologyErrors> {
    let probe: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|e| TopologyErrors {
            errors: vec![format!("YAML invalide : {e}")],
        })?;
    let is_bundle = probe
        .as_mapping()
        .is_some_and(|m| m.contains_key("format_version"));
    if is_bundle {
        let bundle = Bundle::from_yaml(text)?;
        let loaded = build(&bundle.topology)?;
        Ok((Some(bundle), loaded))
    } else {
        Ok((None, load_yaml(text)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MVP: &str = r#"
network:
  name: demo
  seed: 42
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 192.168.1.10/24 }], gateway: 192.168.1.1 }
  - { name: SW1, type: switch, ports: 2 }
  - name: R1
    type: router
    interfaces:
      - { name: eth0, ip: 192.168.1.1/24 }
      - { name: eth1, ip: 10.0.0.1/30 }
    routes:
      - { dest: 192.168.2.0/24, via: 10.0.0.2 }
links:
  - { a: "PC1:eth0", b: "SW1:port1" }
  - { a: "SW1:port2", b: "R1:eth0", latency_us: 500 }
"#;

    fn errs(yaml: &str) -> Vec<String> {
        load_yaml(yaml).unwrap_err().errors
    }

    #[test]
    fn loads_a_valid_topology() {
        let l = load_yaml(MVP).unwrap();
        assert_eq!(l.name, "demo");
        assert_eq!(l.sim.devices().len(), 3);
        assert_eq!(l.sim.links().len(), 2);
        assert!(
            l.sim.trace().is_empty(),
            "la configuration ne doit pas polluer la trace"
        );
        // R1/eth1 a une IP mais pas de câble : seul avertissement attendu.
        assert_eq!(l.warnings.len(), 1, "{:?}", l.warnings);
    }

    #[test]
    fn custom_latency_is_applied() {
        let l = load_yaml(MVP).unwrap();
        assert_eq!(l.sim.links()[1].latency, SimTime::from_micros(500));
        assert_eq!(l.sim.links()[0].latency, LinkParams::default().latency);
    }

    #[test]
    fn switch_ports_are_created() {
        let l = load_yaml(MVP).unwrap();
        let sw = l.sim.device(l.sim.device_id("SW1").unwrap()).unwrap();
        let names: Vec<_> = sw.ifaces.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["port1", "port2"]);
    }

    #[test]
    fn rejects_malformed_yaml_and_unknown_fields() {
        assert!(
            errs("network: [")
                .iter()
                .any(|e| e.starts_with("YAML invalide"))
        );
        let e = errs("network: {name: x}\ndevices: [{name: A, type: host, colour: red}]");
        assert!(e[0].contains("colour"), "{e:?}");
    }

    #[test]
    fn reports_every_structural_error_at_once() {
        let e = errs(
            r#"
network: { name: x }
devices:
  - { name: A, type: toaster }
  - { name: B, type: host, interfaces: [{ name: eth0, ip: 300.1.1.1/24 }] }
  - { name: B, type: host }
  - { name: C, type: host, ports: 4 }
  - { name: S, type: switch, gateway: 1.1.1.1 }
links:
  - { a: "B:eth9", b: "Z:eth0" }
  - { a: "oops", b: "B:eth0" }
"#,
        );
        let all = e.join("\n");
        for needle in [
            "toaster",
            "300.1.1.1",
            "existe déjà",
            "ports",
            "ni passerelle",
            "eth9",
            "Z",
            "Équipement:interface",
        ] {
            assert!(all.contains(needle), "« {needle} » manque dans :\n{all}");
        }
    }

    #[test]
    fn interface_cannot_have_two_cables() {
        let e = errs(
            r#"
network: { name: x }
devices:
  - { name: A, type: host, interfaces: [{ name: eth0 }] }
  - { name: B, type: host, interfaces: [{ name: eth0 }] }
  - { name: C, type: host, interfaces: [{ name: eth0 }] }
links:
  - { a: "A:eth0", b: "B:eth0" }
  - { a: "A:eth0", b: "C:eth0" }
"#,
        );
        assert!(e[0].contains("déjà reliée"), "{e:?}");
    }

    #[test]
    fn misconfigurations_are_warnings_not_errors() {
        let l = load_yaml(
            r#"
network: { name: lab }
devices:
  - { name: A, type: host, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }], gateway: 10.9.9.9 }
  - { name: B, type: host, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }] }
  - { name: C, type: host, interfaces: [{ name: eth0, ip: 172.16.0.1/24 }] }
  - { name: D, type: router, interfaces: [{ name: eth0, ip: 10.5.0.1/24 }], routes: [{ dest: 1.0.0.0/8, via: 99.0.0.1 }] }
links:
  - { a: "A:eth0", b: "B:eth0" }
  - { a: "C:eth0", b: "D:eth0" }
"#,
        )
        .unwrap();
        let all = l.warnings.join("\n");
        for needle in [
            "utilisée plusieurs fois",
            "passerelle 10.9.9.9",
            "inutilisable",
            "réseaux différents",
        ] {
            assert!(all.contains(needle), "« {needle} » manque dans :\n{all}");
        }
    }

    #[test]
    fn seed_comes_from_the_file() {
        let mut l = load_yaml(MVP).unwrap();
        let mut expected = netiv_core::SimRng::new(42);
        assert_eq!(l.sim.rng_mut().next_u64(), expected.next_u64());
    }

    fn ping_trace(l: &mut Loaded) -> Vec<netiv_core::TraceEvent> {
        let pc = l.sim.device_id("PC1").unwrap();
        l.sim.ping(pc, "192.168.2.10".parse().unwrap(), 1).unwrap();
        l.sim.run_until_idle().unwrap();
        l.sim.trace().events().to_vec()
    }

    const FULL: &str = include_str!("../../../examples/mvp.yaml");

    #[test]
    fn export_then_reload_gives_the_same_network() {
        let mut original = load_yaml(FULL).unwrap();
        let yaml = serde_yaml_ng::to_string(&export(&original.sim, "mvp")).unwrap();
        let mut again = load_yaml(&yaml).unwrap();
        assert_eq!(ping_trace(&mut original), ping_trace(&mut again));
    }

    #[test]
    fn export_captures_injected_faults() {
        let mut l = load_yaml(FULL).unwrap();
        let pc = l.sim.device_id("PC1").unwrap();
        l.sim
            .set_gateway(pc, Some("192.168.1.254".parse().unwrap()))
            .unwrap();
        let def = export(&l.sim, "mvp");
        let dev = def.devices.iter().find(|d| d.name == "PC1").unwrap();
        assert_eq!(dev.gateway.as_deref(), Some("192.168.1.254"));
    }

    #[test]
    fn bundle_roundtrip_and_detection() {
        let l = load_yaml(FULL).unwrap();
        let bundle = Bundle::new(
            BundleMeta {
                title: "MVP".into(),
                ..BundleMeta::default()
            },
            export(&l.sim, "mvp"),
        );
        let text = bundle.to_yaml().unwrap();
        let (b, mut loaded) = load_any(&text).unwrap();
        assert_eq!(b.unwrap().metadata.title, "MVP");
        assert_eq!(
            ping_trace(&mut loaded).len(),
            ping_trace(&mut load_yaml(FULL).unwrap()).len()
        );
        // Une topologie simple est aussi acceptée.
        assert!(load_any(FULL).unwrap().0.is_none());
    }

    #[test]
    fn bundle_from_the_future_is_refused_politely() {
        let l = load_yaml(FULL).unwrap();
        let mut b = Bundle::new(BundleMeta::default(), export(&l.sim, "x"));
        b.format_version = 99;
        let e = Bundle::from_yaml(&b.to_yaml().unwrap()).unwrap_err();
        assert!(e.errors[0].contains("version 99"), "{e}");
    }

    #[test]
    fn json_files_are_accepted() {
        let l = load_yaml(FULL).unwrap();
        let json = serde_json::to_string(&export(&l.sim, "mvp")).unwrap();
        assert!(load_yaml(&json).is_ok());
    }

    const SECURE: &str = r#"
network: { name: dmz, seed: 7 }
devices:
  - { name: PC1, type: host, interfaces: [{ name: eth0, ip: 192.168.1.10/24 }], gateway: 192.168.1.1 }
  - name: FW1
    type: firewall
    interfaces:
      - { name: eth0, ip: 192.168.1.1/24 }
      - { name: eth1, ip: 10.9.0.1/24 }
    acl:
      rules:
        - { action: allow, proto: tcp, dst: 10.9.0.0/24, port: "80", in: eth0, comment: web }
        - { action: allow, proto: icmp, in: eth0 }
  - name: SRV
    type: server
    interfaces: [{ name: eth0, ip: 10.9.0.10/24 }]
    gateway: 10.9.0.1
    services:
      - { proto: tcp, port: 80, name: http }
      - { proto: udp, port: 7, name: echo }
  - name: SW1
    type: switch
    interfaces:
      - { name: p1, vlan: 10 }
      - { name: p2, vlan: 20 }
      - { name: tr, trunk: [10, 20], native_vlan: 1 }
links:
  - { a: "PC1:eth0", b: "FW1:eth0" }
  - { a: "FW1:eth1", b: "SRV:eth0" }
"#;

    #[test]
    fn firewall_services_and_vlans_load() {
        let l = load_yaml(SECURE).unwrap();
        let fw = l.sim.device(l.sim.device_id("FW1").unwrap()).unwrap();
        assert_eq!(fw.kind, DeviceKind::Firewall);
        let netiv_core::device::Behavior::Ip(node) = &fw.behavior else {
            panic!("un pare-feu a une pile IP");
        };
        assert_eq!(node.acl.rules.len(), 2);
        assert_eq!(node.acl.default, AclAction::Deny, "défaut d'un pare-feu");
        assert!(node.acl.stateful);
        assert_eq!(node.acl.rules[0].port_from, Some(80));
        assert_eq!(node.acl.rules[0].comment, "web");
        let srv = l.sim.device(l.sim.device_id("SRV").unwrap()).unwrap();
        let netiv_core::device::Behavior::Ip(snode) = &srv.behavior else {
            panic!();
        };
        assert_eq!(snode.services.len(), 2);
        let sw = l.sim.device(l.sim.device_id("SW1").unwrap()).unwrap();
        let netiv_core::device::Behavior::Switch(s) = &sw.behavior else {
            panic!();
        };
        assert_eq!(s.port_mode(IfaceId(0)), PortMode::Access { vlan: 10 });
        assert_eq!(
            s.port_mode(IfaceId(2)),
            PortMode::Trunk {
                allowed: vec![10, 20],
                native: 1
            }
        );
        assert!(l.sim.trace().is_empty());
    }

    #[test]
    fn full_export_roundtrip_preserves_behaviour() {
        let run = |l: &mut Loaded| {
            let pc = l.sim.device_id("PC1").unwrap();
            l.sim
                .command(
                    pc,
                    netiv_core::Command::Curl {
                        dst: "10.9.0.10".parse().unwrap(),
                        port: 80,
                    },
                )
                .unwrap();
            l.sim.run_until_idle().unwrap();
            l.sim.trace().events().to_vec()
        };
        let mut a = load_yaml(SECURE).unwrap();
        let yaml = serde_yaml_ng::to_string(&export(&a.sim, "dmz")).unwrap();
        let mut b = load_yaml(&yaml).unwrap();
        assert_eq!(run(&mut a), run(&mut b));
        // Le pare-feu autorise bien le web, c'est ce qu'on a rechargé.
        assert!(a.sim.trace().events().iter().any(|e| matches!(
            &e.kind,
            netiv_core::TraceKind::CurlResult {
                outcome: netiv_core::trace::CurlOutcome::Ok { .. },
                ..
            }
        )));
    }

    #[test]
    fn export_omits_default_filtering_settings() {
        let l = load_yaml(MVP).unwrap();
        let yaml = serde_yaml_ng::to_string(&export(&l.sim, "x")).unwrap();
        assert!(!yaml.contains("acl"), "{yaml}");
        assert!(!yaml.contains("services"), "{yaml}");
    }

    #[test]
    fn invalid_vlan_acl_and_service_definitions_are_reported() {
        let e = errs(
            r#"
network: { name: x }
devices:
  - name: S
    type: switch
    interfaces:
      - { name: a, vlan: 5000 }
      - { name: b, vlan: 10, trunk: [10] }
      - { name: c, native_vlan: 3 }
      - { name: d, vlan: 7, native_vlan: 2 }
    services: [{ proto: tcp, port: 80, name: http }]
    acl: { rules: [{ action: allow }] }
  - name: H
    type: host
    interfaces: [{ name: eth0, vlan: 10 }]
    services:
      - { proto: sctp, port: 80, name: x }
      - { proto: tcp, port: 0, name: y }
    acl:
      default: maybe
      rules:
        - { action: explode }
        - { action: allow, proto: carrier-pigeon }
        - { action: allow, proto: tcp, port: "8x" }
        - { action: allow, proto: icmp, port: "80" }
        - { action: allow, src: nope }
        - { action: allow, in: eth9 }
"#,
        );
        let all = e.join(
            "
",
        );
        for needle in [
            "VLAN 5000 invalide",
            "exclusifs",
            "n'a de sens que sur un trunk",
            "ne s'appliquent qu'aux ports de switch",
            "n'héberge pas de services",
            "pas de règles de filtrage",
            "protocole inconnu « sctp »",
            "port 0 est invalide",
            "politique par défaut inconnue « maybe »",
            "action inconnue « explode »",
            "protocole inconnu « carrier-pigeon »",
            "port invalide « 8x »",
            "un port n'a de sens que pour tcp, udp ou any",
            "eth9",
        ] {
            assert!(all.contains(needle), "« {needle} » manque dans :\n{all}");
        }
    }

    #[test]
    fn port_range_parsing() {
        assert_eq!(parse_ports("80"), Some((80, 80)));
        assert_eq!(parse_ports("1000-2000"), Some((1000, 2000)));
        assert_eq!(parse_ports("2000-1000"), None);
        assert_eq!(parse_ports("a-b"), None);
        assert_eq!(parse_ports(""), None);
    }

    #[test]
    fn vlan_and_firewall_misconfigurations_are_warnings() {
        let l = load_yaml(
            r#"
network: { name: lab }
devices:
  - { name: S1, type: switch, interfaces: [{ name: t, trunk: [], native_vlan: 10 }, { name: a, vlan: 10 }] }
  - { name: S2, type: switch, interfaces: [{ name: t, trunk: [], native_vlan: 20 }, { name: a, vlan: 30 }] }
  - { name: S3, type: switch, interfaces: [{ name: a, vlan: 40 }] }
  - { name: FW, type: firewall, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }] }
links:
  - { a: "S1:t", b: "S2:t" }
  - { a: "S1:a", b: "S2:a" }
  - { a: "S2:t", b: "S3:a" }
"#,
        );
        // « S2:t » est utilisé deux fois : erreur de câblage, pas de chargement.
        assert!(l.is_err());
        let l = load_yaml(
            r#"
network: { name: lab }
devices:
  - { name: S1, type: switch, interfaces: [{ name: t, trunk: [], native_vlan: 10 }, { name: a, vlan: 10 }, { name: x, vlan: 10 }] }
  - { name: S2, type: switch, interfaces: [{ name: t, trunk: [], native_vlan: 20 }, { name: a, vlan: 30 }, { name: y, trunk: [] }] }
  - { name: FW, type: firewall, interfaces: [{ name: eth0, ip: 10.0.0.1/24 }] }
links:
  - { a: "S1:t", b: "S2:t" }
  - { a: "S1:a", b: "S2:a" }
  - { a: "S1:x", b: "S2:y" }
"#,
        )
        .unwrap();
        let all = l.warnings.join("\n");
        for needle in [
            "VLAN natif différent",
            "VLAN différents (10 et 30)",
            "port d'accès est relié à un trunk",
            "tout le trafic entrant est bloqué",
        ] {
            assert!(all.contains(needle), "« {needle} » manque dans :\n{all}");
        }
    }
}
