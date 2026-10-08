//! Services simulés qu'un équipement peut héberger.

use serde::{Deserialize, Serialize};

use crate::packet::L4Proto;

/// Un service à l'écoute sur un port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Protocole de transport.
    pub proto: L4Proto,
    /// Port d'écoute.
    pub port: u16,
    /// Nom : `http`, `ssh`, `echo`... Détermine la réponse.
    pub name: String,
}

impl Service {
    /// Service TCP.
    pub fn tcp(port: u16, name: &str) -> Self {
        Service {
            proto: L4Proto::Tcp,
            port,
            name: name.to_string(),
        }
    }

    /// Service UDP.
    pub fn udp(port: u16, name: &str) -> Self {
        Service {
            proto: L4Proto::Udp,
            port,
            name: name.to_string(),
        }
    }

    /// Réponse du service à une requête.
    pub fn respond(&self, request: &str) -> String {
        match (self.proto, self.name.as_str()) {
            (L4Proto::Tcp, "http") => "HTTP/1.1 200 OK".to_string(),
            (L4Proto::Tcp, other) => format!("{other} prêt"),
            (L4Proto::Udp, "echo") => request.to_string(),
            (L4Proto::Udp, other) => format!("{other} ok"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_depend_on_the_service() {
        assert_eq!(Service::tcp(80, "http").respond("GET /"), "HTTP/1.1 200 OK");
        assert_eq!(Service::tcp(22, "ssh").respond("x"), "ssh prêt");
        assert_eq!(Service::udp(7, "echo").respond("salut"), "salut");
        assert_eq!(Service::udp(53, "dns").respond("x"), "dns ok");
    }
}
