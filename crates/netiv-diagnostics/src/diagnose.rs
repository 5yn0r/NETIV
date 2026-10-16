//! Diagnostic : sonde, lecture de la trace, cause racine.
//!
//! La méthode suit celle d'un administrateur : on pose une question au réseau
//! (un ping, une requête TCP), puis on cherche **le premier événement qui
//! explique l'échec** dans la trace, couche par couche. Quand une copie saine
//! du réseau (`baseline`) est connue, on y compare la configuration pour dire
//! non seulement *ce qui* est faux mais aussi *ce qui était attendu*.

use std::fmt::Write as _;
use std::net::Ipv4Addr;

use netiv_core::device::Behavior;
use netiv_core::packet::{EtherPayload, IpPayload, L4Proto};
use netiv_core::route::RouteDecision;
use netiv_core::switch::PortMode;
use netiv_core::trace::DropReason;
use netiv_core::{
    DeviceId, IfaceId, Ipv4Net, NetivError, SimTime, Simulation, TraceEvent, TraceKind,
};

use crate::cause::{CauseKind, RootCause};
use crate::l2::{L2Options, l2_domain, owner_of};
use crate::probe::{Probe, ProbeKind, ProbeResult, run_probe};

/// Latence au-delà de laquelle un lien est jugé anormal.
pub const HIGH_LATENCY: SimTime = SimTime::from_millis(100);

/// La question posée : « `src` peut-il joindre `dst` (sur ce service) ? ».
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Émetteur.
    pub src: DeviceId,
    /// Destination.
    pub dst: Ipv4Addr,
    /// Service visé ; `None` = simple ping.
    pub service: Option<(L4Proto, u16)>,
}

impl Request {
    fn probe(&self) -> Probe {
        match self.service {
            None => Probe::ping(self.src, self.dst),
            Some((L4Proto::Tcp, port)) => Probe::tcp(self.src, self.dst, port),
            Some((L4Proto::Udp, port)) => Probe::udp(self.src, self.dst, port),
        }
    }
}

/// État d'un contrôle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    /// Le trafic a franchi cette étape.
    Ok,
    /// Cette étape est celle qui échoue.
    Failed,
    /// L'échec survient avant : non testé.
    NotTested,
}

/// Un contrôle de la liste « couche par couche ».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Nom.
    pub name: &'static str,
    /// Résultat.
    pub status: CheckStatus,
}

/// Verdict global.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Tout répond, rien d'anormal.
    Healthy,
    /// Ça répond, mais quelque chose cloche (latence, pertes).
    Degraded,
    /// Ça ne répond pas.
    Broken,
}

/// Résultat d'un diagnostic.
#[derive(Clone, Debug)]
pub struct Diagnosis {
    /// La question.
    pub request: Request,
    /// Verdict.
    pub status: Status,
    /// Contrôles, dans l'ordre où le trafic les traverse.
    pub checks: Vec<Check>,
    /// Cause racine, si trouvée.
    pub cause: Option<RootCause>,
    /// La sonde exécutée.
    pub probe: ProbeResult,
    /// Texte « source → destination » pour l'affichage.
    pub title: String,
}

const CHECKS: [&str; 10] = [
    "Interface locale",
    "Câbles et liens",
    "Commutation (VLAN)",
    "Adresse IP",
    "Résolution ARP",
    "Passerelle par défaut",
    "Routage (chemin aller)",
    "Routage (chemin retour)",
    "Filtrage (pare-feu / ACL)",
    "Service (port)",
];

fn check_index(c: &RootCause) -> Option<usize> {
    Some(match c.kind {
        CauseKind::InterfaceDown => 0,
        CauseKind::LinkDown | CauseKind::PacketLoss | CauseKind::HighLatency => 1,
        CauseKind::WrongVlan => 2,
        CauseKind::WrongIp
        | CauseKind::WrongMask
        | CauseKind::DhcpServerDown
        | CauseKind::DhcpRelayMissing
        | CauseKind::DhcpPoolExhausted => 3,
        CauseKind::ArpMismatch => 4,
        CauseKind::WrongGateway | CauseKind::RogueDhcp => 5,
        CauseKind::MissingRoute
        | CauseKind::WrongRoute
        | CauseKind::RoutingLoop
        | CauseKind::ForwardingDisabled => {
            if c.on_return_path {
                7
            } else {
                6
            }
        }
        CauseKind::Filtered => 8,
        CauseKind::ServiceDown
        | CauseKind::DnsWrongServer
        | CauseKind::DnsServerDown
        | CauseKind::DnsRecordWrong => 9,
        CauseKind::Unknown => return None,
    })
}

