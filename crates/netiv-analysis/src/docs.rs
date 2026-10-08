//! Documentation générée : topologie, plan d'adressage, VLAN, routage,
//! équipements, services, filtrage et recommandations de sécurité.

use std::fmt::Write as _;

use netiv_core::Simulation;
use netiv_core::device::Behavior;
use netiv_core::switch::PortMode;
use netiv_security::analyze;
use serde::{Deserialize, Serialize};

/// Format d'export.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DocFormat {
    /// Markdown.
    Markdown,
    /// JSON.
    Json,
    /// YAML.
    Yaml,
}

impl std::str::FromStr for DocFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Ok(DocFormat::Markdown),
            "json" => Ok(DocFormat::Json),
            "yaml" | "yml" => Ok(DocFormat::Yaml),
            "pdf" => Err("export PDF non pris en charge : génère le Markdown (md) puis convertis-le, par ex. « pandoc doc.md -o doc.pdf »".to_string()),
            other => Err(format!("format inconnu : « {other} » (md, json, yaml)")),
        }
    }
}

/// Une interface documentée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfaceDoc {
    /// Nom.
    pub name: String,
    /// Adresse MAC.
    pub mac: String,
    /// Adresse IP et préfixe.
    pub ip: Option<String>,
    /// Réseau.
    pub network: Option<String>,
    /// Câble : équipement et interface en face.
    pub connected_to: Option<String>,
    /// Mode du port (switch).
    pub port_mode: Option<String>,
}

/// Une route documentée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDoc {
    /// Destination.
    pub dest: String,
    /// Prochain saut.
    pub via: Option<String>,
    /// Interface de sortie.
    pub iface: String,
    /// Origine (connectée, statique, par défaut).
    pub origin: String,
}

/// Un équipement documenté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDoc {
    /// Nom.
    pub name: String,
    /// Type.
    pub kind: String,
    /// Rôle.
    pub role: String,
    /// Interfaces.
    pub interfaces: Vec<IfaceDoc>,
    /// Passerelle par défaut.
    pub gateway: Option<String>,
    /// Table de routage.
    pub routes: Vec<RouteDoc>,
    /// Services à l'écoute.
    pub services: Vec<String>,
    /// Règles de filtrage.
    pub filtering: Vec<String>,
    /// Politique de filtrage par défaut.
    pub default_policy: Option<String>,
}

/// Une recommandation de sécurité.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationDoc {
    /// Règle.
    pub id: String,
    /// Gravité.
    pub severity: String,
    /// Titre.
    pub title: String,
    /// Recommandation.
    pub recommendation: String,
}

/// La documentation d'un réseau.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkDoc {
    /// Nom du réseau.
    pub name: String,
    /// Équipements.
    pub devices: Vec<DeviceDoc>,
    /// Câbles : `A:iface — B:iface`.
    pub links: Vec<String>,
    /// Recommandations.
    pub recommendations: Vec<RecommendationDoc>,
}

/// Construit la documentation de `sim`.
pub fn build_doc(sim: &Simulation, name: &str) -> NetworkDoc {
    let dn = |d: netiv_core::DeviceId| netiv_core::render::device_name(sim, d).to_string();
    let en = |d, i| format!("{}:{}", dn(d), netiv_core::render::iface_name(sim, d, i));

    let devices = sim
        .devices()
        .iter()
        .map(|d| {
            let interfaces = d
                .ifaces
                .iter()
                .map(|i| {
                    let peer = i
                        .link
                        .and_then(|l| sim.link(l))
                        .and_then(|l| l.peer_of(d.id, i.id))
                        .map(|(pd, pi)| en(pd, pi));
                    let port_mode = match &d.behavior {
                        Behavior::Switch(sw) => Some(match sw.port_mode(i.id) {
                            PortMode::Access { vlan } => format!("accès VLAN {vlan}"),
                            PortMode::Trunk { allowed, native } => format!(
                                "trunk ({}) natif {native}",
                                if allowed.is_empty() {
                                    "tous les VLAN".to_string()
                                } else {
                                    format!(
                                        "VLAN {}",
                                        allowed
                                            .iter()
                                            .map(u16::to_string)
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    )
                                }
                            ),
                        }),
                        Behavior::Ip(_) => None,
                    };
                    IfaceDoc {
                        name: i.name.clone(),
                        mac: i.mac.to_string(),
                        ip: i.ip.map(|n| n.to_string()),
                        network: i.ip.map(|n| n.trunc().to_string()),
                        connected_to: peer,
                        port_mode,
                    }
                })
                .collect();
            let (gateway, routes, services, filtering, default_policy) = match &d.behavior {
                Behavior::Ip(n) => {
                    let (active, _) = n.active_routes(&d.ifaces);
                    (
                        n.gateway.map(|g| g.to_string()),
                        active
                            .iter()
                            .map(|r| RouteDoc {
                                dest: r.dest.to_string(),
                                via: r.next_hop.map(|h| h.to_string()),
                                iface: netiv_core::render::iface_name(sim, d.id, r.iface)
                                    .to_string(),
                                origin: r.origin.to_string(),
                            })
                            .collect(),
                        n.services
                            .iter()
                            .map(|s| format!("{}/{} ({})", s.proto, s.port, s.name))
                            .collect(),
                        n.acl
                            .rules
                            .iter()
                            .map(|r| {
                                r.describe(|i| {
                                    netiv_core::render::iface_name(sim, d.id, i).to_string()
                                })
                            })
                            .collect(),
                        (!n.acl.rules.is_empty() || d.kind == netiv_core::DeviceKind::Firewall)
                            .then(|| n.acl.default.to_string()),
                    )
                }
                Behavior::Switch(_) => (None, Vec::new(), Vec::new(), Vec::new(), None),
            };
            DeviceDoc {
                name: d.name.clone(),
                kind: d.kind.to_string(),
                role: d.effective_role().to_string(),
                interfaces,
                gateway,
                routes,
                services,
                filtering,
                default_policy,
            }
        })
        .collect();
    let links = sim
        .links()
        .iter()
        .map(|l| format!("{} — {}", en(l.a.0, l.a.1), en(l.b.0, l.b.1)))
        .collect();
    let recommendations = analyze(sim)
        .findings
        .into_iter()
        .map(|f| RecommendationDoc {
            id: f.id.to_string(),
            severity: f.severity.label().to_string(),
            title: f.title,
            recommendation: f.recommendation,
        })
        .collect();
    NetworkDoc {
        name: name.to_string(),
        devices,
        links,
        recommendations,
    }
}

