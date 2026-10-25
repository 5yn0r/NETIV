//! API HTTP **locale** de NETIV et interface web.
//!
//! Une fine couche HTTP au-dessus de [`netiv_service`] : chaque route extrait
//! ses paramètres, appelle l'opération du service et traduit le résultat en
//! JSON et en code HTTP. La logique (labs, limites, bac à sable) est dans
//! `netiv-service`, partagée avec le moteur WebAssembly.
//!
//! Chaque « lab » est une `Session` de la CLI : tout ce
//! qu'on sait faire en ligne de commande se fait aussi par
//! `POST /api/labs/{id}/exec`, ce qui garantit que la CLI et le navigateur se
//! comportent à l'identique.
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

mod web;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use netiv_service::{
    CreateLab, DocsQuery, ExecBody, PositionsBody, Service, ServiceConfig, ServiceError,
    TopologyBody, TopologyQuery, TraceQuery, WhatIfBody,
};
use serde::Serialize;
use serde_json::json;

pub use netiv_service::{MAX_BODY, MAX_DEVICES, MAX_LABS, MAX_LINE, MAX_LINKS, MAX_NAME};

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
        let s = ServiceConfig::default();
        ApiConfig {
            profile_name: s.profile_name,
            profile_dir: s.profile_dir,
            labs_dir: s.labs_dir,
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

struct Inner {
    allowed_hosts: Vec<String>,
    service: Mutex<Service>,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

impl AppState {
    fn service(&self) -> MutexGuard<'_, Service> {
        // Un panic dans un handler ne doit pas rendre le serveur inutilisable.
        self.0
            .service
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// ---------------------------------------------------------------------------
// Erreurs
// ---------------------------------------------------------------------------

struct ApiError(StatusCode, String);

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        let status = StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST);
        ApiError(status, e.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// Traduit le résultat d'une opération du service.
fn respond<T: Serialize>(r: Result<T, ServiceError>) -> ApiResult<T> {
    r.map(Json).map_err(ApiError::from)
}

// ---------------------------------------------------------------------------
// Middlewares
// ---------------------------------------------------------------------------

/// Refuse les requêtes dont l'en-tête `Host` n'est pas l'un des nôtres.
async fn host_guard(State(st): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let allowed = &st.0.allowed_hosts;
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
    let service = Service::with_default_storage(ServiceConfig {
        profile_name: cfg.profile_name,
        profile_dir: cfg.profile_dir,
        labs_dir: cfg.labs_dir,
    });
    let state = AppState(Arc::new(Inner {
        allowed_hosts: cfg.allowed_hosts,
        service: Mutex::new(service),
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
        .route(
            "/api/labs/{id}/topology",
            get(get_topology).put(put_topology),
        )
        .route("/api/labs/{id}/topology/validate", post(validate_topology))
        .route("/api/labs/{id}/topology/autoconfig", post(autoconfig))
        .route("/api/labs/{id}/positions", put(put_positions))
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
// Handlers : extraction des paramètres → service → JSON
// ---------------------------------------------------------------------------

async fn info(State(st): State<AppState>) -> Json<netiv_service::Info> {
    Json(st.service().info())
}

async fn library(State(st): State<AppState>) -> Json<netiv_service::Library> {
    Json(st.service().library())
}

async fn profile(State(st): State<AppState>) -> ApiResult<netiv_service::ProfileView> {
    respond(st.service().profile())
}

async fn leaderboard(State(st): State<AppState>) -> Json<netiv_service::Leaderboard> {
    Json(st.service().leaderboard())
}

async fn list_labs(State(st): State<AppState>) -> Json<netiv_service::LabList> {
    Json(st.service().list_labs())
}

async fn create_lab(
    State(st): State<AppState>,
    Json(body): Json<CreateLab>,
) -> ApiResult<netiv_service::Created> {
    respond(st.service().create_lab(&body))
}

async fn get_lab(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<netiv_service::Snapshot> {
    respond(st.service().get_lab(&id))
}

async fn delete_lab(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<netiv_service::Deleted> {
    respond(st.service().delete_lab(&id))
}

async fn exec(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExecBody>,
) -> ApiResult<netiv_service::ExecResult> {
    respond(st.service().exec(&id, &body))
}

async fn trace(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TraceQuery>,
) -> ApiResult<serde_json::Value> {
    respond(st.service().trace(&id, &q))
}

async fn transactions(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    respond(st.service().transactions(&id))
}

async fn journey(
    State(st): State<AppState>,
    Path((id, tid)): Path<(String, u64)>,
) -> ApiResult<serde_json::Value> {
    respond(st.service().journey(&id, tid))
}

async fn device(
    State(st): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    respond(st.service().device(&id, &name))
}

async fn lab_health(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<netiv_service::Health> {
    respond(st.service().health(&id))
}

async fn lab_security(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<netiv_service::Security> {
    respond(st.service().security(&id))
}

async fn lab_docs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DocsQuery>,
) -> ApiResult<netiv_service::FormattedText> {
    respond(st.service().docs(&id, &q))
}

async fn lab_what_if(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<WhatIfBody>,
) -> ApiResult<netiv_service::WhatIf> {
    respond(st.service().what_if(&id, &body))
}

/// La topologie du lab : JSON (éditeur), ou texte YAML / `.netiv` à télécharger.
async fn get_topology(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TopologyQuery>,
) -> ApiResult<netiv_service::TopologyView> {
    respond(st.service().get_topology(&id, &q))
}

/// Remplace le réseau du lab (reconstruction : la trace repart de zéro).
async fn put_topology(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TopologyBody>,
) -> ApiResult<netiv_service::Applied> {
    respond(st.service().put_topology(&id, &body))
}

/// Vérifie une topologie sans rien modifier.
async fn validate_topology(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TopologyBody>,
) -> ApiResult<netiv_service::Validation> {
    respond(st.service().validate_topology(&id, &body))
}

/// Complète l'adressage (IP, passerelles, routes).
async fn autoconfig(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<netiv_service::Autoconfigured> {
    respond(st.service().autoconfig(&id))
}

/// Mémorise la position des équipements sur le schéma (sans reconstruire).
async fn put_positions(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PositionsBody>,
) -> ApiResult<netiv_service::Saved> {
    respond(st.service().put_positions(&id, &body))
}
