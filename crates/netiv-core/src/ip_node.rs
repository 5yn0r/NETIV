//! Pile IP commune aux hôtes, serveurs et routeurs.
//!
//! Un routeur est un hôte avec `forwarding = true` : il partage ARP, ICMP et
//! la table de routage, et ajoute simplement le transfert de paquets.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::net::Ipv4Addr;

use crate::acl::{Acl, Verdict};
use crate::addr::MacAddr;
use crate::device::{Command, Ctx, Effects};
use crate::ids::{IfaceId, TimerId, TraceId};
use crate::packet::{
    ArpOp, ArpPacket, DEFAULT_TTL, EtherPayload, Frame, IcmpMessage, IpPayload, Ipv4Packet,
    L4Proto, TcpFlags, TcpSegment, UdpDatagram, UnreachCode,
};
use crate::route::{self, Route, StaticRoute};
use crate::service::Service;
use crate::time::SimTime;
use crate::trace::{CurlOutcome, DropReason, IcmpErrorKind, TraceKind, UdpOutcome};

/// Délai entre deux tentatives ARP.
pub const ARP_RETRY_INTERVAL: SimTime = SimTime::from_millis(1_000);
/// Nombre total de tentatives ARP avant abandon.
pub const ARP_MAX_ATTEMPTS: u8 = 3;
/// Délai entre deux requêtes d'un même ping.
pub const PING_INTERVAL: SimTime = SimTime::from_millis(1_000);
/// Délai d'attente d'une réponse de ping.
pub const PING_TIMEOUT: SimTime = SimTime::from_millis(2_000);
/// Délai entre deux retransmissions de SYN.
pub const SYN_RETRY_INTERVAL: SimTime = SimTime::from_millis(1_000);
/// Nombre total d'envois de SYN avant abandon.
pub const SYN_MAX_ATTEMPTS: u8 = 3;
/// Inactivité tolérée sur une connexion TCP établie.
pub const TCP_IDLE_TIMEOUT: SimTime = SimTime::from_millis(5_000);
/// Délai d'attente d'une réponse UDP.
pub const UDP_TIMEOUT: SimTime = SimTime::from_millis(2_000);
/// Premier port éphémère.
pub const EPHEMERAL_PORT_START: u16 = 49_152;
/// Requête envoyée par `curl`.
pub const CURL_REQUEST: &str = "GET / HTTP/1.1";

#[derive(Clone, Debug)]
struct Pending {
    iface: IfaceId,
    next_hop: Ipv4Addr,
    packet: Ipv4Packet,
    trace_id: TraceId,
}

#[derive(Clone, Debug)]
struct ArpWait {
    attempts: u8,
    iface: IfaceId,
    trace_id: TraceId,
}

#[derive(Clone, Debug)]
struct Outstanding {
    sent_at: SimTime,
    trace_id: TraceId,
}

#[derive(Clone, Debug)]
struct PingSession {
    dst: Ipv4Addr,
    total: u16,
    sent: u16,
    outstanding: BTreeMap<u16, Outstanding>,
}

/// `(IP locale, port local, IP distante, port distant)`.
type TcpKey = (Ipv4Addr, u16, Ipv4Addr, u16);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TcpState {
    SynSent,
    SynRcvd,
    Established,
    /// Le serveur a répondu et fermé son côté.
    FinSent,
}

#[derive(Clone, Debug)]
struct TcpConn {
    state: TcpState,
    client: bool,
    trace_id: TraceId,
    snd_nxt: u32,
    rcv_nxt: u32,
    syn_attempts: u8,
    service: Option<Service>,
}

#[derive(Clone, Debug)]
struct UdpPending {
    dst: Ipv4Addr,
    port: u16,
    trace_id: TraceId,
}

#[derive(Copy, Clone, Debug)]
enum TimerKind {
    PingNext { id: u16 },
    PingTimeout { id: u16, seq: u16 },
    ArpRetry { target: Ipv4Addr },
    TcpSyn { key: TcpKey },
    TcpIdle { key: TcpKey },
    UdpTimeout { port: u16 },
}

/// État d'un hôte ou d'un routeur.
#[derive(Clone, Debug)]
pub struct IpNode {
    /// Transfère les paquets qui ne lui sont pas destinés (routeur).
    pub forwarding: bool,
    /// Passerelle par défaut.
    pub gateway: Option<Ipv4Addr>,
    /// Routes statiques.
    pub static_routes: Vec<StaticRoute>,
    /// Cache ARP : IP → MAC. Pas de vieillissement au MVP.
    pub arp_cache: BTreeMap<Ipv4Addr, MacAddr>,
    /// Entrées ARP statiques (prioritaires sur le cache dynamique).
    pub static_arp: BTreeMap<Ipv4Addr, MacAddr>,
    /// Services à l'écoute.
    pub services: Vec<Service>,
    /// Règles de filtrage (appliquées aux paquets entrants).
    pub acl: Acl,
    arp_waits: BTreeMap<Ipv4Addr, ArpWait>,
    pending: Vec<Pending>,
    pings: BTreeMap<u16, PingSession>,
    tcp: BTreeMap<TcpKey, TcpConn>,
    udp_pending: BTreeMap<u16, UdpPending>,
    timers: BTreeMap<TimerId, TimerKind>,
    next_timer: u64,
    next_ping_id: u16,
    next_port: u16,
    next_isn: u32,
}

