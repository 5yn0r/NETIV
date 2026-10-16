//! DHCP dans la pile IP : client, serveur et relais.
//!
//! Voir `crate::dhcp` pour les messages, les plages et les simplifications.

use std::net::Ipv4Addr;

use super::{IpNode, TimerKind};
use crate::addr::Ipv4Net;
use crate::device::{Ctx, Effects};
use crate::dhcp::{
    Binding, CLIENT_PORT, ClientState, DhcpEvent, DhcpMessage, DhcpOp, MAX_ATTEMPTS, RETRY_BASE,
    SERVER_PORT,
};
use crate::ids::{IfaceId, TraceId};
use crate::packet::{
    DEFAULT_TTL, EtherPayload, Frame, IpPayload, Ipv4Packet, UdpDatagram, UdpPayload,
};
use crate::time::SimTime;
use crate::trace::{DropReason, TraceKind};

/// Attente avant la nouvelle tentative `attempt` (2 s, 4 s, 8 s…).
fn retry_delay(attempt: u8) -> SimTime {
    let factor = 1u64 << u32::from(attempt.saturating_sub(1)).min(10);
    SimTime::from_micros(RETRY_BASE.as_micros().saturating_mul(factor))
}

fn dhcp_payload(sport: u16, dport: u16, msg: DhcpMessage) -> IpPayload {
    IpPayload::Udp(UdpDatagram {
        src_port: sport,
        dst_port: dport,
        payload: UdpPayload::Dhcp(Box::new(msg)),
    })
}

impl IpNode {
    fn dhcp_event(&self, out: &mut Effects, tid: TraceId, iface: IfaceId, event: DhcpEvent) {
        out.trace(Some(tid), TraceKind::Dhcp { iface, event });
    }

    /// Envoie un message DHCP en diffusion sur `iface` (sans ARP ni route).
    #[allow(clippy::too_many_arguments)]
    fn dhcp_broadcast(
        &self,
        ctx: &Ctx,
        out: &mut Effects,
        iface: IfaceId,
        src: Ipv4Addr,
        ports: (u16, u16),
        msg: DhcpMessage,
        tid: TraceId,
    ) {
        let Some(i) = ctx.iface(iface) else { return };
        let packet = Ipv4Packet {
            src,
            dst: Ipv4Addr::BROADCAST,
            ttl: DEFAULT_TTL,
            payload: dhcp_payload(ports.0, ports.1, msg),
        };
        out.send(
            iface,
            Frame {
                src: i.mac,
                dst: crate::addr::MacAddr::BROADCAST,
                vlan: None,
                payload: EtherPayload::Ipv4(packet),
                trace_id: tid,
            },
        );
    }

    fn alloc_xid(&mut self) -> u32 {
        let x = self.next_xid;
        self.next_xid = self.next_xid.wrapping_add(1);
        x
    }

    fn arm_dhcp_timer(&mut self, out: &mut Effects, iface: IfaceId, attempt: u8) {
        let id = self.set_timer(out, retry_delay(attempt), TimerKind::DhcpRetry { iface });
        self.dhcp_timers.insert(iface, id);
    }

    // ------------------------------------------------------------------
    // Client
    // ------------------------------------------------------------------

    /// Démarre (ou redémarre) le client DHCP d'une interface.
    pub(super) fn dhcp_start(&mut self, ctx: &Ctx, out: &mut Effects, iface: IfaceId, fresh: bool) {
        if !self.dhcp_ifaces.contains(&iface) || ctx.iface(iface).is_none() {
            return;
        }
        if fresh {
            if ctx.iface(iface).and_then(|i| i.ip).is_some() {
                out.configure_iface(iface, None);
            }
            self.forget_learned();
            self.dhcp_clients.remove(&iface);
        }
        let tid = out.new_trace_id();
        self.dhcp_trace.insert(iface, tid);
        self.dhcp_discover(ctx, out, iface, 1);
    }

    fn dhcp_discover(&mut self, ctx: &Ctx, out: &mut Effects, iface: IfaceId, attempt: u8) {
        let Some(mac) = ctx.iface(iface).map(|i| i.mac) else {
            return;
        };
        let Some(&tid) = self.dhcp_trace.get(&iface) else {
            return;
        };
        let xid = self.alloc_xid();
        self.dhcp_clients
            .insert(iface, ClientState::Selecting { xid, attempt });
        self.dhcp_event(out, tid, iface, DhcpEvent::ClientDiscover { attempt });
        let msg = DhcpMessage::new(DhcpOp::Discover, xid, mac);
        self.dhcp_broadcast(
            ctx,
            out,
            iface,
            Ipv4Addr::UNSPECIFIED,
            (CLIENT_PORT, SERVER_PORT),
            msg,
            tid,
        );
        self.arm_dhcp_timer(out, iface, attempt);
    }