/// Diagnostique la joignabilité décrite par `req`.
///
/// `baseline` est, si on l'a, une copie du réseau tel qu'il devrait être.
pub fn diagnose(
    sim: &Simulation,
    baseline: Option<&Simulation>,
    req: &Request,
) -> Result<Diagnosis, NetivError> {
    let probe = run_probe(sim, &req.probe())?;
    let env = Env {
        sim,
        base: baseline,
        req,
        events: &probe.events,
    };

    let src_name = env.name(req.src);
    let what = match req.service {
        None => "ping".to_string(),
        Some((p, port)) => format!("{p}/{port}"),
    };
    let title = format!("{what} {src_name} → {}", req.dst);

    let (status, cause) = if probe.success {
        match env.degradation() {
            Some(c) => (Status::Degraded, Some(c)),
            None => (Status::Healthy, None),
        }
    } else {
        // Un poste sans adresse (ou configuré par un serveur pirate) : la
        // cause est côté DHCP, avant tout le reste.
        let c = crate::dhcpdns::dhcp_cause(sim, req.src)
            .or_else(|| env.classify())
            .or_else(|| env.static_scan())
            .unwrap_or_else(|| {
                RootCause::new(
                    CauseKind::Unknown,
                    &src_name,
                    "aucune cause évidente dans la trace : examiner les tables (show route, show arp, show acl)",
                )
            });
        (Status::Broken, Some(c))
    };

    let fail_at = match (&status, &cause) {
        (Status::Healthy, _) => None,
        (_, Some(c)) => check_index(c),
        _ => None,
    };
    let unknown = status == Status::Broken && fail_at.is_none();
    let checks = CHECKS
        .iter()
        .enumerate()
        .map(|(i, name)| Check {
            name,
            status: match (fail_at, unknown) {
                (_, true) => CheckStatus::NotTested,
                (None, false) => CheckStatus::Ok,
                (Some(f), _) if status == Status::Degraded => {
                    // Une dégradation n'empêche pas le trafic de passer.
                    let _ = f;
                    CheckStatus::Ok
                }
                (Some(f), _) if i < f => CheckStatus::Ok,
                (Some(f), _) if i == f => CheckStatus::Failed,
                (Some(_), _) => CheckStatus::NotTested,
            },
        })
        .collect();

    Ok(Diagnosis {
        request: *req,
        status,
        checks,
        cause,
        probe,
        title,
    })
}

impl Diagnosis {
    /// Rendu texte. `reveal_fix` ajoute la commande qui répare ; `reveal_expected`
    /// ajoute la valeur attendue.
    pub fn render(&self, reveal_expected: bool, reveal_fix: bool) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "DIAGNOSTIC — {}", self.title);
        let verdict = match self.status {
            Status::Healthy => format!(
                "tout répond ({}/{} réponses)",
                self.probe.replies, self.probe.sent
            ),
            Status::Degraded => format!(
                "répond mais dégradé ({}/{} réponses)",
                self.probe.replies, self.probe.sent
            ),
            Status::Broken => format!(
                "PANNE ({}/{} réponses)",
                self.probe.replies, self.probe.sent
            ),
        };
        let _ = writeln!(s, "Résultat : {verdict}");
        let _ = writeln!(s);
        for c in &self.checks {
            let mark = match c.status {
                CheckStatus::Ok => "✓",
                CheckStatus::Failed => "✗",
                CheckStatus::NotTested => "·",
            };
            let _ = writeln!(s, "[{mark}] {}", c.name);
        }
        if let Some(c) = &self.cause {
            let _ = writeln!(s);
            let _ = writeln!(s, "Cause racine : {} sur {}", c.kind, c.device);
            let _ = writeln!(s, "  {}", c.detail);
            if c.on_return_path {
                let _ = writeln!(
                    s,
                    "  (le problème touche le trajet de retour de la réponse)"
                );
            }
            if reveal_expected {
                if let Some(a) = &c.actual {
                    let _ = writeln!(s, "  Constaté : {a}");
                }
                if let Some(e) = &c.expected {
                    let _ = writeln!(s, "  Attendu  : {e}");
                }
            }
            if reveal_fix {
                if let Some(f) = &c.fix {
                    let _ = writeln!(s, "Pour réparer : {f}");
                }
            }
        }
        s
    }
}

// ---------------------------------------------------------------------------

struct Env<'a> {
    sim: &'a Simulation,
    base: Option<&'a Simulation>,
    req: &'a Request,
    events: &'a [TraceEvent],
}