impl IpNode {
    /// Crée un nœud IP ; `forwarding = true` pour un routeur.
    pub fn new(forwarding: bool) -> Self {
        IpNode {
            forwarding,
            gateway: None,
            static_routes: Vec::new(),
            arp_cache: BTreeMap::new(),
            static_arp: BTreeMap::new(),
            services: Vec::new(),
            acl: Acl::default(),
            arp_waits: BTreeMap::new(),
            pending: Vec::new(),
            pings: BTreeMap::new(),
            tcp: BTreeMap::new(),
            udp_pending: BTreeMap::new(),
            timers: BTreeMap::new(),
            next_timer: 0,
            next_ping_id: 1,
            next_port: EPHEMERAL_PORT_START,
            next_isn: 1_000,
        }
    }

    /// Pare-feu : un routeur qui refuse tout par défaut et suit les connexions.
    pub fn firewall() -> Self {
        IpNode {
            acl: Acl::firewall(),
            ..IpNode::new(true)
        }
    }

    /// Oublie tout l'état dynamique (caches, connexions, timers, suivi de
    /// connexion) mais garde la configuration.
    pub fn reset_runtime(&mut self) {
        self.arp_cache.clear();
        self.arp_waits.clear();
        self.pending.clear();
        self.pings.clear();
        self.tcp.clear();
        self.udp_pending.clear();
        self.timers.clear();
        self.acl.reset_tracking();
    }

    /// Routes actives de l'équipement (connectées, statiques, par défaut).
    pub fn active_routes(
        &self,
        ifaces: &[crate::device::Interface],
    ) -> (Vec<Route>, Vec<route::InactiveRoute>) {
        route::build_routes(ifaces, &self.static_routes, self.gateway)
    }

    fn set_timer(&mut self, out: &mut Effects, after: SimTime, kind: TimerKind) {
        let id = TimerId(self.next_timer);
        self.next_timer += 1;
        self.timers.insert(id, kind);
        out.set_timer(after, id);
    }

    // ------------------------------------------------------------------
    // Commandes et timers
    // ------------------------------------------------------------------

    /// Exécute une commande utilisateur.
    pub fn on_command(&mut self, ctx: &Ctx, cmd: Command, out: &mut Effects) {
        match cmd {
            Command::Ping { dst, count } => {
                if count == 0 {
                    return;
                }
                let id = self.next_ping_id;
                self.next_ping_id = id.wrapping_add(1);
                self.pings.insert(
                    id,
                    PingSession {
                        dst,
                        total: count,
                        sent: 0,
                        outstanding: BTreeMap::new(),
                    },
                );
                self.send_echo(ctx, out, id);
            }
            Command::Curl { dst, port } => self.start_curl(ctx, out, dst, port),
            Command::SendUdp { dst, port, data } => self.start_udp(ctx, out, dst, port, data),
        }
    }

    /// Un timer expire. Les timers inconnus (déjà obsolètes) sont ignorés.
    pub fn on_timer(&mut self, ctx: &Ctx, timer: TimerId, out: &mut Effects) {
        let Some(kind) = self.timers.remove(&timer) else {
            return;
        };
        match kind {
            TimerKind::PingNext { id } => self.send_echo(ctx, out, id),
            TimerKind::PingTimeout { id, seq } => {
                if let Some(s) = self.pings.get_mut(&id) {
                    if let Some(o) = s.outstanding.remove(&seq) {
                        out.trace(Some(o.trace_id), TraceKind::PingTimeout { dst: s.dst, seq });
                    }
                }
                self.finish_if_done(id);
            }
            TimerKind::ArpRetry { target } => self.on_arp_retry(ctx, out, target),
            TimerKind::TcpSyn { key } => self.on_syn_retry(ctx, out, key),
            TimerKind::TcpIdle { key } => self.on_tcp_idle(out, key),
            TimerKind::UdpTimeout { port } => {
                if let Some(p) = self.udp_pending.remove(&port) {
                    out.trace(
                        Some(p.trace_id),
                        TraceKind::UdpResult {
                            dst: p.dst,
                            port: p.port,
                            outcome: UdpOutcome::Timeout,
                        },
                    );
                }
            }
        }
    }

    fn send_echo(&mut self, ctx: &Ctx, out: &mut Effects, id: u16) {
        let Some(s) = self.pings.get_mut(&id) else {
            return;
        };
        s.sent += 1;
        let seq = s.sent;
        let dst = s.dst;
        let more = s.sent < s.total;
        let trace_id = out.new_trace_id();
        s.outstanding.insert(
            seq,
            Outstanding {
                sent_at: ctx.now,
                trace_id,
            },
        );
        out.trace(Some(trace_id), TraceKind::PingSent { dst, seq });
        self.set_timer(out, PING_TIMEOUT, TimerKind::PingTimeout { id, seq });
        if more {
            self.set_timer(out, PING_INTERVAL, TimerKind::PingNext { id });
        }

        if ctx.is_local_ip(dst) {
            // Ping de soi-même : bouclage local, sans passer par le réseau.
            if let Some(s) = self.pings.get_mut(&id) {
                s.outstanding.remove(&seq);
            }
            out.trace(
                Some(trace_id),
                TraceKind::PingReply {
                    from: dst,
                    seq,
                    ttl: DEFAULT_TTL,
                    rtt: SimTime::ZERO,
                },
            );
            self.finish_if_done(id);
        } else {
            let payload = IpPayload::Icmp(IcmpMessage::EchoRequest { id, seq });
            self.originate(ctx, out, None, dst, payload, trace_id);
        }
    }

