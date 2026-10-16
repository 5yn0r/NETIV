//! DNS dans la pile IP : résolution (client, avec cache) et réponses
//! (serveur faisant autorité).

use std::net::Ipv4Addr;

use super::{DnsPending, IpNode, TimerKind};
use crate::device::{Command, Ctx, Effects};
use crate::dns::{
    AfterResolve, CACHE_TTL, DnsMessage, DnsOutcome, MAX_ATTEMPTS, PORT, Rcode, TIMEOUT, normalize,
};
use crate::ids::TraceId;
use crate::packet::{IpPayload, Ipv4Packet, UdpDatagram, UdpPayload};
use crate::route;
use crate::trace::TraceKind;

fn dns_payload(sport: u16, dport: u16, msg: DnsMessage) -> IpPayload {
    IpPayload::Udp(UdpDatagram {
        src_port: sport,
        dst_port: dport,
        payload: UdpPayload::Dns(Box::new(msg)),
    })
}

impl IpNode {
    /// Résout `name`, puis exécute `then` vers l'adresse trouvée.
    pub(super) fn start_resolve(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        name: String,
        then: AfterResolve,
    ) {
        let name = normalize(&name);
        let tid = out.new_trace_id();
        // Réponse encore valide en cache : pas de question au serveur.
        if let Some(&(ip, expires)) = self.dns_cache.get(&name) {
            if expires > ctx.now {
                out.trace(
                    Some(tid),
                    TraceKind::DnsLookup {
                        name: name.clone(),
                        server: None,
                    },
                );
                out.trace(
                    Some(tid),
                    TraceKind::DnsResult {
                        name,
                        outcome: DnsOutcome::Resolved(ip),
                        cached: true,
                    },
                );
                self.run_after(ctx, out, ip, then);
                return;
            }
        }
        let Some(server) = self.effective_dns() else {
            out.trace(
                Some(tid),
                TraceKind::DnsLookup {
                    name: name.clone(),
                    server: None,
                },
            );
            out.trace(
                Some(tid),
                TraceKind::DnsResult {
                    name,
                    outcome: DnsOutcome::NoServer,
                    cached: false,
                },
            );
            return;
        };
        out.trace(
            Some(tid),
            TraceKind::DnsLookup {
                name: name.clone(),
                server: Some(server),
            },
        );
        // Aucune route vers le serveur : inutile d'attendre.
        let decision = route::lookup(ctx.ifaces, &self.static_routes, self.gateway, server);
        if decision.chosen_route().is_none() && !ctx.is_local_ip(server) {
            out.trace(Some(tid), TraceKind::RouteLookup(decision));
            out.trace(
                Some(tid),
                TraceKind::DnsResult {
                    name,
                    outcome: DnsOutcome::Unreachable,
                    cached: false,
                },
            );
            return;
        }
        let id = self.next_dns_id;
        self.next_dns_id = self.next_dns_id.wrapping_add(1).max(1);
        let src_port = self.alloc_port();
        self.dns_pending.insert(
            id,
            DnsPending {
                name,
                then,
                trace_id: tid,
                server,
                src_port,
                attempt: 1,
            },
        );
        self.send_dns_query(ctx, out, id);
    }

    fn send_dns_query(&mut self, ctx: &Ctx, out: &mut Effects, id: u16) {
        let Some(p) = self.dns_pending.get(&id).cloned() else {
            return;
        };
        let msg = DnsMessage {
            id,
            name: p.name.clone(),
            response: false,
            rcode: Rcode::NoError,
            answer: None,
        };
        self.set_timer(out, TIMEOUT, TimerKind::DnsTimeout { id });
        self.originate(
            ctx,
            out,
            None,
            p.server,
            dns_payload(p.src_port, PORT, msg),
            p.trace_id,
        );
    }

    /// Pas de réponse à temps : on redemande, ou on abandonne.
    pub(super) fn dns_timeout(&mut self, ctx: &Ctx, out: &mut Effects, id: u16) {
        let Some(p) = self.dns_pending.get_mut(&id) else {
            return;
        };
        if p.attempt < MAX_ATTEMPTS {
            p.attempt += 1;
            self.send_dns_query(ctx, out, id);
            return;
        }
        if let Some(p) = self.dns_pending.remove(&id) {
            out.trace(
                Some(p.trace_id),
                TraceKind::DnsResult {
                    name: p.name,
                    outcome: DnsOutcome::Timeout,
                    cached: false,
                },
            );
        }
    }

    /// Erreur ICMP en réponse à une question : le serveur est injoignable.
    pub(super) fn dns_unreachable(&mut self, out: &mut Effects, src_port: u16) {
        let Some(id) = self
            .dns_pending
            .iter()
            .find(|(_, p)| p.src_port == src_port)
            .map(|(id, _)| *id)
        else {
            return;
        };
        if let Some(p) = self.dns_pending.remove(&id) {
            out.trace(
                Some(p.trace_id),
                TraceKind::DnsResult {
                    name: p.name,
                    outcome: DnsOutcome::Unreachable,
                    cached: false,
                },
            );
        }
    }

    /// Réponse d'un serveur.
    pub(super) fn on_dns_response(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        packet: &Ipv4Packet,
        m: &DnsMessage,
    ) {
        let ok = self
            .dns_pending
            .get(&m.id)
            .is_some_and(|p| p.server == packet.src && p.name == normalize(&m.name));
        if !ok {
            return;
        }
        let Some(p) = self.dns_pending.remove(&m.id) else {
            return;
        };
        let outcome = match (m.rcode, m.answer) {
            (Rcode::NoError, Some(ip)) => {
                self.dns_cache
                    .insert(p.name.clone(), (ip, ctx.now.saturating_add(CACHE_TTL)));
                DnsOutcome::Resolved(ip)
            }
            _ => DnsOutcome::NxDomain,
        };
        out.trace(
            Some(p.trace_id),
            TraceKind::DnsResult {
                name: p.name,
                outcome: outcome.clone(),
                cached: false,
            },
        );
        if let DnsOutcome::Resolved(ip) = outcome {
            self.run_after(ctx, out, ip, p.then);
        }
    }

    /// Le serveur répond à une question.
    pub(super) fn on_dns_query(
        &mut self,
        ctx: &Ctx,
        out: &mut Effects,
        packet: &Ipv4Packet,
        u: &UdpDatagram,
        m: &DnsMessage,
        tid: TraceId,
    ) {
        let Some(srv) = &self.dns_server else { return };
        let answer = srv.lookup(&m.name);
        out.trace(
            Some(tid),
            TraceKind::DnsAnswered {
                name: normalize(&m.name),
                from: packet.src,
                answer,
            },
        );
        let reply = DnsMessage {
            id: m.id,
            name: m.name.clone(),
            response: true,
            rcode: if answer.is_some() {
                Rcode::NoError
            } else {
                Rcode::NxDomain
            },
            answer,
        };
        self.originate(
            ctx,
            out,
            Some(packet.dst),
            packet.src,
            dns_payload(PORT, u.src_port, reply),
            tid,
        );
    }

    /// Lance la commande qui attendait la résolution.
    fn run_after(&mut self, ctx: &Ctx, out: &mut Effects, ip: Ipv4Addr, then: AfterResolve) {
        let cmd = match then {
            AfterResolve::Nothing => return,
            AfterResolve::Ping { count } => Command::Ping { dst: ip, count },
            AfterResolve::Curl { port } => Command::Curl { dst: ip, port },
            AfterResolve::Udp { port, data } => Command::SendUdp {
                dst: ip,
                port,
                data,
            },
        };
        self.on_command(ctx, cmd, out);
    }
}