impl Env<'_> {
    fn name(&self, d: DeviceId) -> String {
        self.sim.device(d).map_or("?".into(), |x| x.name.clone())
    }

    fn iface_name(&self, d: DeviceId, i: IfaceId) -> String {
        self.sim
            .device(d)
            .and_then(|x| x.iface(i))
            .map_or("?".into(), |x| x.name.clone())
    }

    fn label(&self, d: DeviceId, i: IfaceId) -> String {
        format!("{}/{}", self.name(d), self.iface_name(d, i))
    }

    fn node(&self, d: DeviceId) -> Option<&netiv_core::ip_node::IpNode> {
        match &self.sim.device(d)?.behavior {
            Behavior::Ip(n) => Some(n),
            Behavior::Switch(_) => None,
        }
    }

    fn base_node(&self, d: DeviceId) -> Option<&netiv_core::ip_node::IpNode> {
        match &self.base?.device(d)?.behavior {
            Behavior::Ip(n) => Some(n),
            Behavior::Switch(_) => None,
        }
    }

    fn cur_ip(&self, d: DeviceId, i: IfaceId) -> Option<Ipv4Net> {
        self.sim.device(d)?.iface(i)?.ip
    }

    fn base_ip(&self, d: DeviceId, i: IfaceId) -> Option<Ipv4Net> {
        self.base?.device(d)?.iface(i)?.ip
    }

    /// Interface de `d` dont le réseau contient `ip`, même désactivée.
    fn iface_toward(&self, d: DeviceId, ip: Ipv4Addr) -> Option<IfaceId> {
        self.sim
            .device(d)?
            .ifaces
            .iter()
            .find(|i| i.ip.is_some_and(|n| n.contains(ip)))
            .map(|i| i.id)
    }

    // ----------------------------------------------------------------- cas sain

    /// Le trafic passe : y a-t-il quand même une anomalie à signaler ?
    fn degradation(&self) -> Option<RootCause> {
        // Pertes consignées malgré le succès (retransmissions).
        if let Some(e) = self.events.iter().find(|e| {
            matches!(
                e.kind,
                TraceKind::Dropped {
                    reason: DropReason::LinkLoss
                }
            )
        }) {
            return Some(self.packet_loss(e));
        }
        // Lien lent sur le chemin.
        for e in self.events {
            if let TraceKind::FrameSent { iface, .. } = &e.kind {
                let link = self
                    .sim
                    .device(e.device)
                    .and_then(|d| d.iface(*iface))
                    .and_then(|i| i.link)
                    .and_then(|l| self.sim.link(l));
                if let Some(l) = link {
                    if l.latency > HIGH_LATENCY {
                        let mut c = RootCause::new(
                            CauseKind::HighLatency,
                            &self.name(l.a.0),
                            format!(
                                "câble {} ↔ {} : latence de {}",
                                self.label(l.a.0, l.a.1),
                                self.label(l.b.0, l.b.1),
                                l.latency
                            ),
                        );
                        c.actual = Some(l.latency.to_string());
                        let base = self.base_link_latency(l.id);
                        c.expected = base.map(|t| t.to_string());
                        let ms = base.map_or(1, |t| (t.as_micros() / 1_000).max(1));
                        c.fix = Some(format!(
                            "latency {} {} {ms}",
                            self.name(l.a.0),
                            self.iface_name(l.a.0, l.a.1)
                        ));
                        return Some(c);
                    }
                }
            }
        }
        None
    }

    fn base_link_latency(&self, id: netiv_core::LinkId) -> Option<SimTime> {
        self.base?.link(id).map(|l| l.latency)
    }

    // ------------------------------------------------------------- classification

    fn classify(&self) -> Option<RootCause> {
        for (i, e) in self.events.iter().enumerate() {
            let found = match &e.kind {
                TraceKind::Dropped { reason } => match reason {
                    DropReason::LinkDown => Some(self.link_down(e)),
                    DropReason::LinkLoss => Some(self.packet_loss(e)),
                    DropReason::InterfaceDown => Some(self.interface_down(e.device)),
                    DropReason::VlanMismatch => self.wrong_vlan_on_switch(i),
                    DropReason::Filtered => self.filtered(i),
                    DropReason::NoRoute => self.no_route(i),
                    _ => None,
                },
                TraceKind::ArpTimeout { target, .. } => self.arp_timeout(e.device, *target),
                TraceKind::TtlExpired => self.routing_loop(),
                TraceKind::FrameReceived { iface, frame } => {
                    self.forwarding_disabled(i, e.device, *iface, frame)
                }
                TraceKind::CurlResult {
                    outcome: netiv_core::trace::CurlOutcome::Refused,
                    ..
                }
                | TraceKind::UdpResult {
                    outcome: netiv_core::trace::UdpOutcome::PortUnreachable,
                    ..
                } => self.service_down(),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn link_of_drop(&self, e: &TraceEvent) -> Option<netiv_core::Link> {
        let sent = self.events.get(self.index_of(e.caused_by?)?)?;
        let TraceKind::FrameSent { iface, .. } = &sent.kind else {
            return None;
        };
        let l = self.sim.device(sent.device)?.iface(*iface)?.link?;
        self.sim.link(l).copied()
    }

    /// Les `seq` de la trace de la sonde sont leurs index.
    fn index_of(&self, seq: u64) -> Option<usize> {
        usize::try_from(seq).ok()
    }

    fn link_text(&self, l: &netiv_core::Link) -> String {
        format!(
            "câble {} ↔ {}",
            self.label(l.a.0, l.a.1),
            self.label(l.b.0, l.b.1)
        )
    }

    fn link_down(&self, e: &TraceEvent) -> RootCause {
        match self.link_of_drop(e) {
            Some(l) => {
                let mut c = RootCause::new(
                    CauseKind::LinkDown,
                    &self.name(l.a.0),
                    format!("{} est coupé", self.link_text(&l)),
                );
                c.actual = Some("câble coupé".into());
                c.expected = Some("câble en service".into());
                c.fix = Some(format!(
                    "cable {} {} restore",
                    self.name(l.a.0),
                    self.iface_name(l.a.0, l.a.1)
                ));
                c
            }
            None => RootCause::new(
                CauseKind::LinkDown,
                &self.name(e.device),
                "un câble est coupé",
            ),
        }
    }

    fn packet_loss(&self, e: &TraceEvent) -> RootCause {
        match self.link_of_drop(e) {
            Some(l) => {
                let mut c = RootCause::new(
                    CauseKind::PacketLoss,
                    &self.name(l.a.0),
                    format!(
                        "{} perd {:.0} % des trames",
                        self.link_text(&l),
                        l.loss * 100.0
                    ),
                );
                c.actual = Some(format!("{:.0} % de perte", l.loss * 100.0));
                c.expected = Some("0 % de perte".into());
                c.fix = Some(format!(
                    "loss {} {} 0",
                    self.name(l.a.0),
                    self.iface_name(l.a.0, l.a.1)
                ));
                c
            }
            None => RootCause::new(
                CauseKind::PacketLoss,
                &self.name(e.device),
                "des trames sont perdues",
            ),
        }
    }

    fn interface_down(&self, d: DeviceId) -> RootCause {
        let dev = self.sim.device(d);
        let down = dev.and_then(|x| {
            x.ifaces
                .iter()
                .find(|i| !i.admin_up && i.link.is_some())
                .or_else(|| x.ifaces.iter().find(|i| !i.admin_up))
        });
        match down {
            Some(i) => {
                let mut c = RootCause::new(
                    CauseKind::InterfaceDown,
                    &self.name(d),
                    format!("l'interface {} est désactivée", self.label(d, i.id)),
                );
                c.actual = Some("down".into());
                c.expected = Some("up".into());
                c.fix = Some(format!("iface {} {} up", self.name(d), i.name));
                c
            }
            None => RootCause::new(
                CauseKind::InterfaceDown,
                &self.name(d),
                "une interface est désactivée",
            ),
        }
    }

    fn last_lookup_before(&self, i: usize, d: DeviceId) -> Option<&RouteDecision> {
        self.events[..i].iter().rev().find_map(|e| match &e.kind {
            TraceKind::RouteLookup(dec) if e.device == d => Some(dec),
            _ => None,
        })
    }

    fn no_route(&self, i: usize) -> Option<RootCause> {
        let d = self.events[i].device;
        let dec = self.last_lookup_before(i, d)?;
        let dname = self.name(d);

        // Une interface désactivée explique l'absence de route ?
        if let Some(down) = self.sim.device(d)?.ifaces.iter().find(|ifc| {
            !ifc.admin_up
                && ifc.ip.is_some_and(|n| {
                    n.contains(dec.dst) || dec.inactive.iter().any(|r| n.contains(r.via))
                })
        }) {
            return Some(self.interface_down_named(d, down.id));
        }

        if let Some(inact) = dec.inactive.first() {
            // Prochain saut hors de tout réseau connecté.
            return Some(match inact.origin {
                netiv_core::route::RouteOrigin::Default => self.bad_gateway(d, inact.via),
                _ => self.wrong_route(
                    d,
                    inact.dest,
                    inact.via,
                    format!(
                        "la route {} via {} est inutilisable : {} n'est dans aucun réseau connecté de {dname}",
                        inact.dest.trunc(),
                        inact.via,
                        inact.via
                    ),
                ),
            });
        }

        // Aucune route du tout.
        let to_src = self.sim.device(self.req.src).is_some_and(|s| {
            s.ifaces
                .iter()
                .any(|i| i.ip.is_some_and(|n| n.addr() == dec.dst))
        });
        let mut c = RootCause::new(
            CauseKind::MissingRoute,
            &dname,
            format!("{dname} n'a aucune route vers {}", dec.dst),
        );
        c.on_return_path = to_src;
        c.actual = Some("aucune route".into());
        if let Some(bn) = self.base_node(d) {
            // Route (ou passerelle) présente dans la configuration saine.
            if let Some(r) = bn.static_routes.iter().find(|r| r.dest.contains(dec.dst)) {
                c.expected = Some(format!("{} via {}", r.dest.trunc(), r.via));
                c.fix = Some(format!(
                    "route add {dname} {} via {}",
                    r.dest.trunc(),
                    r.via
                ));
            } else if let Some(g) = bn.gateway {
                c.expected = Some(format!("passerelle par défaut {g}"));
                c.fix = Some(format!("gateway {dname} {g}"));
            }
        }
        if c.fix.is_none() {
            c.fix = Some(format!(
                "route add {dname} <réseau de destination> via <prochain saut>"
            ));
        }
        Some(c)
    }

    fn interface_down_named(&self, d: DeviceId, i: IfaceId) -> RootCause {
        let mut c = RootCause::new(
            CauseKind::InterfaceDown,
            &self.name(d),
            format!("l'interface {} est désactivée", self.label(d, i)),
        );
        c.actual = Some("down".into());
        c.expected = Some("up".into());
        c.fix = Some(format!(
            "iface {} {} up",
            self.name(d),
            self.iface_name(d, i)
        ));
        c
    }

    /// Route statique de `d` dont le prochain saut est faux. La copie saine
    /// donne la route correcte à remettre.
    fn wrong_route(&self, d: DeviceId, dest: Ipv4Net, bad_via: Ipv4Addr, why: String) -> RootCause {
        let dname = self.name(d);
        let mut c = RootCause::new(CauseKind::WrongRoute, &dname, why);
        c.actual = Some(format!("{} via {bad_via}", dest.trunc()));
        let good = self.base_node(d).and_then(|n| {
            n.static_routes
                .iter()
                .find(|r| r.dest.trunc() == dest.trunc())
                .copied()
        });
        match good {
            Some(r) => {
                c.expected = Some(format!("{} via {}", r.dest.trunc(), r.via));
                c.fix = Some(format!(
                    "route del {dname} {} ; route add {dname} {} via {}",
                    dest.trunc(),
                    r.dest.trunc(),
                    r.via
                ));
            }
            None => c.fix = Some(format!("route del {dname} {}", dest.trunc())),
        }
        c
    }

    /// La passerelle de `d` est injoignable : mauvaise passerelle, ou adresse
    /// / masque de `d` modifié ? La copie saine permet de trancher.
    fn bad_gateway(&self, d: DeviceId, via: Ipv4Addr) -> RootCause {
        let dname = self.name(d);
        let base_gw = self.base_node(d).and_then(|n| n.gateway);
        let cur_gw = self.node(d).and_then(|n| n.gateway);

        if self.base.is_some() && base_gw == cur_gw {
            // La passerelle n'a pas changé : c'est l'adressage de `d` qui est faux.
            if let Some(c) = self.config_drift_ip(d) {
                return c;
            }
        }
        let mut c = RootCause::new(
            CauseKind::WrongGateway,
            &dname,
            format!(
                "la passerelle par défaut {via} n'appartient à aucun réseau connecté de {dname}"
            ),
        );
        c.actual = Some(via.to_string());
        if let Some(g) = base_gw {
            c.expected = Some(g.to_string());
            c.fix = Some(format!("gateway {dname} {g}"));
        } else {
            c.fix = Some(format!("gateway {dname} <adresse du routeur local>"));
        }
        c
    }

    /// Une interface de `d` n'a plus l'adresse de la copie saine.
    fn config_drift_ip(&self, d: DeviceId) -> Option<RootCause> {
        let dev = self.sim.device(d)?;
        for ifc in &dev.ifaces {
            let (cur, base) = (self.cur_ip(d, ifc.id), self.base_ip(d, ifc.id));
            if cur == base || base.is_none() {
                continue;
            }
            let (cur, base) = (cur?, base?);
            let kind = if cur.addr() == base.addr() {
                CauseKind::WrongMask
            } else {
                CauseKind::WrongIp
            };
            let mut c = RootCause::new(
                kind,
                &dev.name,
                format!(
                    "{} est configurée en {cur} au lieu de {base}",
                    self.label(d, ifc.id)
                ),
            );
            c.actual = Some(cur.to_string());
            c.expected = Some(base.to_string());
            c.fix = Some(format!("ip {} {} {base}", dev.name, ifc.name));
            return Some(c);
        }
        None
    }

    fn arp_timeout(&self, d: DeviceId, target: Ipv4Addr) -> Option<RootCause> {
        let dname = self.name(d);
        let is_gateway = self.node(d).and_then(|n| n.gateway) == Some(target);
        let owner = owner_of(self.sim, target);

        let Some((o, oi)) = owner else {
            // Personne n'a cette adresse. Elle a-t-elle changé depuis la copie saine ?
            if let Some((bo, boi)) = self.base.and_then(|b| owner_of(b, target)) {
                if let Some(c) = self.config_drift_ip(bo) {
                    return Some(c);
                }
                let _ = boi;
            }
            if is_gateway {
                return Some(self.bad_gateway(d, target));
            }
            let dest = self
                .node(d)
                .and_then(|n| n.static_routes.iter().find(|r| r.via == target))
                .map(|r| r.dest);
            let why = format!("{dname} cherche {target} mais aucune machine n'a cette adresse");
            return Some(match dest {
                Some(dest) => self.wrong_route(d, dest, target, why),
                None => {
                    let mut c = RootCause::new(CauseKind::WrongRoute, &dname, why);
                    c.actual = Some(target.to_string());
                    c
                }
            });
        };

        let oname = self.name(o);
        let oiface = self.sim.device(o)?.iface(oi)?;
        if !oiface.admin_up {
            return Some(self.interface_down_named(o, oi));
        }

        let di = self.iface_toward(d, target)?;
        let d_net = self.cur_ip(d, di)?;
        let o_net = oiface.ip?;

        if d_net.trunc() != o_net.trunc() {
            // Même « adresse locale » pour d, mais pas dans le même sous-réseau
            // que son propriétaire : un masque est faux.
            let (culprit, ci) = if self
                .base_ip(o, oi)
                .is_some_and(|b| Some(b) != self.cur_ip(o, oi))
            {
                (o, oi)
            } else {
                (d, di)
            };
            let cur = self.cur_ip(culprit, ci)?;
            let mut c = RootCause::new(
                CauseKind::WrongMask,
                &self.name(culprit),
                format!(
                    "{} ({}) croit que {target} est sur son réseau local, mais {oname} est dans un autre sous-réseau ({o_net})",
                    self.label(culprit, ci),
                    cur
                ),
            );
            c.actual = Some(cur.to_string());
            if let Some(b) = self.base_ip(culprit, ci) {
                c.expected = Some(b.to_string());
                c.fix = Some(format!(
                    "ip {} {} {b}",
                    self.name(culprit),
                    self.iface_name(culprit, ci)
                ));
            } else {
                c.fix = Some(format!(
                    "ip {} {} <adresse/masque correct>",
                    self.name(culprit),
                    self.iface_name(culprit, ci)
                ));
            }
            return Some(c);
        }

        // Même sous-réseau : la liaison de couche 2 est-elle en cause ?
        let now = L2Options {
            vlan_aware: true,
            honor_state: true,
        };
        if l2_domain(self.sim, d, di, now).contains(&(o, oi)) {
            return None;
        }
        let no_vlan = L2Options {
            vlan_aware: false,
            honor_state: true,
        };
        if l2_domain(self.sim, d, di, no_vlan).contains(&(o, oi)) {
            return Some(self.wrong_vlan_between((d, di), (o, oi)));
        }
        // Un câble ou une interface sur le chemin est coupé.
        self.physical_break((d, di), (o, oi))
    }

    fn wrong_vlan_between(&self, a: (DeviceId, IfaceId), b: (DeviceId, IfaceId)) -> RootCause {
        // Compare avec la copie saine : quel port a changé de mode ?
        if let Some(base) = self.base {
            for dev in self.sim.devices() {
                let (Behavior::Switch(cur), Some(Behavior::Switch(old))) =
                    (&dev.behavior, base.device(dev.id).map(|x| &x.behavior))
                else {
                    continue;
                };
                for ifc in &dev.ifaces {
                    let (m, o) = (cur.port_mode(ifc.id), old.port_mode(ifc.id));
                    if m != o {
                        return self.vlan_cause(dev.id, ifc.id, &m, Some(&o));
                    }
                }
            }
        }
        // Sans copie saine : on rapproche les ports d'accès des deux extrémités.
        let port_of = |(d, i): (DeviceId, IfaceId)| {
            let link = self.sim.link(self.sim.device(d)?.iface(i)?.link?)?;
            let (pd, pi) = link.peer_of(d, i)?;
            Some((pd, pi))
        };
        let (pa, pb) = (port_of(a), port_of(b));
        let mut c = RootCause::new(
            CauseKind::WrongVlan,
            &self.name(a.0),
            format!(
                "{} et {} sont dans le même sous-réseau mais pas dans le même VLAN",
                self.label(a.0, a.1),
                self.label(b.0, b.1)
            ),
        );
        if let (Some(x), Some(y)) = (pa, pb) {
            if let Some(detail) = self.vlan_pair_detail(x, y) {
                c.detail = detail;
            }
        }
        c
    }

    /// Description des ports de switch auxquels sont branchées les deux extrémités.
    fn vlan_pair_detail(&self, x: (DeviceId, IfaceId), y: (DeviceId, IfaceId)) -> Option<String> {
        let mode = |(d, i): (DeviceId, IfaceId)| match &self.sim.device(d)?.behavior {
            Behavior::Switch(sw) => Some(sw.port_mode(i)),
            Behavior::Ip(_) => None,
        };
        Some(format!(
            "{} ({:?}) et {} ({:?}) ne sont pas dans le même VLAN",
            self.label(x.0, x.1),
            mode(x)?,
            self.label(y.0, y.1),
            mode(y)?
        ))
    }

    fn vlan_cause(
        &self,
        sw: DeviceId,
        port: IfaceId,
        now: &PortMode,
        was: Option<&PortMode>,
    ) -> RootCause {
        let fmt = |m: &PortMode| match m {
            PortMode::Access { vlan } => format!("accès VLAN {vlan}"),
            PortMode::Trunk { allowed, native } => format!(
                "trunk ({}) natif {native}",
                if allowed.is_empty() {
                    "tous les VLAN".to_string()
                } else {
                    allowed
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                }
            ),
        };
        let mut c = RootCause::new(
            CauseKind::WrongVlan,
            &self.name(sw),
            format!("le port {} est en {}", self.label(sw, port), fmt(now)),
        );
        c.actual = Some(fmt(now));
        if let Some(w) = was {
            c.expected = Some(fmt(w));
            c.fix = Some(match w {
                PortMode::Access { vlan } => {
                    format!(
                        "port {} {} access {vlan}",
                        self.name(sw),
                        self.iface_name(sw, port)
                    )
                }
                PortMode::Trunk { allowed, native } => format!(
                    "port {} {} trunk {} native {native}",
                    self.name(sw),
                    self.iface_name(sw, port),
                    allowed
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            });
        }
        c
    }

    fn wrong_vlan_on_switch(&self, i: usize) -> Option<RootCause> {
        let e = &self.events[i];
        let port = self.events[..i].iter().rev().find_map(|x| match &x.kind {
            TraceKind::FrameReceived { iface, .. } if x.device == e.device => Some(*iface),
            _ => None,
        })?;
        let Behavior::Switch(sw) = &self.sim.device(e.device)?.behavior else {
            return None;
        };
        let was = self.base.and_then(|b| match &b.device(e.device)?.behavior {
            Behavior::Switch(s) => Some(s.port_mode(port)),
            Behavior::Ip(_) => None,
        });
        Some(self.vlan_cause(e.device, port, &sw.port_mode(port), was.as_ref()))
    }

    /// Cherche un câble coupé ou une interface désactivée entre deux extrémités.
    fn physical_break(&self, a: (DeviceId, IfaceId), b: (DeviceId, IfaceId)) -> Option<RootCause> {
        let structure = L2Options {
            vlan_aware: false,
            honor_state: false,
        };
        if !l2_domain(self.sim, a.0, a.1, structure).contains(&b) {
            return None;
        }
        // Chemin existant en structure mais pas en état : cherche le coupable.
        for l in self.sim.links() {
            if !l.up {
                let mut c = RootCause::new(
                    CauseKind::LinkDown,
                    &self.name(l.a.0),
                    format!("{} est coupé", self.link_text(l)),
                );
                c.fix = Some(format!(
                    "cable {} {} restore",
                    self.name(l.a.0),
                    self.iface_name(l.a.0, l.a.1)
                ));
                return Some(c);
            }
        }
        for dev in self.sim.devices() {
            if let Some(ifc) = dev.ifaces.iter().find(|i| !i.admin_up && i.link.is_some()) {
                return Some(self.interface_down_named(dev.id, ifc.id));
            }
        }
        None
    }

    fn routing_loop(&self) -> Option<RootCause> {
        // Équipements qui reçoivent plusieurs fois le paquet aller.
        let mut seen: Vec<(DeviceId, usize)> = Vec::new();
        for e in self.events {
            if let TraceKind::FrameReceived { frame, .. } = &e.kind {
                if let EtherPayload::Ipv4(p) = &frame.payload {
                    if p.dst == self.req.dst && !crate::l2::is_switch(self.sim, e.device) {
                        match seen.iter_mut().find(|(d, _)| *d == e.device) {
                            Some((_, n)) => *n += 1,
                            None => seen.push((e.device, 1)),
                        }
                    }
                }
            }
        }
        let members: Vec<DeviceId> = seen
            .iter()
            .filter(|(_, n)| *n >= 2)
            .map(|(d, _)| *d)
            .collect();
        if members.len() < 2 {
            return None;
        }
        // La route fautive : l'une des routes statiques d'un membre qui renvoie
        // vers un autre membre pour la destination.
        let mut culprit = None;
        for m in &members {
            let node = self.node(*m)?;
            for r in &node.static_routes {
                let toward_member =
                    owner_of(self.sim, r.via).is_some_and(|(o, _)| members.contains(&o) && o != *m);
                let in_base = self
                    .base_node(*m)
                    .is_some_and(|b| b.static_routes.iter().any(|x| x == r));
                if toward_member && r.dest.contains(self.req.dst) && !in_base {
                    culprit = Some((*m, *r));
                }
            }
        }
        let names: Vec<String> = members.iter().map(|d| self.name(*d)).collect();
        let (dev, detail, fix, actual) = match culprit {
            Some((m, r)) => (
                self.name(m),
                format!(
                    "{} se renvoient les paquets pour {} : la route {} via {} sur {} crée une boucle",
                    names.join(" et "),
                    self.req.dst,
                    r.dest.trunc(),
                    r.via,
                    self.name(m)
                ),
                Some(format!("route del {} {}", self.name(m), r.dest.trunc())),
                Some(format!("{} via {}", r.dest.trunc(), r.via)),
            ),
            None => (
                names.first().cloned().unwrap_or_default(),
                format!(
                    "{} se renvoient les paquets pour {} (TTL épuisé)",
                    names.join(" et "),
                    self.req.dst
                ),
                None,
                None,
            ),
        };
        let mut c = RootCause::new(CauseKind::RoutingLoop, &dev, detail);
        c.fix = fix;
        c.actual = actual;
        Some(c)
    }

    fn forwarding_disabled(
        &self,
        i: usize,
        d: DeviceId,
        iface: IfaceId,
        frame: &netiv_core::packet::Frame,
    ) -> Option<RootCause> {
        let dev = self.sim.device(d)?;
        let ifc = dev.iface(iface)?;
        let EtherPayload::Ipv4(p) = &frame.payload else {
            return None;
        };
        let node = self.node(d)?;
        let transit = frame.dst == ifc.mac
            && !dev
                .ifaces
                .iter()
                .any(|x| x.ip.is_some_and(|n| n.addr() == p.dst));
        let dropped_next = matches!(
            self.events.get(i + 1).map(|e| &e.kind),
            Some(TraceKind::Dropped {
                reason: DropReason::NotForUs
            })
        );
        if transit && dropped_next && !node.forwarding {
            let mut c = RootCause::new(
                CauseKind::ForwardingDisabled,
                &dev.name,
                format!(
                    "{} reçoit un paquet pour {} mais ne le transfère pas : le routage est désactivé",
                    dev.name, p.dst
                ),
            );
            c.actual = Some("transfert désactivé".into());
            c.expected = Some("transfert activé".into());
            c.fix = Some(format!("forwarding {} on", dev.name));
            return Some(c);
        }
        None
    }

    fn service_down(&self) -> Option<RootCause> {
        let (proto, port) = self.req.service?;
        let (o, _) = owner_of(self.sim, self.req.dst)?;
        let oname = self.name(o);
        let mut c = RootCause::new(
            CauseKind::ServiceDown,
            &oname,
            format!("{oname} est joignable mais rien n'écoute sur {proto}/{port}"),
        );
        c.actual = Some("aucun service".into());
        let base_name = self
            .base_node(o)
            .and_then(|n| {
                n.services
                    .iter()
                    .find(|s| s.proto == proto && s.port == port)
            })
            .map(|s| s.name.clone());
        if let Some(n) = &base_name {
            c.expected = Some(format!("service {n} sur {proto}/{port}"));
        }
        c.fix = Some(format!(
            "service add {oname} {proto} {port} {}",
            base_name.unwrap_or_else(|| "http".to_string())
        ));
        Some(c)
    }

    fn filtered(&self, i: usize) -> Option<RootCause> {
        let e = &self.events[i];
        let d = e.device;
        let dname = self.name(d);
        let (rule, text) = self.events[..i].iter().rev().find_map(|x| match &x.kind {
            TraceKind::AclDecision { rule, text } if x.device == d => Some((*rule, text.clone())),
            _ => None,
        })?;
        let ingress = self.events[..i].iter().rev().find_map(|x| match &x.kind {
            TraceKind::FrameReceived { iface, .. } if x.device == d => Some(*iface),
            _ => None,
        });
        let mut c = RootCause::new(
            CauseKind::Filtered,
            &dname,
            format!("{dname} bloque le trafic : {text}"),
        );
        c.actual = Some(text);
        match rule {
            Some(n) => {
                c.fix = Some(format!("acl del {dname} {}", n + 1));
                c.expected = Some("aucune règle ne bloque ce flux".into());
            }
            None => {
                let base_default = self.base_node(d).map(|n| n.acl.default);
                if let Some(bd) = base_default {
                    if self.node(d).is_some_and(|n| n.acl.default != bd) {
                        c.expected = Some(format!("politique par défaut : {bd}"));
                        c.fix = Some(format!("acl default {dname} {bd}"));
                        return Some(c);
                    }
                }
                let proto = match self.req.service {
                    None => "icmp".to_string(),
                    Some((p, _)) => p.to_string(),
                };
                let port = self
                    .req
                    .service
                    .map_or(String::new(), |(_, p)| format!(" port {p}"));
                let inif = ingress
                    .map(|i| format!(" in {}", self.iface_name(d, i)))
                    .unwrap_or_default();
                c.fix = Some(format!(
                    "acl add {dname} allow {proto} dst {}/32{port}{inif}",
                    self.req.dst
                ));
                c.expected = Some("une règle autorise ce flux".into());
            }
        }
        Some(c)
    }

    // ----------------------------------------------------------- analyse statique

    /// Dernier recours : examine la configuration sans s'appuyer sur la trace.
    fn static_scan(&self) -> Option<RootCause> {
        // Entrée ARP statique incohérente avec le véritable propriétaire de l'IP.
        for dev in self.sim.devices() {
            let Behavior::Ip(n) = &dev.behavior else {
                continue;
            };
            for (ip, mac) in &n.static_arp {
                let Some((o, oi)) = owner_of(self.sim, *ip) else {
                    continue;
                };
                let real = self.sim.device(o)?.iface(oi)?.mac;
                if real != *mac {
                    let mut c = RootCause::new(
                        CauseKind::ArpMismatch,
                        &dev.name,
                        format!(
                            "{} associe {ip} à {mac}, mais {} possède {ip} avec la MAC {real}",
                            dev.name,
                            self.name(o)
                        ),
                    );
                    c.actual = Some(format!("{ip} → {mac}"));
                    c.expected = Some(format!("{ip} → {real}"));
                    c.fix = Some(format!("arp {} del {ip}", dev.name));
                    return Some(c);
                }
            }
        }
        // Routeur qui ne transfère plus.
        for dev in self.sim.devices() {
            if matches!(
                dev.kind,
                netiv_core::DeviceKind::Router | netiv_core::DeviceKind::Firewall
            ) {
                if let Behavior::Ip(n) = &dev.behavior {
                    if !n.forwarding {
                        let mut c = RootCause::new(
                            CauseKind::ForwardingDisabled,
                            &dev.name,
                            format!("{} ne transfère pas les paquets", dev.name),
                        );
                        c.fix = Some(format!("forwarding {} on", dev.name));
                        return Some(c);
                    }
                }
            }
        }
        None
    }
}

/// Pour les appelants qui veulent une requête depuis un type de sonde.
impl From<&Probe> for Request {
    fn from(p: &Probe) -> Self {
        Request {
            src: p.src,
            dst: p.dst,
            service: match p.kind {
                ProbeKind::Ping { .. } => None,
                ProbeKind::Tcp { port } => Some((L4Proto::Tcp, port)),
                ProbeKind::Udp { port } => Some((L4Proto::Udp, port)),
            },
        }
    }
}

#[allow(dead_code)]
fn _assert_payload_is_exhaustive(p: &IpPayload) {
    match p {
        IpPayload::Icmp(_) | IpPayload::Udp(_) | IpPayload::Tcp(_) => {}
    }
}
