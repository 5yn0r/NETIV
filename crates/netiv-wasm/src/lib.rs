//! NETIV dans le navigateur.
//!
//! [`NetivEngine`] (classe JavaScript) contient un [`netiv_service::Service`]
//! entièrement en mémoire : aucun serveur, aucun fichier. Sa méthode
//! `request(method, path, body)` suit le même contrat que l'API HTTP de
//! `netiv serve` (mêmes chemins, mêmes JSON, mêmes codes d'erreur) : l'interface
//! web existante fonctionne sans réécriture.
//!
//! La logique est dans [`Engine`], du Rust ordinaire testable en natif ; la
//! classe JavaScript n'est qu'une traduction des types.

use std::path::PathBuf;
use std::sync::Arc;

use netiv_cli::{MemoryStorage, Storage};
use netiv_service::{Reply, Service, ServiceConfig};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

/// Dossier (virtuel) des profils.
const PROFILE_DIR: &str = "profils";
/// Dossier (virtuel) de la bibliothèque de labs.
const LABS_DIR: &str = "labs";

/// Options de création du moteur (JSON, toutes facultatives).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Options {
    /// Nom de l'élève (« eleve » par défaut).
    pub profile_name: Option<String>,
    /// Profil sauvegardé ailleurs (texte JSON de `exportProfile`).
    pub profile: Option<String>,
    /// Bibliothèque de labs : nom → texte `.netiv` (ou topologie YAML).
    #[serde(default)]
    pub labs: std::collections::BTreeMap<String, String>,
}

/// Erreur rendue à JavaScript : `{ status, error }`, comme une réponse
/// d'erreur de l'API HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    /// Code HTTP équivalent.
    pub status: u16,
    /// Message.
    pub error: String,
}

