#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Le routeur sans HTTP de `netiv-service` (utilisé par le moteur
//! WebAssembly) répond exactement comme l'API HTTP : mêmes codes, mêmes JSON.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, header};
use http_body_util::BodyExt;
use netiv_api::{ApiConfig, router};
use netiv_service::{Service, ServiceConfig};
use serde_json::{Value, json};
use tower::ServiceExt;

fn labs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../labs")
}

async fn http(app: &Router, method: &str, uri: &str, body: Option<&Value>) -> (u16, Value) {
    let mut b = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_in_process_router_answers_like_the_http_api() {
    let app = router(ApiConfig {
        labs_dir: labs_dir(),
        ..ApiConfig::default()
    });
    let mut svc = Service::with_default_storage(ServiceConfig {
        labs_dir: labs_dir(),
        ..ServiceConfig::default()
    });

    let script: Vec<(&str, &str, Option<Value>)> = vec![
        ("GET", "/api/info", None),
        ("GET", "/api/library", None),
        ("GET", "/api/profile", None),
        ("GET", "/api/leaderboard", None),
        ("POST", "/api/labs", Some(json!({ "builtin": "office" }))),
        ("POST", "/api/labs", Some(json!({ "lab": "bureau" }))),
        (
            "POST",
            "/api/labs/lab-1/exec",
            Some(json!({ "line": "ping PC1 192.168.1.1 2" })),
        ),
        (
            "POST",
            "/api/labs/lab-1/exec",
            Some(json!({ "line": "ping ZZZ 1.2.3.4" })),
        ),
        ("GET", "/api/labs/lab-1/trace?since=3&limit=20", None),
        ("GET", "/api/labs/lab-1/transactions", None),
        ("GET", "/api/labs/lab-1/journey/1", None),
        ("GET", "/api/labs/lab-1/journey/999", None),
        ("GET", "/api/labs/lab-1/device/PC1", None),
        ("GET", "/api/labs/lab-1/health", None),
        ("GET", "/api/labs/lab-1/security", None),
        ("GET", "/api/labs/lab-1/docs?format=yaml", None),
        ("GET", "/api/labs/lab-1/topology?format=netiv", None),
        ("GET", "/api/labs/lab-1/topology?format=zzz", None),
        (
            "POST",
            "/api/labs/lab-1/what-if",
            Some(json!({ "scenario": "R1 goes down" })),
        ),
        ("POST", "/api/labs/lab-1/topology/autoconfig", None),
        (
            "PUT",
            "/api/labs/lab-1/positions",
            Some(json!({ "positions": { "PC1": [3, 4] } })),
        ),
        ("GET", "/api/labs/lab-1", None),
        (
            "POST",
            "/api/labs",
            Some(json!({ "challenge": "beginner-01" })),
        ),
        ("GET", "/api/labs/lab-3/health", None),
        (
            "POST",
            "/api/labs/lab-3/exec",
            Some(json!({ "line": "hint" })),
        ),
        ("POST", "/api/labs", Some(json!({}))),
        ("POST", "/api/labs", Some(json!({ "lab": "absent" }))),
        ("GET", "/api/labs/lab-99", None),
        ("GET", "/api/labs", None),
        ("DELETE", "/api/labs/lab-2", None),
        ("GET", "/api/labs", None),
    ];
    for (method, uri, body) in &script {
        let (hs, hb) = http(&app, method, uri, body.as_ref()).await;
        let text = body.as_ref().map(ToString::to_string);
        let r = svc.request(method, uri, text.as_deref());
        assert_eq!(r.status, hs, "{method} {uri}");
        // Le temps restant d'un challenge dépend de l'horloge réelle.
        let strip = |mut v: Value| {
            if let Some(c) = v.pointer_mut("/snapshot/challenge/remaining_secs") {
                *c = Value::Null;
            }
            v
        };
        assert_eq!(strip(r.body), strip(hb), "{method} {uri}");
    }

    // Rejets produits par axum avant le service : seuls les codes comptent.
    for (method, uri, body) in [
        ("POST", "/api/labs", json!({ "zzz": 1 })),
        ("POST", "/api/labs/lab-1/exec", json!({ "ligne": "x" })),
    ] {
        let (hs, _) = http(&app, method, uri, Some(&body)).await;
        let r = svc.request(method, uri, Some(&body.to_string()));
        assert_eq!(r.status, hs, "{method} {uri}");
    }
}
