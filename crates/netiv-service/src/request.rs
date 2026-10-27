//! Les routes de l'API HTTP, sans HTTP : `Service::request(méthode, chemin,
//! corps)` rend le même JSON et le même code que `netiv serve`. C'est ce qui
//! permet à l'interface web de fonctionner telle quelle dans le navigateur
//! (WebAssembly), sans serveur.

use std::collections::BTreeMap;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{DocsQuery, MAX_BODY, Service, ServiceResult, TopologyQuery, TraceQuery};

/// Réponse au format de l'API HTTP.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reply {
    /// Code HTTP (200, 400, 403, 404, 405, 409, 413, 422).
    pub status: u16,
    /// Corps JSON ; `{ "error": "…" }` en cas d'erreur.
    pub body: Value,
}

impl Reply {
    fn error(status: u16, msg: impl Into<String>) -> Self {
        Reply {
            status,
            body: json!({ "error": msg.into() }),
        }
    }
}

fn reply<T: Serialize>(r: ServiceResult<T>) -> Reply {
    match r {
        Ok(v) => match serde_json::to_value(v) {
            Ok(body) => Reply { status: 200, body },
            Err(e) => Reply::error(500, format!("réponse impossible à produire : {e}")),
        },
        Err(e) => Reply::error(e.status(), e.message),
    }
}

/// Décode un `%XX` d'URL (et `+` en espace dans une requête).
fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = s
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> BTreeMap<String, String> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k, true), percent_decode(v, true)),
            None => (percent_decode(p, true), String::new()),
        })
        .collect()
}

fn number<T: std::str::FromStr>(
    q: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<T>, Reply> {
    match q.get(key) {
        None => Ok(None),
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|_| Reply::error(400, format!("paramètre « {key} » invalide : « {v} »"))),
    }
}

/// Décode un corps JSON comme le fait axum : 400 pour un JSON mal formé, 422
/// pour un JSON bien formé mais qui ne correspond pas à ce qui est attendu.
fn body<T: DeserializeOwned>(body: Option<&str>) -> Result<T, Reply> {
    let Some(text) = body.filter(|b| !b.trim().is_empty()) else {
        return Err(Reply::error(400, "corps JSON attendu"));
    };
    serde_json::from_str(text).map_err(|e| {
        let status = if e.is_data() { 422 } else { 400 };
        Reply::error(status, format!("corps JSON invalide : {e}"))
    })
}

impl Service {
    /// Traite une requête de l'API HTTP (`/api/...`) : mêmes chemins, mêmes
    /// corps JSON, mêmes réponses et mêmes codes que `netiv serve`.
    ///
    /// `uri` peut contenir une requête (`?since=10&limit=50`) ; `body` est le
    /// corps JSON des `POST` et `PUT`.
    pub fn request(&mut self, method: &str, uri: &str, body_text: Option<&str>) -> Reply {
        if body_text.is_some_and(|b| b.len() > MAX_BODY) {
            return Reply::error(413, "requête trop volumineuse");
        }
        match self.route(&method.to_ascii_uppercase(), uri, body_text) {
            Ok(r) | Err(r) => r,
        }
    }

    fn route(&mut self, method: &str, uri: &str, b: Option<&str>) -> Result<Reply, Reply> {
        let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
        let q = parse_query(query);
        let parts: Vec<String> = path
            .split('/')
            .filter(|p| !p.is_empty())
            .map(|p| percent_decode(p, false))
            .collect();
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let fmt = || q.get("format").cloned();

        let r = match (method, parts.as_slice()) {
            ("GET", ["api", "info"]) => reply(Ok(self.info())),
            ("GET", ["api", "library"]) => reply(Ok(self.library())),
            ("GET", ["api", "profile"]) => reply(self.profile()),
            ("GET", ["api", "leaderboard"]) => reply(Ok(self.leaderboard())),
            ("GET", ["api", "labs"]) => reply(Ok(self.list_labs())),
            ("POST", ["api", "labs"]) => reply(self.create_lab(&body(b)?)),
            ("GET", ["api", "labs", id]) => reply(self.get_lab(id)),
            ("DELETE", ["api", "labs", id]) => reply(self.delete_lab(id)),
            ("POST", ["api", "labs", id, "exec"]) => reply(self.exec(id, &body(b)?)),
            ("GET", ["api", "labs", id, "trace"]) => {
                let tq = TraceQuery {
                    since: number(&q, "since")?,
                    limit: number(&q, "limit")?,
                };
                reply(self.trace(id, &tq))
            }
            ("GET", ["api", "labs", id, "transactions"]) => reply(self.transactions(id)),
            ("GET", ["api", "labs", id, "journey", tid]) => {
                let tid: u64 = tid
                    .parse()
                    .map_err(|_| Reply::error(400, format!("transaction invalide : « {tid} »")))?;
                reply(self.journey(id, tid))
            }
            ("GET", ["api", "labs", id, "device", name]) => reply(self.device(id, name)),
            ("GET", ["api", "labs", id, "health"]) => reply(self.health(id)),
            ("GET", ["api", "labs", id, "security"]) => reply(self.security(id)),
            ("GET", ["api", "labs", id, "docs"]) => {
                reply(self.docs(id, &DocsQuery { format: fmt() }))
            }
            ("POST", ["api", "labs", id, "what-if"]) => reply(self.what_if(id, &body(b)?)),
            ("GET", ["api", "labs", id, "topology"]) => {
                reply(self.get_topology(id, &TopologyQuery { format: fmt() }))
            }
            ("PUT", ["api", "labs", id, "topology"]) => reply(self.put_topology(id, &body(b)?)),
            ("POST", ["api", "labs", id, "topology", "validate"]) => {
                reply(self.validate_topology(id, &body(b)?))
            }
            ("POST", ["api", "labs", id, "topology", "autoconfig"]) => reply(self.autoconfig(id)),
            ("PUT", ["api", "labs", id, "positions"]) => reply(self.put_positions(id, &body(b)?)),
            (_, p) if known_path(p) => Reply::error(405, "méthode non autorisée"),
            _ => Reply::error(404, "route inconnue"),
        };
        Ok(r)
    }
}

/// Le chemin existe-t-il (avec une autre méthode) ?
fn known_path(p: &[&str]) -> bool {
    matches!(
        p,
        [
            "api",
            "info" | "library" | "profile" | "leaderboard" | "labs"
        ] | ["api", "labs", _]
            | [
                "api",
                "labs",
                _,
                "exec"
                    | "trace"
                    | "transactions"
                    | "health"
                    | "security"
                    | "docs"
                    | "what-if"
                    | "topology"
                    | "positions"
            ]
            | ["api", "labs", _, "journey" | "device", _]
            | ["api", "labs", _, "topology", "validate" | "autoconfig"]
    )
}
