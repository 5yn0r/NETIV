//! API HTTP **locale** de NETIV et interface web.
//!
//! Chaque « lab » est une [`Session`] de la CLI : tout ce qu'on sait faire en
//! ligne de commande se fait aussi par `POST /api/labs/{id}/exec`, ce qui
//! garantit que la CLI et le navigateur se comportent à l'identique.
//!
//! Sécurité (le serveur écoute en local et pilote un simulateur, mais il reste
//! une porte ouverte sur la machine de l'utilisateur) :
//! * liaison à `127.0.0.1` par défaut ;
//! * l'en-tête `Host` est vérifié (protection contre le « DNS rebinding ») ;
//! * les requêtes `POST` exigent `Content-Type: application/json` et aucune
//!   en-tête CORS n'est émise : une page tierce ne peut pas les envoyer ;
//! * les sessions sont en **bac à sable** : aucune lecture ni écriture de
//!   fichier de l'utilisateur ;
//! * taille des requêtes, des lignes de commande et nombre de labs bornés.

mod snapshot;
mod web;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use netiv_analysis::{DocFormat, generate_docs, health, what_if};
use netiv_cli::Session;
use netiv_core::TraceId;
use netiv_education::{builtin, learn};
use netiv_security::analyze;
use serde::Deserialize;
use serde_json::{Value, json};

/// Nombre maximal de labs ouverts en même temps.
pub const MAX_LABS: usize = 16;
/// Longueur maximale d'une ligne de commande.
pub const MAX_LINE: usize = 1_000;
/// Taille maximale d'une requête.
pub const MAX_BODY: usize = 1 << 20;

/// Réglages du serveur.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Nom du profil d'élève.
    pub profile_name: String,
    /// Dossier des profils (progression sauvegardée) ; `None` : en mémoire.
    pub profile_dir: Option<PathBuf>,
    /// Dossier de la bibliothèque de labs.
    pub labs_dir: PathBuf,
    /// Valeurs acceptées pour l'en-tête `Host` ; vide : pas de contrôle.
    pub allowed_hosts: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        ApiConfig {
            profile_name: "eleve".to_string(),
            profile_dir: None,
            labs_dir: PathBuf::from("labs"),
            allowed_hosts: Vec::new(),
        }
    }
}

impl ApiConfig {
    /// Hôtes à accepter quand on écoute sur `addr`.
    pub fn allowed_hosts_for(addr: SocketAddr) -> Vec<String> {
        let port = addr.port();
        vec![
            addr.to_string(),
            format!("localhost:{port}"),
            format!("127.0.0.1:{port}"),
            format!("[::1]:{port}"),
        ]
    }
}

struct Lab {
    session: Session,
}

struct Inner {
    cfg: ApiConfig,
    labs: Mutex<BTreeMap<String, Lab>>,
    next: AtomicU64,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

impl AppState {
    fn labs(&self) -> MutexGuard<'_, BTreeMap<String, Lab>> {
        // Un panic dans un handler ne doit pas rendre le serveur inutilisable.
        self.0
            .labs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn profile_path(&self) -> Option<PathBuf> {
        let dir = self.0.cfg.profile_dir.as_ref()?;
        Some(dir.join(format!("{}.json", safe_name(&self.0.cfg.profile_name))))
    }

    fn new_session(&self) -> Result<Session, ApiError> {
        let session = match self.profile_path() {
            Some(path) => Session::with_profile(&self.0.cfg.profile_name, path)
                .map_err(|e| ApiError::bad(e.to_string()))?,
            None => Session::new(),
        };
        let mut session = session.sandboxed();
        session.set_labs_dir(self.0.cfg.labs_dir.clone());
        Ok(session)
    }
}

/// Nom de fichier sûr : lettres, chiffres, `-` et `_`.
fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "eleve".to_string() } else { s }
}

// ---------------------------------------------------------------------------
// Erreurs
// ---------------------------------------------------------------------------

struct ApiError(StatusCode, String);