impl NetworkDoc {
    /// Rendu Markdown.
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# Documentation réseau — {}\n", self.name);
        let _ = writeln!(
            s,
            "{} équipements, {} câbles.\n",
            self.devices.len(),
            self.links.len()
        );

        let _ = writeln!(s, "## Topologie\n");
        for l in &self.links {
            let _ = writeln!(s, "- {l}");
        }

        let _ = writeln!(s, "\n## Plan d'adressage\n");
        let _ = writeln!(s, "| Équipement | Interface | Adresse | Réseau | Relié à |");
        let _ = writeln!(s, "|---|---|---|---|---|");
        for d in &self.devices {
            for i in d.interfaces.iter().filter(|i| i.ip.is_some()) {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} |",
                    d.name,
                    i.name,
                    i.ip.as_deref().unwrap_or("-"),
                    i.network.as_deref().unwrap_or("-"),
                    i.connected_to.as_deref().unwrap_or("-")
                );
            }
        }

        let switches: Vec<&DeviceDoc> =
            self.devices.iter().filter(|d| d.kind == "switch").collect();
        if !switches.is_empty() {
            let _ = writeln!(s, "\n## VLAN et ports de switch\n");
            let _ = writeln!(s, "| Switch | Port | Mode | Relié à |");
            let _ = writeln!(s, "|---|---|---|---|");
            for d in switches {
                for i in &d.interfaces {
                    let _ = writeln!(
                        s,
                        "| {} | {} | {} | {} |",
                        d.name,
                        i.name,
                        i.port_mode.as_deref().unwrap_or("-"),
                        i.connected_to.as_deref().unwrap_or("-")
                    );
                }
            }
        }

        let _ = writeln!(s, "\n## Routage\n");
        for d in self.devices.iter().filter(|d| !d.routes.is_empty()) {
            let _ = writeln!(s, "### {}\n", d.name);
            let _ = writeln!(s, "| Destination | Prochain saut | Interface | Origine |");
            let _ = writeln!(s, "|---|---|---|---|");
            for r in &d.routes {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} |",
                    r.dest,
                    r.via.as_deref().unwrap_or("-"),
                    r.iface,
                    r.origin
                );
            }
            let _ = writeln!(s);
        }

        let _ = writeln!(s, "## Équipements\n");
        let _ = writeln!(s, "| Nom | Type | Rôle | Passerelle | Services |");
        let _ = writeln!(s, "|---|---|---|---|---|");
        for d in &self.devices {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                d.name,
                d.kind,
                d.role,
                d.gateway.as_deref().unwrap_or("-"),
                if d.services.is_empty() {
                    "-".to_string()
                } else {
                    d.services.join(", ")
                }
            );
        }

        let filtered: Vec<&DeviceDoc> = self
            .devices
            .iter()
            .filter(|d| !d.filtering.is_empty() || d.default_policy.is_some())
            .collect();
        if !filtered.is_empty() {
            let _ = writeln!(s, "\n## Filtrage\n");
            for d in filtered {
                let _ = writeln!(
                    s,
                    "### {} (politique par défaut : {})\n",
                    d.name,
                    d.default_policy.as_deref().unwrap_or("allow")
                );
                for (i, r) in d.filtering.iter().enumerate() {
                    let _ = writeln!(s, "{}. {r}", i + 1);
                }
                let _ = writeln!(s);
            }
        }

        let _ = writeln!(s, "## Recommandations de sécurité\n");
        if self.recommendations.is_empty() {
            let _ = writeln!(s, "Aucune recommandation : l'analyse n'a rien relevé.");
        }
        for r in &self.recommendations {
            let _ = writeln!(
                s,
                "- **[{}] {}** ({}) — {}",
                r.severity, r.title, r.id, r.recommendation
            );
        }
        s
    }
}

/// Génère la documentation de `sim` dans le format voulu.
pub fn generate_docs(sim: &Simulation, name: &str, format: DocFormat) -> Result<String, String> {
    let doc = build_doc(sim, name);
    match format {
        DocFormat::Markdown => Ok(doc.to_markdown()),
        DocFormat::Json => serde_json::to_string_pretty(&doc).map_err(|e| e.to_string()),
        DocFormat::Yaml => serde_yaml_ng::to_string(&doc).map_err(|e| e.to_string()),
    }
}