    fn finish_if_done(&mut self, id: u16) {
        let done = self
            .pings
            .get(&id)
            .is_some_and(|s| s.sent >= s.total && s.outstanding.is_empty());
        if done {
            self.pings.remove(&id);
        }
    }

    // ------------------------------------------------------------------
    // Émission
    // ------------------------------------------------------------------

    /// Choisit la route vers `dst` (et consigne la décision). Retourne la
    /// route et l'adresse source à utiliser.
    fn plan_route(
        &self,
        ctx: &Ctx,
        out: &mut Effects,
        dst: Ipv4Addr,
        trace_id: TraceId,
    ) -> Option<(Route, Ipv4Addr)> {
        let decision = route::lookup(ctx.ifaces, &self.static_routes, self.gateway, dst);
        let chosen = decision.chosen_route().copied();
        out.trace(Some(trace_id), TraceKind::RouteLookup(decision));
        let Some(route) = chosen else {
            out.trace(
                Some(trace_id),
                TraceKind::Dropped {
                    reason: DropReason::NoRoute,
                },
            );
            return None;
        };
        let src = ctx.iface(route.iface).and_then(|i| i.ip)?.addr();
        Some((route, src))
    }

    /// Crée un paquet et l'envoie. `src = None` : adresse de l'interface de
    /// sortie.
    fn originate(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        src: Option<Ipv4Addr>,
        dst: Ipv4Addr,
        payload: IpPayload,
        trace_id: TraceId,
    ) {
        let Some((route, route_src)) = self.plan_route(ctx, out, dst, trace_id) else {
            return;
        };
        let packet = Ipv4Packet {
            src: src.unwrap_or(route_src),
            dst,
            ttl: DEFAULT_TTL,
            payload,
        };
        self.emit(ctx, out, route, packet, trace_id);
    }

    /// Envoie un paquet par `route` : directement si la MAC du prochain saut
    /// est connue, sinon après résolution ARP.
    fn emit(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        route: Route,
        packet: Ipv4Packet,
        trace_id: TraceId,
    ) {
        let next_hop = route.next_hop.unwrap_or(packet.dst);
        let Some(iface) = ctx.iface(route.iface) else {
            return;
        };
        if let Some(&mac) = self
            .static_arp
            .get(&next_hop)
            .or_else(|| self.arp_cache.get(&next_hop))
        {
            out.send(
                route.iface,
                Frame {
                    src: iface.mac,
                    dst: mac,
                    vlan: None,
                    payload: EtherPayload::Ipv4(packet),
                    trace_id,
                },
            );
            return;
        }
        self.pending.push(Pending {
            iface: route.iface,
            next_hop,
            packet,
            trace_id,
        });
        if let Entry::Vacant(slot) = self.arp_waits.entry(next_hop) {
            slot.insert(ArpWait {
                attempts: 1,
                iface: route.iface,
                trace_id,
            });
            self.send_arp_request(ctx, out, route.iface, next_hop, 1, trace_id);
            self.set_timer(
                out,
                ARP_RETRY_INTERVAL,
                TimerKind::ArpRetry { target: next_hop },
            );
        }
    }

    fn send_arp_request(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        iface_id: IfaceId,
        target: Ipv4Addr,
        attempt: u8,
        trace_id: TraceId,
    ) {
        let Some(iface) = ctx.iface(iface_id) else {
            return;
        };
        let Some(net) = iface.ip else {
            return;
        };
        out.trace(
            Some(trace_id),
            TraceKind::ArpRequestSent { target, attempt },
        );
        out.send(
            iface_id,
            Frame {
                src: iface.mac,
                dst: MacAddr::BROADCAST,
                vlan: None,
                payload: EtherPayload::Arp(ArpPacket {
                    op: ArpOp::Request,
                    sender_mac: iface.mac,
                    sender_ip: net.addr(),
                    target_mac: MacAddr([0; 6]),
                    target_ip: target,
                }),
                trace_id,
            },
        );
    }