impl EngineError {
    fn bad(msg: impl Into<String>) -> Self {
        EngineError {
            status: 400,
            error: msg.into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({ "status": self.status, "error": self.error })
    }
}

impl From<netiv_service::ServiceError> for EngineError {
    fn from(e: netiv_service::ServiceError) -> Self {
        EngineError {
            status: e.status(),
            error: e.message,
        }
    }
}

/// Le moteur, en Rust ordinaire.
pub struct Engine {
    service: Service,
    store: MemoryStorage,
}

impl Engine {
    /// Crée le moteur. `clock` donne l'heure en secondes (en WebAssembly :
    /// `Date.now()`), utilisée pour chronométrer les challenges.
    pub fn new(
        options: Options,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, EngineError> {
        let store = MemoryStorage::new();
        for (name, text) in &options.labs {
            let name = netiv_service::token(name, "nom de lab")?;
            store
                .write(&PathBuf::from(LABS_DIR).join(format!("{name}.netiv")), text)
                .map_err(|e| EngineError::bad(e.to_string()))?;
        }
        let mut service = Service::new(
            ServiceConfig {
                profile_name: options.profile_name.unwrap_or_else(|| "eleve".to_string()),
                profile_dir: Some(PathBuf::from(PROFILE_DIR)),
                labs_dir: PathBuf::from(LABS_DIR),
            },
            Box::new(store.clone()),
        );
        service.set_clock(clock);
        if let Some(profile) = &options.profile {
            service.import_profile(profile)?;
        }
        Ok(Engine { service, store })
    }

    /// Crée le moteur à partir d'options en JSON (`None` ou vide : défauts).
    pub fn from_json(
        options: Option<&str>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, EngineError> {
        let options = match options.map(str::trim).filter(|s| !s.is_empty()) {
            Some(text) => serde_json::from_str(text)
                .map_err(|e| EngineError::bad(format!("options invalides : {e}")))?,
            None => Options::default(),
        };
        Self::new(options, clock)
    }

    /// Requête de l'API (voir [`Service::request`]) : le corps de la réponse
    /// si elle réussit, l'erreur `{ status, error }` sinon.
    pub fn request(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<Value, EngineError> {
        let Reply { status, body } = self.service.request(method, path, body);
        if status == 200 {
            return Ok(body);
        }
        Err(EngineError {
            status,
            error: body["error"]
                .as_str()
                .map_or_else(|| format!("erreur {status}"), str::to_string),
        })
    }

    /// Le profil de l'élève (JSON), à sauvegarder ailleurs.
    pub fn export_profile(&self) -> Result<String, EngineError> {
        Ok(self.service.export_profile()?)
    }

    /// Remplace le profil de l'élève (JSON de [`Engine::export_profile`]).
    pub fn import_profile(&mut self, json: &str) -> Result<(), EngineError> {
        Ok(self.service.import_profile(json)?)
    }

    /// Le réseau d'un lab comme fichier `.netiv`.
    pub fn export_lab(&mut self, id: &str) -> Result<String, EngineError> {
        Ok(self.service.export_lab(id)?)
    }

    /// Ouvre un lab à partir du texte d'un fichier `.netiv` ; rend la même
    /// réponse que `POST /api/labs`.
    pub fn import_lab(&mut self, text: &str) -> Result<Value, EngineError> {
        let created = self.service.import_lab(text)?;
        serde_json::to_value(created).map_err(|e| EngineError::bad(e.to_string()))
    }

    /// Fichiers écrits par le moteur dans son stockage en mémoire.
    pub fn files(&self) -> Vec<String> {
        self.store
            .paths()
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }
}

/// Version de NETIV.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// Classe JavaScript
// ---------------------------------------------------------------------------

/// Heure du navigateur, en secondes.
fn browser_seconds() -> u64 {
    // `Date.now()` est en millisecondes, toujours positif.
    (js_sys::Date::now() / 1000.0) as u64
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn js_error(e: &EngineError) -> JsValue {
    to_js(&e.to_json())
}

/// Corps d'une requête venu de JavaScript : rien, un texte JSON ou un objet.
fn body_text(body: &JsValue) -> Result<Option<String>, JsValue> {
    if body.is_undefined() || body.is_null() {
        return Ok(None);
    }
    if let Some(s) = body.as_string() {
        return Ok(Some(s));
    }
    js_sys::JSON::stringify(body)
        .map(|s| Some(String::from(s)))
        .map_err(|_| {
            js_error(&EngineError::bad(
                "corps de requête impossible à convertir en JSON",
            ))
        })
}

/// Le simulateur NETIV, dans le navigateur.
///
/// ```js
/// import init, { NetivEngine } from './pkg/netiv_wasm.js';
/// await init();
/// const engine = new NetivEngine(JSON.stringify({ profileName: 'awa' }));
/// const lab = engine.request('POST', '/api/labs', { builtin: 'office' });
/// ```
#[wasm_bindgen]
pub struct NetivEngine {
    engine: Engine,
}

#[wasm_bindgen]
impl NetivEngine {
    /// Crée le moteur. `options` : texte JSON `{ profileName, profile, labs }`
    /// (tout est facultatif).
    #[wasm_bindgen(constructor)]
    pub fn new(options: Option<String>) -> Result<NetivEngine, JsValue> {
        #[cfg(feature = "dev")]
        console_error_panic_hook::set_once();
        Engine::from_json(options.as_deref(), Arc::new(browser_seconds))
            .map(|engine| NetivEngine { engine })
            .map_err(|e| js_error(&e))
    }

    /// Même contrat que l'API HTTP : rend l'objet JSON de la réponse, ou
    /// lève `{ status, error }`. `body` : objet, texte JSON ou rien.
    pub fn request(&mut self, method: &str, path: &str, body: JsValue) -> Result<JsValue, JsValue> {
        let body = body_text(&body)?;
        self.engine
            .request(method, path, body.as_deref())
            .map(|v| to_js(&v))
            .map_err(|e| js_error(&e))
    }

    /// Le profil de l'élève (texte JSON), à sauvegarder ailleurs.
    #[wasm_bindgen(js_name = exportProfile)]
    pub fn export_profile(&self) -> Result<String, JsValue> {
        self.engine.export_profile().map_err(|e| js_error(&e))
    }

    /// Remplace le profil de l'élève (texte JSON de `exportProfile`).
    #[wasm_bindgen(js_name = importProfile)]
    pub fn import_profile(&mut self, json: &str) -> Result<(), JsValue> {
        self.engine.import_profile(json).map_err(|e| js_error(&e))
    }

    /// Le réseau du lab `id` comme fichier `.netiv` (texte).
    #[wasm_bindgen(js_name = exportLab)]
    pub fn export_lab(&mut self, id: &str) -> Result<String, JsValue> {
        self.engine.export_lab(id).map_err(|e| js_error(&e))
    }

    /// Ouvre un lab à partir d'un texte `.netiv` ; rend `{ id, output, snapshot }`.
    #[wasm_bindgen(js_name = importLab)]
    pub fn import_lab(&mut self, text: &str) -> Result<JsValue, JsValue> {
        self.engine
            .import_lab(text)
            .map(|v| to_js(&v))
            .map_err(|e| js_error(&e))
    }

    /// Version de NETIV.
    pub fn version() -> String {
        version().to_string()
    }

    /// Catalogue des challenges et cyber ranges livrés (sans les solutions).
    pub fn catalog() -> JsValue {
        serde_json::to_value(netiv_service::catalog())
            .map(|v| to_js(&v))
            .unwrap_or(JsValue::NULL)
    }
}