impl ApiError {
    fn bad(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::BAD_REQUEST, msg.into())
    }

    fn not_found(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::NOT_FOUND, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

// ---------------------------------------------------------------------------
// Middlewares
// ---------------------------------------------------------------------------

/// Refuse les requêtes dont l'en-tête `Host` n'est pas l'un des nôtres.
async fn host_guard(State(st): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let allowed = &st.0.cfg.allowed_hosts;
    if !allowed.is_empty() {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .map(str::to_ascii_lowercase);
        if !host.is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(&h))) {
            return ApiError(StatusCode::FORBIDDEN, "hôte non autorisé".to_string())
                .into_response();
        }
    }
    next.run(req).await
}

/// En-têtes de sécurité sur toutes les réponses.
async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'",
        ),
    );
    if is_api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

// ---------------------------------------------------------------------------
// Routeur
// ---------------------------------------------------------------------------

/// Construit le routeur de l'API et de l'interface web.
pub fn router(cfg: ApiConfig) -> Router {
    let state = AppState(Arc::new(Inner {
        cfg,
        labs: Mutex::new(BTreeMap::new()),
        next: AtomicU64::new(1),
    }));
    Router::new()
        .route("/", get(web::index))
        .route("/app.js", get(web::app_js))
        .route("/style.css", get(web::style_css))
        .route("/favicon.svg", get(web::favicon))
        .route("/api/info", get(info))
        .route("/api/library", get(library))
        .route("/api/profile", get(profile))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/labs", get(list_labs).post(create_lab))
        .route("/api/labs/{id}", get(get_lab).delete(delete_lab))
        .route("/api/labs/{id}/exec", post(exec))
        .route("/api/labs/{id}/trace", get(trace))
        .route("/api/labs/{id}/transactions", get(transactions))
        .route("/api/labs/{id}/journey/{tid}", get(journey))
        .route("/api/labs/{id}/device/{name}", get(device))
        .route("/api/labs/{id}/health", get(lab_health))
        .route("/api/labs/{id}/security", get(lab_security))
        .route("/api/labs/{id}/docs", get(lab_docs))
        .route("/api/labs/{id}/what-if", post(lab_what_if))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(state.clone(), host_guard))
        .with_state(state)
}

/// Lance le serveur sur `addr` jusqu'à Ctrl-C.
pub async fn serve(addr: SocketAddr, mut cfg: ApiConfig) -> std::io::Result<()> {
    if cfg.allowed_hosts.is_empty() {
        cfg.allowed_hosts = ApiConfig::allowed_hosts_for(addr);
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(cfg))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn info(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "name": "NETIV",
        "version": env!("CARGO_PKG_VERSION"),
        "profile": st.0.cfg.profile_name,
        "persistent": st.profile_path().is_some(),
        "sandbox": true,
        "max_labs": MAX_LABS,
    }))
}

/// Fichiers de la bibliothèque : `(nom, titre, auteur, genre)`.
fn lab_files(dir: &FsPath) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows: Vec<Value> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?;
            if !["netiv", "yaml", "yml"].contains(&ext) {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let text = std::fs::read_to_string(&path).ok()?;
            let (title, author, kind) = match netiv_topology::Bundle::from_yaml(&text) {
                Ok(b) => (
                    b.metadata.title,
                    b.metadata.author,
                    if b.challenge.is_some() {
                        "challenge"
                    } else {
                        "network"
                    },
                ),
                Err(_) => (name.clone(), String::new(), "network"),
            };
            Some(json!({ "name": name, "title": title, "author": author, "kind": kind }))
        })
        .collect();
    rows.sort_by_key(|v| v["name"].as_str().unwrap_or_default().to_string());
    rows
}

async fn library(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "topologies": builtin::TOPOLOGY_NAMES,
        "challenges": builtin::challenges().iter().map(|c| json!({
            "id": c.id, "title": c.title, "topic": c.topic,
            "difficulty": c.difficulty.slug(), "minutes": c.time_limit_minutes,
        })).collect::<Vec<_>>(),
        "ranges": builtin::ranges().iter().map(|r| json!({
            "id": r.id, "title": r.title, "difficulty": r.difficulty.slug(), "minutes": r.time_limit_minutes,
        })).collect::<Vec<_>>(),
        "paths": learn::paths().iter().map(|p| json!({
            "id": p.id, "title": p.title, "lessons": p.lessons.len(),
        })).collect::<Vec<_>>(),
        "labs": lab_files(&st.0.cfg.labs_dir),
        "causes": netiv_diagnostics::CauseKind::ALL.iter().map(|k| json!({
            "slug": k.slug(), "label": k.label(),
        })).collect::<Vec<_>>(),
        "behaviors": ["port-scan", "host-sweep", "lateral-movement", "beaconing", "arp-spoofing", "repeated-denials"],
        "generator": {
            "types": netiv_topology::generate::TopoKind::ALL.map(netiv_topology::generate::TopoKind::slug),
            "topics": netiv_education::generate::TOPICS,
        },
    }))
}