    fn on_arp_retry(&mut self, ctx: &Ctx, out: &mut Effects, target: Ipv4Addr) {
        let Some(wait) = self.arp_waits.get_mut(&target) else {
            return; // résolu entre-temps
        };
        if wait.attempts < ARP_MAX_ATTEMPTS {
            wait.attempts += 1;
            let (attempt, iface, trace_id) = (wait.attempts, wait.iface, wait.trace_id);
            self.send_arp_request(ctx, out, iface, target, attempt, trace_id);
            self.set_timer(out, ARP_RETRY_INTERVAL, TimerKind::ArpRetry { target });
            return;
        }
        let Some(wait) = self.arp_waits.remove(&target) else {
            return;
        };
        let (dropped, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.next_hop == target);
        self.pending = kept;
        out.trace(
            Some(wait.trace_id),
            TraceKind::ArpTimeout {
                target,
                dropped: dropped.len(),
            },
        );
        for p in dropped {
            out.trace(
                Some(p.trace_id),
                TraceKind::Dropped {
                    reason: DropReason::ArpUnresolved,
                },
            );
        }
    }

    // ------------------------------------------------------------------
    // Réception
    // ------------------------------------------------------------------

    /// Traite une trame reçue sur `iface_id`.
    pub fn on_frame(&mut self, ctx: &Ctx, iface_id: IfaceId, frame: Frame, out: &mut Effects) {
        let Some(iface) = ctx.iface(iface_id) else {
            return;
        };
        let tid = frame.trace_id;
        if frame.dst != iface.mac && !frame.dst.is_broadcast() {
            out.trace(
                Some(tid),
                TraceKind::Dropped {
                    reason: DropReason::NotForUs,
                },
            );
            return;
        }
        match frame.payload {
            EtherPayload::Arp(arp) => self.on_arp(ctx, out, iface_id, arp, tid),
            EtherPayload::Ipv4(packet) => self.on_ipv4(ctx, out, iface_id, packet, tid),
        }
    }