    /// Pas de réponse à temps : nouvelle tentative, ou abandon.
    pub(super) fn dhcp_retry(&mut self, ctx: &Ctx, out: &mut Effects, iface: IfaceId) {
        let attempt = match self.dhcp_clients.get(&iface) {
            Some(
                ClientState::Selecting { attempt, .. } | ClientState::Requesting { attempt, .. },
            ) => *attempt,
            _ => return,
        };
        if attempt >= MAX_ATTEMPTS {
            self.dhcp_clients.insert(iface, ClientState::Failed);
            if let Some(tid) = self.dhcp_trace.remove(&iface) {
                self.dhcp_event(
                    out,
                    tid,
                    iface,
                    DhcpEvent::ClientFailed { attempts: attempt },
                );
            }
            return;
        }
        self.dhcp_discover(ctx, out, iface, attempt + 1);
    }

    /// Rend l'adresse obtenue sur `iface`.
    pub(super) fn dhcp_release(&mut self, ctx: &Ctx, out: &mut Effects, iface: IfaceId) {
        let Some(ClientState::Bound(b)) = self.dhcp_clients.get(&iface).cloned() else {
            return;
        };
        let Some(mac) = ctx.iface(iface).map(|i| i.mac) else {
            return;
        };
        let tid = out.new_trace_id();
        self.dhcp_event(
            out,
            tid,
            iface,
            DhcpEvent::ClientReleased { ip: b.ip.addr() },
        );
        let mut msg = DhcpMessage::new(DhcpOp::Release, self.alloc_xid(), mac);
        msg.ciaddr = Some(b.ip.addr());
        msg.server_id = Some(b.server);
        // Envoyé avant de perdre l'adresse : l'unicast part encore de b.ip.
        self.originate(
            ctx,
            out,
            Some(b.ip.addr()),
            b.server,
            dhcp_payload(CLIENT_PORT, SERVER_PORT, msg),
            tid,
        );
        out.configure_iface(iface, None);
        self.forget_learned();
        self.dhcp_clients.remove(&iface);
        if let Some(t) = self.dhcp_timers.remove(&iface) {
            self.timers.remove(&t);
        }
    }

    /// Oublie la passerelle et le DNS reçus par DHCP.
    fn forget_learned(&mut self) {
        if self.gateway_learned {
            self.gateway = None;
            self.gateway_learned = false;
        }
        self.dns_learned = None;
    }

