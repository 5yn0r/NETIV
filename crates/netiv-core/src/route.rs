//! Table de routage et décision « longest prefix match » explicable.

use std::fmt;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::addr::Ipv4Net;
use crate::device::Interface;
use crate::ids::IfaceId;

/// Route statique configurée par l'utilisateur : `dest` via `via`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticRoute {
    /// Réseau de destination.
    pub dest: Ipv4Net,
    /// Prochain saut.
    pub via: Ipv4Addr,
}

/// D'où vient une route.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteOrigin {
    /// Réseau directement connecté à une interface.
    Connected,
    /// Route statique.
    Static,
    /// Passerelle par défaut.
    Default,
}

impl fmt::Display for RouteOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RouteOrigin::Connected => "connectée",
            RouteOrigin::Static => "statique",
            RouteOrigin::Default => "par défaut",
        })
    }
}

/// Route active (utilisable).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// Réseau de destination (adresse réseau + préfixe).
    pub dest: Ipv4Net,
    /// Prochain saut ; `None` si la destination est sur le réseau connecté.
    pub next_hop: Option<Ipv4Addr>,
    /// Interface de sortie.
    pub iface: IfaceId,
    /// Origine.
    pub origin: RouteOrigin,
}

/// Une route examinée lors du lookup, avec le résultat du test.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCandidate {
    /// La route examinée.
    pub route: Route,
    /// La destination appartient-elle à son réseau ?
    pub matches: bool,
}

/// Route configurée mais inutilisable : son prochain saut n'est sur aucun
/// réseau connecté.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InactiveRoute {
    /// Réseau de destination.
    pub dest: Ipv4Net,
    /// Prochain saut configuré.
    pub via: Ipv4Addr,
    /// Origine (statique ou passerelle par défaut).
    pub origin: RouteOrigin,
}

/// Résultat complet d'un lookup : toutes les routes examinées et la gagnante.
/// C'est la matière première de l'Explain Engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    /// Destination cherchée.
    pub dst: Ipv4Addr,
    /// Toutes les routes actives, dans l'ordre d'examen.
    pub candidates: Vec<RouteCandidate>,
    /// Routes configurées mais inutilisables.
    pub inactive: Vec<InactiveRoute>,
    /// Index (dans `candidates`) de la route retenue.
    pub chosen: Option<usize>,
}

impl RouteDecision {
    /// Route retenue.
    pub fn chosen_route(&self) -> Option<&Route> {
        self.chosen
            .and_then(|i| self.candidates.get(i))
            .map(|c| &c.route)
    }
}

/// Construit les routes actives et inactives d'un équipement.
///
/// Les routes connectées sont *dérivées* des interfaces (jamais stockées),
/// elles ne peuvent donc pas être désynchronisées de la configuration.
pub fn build_routes(
    ifaces: &[Interface],
    statics: &[StaticRoute],
    gateway: Option<Ipv4Addr>,
) -> (Vec<Route>, Vec<InactiveRoute>) {
    let mut active = Vec::new();
    let mut inactive = Vec::new();

    for ifc in ifaces {
        if let (true, Some(net)) = (ifc.admin_up, ifc.ip) {
            active.push(Route {
                dest: net.trunc(),
                next_hop: None,
                iface: ifc.id,
                origin: RouteOrigin::Connected,
            });
        }
    }

    let via_routes = statics
        .iter()
        .map(|s| (s.dest.trunc(), s.via, RouteOrigin::Static))
        .chain(gateway.map(|gw| (default_net(), gw, RouteOrigin::Default)));

    for (dest, via, origin) in via_routes {
        let out = ifaces
            .iter()
            .find(|i| i.admin_up && i.ip.is_some_and(|n| n.contains(via)));
        match out {
            Some(ifc) => active.push(Route {
                dest,
                next_hop: Some(via),
                iface: ifc.id,
                origin,
            }),
            None => inactive.push(InactiveRoute { dest, via, origin }),
        }
    }
    (active, inactive)
}

/// Longest prefix match. Retourne toujours la décision complète.
///
/// À préfixe égal, la première route examinée gagne (connectées, puis
/// statiques, puis passerelle par défaut).
pub fn lookup(
    ifaces: &[Interface],
    statics: &[StaticRoute],
    gateway: Option<Ipv4Addr>,
    dst: Ipv4Addr,
) -> RouteDecision {
    let (routes, inactive) = build_routes(ifaces, statics, gateway);
    let candidates: Vec<RouteCandidate> = routes
        .into_iter()
        .map(|route| RouteCandidate {
            matches: route.dest.contains(dst),
            route,
        })
        .collect();

    let mut chosen: Option<usize> = None;
    for (i, c) in candidates.iter().enumerate() {
        if !c.matches {
            continue;
        }
        let better = match chosen {
            None => true,
            Some(j) => c.route.dest.prefix() > candidates[j].route.dest.prefix(),
        };
        if better {
            chosen = Some(i);
        }
    }
    RouteDecision {
        dst,
        candidates,
        inactive,
        chosen,
    }
}