async fn profile(State(st): State<AppState>) -> ApiResult {
    let session = st.new_session()?;
    let p = session.profile();
    let xp = p.xp();
    let (level, level_name, next) = netiv_education::profile::level_for(xp);
    Ok(Json(json!({
        "name": p.name,
        "xp": xp,
        "level": level,
        "level_name": level_name,
        "next_level_xp": next,
        "completed": p.completed_count(),
        "completed_ids": p.completed.iter().map(|(id, c)| json!({ "id": id, "score": c.score })).collect::<Vec<_>>(),
        "badges": netiv_education::BADGES.iter().map(|b| json!({
            "id": b.id, "name": b.name, "how": b.how, "earned": p.badges.contains(b.id),
        })).collect::<Vec<_>>(),
        "dashboard": p.dashboard(&builtin::catalog()),
    })))
}

async fn leaderboard(State(st): State<AppState>) -> ApiResult {
    let Some(dir) = st.0.cfg.profile_dir.clone() else {
        return Ok(Json(json!({ "entries": [], "skipped": 0 })));
    };
    let (board, skipped) = match netiv_education::profile::leaderboard(&dir) {
        Ok(b) => b,
        Err(_) => return Ok(Json(json!({ "entries": [], "skipped": 0 }))),
    };
    Ok(Json(json!({
        "entries": board.iter().enumerate().map(|(i, e)| json!({
            "rank": i + 1, "name": e.name, "xp": e.xp, "level": e.level,
            "level_name": e.level_name, "completed": e.completed, "badges": e.badges,
        })).collect::<Vec<_>>(),
        "skipped": skipped,
    })))
}

async fn list_labs(State(st): State<AppState>) -> Json<Value> {
    let labs = st.labs();
    Json(json!({
        "labs": labs.iter().map(|(id, l)| json!({
            "id": id,
            "name": l.session.network_name(),
            "challenge": l.session.challenge_info().map(|c| c.id),
        })).collect::<Vec<_>>(),
    }))
}

/// Corps de `POST /api/labs` : exactement une source.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLab {
    builtin: Option<String>,
    yaml: Option<String>,
    generate: Option<GenerateTopology>,
    challenge: Option<String>,
    range: Option<String>,
    lab: Option<String>,
    generate_challenge: Option<GenerateChallenge>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateTopology {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default = "default_routers")]
    routers: usize,
    #[serde(default = "default_hosts")]
    hosts: usize,
    #[serde(default = "default_seed")]
    seed: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateChallenge {
    topic: String,
    #[serde(default = "default_difficulty")]
    difficulty: String,
    #[serde(default = "default_seed")]
    seed: u64,
}

fn default_routers() -> usize {
    4
}
fn default_hosts() -> usize {
    10
}
fn default_seed() -> u64 {
    1
}
fn default_difficulty() -> String {
    "medium".to_string()
}

/// Mots sûrs pour être recopiés dans une commande (jamais de séparateur).
fn token(s: &str, what: &str) -> Result<String, ApiError> {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        Ok(s.to_string())
    } else {
        Err(ApiError::bad(format!("{what} invalide : « {s} »")))
    }
}

