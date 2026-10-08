//! What-if : « que se passe-t-il si… ? » sans toucher au réseau réel.
//!
//! On applique le changement sur une **copie**, on mesure les flux avant et
//! après, et on décrit ce qui casse.

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::device::Behavior;
use netiv_core::packet::L4Proto;
use netiv_core::route::build_routes;
use netiv_core::{DeviceId, DeviceKind, Fault, Ipv4Net, Simulation};

use crate::reach::{self, FlowId, Matrix};

/// Un changement hypothétique.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// L'équipement tombe en panne : toutes ses interfaces s'éteignent.
    DeviceDown(String),
    /// On débranche tous les câbles de l'équipement.
    Disconnect(String),
    /// Un protocole (et éventuellement un port) est bloqué par les routeurs et
    /// pare-feu, ou par un seul équipement.
    Block {
        /// Protocole.
        proto: RuleProto,
        /// Port.
        port: Option<u16>,
        /// Équipement visé ; absent = tous les routeurs et pare-feu.
        on: Option<String>,
    },
    /// Une route statique disparaît.
    RemoveRoute {
        /// Réseau.
        dest: Ipv4Net,
        /// Équipement visé ; absent = tous ceux qui l'ont.
        on: Option<String>,
    },
    /// Un câble est coupé (`R1:eth1`).
    CutLink(String),
    /// Un service s'arrête.
    StopService {
        /// Protocole.
        proto: L4Proto,
        /// Port.
        port: u16,
        /// Équipement.
        on: String,
    },
}

fn parse_proto_port(s: &str) -> Result<(RuleProto, Option<u16>), String> {
    let s = s.to_ascii_lowercase();
    let (p, port) = match s.split_once('/') {
        Some((p, port)) => (p, Some(port)),
        None => (s.as_str(), None),
    };
    let proto = match p {
        "tcp" => RuleProto::Tcp,
        "udp" => RuleProto::Udp,
        "icmp" | "ping" => RuleProto::Icmp,
        "any" => RuleProto::Any,
        other => return Err(format!("protocole inconnu : « {other} »")),
    };
    let port = port
        .map(|p| {
            p.parse::<u16>()
                .map_err(|_| format!("port invalide : « {p} »"))
        })
        .transpose()?;
    if port.is_some() && !matches!(proto, RuleProto::Tcp | RuleProto::Udp) {
        return Err("un port n'a de sens que pour tcp ou udp".to_string());
    }
    Ok((proto, port))
}