    /// Réponse d'un serveur reçue par le client.
    fn dhcp_client_receive(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        iface: IfaceId,
        m: &DhcpMessage,
        tid: TraceId,
    ) {
        let mine = ctx.iface(iface).is_some_and(|i| i.mac == m.client_mac)
            && self.dhcp_ifaces.contains(&iface);
        let state = self.dhcp_clients.get(&iface).cloned();
        // Notre propre échange garde son numéro de transaction (même après
        // une nouvelle tentative) ; un message étranger garde le sien.
        let tid = if mine {
            self.dhcp_trace.get(&iface).copied().unwrap_or(tid)
        } else {
            tid
        };
        match (mine, m.op, state) {
            (true, DhcpOp::Offer, Some(ClientState::Selecting { xid, attempt }))
                if xid == m.xid =>
            {
                let (Some(ip), Some(server)) = (m.yiaddr, m.server_id) else {
                    return;
                };
                self.dhcp_event(out, tid, iface, DhcpEvent::ClientRequest { ip, server });
                let mut req = DhcpMessage::new(DhcpOp::Request, xid, m.client_mac);
                req.requested = Some(ip);
                req.server_id = Some(server);
                self.dhcp_clients.insert(
                    iface,
                    ClientState::Requesting {
                        xid,
                        attempt,
                        offer: Box::new(m.clone()),
                    },
                );
                self.dhcp_broadcast(
                    ctx,
                    out,
                    iface,
                    Ipv4Addr::UNSPECIFIED,
                    (CLIENT_PORT, SERVER_PORT),
                    req,
                    tid,
                );
                self.arm_dhcp_timer(out, iface, attempt);
            }
            (true, DhcpOp::Ack, Some(ClientState::Requesting { xid, .. })) if xid == m.xid => {
                let (Some(ip), Some(server)) = (m.yiaddr, m.server_id) else {
                    return;
                };
                let Ok(net) = Ipv4Net::new(ip, m.prefix.unwrap_or(24)) else {
                    return;
                };
                let binding = Binding {
                    ip: net,
                    server,
                    gateway: m.gateway,
                    dns: m.dns,
                    lease_s: m.lease_s,
                };
                out.configure_iface(iface, Some(net));
                if let Some(g) = m.gateway {
                    if self.gateway.is_none() || self.gateway_learned {
                        self.gateway = Some(g);
                        self.gateway_learned = true;
                    }
                }
                self.dns_learned = m.dns;
                if let Some(t) = self.dhcp_timers.remove(&iface) {
                    self.timers.remove(&t);
                }
                self.dhcp_trace.remove(&iface);
                self.dhcp_clients
                    .insert(iface, ClientState::Bound(binding.clone()));
                self.dhcp_event(out, tid, iface, DhcpEvent::ClientBound(binding));
            }
            (
                true,
                DhcpOp::Nak,
                Some(
                    ClientState::Requesting { xid, attempt, .. }
                    | ClientState::Selecting { xid, attempt },
                ),
            ) if xid == m.xid => {
                self.dhcp_event(out, tid, iface, DhcpEvent::ClientNak);
                if let Some(t) = self.dhcp_timers.remove(&iface) {
                    self.timers.remove(&t);
                }
                if attempt >= MAX_ATTEMPTS {
                    self.dhcp_clients.insert(iface, ClientState::Failed);
                    self.dhcp_trace.remove(&iface);
                    self.dhcp_event(
                        out,
                        tid,
                        iface,
                        DhcpEvent::ClientFailed { attempts: attempt },
                    );
                } else {
                    self.dhcp_discover(ctx, out, iface, attempt + 1);
                }
            }
            // Réponse destinée à un autre client, ou périmée.
            _ => out.trace(
                Some(tid),
                TraceKind::Dropped {
                    reason: DropReason::NotForUs,
                },
            ),
        }
    }

    // ------------------------------------------------------------------
    // Réception : client, serveur ou relais
    // ------------------------------------------------------------------