async fn create_lab(State(st): State<AppState>, Json(body): Json<CreateLab>) -> ApiResult {
    let sources = [
        body.builtin.is_some(),
        body.yaml.is_some(),
        body.generate.is_some(),
        body.challenge.is_some(),
        body.range.is_some(),
        body.lab.is_some(),
        body.generate_challenge.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if sources != 1 {
        return Err(ApiError::bad(
            "indique exactement une source : builtin, yaml, generate, challenge, range, lab ou generate_challenge",
        ));
    }
    if st.labs().len() >= MAX_LABS {
        return Err(ApiError::bad(format!(
            "trop de labs ouverts (maximum {MAX_LABS}) : ferme-en un"
        )));
    }
    let mut session = st.new_session()?;
    let mut out: Vec<u8> = Vec::new();
    let run = |session: &mut Session, line: &str, out: &mut Vec<u8>| -> Result<(), ApiError> {
        session
            .exec(line, out)
            .map(|_| ())
            .map_err(|e| ApiError::bad(e.to_string()))
    };

    if let Some(name) = &body.builtin {
        let text = builtin::topology(name)
            .ok_or_else(|| ApiError::bad(format!("réseau inconnu : « {name} »")))?;
        session
            .load_yaml(text, &mut out)
            .map_err(|e| ApiError::bad(e.to_string()))?;
    } else if let Some(yaml) = &body.yaml {
        session
            .load_yaml(yaml, &mut out)
            .map_err(|e| ApiError::bad(e.to_string()))?;
    } else if let Some(g) = &body.generate {
        let line = format!(
            "topology generate --type {} --routers {} --hosts {} --seed {}",
            token(&g.kind, "type")?,
            g.routers,
            g.hosts,
            g.seed
        );
        run(&mut session, &line, &mut out)?;
    } else if let Some(id) = &body.challenge {
        run(
            &mut session,
            &format!("challenge start {}", token(id, "challenge")?),
            &mut out,
        )?;
    } else if let Some(id) = &body.range {
        run(
            &mut session,
            &format!("range start {}", token(id, "scénario")?),
            &mut out,
        )?;
    } else if let Some(name) = &body.lab {
        let name = token(name, "lab")?;
        let dir = &st.0.cfg.labs_dir;
        let path = ["netiv", "yaml", "yml"]
            .iter()
            .map(|e| dir.join(format!("{name}.{e}")))
            .find(|p| p.is_file())
            .ok_or_else(|| ApiError::not_found(format!("lab inconnu : « {name} »")))?;
        let text = std::fs::read_to_string(&path).map_err(|e| ApiError::bad(e.to_string()))?;
        session
            .load_yaml(&text, &mut out)
            .map_err(|e| ApiError::bad(e.to_string()))?;
        if text.contains("challenge:") && text.contains("format_version") {
            // Un lab qui porte un challenge : on le démarre.
            if session.start_challenge_text(&text, &mut out).is_err() {
                // Un lab de réseau pur n'a pas de challenge : on garde le réseau.
            }
        }
    } else if let Some(g) = &body.generate_challenge {
        let line = format!(
            "challenge generate --topic {} --difficulty {} --seed {}",
            token(&g.topic, "thème")?,
            token(&g.difficulty, "difficulté")?,
            g.seed
        );
        run(&mut session, &line, &mut out)?;
    }

    let id = format!("lab-{}", st.0.next.fetch_add(1, Ordering::SeqCst));
    let snap = snapshot::snapshot(&id, &session);
    st.labs().insert(id.clone(), Lab { session });
    Ok(Json(json!({
        "id": id,
        "output": String::from_utf8_lossy(&out),
        "snapshot": snap,
    })))
}

/// Exécute `f` sur le lab `id`.
fn with_lab<T>(
    st: &AppState,
    id: &str,
    f: impl FnOnce(&mut Session) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let mut labs = st.labs();
    let lab = labs
        .get_mut(id)
        .ok_or_else(|| ApiError::not_found(format!("lab inconnu : « {id} »")))?;
    f(&mut lab.session)
}

async fn get_lab(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    with_lab(&st, &id, |s| Ok(Json(snapshot::snapshot(&id, s))))
}

async fn delete_lab(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    st.labs()
        .remove(&id)
        .map(|_| Json(json!({ "deleted": id })))
        .ok_or_else(|| ApiError::not_found(format!("lab inconnu : « {id} »")))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecBody {
    line: String,
}

async fn exec(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExecBody>,
) -> ApiResult {
    if body.line.len() > MAX_LINE {
        return Err(ApiError::bad(format!(
            "ligne trop longue (maximum {MAX_LINE} caractères)"
        )));
    }
    with_lab(&st, &id, |session| {
        // Plusieurs labs peuvent partager le même profil : on relit avant d'agir.
        session.reload_profile();
        let before = session.simulation().map_or(0, |s| s.trace().len());
        let mut out: Vec<u8> = Vec::new();
        let result = session.exec(&body.line, &mut out);
        let quit = matches!(result, Ok(netiv_cli::Flow::Quit));
        Ok(Json(json!({
            "output": String::from_utf8_lossy(&out),
            "error": result.as_ref().err().map(ToString::to_string),
            "quit": quit,
            "new_events_from": before,
            "snapshot": snapshot::snapshot(&id, session),
        })))
    })
}

#[derive(Debug, Deserialize)]
struct TraceQuery {
    since: Option<usize>,
    limit: Option<usize>,
}

async fn trace(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TraceQuery>,
) -> ApiResult {
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        Ok(Json(snapshot::trace_json(
            sim,
            q.since.unwrap_or(0),
            q.limit.unwrap_or(500).min(2_000),
        )))
    })
}

