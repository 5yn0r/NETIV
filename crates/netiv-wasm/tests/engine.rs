#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Le moteur du navigateur, compilé en natif : mêmes sorties que la CLI.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use netiv_wasm::{Engine, Options};
use serde_json::{Value, json};

/// Les commandes du test golden de la CLI (crates/netiv-cli/tests/session.rs).
const GOLDEN_SCRIPT: [&str; 7] = [
    "ping PC1 192.168.2.10 2",
    "trace 1",
    "inspect 19",
    "explain 25",
    "show route R1",
    "show arp PC1",
    "show mac SW1",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn engine_at(now: Arc<AtomicU64>) -> Engine {
    Engine::new(
        Options::default(),
        Arc::new(move || now.load(Ordering::SeqCst)),
    )
    .unwrap()
}

fn exec(e: &mut Engine, id: &str, line: &str) -> Value {
    let body = json!({ "line": line }).to_string();
    e.request("POST", &format!("/api/labs/{id}/exec"), Some(&body))
        .unwrap()
}

#[test]
fn the_golden_scenario_gives_the_same_output_as_the_cli() {
    let mvp = std::fs::read_to_string(root().join("examples/mvp.yaml")).unwrap();
    let golden =
        std::fs::read_to_string(root().join("crates/netiv-cli/tests/golden/mvp_session.txt"))
            .unwrap();
    let mut e = engine_at(Arc::new(AtomicU64::new(0)));
    let body = json!({ "yaml": mvp }).to_string();
    let created = e.request("POST", "/api/labs", Some(&body)).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let mut out = String::new();
    for line in GOLDEN_SCRIPT {
        let r = exec(&mut e, &id, line);
        assert!(r["error"].is_null(), "{line} : {r}");
        out.push_str(&format!(
            "netiv> {line}\n{}\n",
            r["output"].as_str().unwrap()
        ));
    }
    assert_eq!(out, golden);
}

#[test]
fn a_beginner_challenge_is_won_and_saved_in_the_exported_profile() {
    let now = Arc::new(AtomicU64::new(10_000));
    let mut e = engine_at(now.clone());
    let created = e
        .request("POST", "/api/labs", Some(r#"{"challenge":"beginner-01"}"#))
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    exec(&mut e, &id, "gateway PC1 192.168.1.1");
    exec(&mut e, &id, "challenge check");
    now.store(10_120, Ordering::SeqCst);
    let r = exec(&mut e, &id, "answer PC1 wrong-gateway");
    let detail = &r["snapshot"]["challenge"]["score_detail"];
    assert_eq!(detail["elapsed_secs"], 120, "{r}");
    assert!(detail["total"].as_u64().unwrap() >= 80, "{detail}");

    let profile = e.export_profile().unwrap();
    assert!(profile.contains("beginner-01"), "{profile}");
    assert_eq!(e.files(), ["profils/eleve.json"]);

    // Un nouveau moteur repart du profil sauvegardé ailleurs.
    let options: Options =
        serde_json::from_value(json!({ "profileName": "eleve", "profile": profile })).unwrap();
    let mut again = Engine::new(options, Arc::new(|| 0)).unwrap();
    let p = again.request("GET", "/api/profile", None).unwrap();
    assert_eq!(p["completed"], 1);
}

#[test]
fn errors_come_back_as_status_and_message() {
    let mut e = engine_at(Arc::new(AtomicU64::new(0)));
    let err = e.request("GET", "/api/labs/lab-7", None).unwrap_err();
    assert_eq!(err.status, 404);
    assert!(err.error.contains("lab inconnu"), "{err:?}");
    let err = e
        .request("POST", "/api/labs", Some(r#"{"builtin":"nope"}"#))
        .unwrap_err();
    assert_eq!(err.status, 400);
    // Les commandes qui touchent au disque expliquent pourquoi elles sont indisponibles.
    let created = e
        .request("POST", "/api/labs", Some(r#"{"builtin":"office"}"#))
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let r = exec(&mut e, &id, "load /etc/passwd");
    assert!(r["error"].as_str().unwrap().contains("indisponible"), "{r}");
    assert!(Engine::from_json(Some("{ cassé"), Arc::new(|| 0)).is_err());
    assert!(Engine::from_json(Some(r#"{"inconnu":1}"#), Arc::new(|| 0)).is_err());
}

#[test]
fn labs_travel_between_engines_and_the_library_can_be_provided() {
    let mut a = engine_at(Arc::new(AtomicU64::new(0)));
    let created = a
        .request("POST", "/api/labs", Some(r#"{"builtin":"dmz"}"#))
        .unwrap();
    let text = a.export_lab(created["id"].as_str().unwrap()).unwrap();

    let options: Options = serde_json::from_value(json!({ "labs": { "partage": text } })).unwrap();
    let mut b = Engine::new(options, Arc::new(|| 0)).unwrap();
    let lib = b.request("GET", "/api/library", None).unwrap();
    assert_eq!(lib["labs"][0]["name"], "partage");
    let opened = b
        .request("POST", "/api/labs", Some(r#"{"lab":"partage"}"#))
        .unwrap();
    assert_eq!(
        opened["snapshot"]["devices"],
        created["snapshot"]["devices"]
    );
    let imported = b.import_lab(&text).unwrap();
    assert_eq!(imported["id"], "lab-2");
    let bad: Result<Options, _> = serde_json::from_value(json!({ "labs": { "../x": "y" } }));
    assert!(Engine::new(bad.unwrap(), Arc::new(|| 0)).is_err());
}