impl Change {
    /// Lit une phrase : `R1 goes down`, `disconnect SW2`, `block tcp/443`,
    /// `remove route 10.10.0.0/16 [on R1]`, `cut link R1:eth1`,
    /// `stop tcp/80 on SERVER`.
    pub fn parse(text: &str) -> Result<Change, String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
        let w = |i: usize| lower.get(i).map(String::as_str);
        match (w(0), w(1), w(2), w(3)) {
            (Some("disconnect" | "unplug" | "débrancher"), Some(_), None, _) => {
                Ok(Change::Disconnect(words[1].to_string()))
            }
            (Some(_), Some("goes"), Some("down"), None) | (Some(_), Some("down"), None, _) => {
                Ok(Change::DeviceDown(words[0].to_string()))
            }
            (Some("block" | "bloquer"), Some(_), rest, on) => {
                let (proto, port) = parse_proto_port(words[1])?;
                let on = match (rest, on) {
                    (None, _) => None,
                    (Some("on"), Some(_)) if words.len() == 4 => Some(words[3].to_string()),
                    _ => return Err("attendu : block <proto[/port]> [on <équipement>]".to_string()),
                };
                Ok(Change::Block { proto, port, on })
            }
            (Some("remove"), Some("route"), Some(_), rest) => {
                let dest = words[2].parse::<Ipv4Net>().map_err(|e| e.to_string())?;
                let on = match (rest, words.len()) {
                    (None, 3) => None,
                    (Some("on"), 5) => Some(words[4].to_string()),
                    _ => return Err("attendu : remove route <cidr> [on <équipement>]".to_string()),
                };
                Ok(Change::RemoveRoute { dest, on })
            }
            (Some("cut"), Some("link"), Some(_), None) => Ok(Change::CutLink(words[2].to_string())),
            (Some("stop"), Some(_), Some("on"), Some(_)) if words.len() == 4 => {
                let (proto, port) = parse_proto_port(words[1])?;
                let (RuleProto::Tcp | RuleProto::Udp, Some(port)) = (proto, port) else {
                    return Err("attendu : stop <tcp|udp>/<port> on <équipement>".to_string());
                };
                let proto = if proto == RuleProto::Tcp {
                    L4Proto::Tcp
                } else {
                    L4Proto::Udp
                };
                Ok(Change::StopService {
                    proto,
                    port,
                    on: words[3].to_string(),
                })
            }
            _ => Err(format!(
                "scénario non compris : « {text} ». Exemples : « R1 goes down », « disconnect SW2 », « block tcp/443 », « remove route 10.10.0.0/16 », « cut link R1:eth1 », « stop tcp/80 on SERVER »"
            )),
        }
    }

    /// Traduit le changement en pannes du moteur.
    pub fn to_faults(&self, sim: &Simulation) -> Result<Vec<Fault>, String> {
        let dev = |name: &str| sim.device_id(name).map_err(|e| e.to_string());
        match self {
            Change::DeviceDown(name) => {
                let d = dev(name)?;
                let faults: Vec<Fault> = sim
                    .device(d)
                    .map(|x| {
                        x.ifaces
                            .iter()
                            .map(|i| Fault::InterfaceDown {
                                device: d,
                                iface: i.id,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(faults)
            }
            Change::Disconnect(name) => {
                let d = dev(name)?;
                Ok(sim
                    .device(d)
                    .map(|x| {
                        x.ifaces
                            .iter()
                            .filter_map(|i| i.link)
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|link| Fault::LinkDown { link })
                    .collect())
            }
            Change::Block { proto, port, on } => {
                let targets: Vec<DeviceId> = match on {
                    Some(n) => vec![dev(n)?],
                    None => sim
                        .devices()
                        .iter()
                        .filter(|d| matches!(d.kind, DeviceKind::Router | DeviceKind::Firewall))
                        .map(|d| d.id)
                        .collect(),
                };
                Ok(targets
                    .into_iter()
                    .map(|device| {
                        let mut rule = AclRule::new(AclAction::Deny, *proto);
                        rule.port_from = *port;
                        rule.port_to = *port;
                        Fault::AclInsert { device, rule }
                    })
                    .collect())
            }
            Change::RemoveRoute { dest, on } => {
                let holders: Vec<DeviceId> = match on {
                    Some(n) => vec![dev(n)?],
                    None => sim
                        .devices()
                        .iter()
                        .filter(|d| {
                            matches!(&d.behavior, Behavior::Ip(n)
                                if n.static_routes.iter().any(|r| r.dest.trunc() == dest.trunc()))
                        })
                        .map(|d| d.id)
                        .collect(),
                };
                if holders.is_empty() {
                    return Err(format!(
                        "aucun équipement n'a de route statique vers {}",
                        dest.trunc()
                    ));
                }
                Ok(holders
                    .into_iter()
                    .map(|device| Fault::RemoveRoutes {
                        device,
                        dest: *dest,
                    })
                    .collect())
            }
            Change::CutLink(endpoint) => {
                let (d, i) = endpoint
                    .split_once(':')
                    .ok_or_else(|| format!("« {endpoint} » : attendu Équipement:interface"))?;
                let did = dev(d)?;
                let iface = sim.iface_id(did, i).map_err(|e| e.to_string())?;
                let link = sim
                    .device(did)
                    .and_then(|x| x.iface(iface))
                    .and_then(|x| x.link)
                    .ok_or_else(|| format!("{endpoint} n'est reliée à aucun câble"))?;
                Ok(vec![Fault::LinkDown { link }])
            }
            Change::StopService { proto, port, on } => Ok(vec![Fault::StopService {
                device: dev(on)?,
                proto: *proto,
                port: *port,
            }]),
        }
    }
}

/// Conséquences d'un changement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impact {
    /// Le scénario, tel qu'il a été compris.
    pub scenario: String,
    /// Flux mesurés.
    pub flows: usize,
    /// Flux qui aboutissaient avant.
    pub ok_before: usize,
    /// Flux qui aboutissent après.
    pub ok_after: usize,
    /// Flux perdus.
    pub broken: Vec<String>,
    /// Machines dont au moins un flux est perdu.
    pub affected_hosts: Vec<String>,
    /// Services qui ne répondent plus à au moins une machine.
    pub affected_services: Vec<String>,
    /// Machines qui n'ont plus aucun flux.
    pub isolated_hosts: Vec<String>,
    /// Routes statiques ou par défaut devenues inutilisables.
    pub broken_routes: usize,
    /// Groupes de machines séparés avant le changement.
    pub segments_before: usize,
    /// Groupes de machines séparés après.
    pub segments_after: usize,
}

impl Impact {
    /// Nouveaux groupes isolés les uns des autres.
    pub fn isolated_segments(&self) -> usize {
        self.segments_after.saturating_sub(self.segments_before)
    }

    /// Le changement ne casse rien ?
    pub fn is_harmless(&self) -> bool {
        self.broken.is_empty() && self.broken_routes == 0 && self.isolated_segments() == 0
    }

    /// Rendu texte.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "ANALYSE D'IMPACT — {}", self.scenario);
        let _ = writeln!(
            s,
            "Flux mesurés       : {} ({} fonctionnaient → {} fonctionnent)",
            self.flows, self.ok_before, self.ok_after
        );
        let _ = writeln!(s, "Machines affectées : {}", self.affected_hosts.len());
        let _ = writeln!(s, "Services affectés  : {}", self.affected_services.len());
        let _ = writeln!(s, "Routes cassées     : {}", self.broken_routes);
        let _ = writeln!(s, "Segments isolés    : {}", self.isolated_segments());
        if self.is_harmless() {
            let _ = writeln!(
                s,
                "\nAucun impact : tous les flux mesurés continuent de fonctionner."
            );
            return s;
        }
        if !self.isolated_hosts.is_empty() {
            let _ = writeln!(
                s,
                "\nTotalement isolées : {}",
                self.isolated_hosts.join(", ")
            );
        }
        if !self.affected_services.is_empty() {
            let _ = writeln!(s, "Services : {}", self.affected_services.join(", "));
        }
        let _ = writeln!(s, "\nFlux perdus ({}) :", self.broken.len());
        for b in self.broken.iter().take(15) {
            let _ = writeln!(s, "  ✗ {b}");
        }
        if self.broken.len() > 15 {
            let _ = writeln!(s, "  … et {} autres", self.broken.len() - 15);
        }
        s
    }
}

/// Routes (statiques et passerelle) utilisables, tous équipements confondus.
fn usable_routes(sim: &Simulation) -> usize {
    sim.devices()
        .iter()
        .map(|d| match &d.behavior {
            Behavior::Ip(n) => {
                let (active, _) = build_routes(&d.ifaces, &n.static_routes, n.gateway);
                active
                    .iter()
                    .filter(|r| r.origin != netiv_core::route::RouteOrigin::Connected)
                    .count()
            }
            Behavior::Switch(_) => 0,
        })
        .sum()
}

/// Groupes de machines encore reliés par des câbles et interfaces en service.
fn segments(sim: &Simulation) -> usize {
    let n = sim.devices().len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in sim.links() {
        let up = |(d, i): (DeviceId, netiv_core::IfaceId)| {
            sim.device(d)
                .and_then(|x| x.iface(i))
                .is_some_and(|x| x.admin_up)
        };
        if l.up && up(l.a) && up(l.b) {
            let (a, b) = (l.a.0.0 as usize, l.b.0.0 as usize);
            adj[a].push(b);
            adj[b].push(a);
        }
    }
    let ends = reach::end_devices(sim);
    let mut seen = vec![false; n];
    let mut groups = 0;
    for e in &ends {
        let s = e.0 as usize;
        if seen[s] {
            continue;
        }
        groups += 1;
        let mut q = VecDeque::from([s]);
        seen[s] = true;
        while let Some(x) = q.pop_front() {
            for &y in &adj[x] {
                if !seen[y] {
                    seen[y] = true;
                    q.push_back(y);
                }
            }
        }
    }
    groups
}

/// Mesure l'impact de `text` sur `sim`, sans la modifier.
pub fn what_if(sim: &Simulation, text: &str) -> Result<Impact, String> {
    let change = Change::parse(text)?;
    let faults = change.to_faults(sim)?;
    let flows = reach::flows(sim);
    let before: Matrix = reach::measure_all(sim, &flows);

    let mut after_sim = sim.fresh_clone();
    for f in faults {
        after_sim
            .inject_fault(f, false)
            .map_err(|e| e.to_string())?;
    }
    let after = reach::measure_all(&after_sim, &flows);

    let broken_ids: Vec<&FlowId> = flows
        .iter()
        .filter(|f| before.results.get(f) == Some(&true) && after.results.get(f) == Some(&false))
        .collect();
    let name = |d: DeviceId| sim.device(d).map_or("?".to_string(), |x| x.name.clone());

    let mut hosts: BTreeSet<String> = BTreeSet::new();
    let mut services: BTreeSet<String> = BTreeSet::new();
    for f in &broken_ids {
        hosts.insert(name(f.src));
        hosts.insert(name(f.dst));
        if let Some((p, port)) = f.service {
            services.insert(format!("{p}/{port} sur {}", name(f.dst)));
        }
    }
    // Machines qui avaient des flux et n'en ont plus aucun.
    let isolated: Vec<String> = reach::end_devices(sim)
        .into_iter()
        .filter(|d| {
            let involved = |m: &Matrix| {
                m.results
                    .iter()
                    .any(|(f, ok)| *ok && (f.src == *d || f.dst == *d))
            };
            involved(&before) && !involved(&after)
        })
        .map(name)
        .collect();

    Ok(Impact {
        scenario: text.trim().to_string(),
        flows: flows.len(),
        ok_before: before.ok_count(),
        ok_after: after.ok_count(),
        broken: broken_ids.iter().map(|f| reach::label(sim, f)).collect(),
        affected_hosts: hosts.into_iter().collect(),
        affected_services: services.into_iter().collect(),
        isolated_hosts: isolated,
        broken_routes: usable_routes(sim).saturating_sub(usable_routes(&after_sim)),
        segments_before: segments(sim),
        segments_after: segments(&after_sim),
    })
}