    /// Un message DHCP arrive.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn on_dhcp(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        packet: &Ipv4Packet,
        u: &UdpDatagram,
        m: &DhcpMessage,
        tid: TraceId,
    ) {
        if u.dst_port == CLIENT_PORT && !m.op.from_client() {
            self.dhcp_client_receive(ctx, out, ingress, m, tid);
            return;
        }
        if u.dst_port != SERVER_PORT {
            return;
        }
        let serving = self.dhcp_server.as_ref().is_some_and(|s| s.enabled);
        if serving && m.op.from_client() {
            self.dhcp_serve(ctx, out, ingress, packet, m, tid);
            return;
        }
        // Relais : une demande de client diffusée sur une interface relais…
        if m.op.from_client() && packet.dst == Ipv4Addr::BROADCAST {
            if let (Some(&server), Some(gi)) = (
                self.dhcp_relay.get(&ingress),
                ctx.iface(ingress).and_then(|i| i.ip).map(|n| n.addr()),
            ) {
                let mut fwd = m.clone();
                fwd.giaddr = fwd.giaddr.or(Some(gi));
                self.dhcp_event(out, tid, ingress, DhcpEvent::Relayed { to: server });
                self.originate(
                    ctx,
                    out,
                    Some(gi),
                    server,
                    dhcp_payload(SERVER_PORT, SERVER_PORT, fwd),
                    tid,
                );
                return;
            }
        }
        // … ou la réponse du serveur, à rediffuser sur le réseau du client.
        if !m.op.from_client() {
            if let Some(gi) = m.giaddr {
                if let Some(back) = ctx
                    .ifaces
                    .iter()
                    .find(|i| i.ip.is_some_and(|n| n.addr() == gi))
                {
                    let back_id = back.id;
                    self.dhcp_event(out, tid, back_id, DhcpEvent::RelayedBack { op: m.op });
                    self.dhcp_broadcast(
                        ctx,
                        out,
                        back_id,
                        gi,
                        (SERVER_PORT, CLIENT_PORT),
                        m.clone(),
                        tid,
                    );
                    return;
                }
            }
        }
        // Personne ici ne s'en occupe.
        out.trace(
            Some(tid),
            TraceKind::Dropped {
                reason: DropReason::NotForUs,
            },
        );
    }

    /// Le serveur traite une demande de client.
    fn dhcp_serve(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        ingress: IfaceId,
        packet: &Ipv4Packet,
        m: &DhcpMessage,
        tid: TraceId,
    ) {
        let ingress_ip = ctx.iface(ingress).and_then(|i| i.ip).map(|n| n.addr());
        // Adresse qui désigne le réseau du client : le relais, sinon nous-mêmes.
        let Some(hint) = m.giaddr.or(ingress_ip) else {
            return;
        };
        let server_ip = if m.giaddr.is_some() && packet.dst != Ipv4Addr::BROADCAST {
            packet.dst
        } else {
            match ingress_ip {
                Some(ip) => ip,
                None => return,
            }
        };
        let now = ctx.now;
        let Some(srv) = self.dhcp_server.as_mut() else {
            return;
        };
        let reply = match m.op {
            DhcpOp::Discover => {
                let Some(pi) = srv.pool_for(hint) else {
                    out.trace(
                        Some(tid),
                        TraceKind::Dhcp {
                            iface: ingress,
                            event: DhcpEvent::ServerNoPool { subnet: hint },
                        },
                    );
                    return;
                };
                let pool = srv.pools[pi].clone();
                let Some(ip) = srv.offer(pi, m.client_mac, now) else {
                    out.trace(
                        Some(tid),
                        TraceKind::Dhcp {
                            iface: ingress,
                            event: DhcpEvent::ServerExhausted {
                                network: pool.network.trunc(),
                            },
                        },
                    );
                    return;
                };
                out.trace(
                    Some(tid),
                    TraceKind::Dhcp {
                        iface: ingress,
                        event: DhcpEvent::ServerOffer {
                            ip,
                            mac: m.client_mac,
                        },
                    },
                );
                let mut r = DhcpMessage::new(DhcpOp::Offer, m.xid, m.client_mac);
                r.yiaddr = Some(ip);
                r.prefix = Some(pool.network.prefix());
                r.gateway = pool.gateway;
                r.dns = pool.dns;
                r.lease_s = pool.lease_s;
                r
            }
            DhcpOp::Request => {
                let Some(ip) = m.requested else { return };
                if m.server_id.is_some_and(|s| s != server_ip) {
                    // Le client a choisi un autre serveur : on libère l'offre.
                    srv.release(ip, m.client_mac);
                    return;
                }
                match srv.commit(ip, m.client_mac, now).cloned() {
                    Some(pool) => {
                        out.trace(
                            Some(tid),
                            TraceKind::Dhcp {
                                iface: ingress,
                                event: DhcpEvent::ServerAck {
                                    ip,
                                    mac: m.client_mac,
                                },
                            },
                        );
                        let mut r = DhcpMessage::new(DhcpOp::Ack, m.xid, m.client_mac);
                        r.yiaddr = Some(ip);
                        r.prefix = Some(pool.network.prefix());
                        r.gateway = pool.gateway;
                        r.dns = pool.dns;
                        r.lease_s = pool.lease_s;
                        r
                    }
                    None => {
                        out.trace(
                            Some(tid),
                            TraceKind::Dhcp {
                                iface: ingress,
                                event: DhcpEvent::ServerNak {
                                    ip,
                                    mac: m.client_mac,
                                },
                            },
                        );
                        DhcpMessage::new(DhcpOp::Nak, m.xid, m.client_mac)
                    }
                }
            }
            DhcpOp::Release => {
                if let Some(ip) = m.ciaddr {
                    if srv.release(ip, m.client_mac) {
                        out.trace(
                            Some(tid),
                            TraceKind::Dhcp {
                                iface: ingress,
                                event: DhcpEvent::ServerReleased { ip },
                            },
                        );
                    }
                }
                return;
            }
            DhcpOp::Offer | DhcpOp::Ack | DhcpOp::Nak => return,
        };
        let mut reply = reply;
        reply.server_id = Some(server_ip);
        reply.giaddr = m.giaddr;
        match m.giaddr {
            // Via un relais : réponse en unicast au relais.
            Some(gi) if packet.dst != Ipv4Addr::BROADCAST => {
                self.originate(
                    ctx,
                    out,
                    Some(server_ip),
                    gi,
                    dhcp_payload(SERVER_PORT, SERVER_PORT, reply),
                    tid,
                );
            }
            // Même réseau : le client n'a pas encore d'adresse, on diffuse.
            _ => self.dhcp_broadcast(
                ctx,
                out,
                ingress,
                server_ip,
                (SERVER_PORT, CLIENT_PORT),
                reply,
                tid,
            ),
        }
    }
}