async fn transactions(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        Ok(Json(snapshot::transactions(sim, 50)))
    })
}

async fn journey(State(st): State<AppState>, Path((id, tid)): Path<(String, u64)>) -> ApiResult {
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        snapshot::journey_json(sim, TraceId(tid))
            .map(Json)
            .ok_or_else(|| ApiError::not_found(format!("aucune transaction #{tid}")))
    })
}

async fn device(State(st): State<AppState>, Path((id, name)): Path<(String, String)>) -> ApiResult {
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        snapshot::device_json(sim, &name)
            .map(Json)
            .ok_or_else(|| ApiError::not_found(format!("équipement inconnu : « {name} »")))
    })
}

/// Les analyses qui trahiraient la panne d'un challenge sont verrouillées.
fn not_during_challenge(s: &Session) -> Result<(), ApiError> {
    if s.challenge_info().is_some_and(|c| !c.finished) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "indisponible pendant un challenge : c'est à toi de trouver".to_string(),
        ));
    }
    Ok(())
}

async fn lab_health(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    with_lab(&st, &id, |s| {
        not_during_challenge(s)?;
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        let h = health(sim);
        Ok(Json(json!({
            "global": h.global(),
            "categories": h.categories.iter().map(|c| json!({
                "name": c.name, "score": c.score, "weight": c.weight, "notes": c.notes,
            })).collect::<Vec<_>>(),
            "text": h.render(),
        })))
    })
}

async fn lab_security(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult {
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        let r = analyze(sim);
        let (high, medium, low) = r.counts();
        Ok(Json(json!({
            "score": r.score(),
            "counts": { "high": high, "medium": medium, "low": low },
            "findings": r.findings.iter().map(|f| json!({
                "id": f.id, "severity": f.severity.label(), "title": f.title,
                "devices": f.devices, "detail": f.detail, "recommendation": f.recommendation,
            })).collect::<Vec<_>>(),
            "text": r.render(),
        })))
    })
}

#[derive(Debug, Deserialize)]
struct DocsQuery {
    format: Option<String>,
}

async fn lab_docs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DocsQuery>,
) -> ApiResult {
    with_lab(&st, &id, |s| {
        let format: DocFormat = q
            .format
            .as_deref()
            .unwrap_or("md")
            .parse()
            .map_err(ApiError::bad)?;
        let name = s.network_name().unwrap_or("reseau").to_string();
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        let text = generate_docs(sim, &name, format).map_err(ApiError::bad)?;
        Ok(Json(
            json!({ "format": q.format.unwrap_or_else(|| "md".into()), "text": text }),
        ))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WhatIfBody {
    scenario: String,
}

async fn lab_what_if(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<WhatIfBody>,
) -> ApiResult {
    if body.scenario.len() > MAX_LINE {
        return Err(ApiError::bad("scénario trop long"));
    }
    with_lab(&st, &id, |s| {
        let sim = s
            .simulation()
            .ok_or_else(|| ApiError::bad("aucun réseau chargé"))?;
        let i = what_if(sim, &body.scenario).map_err(ApiError::bad)?;
        Ok(Json(json!({
            "scenario": i.scenario,
            "flows": i.flows,
            "ok_before": i.ok_before,
            "ok_after": i.ok_after,
            "broken": i.broken,
            "affected_hosts": i.affected_hosts,
            "affected_services": i.affected_services,
            "isolated_hosts": i.isolated_hosts,
            "broken_routes": i.broken_routes,
            "isolated_segments": i.isolated_segments(),
            "harmless": i.is_harmless(),
            "text": i.render(),
        })))
    })
}