    fn on_arp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        iface_id: IfaceId,
        arp: ArpPacket,
        tid: TraceId,
    ) {
        let Some(iface) = ctx.iface(iface_id) else {
            return;
        };
        match arp.op {
            ArpOp::Request => {
                if iface.ip.map(|n| n.addr()) != Some(arp.target_ip) {
                    out.trace(
                        Some(tid),
                        TraceKind::Dropped {
                            reason: DropReason::NotForUs,
                        },
                    );
                    return;
                }
                self.learn_and_flush(ctx, out, arp.sender_ip, arp.sender_mac, tid);
                out.trace(Some(tid), TraceKind::ArpReplySent { to: arp.sender_ip });
                out.send(
                    iface_id,
                    Frame {
                        src: iface.mac,
                        dst: arp.sender_mac,
                        vlan: None,
                        payload: EtherPayload::Arp(ArpPacket {
                            op: ArpOp::Reply,
                            sender_mac: iface.mac,
                            sender_ip: arp.target_ip,
                            target_mac: arp.sender_mac,
                            target_ip: arp.sender_ip,
                        }),
                        trace_id: tid,
                    },
                );
            }
            ArpOp::Reply => {
                self.learn_and_flush(ctx, out, arp.sender_ip, arp.sender_mac, tid);
            }
        }
    }

    /// Mémorise `ip → mac`, termine une éventuelle résolution en cours et
    /// envoie les paquets qui attendaient cette adresse.
    fn learn_and_flush(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ip: Ipv4Addr,
        mac: MacAddr,
        tid: TraceId,
    ) {
        if self.arp_cache.insert(ip, mac) != Some(mac) {
            out.trace(Some(tid), TraceKind::ArpCacheUpdated { ip, mac });
        }
        self.arp_waits.remove(&ip);
        let (ready, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.next_hop == ip);
        self.pending = kept;
        for p in ready {
            let Some(iface) = ctx.iface(p.iface) else {
                continue;
            };
            out.send(
                p.iface,
                Frame {
                    src: iface.mac,
                    dst: mac,
                    vlan: None,
                    payload: EtherPayload::Ipv4(p.packet),
                    trace_id: p.trace_id,
                },
            );
        }
    }

    fn on_ipv4(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        packet: Ipv4Packet,
        tid: TraceId,
    ) {
        // Filtrage : avant tout traitement, que le paquet nous soit destiné
        // ou qu'on doive le transférer.
        if let Verdict::Deny { rule } = self.acl.check(ingress, &packet) {
            let text = match rule.and_then(|i| self.acl.rules.get(i).map(|r| (i, r))) {
                Some((i, r)) => format!(
                    "règle n°{} : {}",
                    i + 1,
                    r.describe(|id| ctx.iface(id).map_or("?".to_string(), |x| x.name.clone()))
                ),
                None => format!("politique par défaut : {}", self.acl.default),
            };
            out.trace(Some(tid), TraceKind::AclDecision { rule, text });
            out.trace(
                Some(tid),
                TraceKind::Dropped {
                    reason: DropReason::Filtered,
                },
            );
            return;
        }
        if ctx.is_local_ip(packet.dst) {
            self.deliver_local(ctx, out, ingress, packet, tid);
        } else if self.forwarding {
            self.forward(ctx, out, ingress, packet, tid);
        } else {
            out.trace(
                Some(tid),
                TraceKind::Dropped {
                    reason: DropReason::NotForUs,
                },
            );
        }
    }

    fn deliver_local(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        packet: Ipv4Packet,
        tid: TraceId,
    ) {
        match &packet.payload {
            IpPayload::Icmp(msg) => self.deliver_icmp(ctx, out, &packet, msg, tid),
            IpPayload::Udp(u) => self.deliver_udp(ctx, out, ingress, &packet, u, tid),
            IpPayload::Tcp(t) => self.deliver_tcp(ctx, out, &packet, t, tid),
        }
    }

    fn deliver_icmp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        packet: &Ipv4Packet,
        msg: &IcmpMessage,
        tid: TraceId,
    ) {
        match msg {
            IcmpMessage::EchoRequest { id, seq } => {
                let reply = IpPayload::Icmp(IcmpMessage::EchoReply { id: *id, seq: *seq });
                self.originate(ctx, out, Some(packet.dst), packet.src, reply, tid);
            }
            IcmpMessage::EchoReply { id, seq } => {
                if let Some(s) = self.pings.get_mut(id) {
                    if let Some(o) = s.outstanding.remove(seq) {
                        out.trace(
                            Some(tid),
                            TraceKind::PingReply {
                                from: packet.src,
                                seq: *seq,
                                ttl: packet.ttl,
                                rtt: ctx.now.saturating_since(o.sent_at),
                            },
                        );
                    }
                }
                self.finish_if_done(*id);
            }
            IcmpMessage::TimeExceeded { original } => {
                self.on_icmp_error(out, packet.src, original, IcmpErrorKind::TtlExpired, tid);
            }
            IcmpMessage::DestUnreachable { code, original } => {
                let kind = match code {
                    UnreachCode::Net => IcmpErrorKind::NetUnreachable,
                    UnreachCode::Port => IcmpErrorKind::PortUnreachable,
                };
                self.on_icmp_error(out, packet.src, original, kind, tid);
            }
        }
    }

    /// Une erreur ICMP revient : elle concerne un ping, une connexion TCP ou
    /// un échange UDP que nous avons lancé.
    fn on_icmp_error(
        &mut self,
        out: &mut Effects,
        from: Ipv4Addr,
        original: &Ipv4Packet,
        error: IcmpErrorKind,
        tid: TraceId,
    ) {
        match &original.payload {
            IpPayload::Icmp(IcmpMessage::EchoRequest { id, seq }) => {
                if let Some(s) = self.pings.get_mut(id) {
                    if s.outstanding.remove(seq).is_some() {
                        out.trace(
                            Some(tid),
                            TraceKind::PingError {
                                from,
                                seq: *seq,
                                error,
                            },
                        );
                    }
                }
                self.finish_if_done(*id);
            }
            IpPayload::Tcp(seg) => {
                let key = (original.src, seg.src_port, original.dst, seg.dst_port);
                if self.tcp.get(&key).is_some_and(|c| c.client) {
                    if let Some(c) = self.tcp.remove(&key) {
                        out.trace(
                            Some(c.trace_id),
                            TraceKind::CurlResult {
                                dst: original.dst,
                                port: seg.dst_port,
                                outcome: CurlOutcome::Error(error),
                            },
                        );
                    }
                }
            }
            IpPayload::Udp(u) => {
                if let Some(p) = self.udp_pending.remove(&u.src_port) {
                    let outcome = if error == IcmpErrorKind::PortUnreachable {
                        UdpOutcome::PortUnreachable
                    } else {
                        UdpOutcome::Error(error)
                    };
                    out.trace(
                        Some(p.trace_id),
                        TraceKind::UdpResult {
                            dst: p.dst,
                            port: p.port,
                            outcome,
                        },
                    );
                }
            }
            IpPayload::Icmp(_) => {}
        }
    }

    // ------------------------------------------------------------------
    // UDP
    // ------------------------------------------------------------------

    fn alloc_port(&mut self) -> u16 {
        let p = self.next_port;
        self.next_port = if p == u16::MAX {
            EPHEMERAL_PORT_START
        } else {
            p + 1
        };
        p
    }

    fn start_udp(&mut self, ctx: &Ctx, out: &mut Effects, dst: Ipv4Addr, port: u16, data: String) {
        let tid = out.new_trace_id();
        out.trace(Some(tid), TraceKind::UdpSent { dst, port });

        if ctx.is_local_ip(dst) {
            let outcome = match self
                .services
                .iter()
                .find(|s| s.proto == L4Proto::Udp && s.port == port)
            {
                Some(svc) => UdpOutcome::Reply {
                    data: svc.respond(&data),
                },
                None => UdpOutcome::PortUnreachable,
            };
            out.trace(Some(tid), TraceKind::UdpResult { dst, port, outcome });
            return;
        }
        let Some((route, src)) = self.plan_route(ctx, out, dst, tid) else {
            out.trace(
                Some(tid),
                TraceKind::UdpResult {
                    dst,
                    port,
                    outcome: UdpOutcome::Error(IcmpErrorKind::NetUnreachable),
                },
            );
            return;
        };
        let sport = self.alloc_port();
        self.udp_pending.insert(
            sport,
            UdpPending {
                dst,
                port,
                trace_id: tid,
            },
        );
        self.set_timer(out, UDP_TIMEOUT, TimerKind::UdpTimeout { port: sport });
        let packet = Ipv4Packet {
            src,
            dst,
            ttl: DEFAULT_TTL,
            payload: IpPayload::Udp(UdpDatagram {
                src_port: sport,
                dst_port: port,
                data,
            }),
        };
        self.emit(ctx, out, route, packet, tid);
    }

    fn deliver_udp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        packet: &Ipv4Packet,
        u: &UdpDatagram,
        tid: TraceId,
    ) {
        // Réponse à une requête que nous avons envoyée ?
        if let Some(p) = self.udp_pending.get(&u.dst_port) {
            if p.dst == packet.src && p.port == u.src_port {
                let p = self.udp_pending.remove(&u.dst_port);
                if let Some(p) = p {
                    out.trace(
                        Some(tid),
                        TraceKind::UdpResult {
                            dst: p.dst,
                            port: p.port,
                            outcome: UdpOutcome::Reply {
                                data: u.data.clone(),
                            },
                        },
                    );
                }
                return;
            }
        }
        let svc = self
            .services
            .iter()
            .find(|s| s.proto == L4Proto::Udp && s.port == u.dst_port)
            .cloned();
        match svc {
            Some(svc) => {
                out.trace(
                    Some(tid),
                    TraceKind::ServiceRequest {
                        from: packet.src,
                        port: u.dst_port,
                        service: svc.name.clone(),
                    },
                );
                let reply = IpPayload::Udp(UdpDatagram {
                    src_port: u.dst_port,
                    dst_port: u.src_port,
                    data: svc.respond(&u.data),
                });
                self.originate(ctx, out, Some(packet.dst), packet.src, reply, tid);
            }
            None => self.send_icmp_error(
                ctx,
                out,
                ingress,
                packet,
                IcmpErrorKind::PortUnreachable,
                tid,
            ),
        }
    }

    // ------------------------------------------------------------------
    // TCP
    // ------------------------------------------------------------------

    fn alloc_isn(&mut self) -> u32 {
        let isn = self.next_isn;
        self.next_isn = self.next_isn.wrapping_add(1_000);
        isn
    }

    #[allow(clippy::too_many_arguments)]
    fn send_tcp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        key: TcpKey,
        seq: u32,
        ack: u32,
        flags: TcpFlags,
        data: String,
        tid: TraceId,
    ) {
        let seg = TcpSegment {
            src_port: key.1,
            dst_port: key.3,
            seq,
            ack,
            flags,
            data,
        };
        self.originate(ctx, out, Some(key.0), key.2, IpPayload::Tcp(seg), tid);
    }

    fn start_curl(&mut self, ctx: &Ctx, out: &mut Effects, dst: Ipv4Addr, port: u16) {
        let tid = out.new_trace_id();
        out.trace(Some(tid), TraceKind::CurlStarted { dst, port });

        if ctx.is_local_ip(dst) {
            let outcome = match self
                .services
                .iter()
                .find(|s| s.proto == L4Proto::Tcp && s.port == port)
            {
                Some(svc) => CurlOutcome::Ok {
                    response: svc.respond(CURL_REQUEST),
                },
                None => CurlOutcome::Refused,
            };
            out.trace(Some(tid), TraceKind::CurlResult { dst, port, outcome });
            return;
        }
        let Some((route, src)) = self.plan_route(ctx, out, dst, tid) else {
            out.trace(
                Some(tid),
                TraceKind::CurlResult {
                    dst,
                    port,
                    outcome: CurlOutcome::Error(IcmpErrorKind::NetUnreachable),
                },
            );
            return;
        };
        let key = (src, self.alloc_port(), dst, port);
        let isn = self.alloc_isn();
        self.tcp.insert(
            key,
            TcpConn {
                state: TcpState::SynSent,
                client: true,
                trace_id: tid,
                snd_nxt: isn.wrapping_add(1),
                rcv_nxt: 0,
                syn_attempts: 1,
                service: None,
            },
        );
        let packet = Ipv4Packet {
            src,
            dst,
            ttl: DEFAULT_TTL,
            payload: IpPayload::Tcp(TcpSegment {
                src_port: key.1,
                dst_port: port,
                seq: isn,
                ack: 0,
                flags: TcpFlags::SYN,
                data: String::new(),
            }),
        };
        self.emit(ctx, out, route, packet, tid);
        self.set_timer(out, SYN_RETRY_INTERVAL, TimerKind::TcpSyn { key });
    }

    fn on_syn_retry(&mut self, ctx: &Ctx, out: &mut Effects, key: TcpKey) {
        let Some(conn) = self.tcp.get_mut(&key) else {
            return;
        };
        if conn.state != TcpState::SynSent {
            return;
        }
        let tid = conn.trace_id;
        if conn.syn_attempts < SYN_MAX_ATTEMPTS {
            conn.syn_attempts += 1;
            let seq = conn.snd_nxt.wrapping_sub(1);
            self.send_tcp(ctx, out, key, seq, 0, TcpFlags::SYN, String::new(), tid);
            self.set_timer(out, SYN_RETRY_INTERVAL, TimerKind::TcpSyn { key });
        } else {
            self.tcp.remove(&key);
            out.trace(
                Some(tid),
                TraceKind::CurlResult {
                    dst: key.2,
                    port: key.3,
                    outcome: CurlOutcome::Timeout,
                },
            );
        }
    }

    fn on_tcp_idle(&mut self, out: &mut Effects, key: TcpKey) {
        let Some(conn) = self.tcp.get(&key) else {
            return;
        };
        if conn.state == TcpState::SynSent {
            return;
        }
        let (client, tid) = (conn.client, conn.trace_id);
        self.tcp.remove(&key);
        if client {
            out.trace(
                Some(tid),
                TraceKind::CurlResult {
                    dst: key.2,
                    port: key.3,
                    outcome: CurlOutcome::Timeout,
                },
            );
        }
    }

    fn deliver_tcp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        packet: &Ipv4Packet,
        seg: &TcpSegment,
        tid: TraceId,
    ) {
        let key: TcpKey = (packet.dst, seg.dst_port, packet.src, seg.src_port);
        let seg_len = u32::try_from(seg.data.len()).unwrap_or(u32::MAX);

        let Some(mut conn) = self.tcp.remove(&key) else {
            self.tcp_no_connection(ctx, out, key, seg, seg_len, tid);
            return;
        };
        let ctid = conn.trace_id;
        let mut keep = true;

        match (conn.client, conn.state) {
            (true, TcpState::SynSent) => {
                if seg.flags.rst {
                    keep = false;
                    out.trace(
                        Some(ctid),
                        TraceKind::CurlResult {
                            dst: key.2,
                            port: key.3,
                            outcome: CurlOutcome::Refused,
                        },
                    );
                } else if seg.flags.syn && seg.flags.ack && seg.ack == conn.snd_nxt {
                    conn.rcv_nxt = seg.seq.wrapping_add(1);
                    conn.state = TcpState::Established;
                    out.trace(
                        Some(ctid),
                        TraceKind::TcpEstablished {
                            peer: key.2,
                            peer_port: key.3,
                            local_port: key.1,
                        },
                    );
                    let req = CURL_REQUEST.to_string();
                    let req_len = u32::try_from(req.len()).unwrap_or(0);
                    let (seq, ack) = (conn.snd_nxt, conn.rcv_nxt);
                    conn.snd_nxt = conn.snd_nxt.wrapping_add(req_len);
                    self.send_tcp(ctx, out, key, seq, ack, TcpFlags::ACK, req, ctid);
                    self.set_timer(out, TCP_IDLE_TIMEOUT, TimerKind::TcpIdle { key });
                }
            }
            (true, TcpState::Established) => {
                if seg.flags.rst {
                    keep = false;
                    out.trace(
                        Some(ctid),
                        TraceKind::CurlResult {
                            dst: key.2,
                            port: key.3,
                            outcome: CurlOutcome::Refused,
                        },
                    );
                } else if !seg.data.is_empty() || seg.flags.fin {
                    keep = false;
                    out.trace(
                        Some(ctid),
                        TraceKind::CurlResult {
                            dst: key.2,
                            port: key.3,
                            outcome: CurlOutcome::Ok {
                                response: seg.data.clone(),
                            },
                        },
                    );
                    let ack = seg
                        .seq
                        .wrapping_add(seg_len)
                        .wrapping_add(u32::from(seg.flags.fin));
                    self.send_tcp(
                        ctx,
                        out,
                        key,
                        conn.snd_nxt,
                        ack,
                        TcpFlags::FIN_ACK,
                        String::new(),
                        ctid,
                    );
                }
            }
            (false, TcpState::SynRcvd) => {
                if seg.flags.syn && !seg.flags.ack {
                    // SYN retransmis : on répète le SYN-ACK.
                    let (seq, ack) = (conn.snd_nxt.wrapping_sub(1), conn.rcv_nxt);
                    self.send_tcp(
                        ctx,
                        out,
                        key,
                        seq,
                        ack,
                        TcpFlags::SYN_ACK,
                        String::new(),
                        ctid,
                    );
                } else if seg.flags.rst {
                    keep = false;
                } else if seg.flags.ack {
                    conn.state = TcpState::Established;
                    out.trace(
                        Some(ctid),
                        TraceKind::TcpEstablished {
                            peer: key.2,
                            peer_port: key.3,
                            local_port: key.1,
                        },
                    );
                    conn.rcv_nxt = seg.seq.wrapping_add(seg_len);
                    if !seg.data.is_empty() {
                        self.serve_request(ctx, out, key, &mut conn, &seg.data);
                    }
                }
            }
            (false, TcpState::Established) => {
                if seg.flags.rst {
                    keep = false;
                } else if !seg.data.is_empty() {
                    conn.rcv_nxt = seg.seq.wrapping_add(seg_len);
                    self.serve_request(ctx, out, key, &mut conn, &seg.data);
                }
            }
            (false, TcpState::FinSent) => {
                if seg.flags.ack || seg.flags.fin || seg.flags.rst {
                    keep = false;
                }
            }
            // États impossibles pour ce rôle : ignorés.
            (true, TcpState::SynRcvd | TcpState::FinSent) | (false, TcpState::SynSent) => {}
        }
        if keep {
            self.tcp.insert(key, conn);
        }
    }

    /// Un segment arrive sans connexion correspondante.
    fn tcp_no_connection(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        key: TcpKey,
        seg: &TcpSegment,
        seg_len: u32,
        tid: TraceId,
    ) {
        if seg.flags.rst {
            return;
        }
        if seg.flags.syn && !seg.flags.ack {
            let svc = self
                .services
                .iter()
                .find(|s| s.proto == L4Proto::Tcp && s.port == seg.dst_port)
                .cloned();
            match svc {
                Some(svc) => {
                    let isn = self.alloc_isn();
                    let rcv_nxt = seg.seq.wrapping_add(1);
                    self.tcp.insert(
                        key,
                        TcpConn {
                            state: TcpState::SynRcvd,
                            client: false,
                            trace_id: tid,
                            snd_nxt: isn.wrapping_add(1),
                            rcv_nxt,
                            syn_attempts: 0,
                            service: Some(svc),
                        },
                    );
                    self.send_tcp(
                        ctx,
                        out,
                        key,
                        isn,
                        rcv_nxt,
                        TcpFlags::SYN_ACK,
                        String::new(),
                        tid,
                    );
                    self.set_timer(out, TCP_IDLE_TIMEOUT, TimerKind::TcpIdle { key });
                    return;
                }
                None => {
                    out.trace(
                        Some(tid),
                        TraceKind::TcpResetSent {
                            to: key.2,
                            port: key.1,
                        },
                    );
                    let ack = seg.seq.wrapping_add(1);
                    self.send_tcp(ctx, out, key, 0, ack, TcpFlags::RST_ACK, String::new(), tid);
                    return;
                }
            }
        }
        // Segment orphelin (connexion inconnue) : on réinitialise.
        out.trace(
            Some(tid),
            TraceKind::TcpResetSent {
                to: key.2,
                port: key.1,
            },
        );
        let ack = seg.seq.wrapping_add(seg_len);
        self.send_tcp(
            ctx,
            out,
            key,
            seg.ack,
            ack,
            TcpFlags::RST_ACK,
            String::new(),
            tid,
        );
    }

    /// Le service traite la requête, répond et ferme son côté.
    fn serve_request(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        key: TcpKey,
        conn: &mut TcpConn,
        request: &str,
    ) {
        let Some(svc) = conn.service.clone() else {
            return;
        };
        out.trace(
            Some(conn.trace_id),
            TraceKind::ServiceRequest {
                from: key.2,
                port: key.1,
                service: svc.name.clone(),
            },
        );
        let response = svc.respond(request);
        let len = u32::try_from(response.len()).unwrap_or(0);
        let (seq, ack) = (conn.snd_nxt, conn.rcv_nxt);
        conn.snd_nxt = conn.snd_nxt.wrapping_add(len).wrapping_add(1);
        conn.state = TcpState::FinSent;
        let tid = conn.trace_id;
        self.send_tcp(ctx, out, key, seq, ack, TcpFlags::FIN_ACK, response, tid);
    }

    fn forward(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        mut packet: Ipv4Packet,
        tid: TraceId,
    ) {
        if packet.ttl <= 1 {
            out.trace(Some(tid), TraceKind::TtlExpired);
            self.send_icmp_error(ctx, out, ingress, &packet, IcmpErrorKind::TtlExpired, tid);
            return;
        }
        let from = packet.ttl;
        packet.ttl -= 1;
        out.trace(
            Some(tid),
            TraceKind::TtlDecremented {
                from,
                to: packet.ttl,
            },
        );

        let decision = route::lookup(ctx.ifaces, &self.static_routes, self.gateway, packet.dst);
        let chosen = decision.chosen_route().copied();
        out.trace(Some(tid), TraceKind::RouteLookup(decision));
        match chosen {
            Some(route) => self.emit(ctx, out, route, packet, tid),
            None => {
                out.trace(
                    Some(tid),
                    TraceKind::Dropped {
                        reason: DropReason::NoRoute,
                    },
                );
                self.send_icmp_error(
                    ctx,
                    out,
                    ingress,
                    &packet,
                    IcmpErrorKind::NetUnreachable,
                    tid,
                );
            }
        }
    }

    /// Répond par une erreur ICMP à l'émetteur de `offending`, sauf si
    /// `offending` est déjà une erreur ICMP.
    fn send_icmp_error(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        offending: &Ipv4Packet,
        kind: IcmpErrorKind,
        tid: TraceId,
    ) {
        if offending.is_icmp_error() {
            return;
        }
        let Some(src) = ctx.iface(ingress).and_then(|i| i.ip).map(|n| n.addr()) else {
            return;
        };
        let original = Box::new(offending.clone());
        let msg = match kind {
            IcmpErrorKind::TtlExpired => IcmpMessage::TimeExceeded { original },
            IcmpErrorKind::NetUnreachable => IcmpMessage::DestUnreachable {
                code: UnreachCode::Net,
                original,
            },
            IcmpErrorKind::PortUnreachable => IcmpMessage::DestUnreachable {
                code: UnreachCode::Port,
                original,
            },
        };
        self.originate(
            ctx,
            out,
            Some(src),
            offending.src,
            IpPayload::Icmp(msg),
            tid,
        );
    }
}