fn default_net() -> Ipv4Net {
    // 0.0.0.0/0 est toujours valide.
    Ipv4Net::new(Ipv4Addr::UNSPECIFIED, 0).unwrap_or_else(|_| unreachable!("/0 est valide"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::MacAddr;
    use crate::ids::LinkId;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn ifc(id: u32, cidr: Option<&str>, up: bool) -> Interface {
        Interface {
            id: IfaceId(id),
            name: format!("eth{id}"),
            mac: MacAddr::from_index(id),
            ip: cidr.map(|c| c.parse().unwrap()),
            admin_up: up,
            management: false,
            link: Some(LinkId(id)),
        }
    }

    fn st(dest: &str, via: &str) -> StaticRoute {
        StaticRoute {
            dest: dest.parse().unwrap(),
            via: ip(via),
        }
    }

    #[test]
    fn connected_routes_are_derived_from_interfaces() {
        let ifaces = [ifc(0, Some("192.168.1.1/24"), true), ifc(1, None, true)];
        let (active, inactive) = build_routes(&ifaces, &[], None);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].dest.to_string(), "192.168.1.0/24");
        assert_eq!(active[0].next_hop, None);
        assert!(inactive.is_empty());
    }

    #[test]
    fn down_interface_has_no_connected_route() {
        let ifaces = [ifc(0, Some("192.168.1.1/24"), false)];
        assert!(build_routes(&ifaces, &[], None).0.is_empty());
    }

    #[test]
    fn longest_prefix_wins() {
        let ifaces = [
            ifc(0, Some("192.168.1.1/24"), true),
            ifc(1, Some("10.0.0.1/30"), true),
            ifc(2, Some("172.16.0.1/16"), true),
        ];
        let statics = [
            st("192.168.0.0/16", "10.0.0.2"),
            st("192.168.2.0/24", "172.16.0.9"),
        ];
        let d = lookup(&ifaces, &statics, Some(ip("10.0.0.2")), ip("192.168.2.10"));
        let r = d.chosen_route().unwrap();
        assert_eq!(r.dest.to_string(), "192.168.2.0/24");
        assert_eq!(r.iface, IfaceId(2));
        assert_eq!(r.next_hop, Some(ip("172.16.0.9")));
    }

    #[test]
    fn decision_lists_every_candidate_with_match_flag() {
        let ifaces = [
            ifc(0, Some("192.168.1.1/24"), true),
            ifc(1, Some("10.0.0.1/30"), true),
        ];
        let d = lookup(&ifaces, &[], Some(ip("10.0.0.2")), ip("8.8.8.8"));
        assert_eq!(d.candidates.len(), 3);
        let flags: Vec<bool> = d.candidates.iter().map(|c| c.matches).collect();
        assert_eq!(flags, [false, false, true]);
        assert_eq!(d.chosen_route().unwrap().origin, RouteOrigin::Default);
    }

    #[test]
    fn no_route_when_nothing_matches() {
        let ifaces = [ifc(0, Some("192.168.1.1/24"), true)];
        let d = lookup(&ifaces, &[], None, ip("8.8.8.8"));
        assert!(d.chosen.is_none());
        assert!(d.chosen_route().is_none());
    }

    #[test]
    fn gateway_outside_connected_networks_is_inactive() {
        let ifaces = [ifc(0, Some("192.168.1.10/24"), true)];
        let d = lookup(&ifaces, &[], Some(ip("192.168.10.254")), ip("8.8.8.8"));
        assert!(d.chosen.is_none());
        assert_eq!(d.inactive.len(), 1);
        assert_eq!(d.inactive[0].via, ip("192.168.10.254"));
        assert_eq!(d.inactive[0].origin, RouteOrigin::Default);
    }

    #[test]
    fn tie_goes_to_first_examined_route() {
        let ifaces = [
            ifc(0, Some("10.0.0.1/24"), true),
            ifc(1, Some("10.0.1.1/24"), true),
        ];
        let statics = [st("10.9.0.0/16", "10.0.0.2"), st("10.9.0.0/16", "10.0.1.2")];
        let d = lookup(&ifaces, &statics, None, ip("10.9.1.1"));
        assert_eq!(d.chosen_route().unwrap().next_hop, Some(ip("10.0.0.2")));
    }

    #[test]
    fn static_route_dest_is_normalised_to_network() {
        let ifaces = [ifc(0, Some("10.0.0.1/30"), true)];
        let statics = [st("192.168.2.77/24", "10.0.0.2")];
        let (active, _) = build_routes(&ifaces, &statics, None);
        assert_eq!(active[1].dest.to_string(), "192.168.2.0/24");
    }
}
