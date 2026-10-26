#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Le service, sans HTTP : opérations typées et routeur `request`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use netiv_cli::{MemoryStorage, Storage};
use netiv_service::{
    CreateLab, ErrorKind, ExecBody, MAX_LABS, Service, ServiceConfig, TopologyQuery, TopologyView,
};
use serde_json::{Value, json};

/// Service tout en mémoire, comme dans le navigateur.
fn memory_service() -> (Service, MemoryStorage) {
    let store = MemoryStorage::new();
    let svc = Service::new(
        ServiceConfig {
            profile_name: "alice".to_string(),
            profile_dir: Some(PathBuf::from("profils")),
            labs_dir: PathBuf::from("labs"),
        },
        Box::new(store.clone()),
    );
    (svc, store)
}

fn req(svc: &mut Service, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let text = body.map(|b| b.to_string());
    let r = svc.request(method, uri, text.as_deref());
    (r.status, r.body)
}

fn ok(svc: &mut Service, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, body) = req(svc, method, uri, body);
    assert_eq!(status, 200, "{method} {uri} → {body}");
    body
}

fn exec(svc: &mut Service, id: &str, line: &str) -> Value {
    ok(
        svc,
        "POST",
        &format!("/api/labs/{id}/exec"),
        Some(json!({ "line": line })),
    )
}

#[test]
fn typed_operations_open_run_and_observe_a_lab() {
    let (mut svc, _) = memory_service();
    let created = svc
        .create_lab(&CreateLab {
            builtin: Some("office".to_string()),
            ..CreateLab::default()
        })
        .unwrap();
    assert_eq!(created.id, "lab-1");
    assert!(created.output.contains("office"), "{}", created.output);

    let r = svc
        .exec(
            &created.id,
            &ExecBody {
                line: "ping PC1 192.168.1.1".to_string(),
            },
        )
        .unwrap();
    assert!(r.error.is_none(), "{r:?}");
    assert!(r.output.contains("réponse"), "{}", r.output);
    let tx = svc.transactions(&created.id).unwrap();
    let tid = tx["transactions"][0]["id"].as_u64().unwrap();
    assert!(svc.journey(&created.id, tid).unwrap()["text"].is_string());
    assert!(svc.device(&created.id, "PC1").unwrap()["sections"].is_array());
    assert_eq!(
        svc.device(&created.id, "ZZZ").unwrap_err().kind,
        ErrorKind::NotFound
    );

    // Une commande qui échoue n'est pas une erreur du service.
    let r = svc
        .exec(
            &created.id,
            &ExecBody {
                line: "ping ZZZ 1.2.3.4".to_string(),
            },
        )
        .unwrap();
    assert!(r.error.is_some());

    assert_eq!(
        svc.get_lab("lab-99").unwrap_err().status(),
        404,
        "lab inconnu"
    );
    assert_eq!(svc.delete_lab("lab-1").unwrap().deleted, "lab-1");
    assert!(svc.list_labs().labs.is_empty());
}

#[test]
fn the_request_router_mirrors_the_http_api() {
    let (mut svc, _) = memory_service();
    let info = ok(&mut svc, "GET", "/api/info", None);
    assert_eq!(info["name"], "NETIV");
    assert_eq!(info["persistent"], true);
    let lib = ok(&mut svc, "GET", "/api/library", None);
    assert_eq!(lib["challenges"].as_array().unwrap().len(), 28);
    assert_eq!(lib["ranges"].as_array().unwrap().len(), 3);

    let created = ok(
        &mut svc,
        "POST",
        "/api/labs",
        Some(json!({ "builtin": "office" })),
    );
    let id = created["id"].as_str().unwrap().to_string();
    let r = exec(&mut svc, &id, "ping PC1 192.168.1.1");
    let from = r["new_events_from"].as_u64().unwrap();
    let trace = ok(
        &mut svc,
        "GET",
        &format!("/api/labs/{id}/trace?since={from}&limit=5"),
        None,
    );
    assert_eq!(trace["events"].as_array().unwrap().len(), 5);
    assert_eq!(trace["since"], from);

    // Les noms dans le chemin sont décodés comme le ferait un serveur.
    let dev = ok(
        &mut svc,
        "GET",
        &format!("/api/labs/{id}/device/PC%31"),
        None,
    );
    assert_eq!(dev["name"], "PC1");

    let yaml = ok(
        &mut svc,
        "GET",
        &format!("/api/labs/{id}/topology?format=yaml"),
        None,
    );
    assert_eq!(yaml["format"], "yaml");
    let topo = ok(&mut svc, "GET", &format!("/api/labs/{id}/topology"), None);
    assert!(topo["topology"]["devices"].is_array());
    let applied = ok(
        &mut svc,
        "PUT",
        &format!("/api/labs/{id}/topology"),
        Some(json!({ "topology": topo["topology"] })),
    );
    assert!(applied["snapshot"]["devices"].is_array());
    assert!(ok(&mut svc, "GET", &format!("/api/labs/{id}/health"), None)["global"].is_u64());
    assert!(ok(&mut svc, "GET", &format!("/api/labs/{id}/security"), None)["score"].is_u64());
    assert_eq!(
        ok(
            &mut svc,
            "GET",
            &format!("/api/labs/{id}/docs?format=json"),
            None
        )["format"],
        "json"
    );
    let w = ok(
        &mut svc,
        "POST",
        &format!("/api/labs/{id}/what-if"),
        Some(json!({ "scenario": "R1 goes down" })),
    );
    assert!(w["flows"].is_u64());
    assert_eq!(
        ok(
            &mut svc,
            "PUT",
            &format!("/api/labs/{id}/positions"),
            Some(json!({ "positions": { "PC1": [1, 2] } }))
        )["saved"],
        1
    );
    assert_eq!(
        ok(&mut svc, "DELETE", &format!("/api/labs/{id}"), None)["deleted"],
        id.as_str()
    );
}

#[test]
fn the_request_router_returns_the_http_error_codes() {
    let (mut svc, _) = memory_service();
    assert_eq!(req(&mut svc, "GET", "/api/nope", None).0, 404);
    assert_eq!(req(&mut svc, "DELETE", "/api/info", None).0, 405);
    assert_eq!(req(&mut svc, "GET", "/api/labs/lab-9", None).0, 404);
    assert_eq!(
        req(&mut svc, "POST", "/api/labs", Some(json!({}))).0,
        400,
        "aucune source"
    );
    assert_eq!(
        req(&mut svc, "POST", "/api/labs", Some(json!({ "zzz": 1 }))).0,
        422,
        "champ inconnu"
    );
    let r = svc.request("POST", "/api/labs", Some("{ pas du json"));
    assert_eq!(r.status, 400);
    assert!(r.body["error"].is_string());
    assert_eq!(svc.request("POST", "/api/labs", None).status, 400);
    let huge = "x".repeat(netiv_service::MAX_BODY + 1);
    assert_eq!(svc.request("POST", "/api/labs", Some(&huge)).status, 413);
    assert_eq!(
        req(
            &mut svc,
            "POST",
            "/api/labs",
            Some(json!({ "challenge": "beginner-01 ; load /etc/passwd" }))
        )
        .0,
        400
    );

    // Pendant un challenge : analyses interdites (403), édition verrouillée (409).
    let created = ok(
        &mut svc,
        "POST",
        "/api/labs",
        Some(json!({ "challenge": "beginner-01" })),
    );
    let id = created["id"].as_str().unwrap();
    assert_eq!(
        req(&mut svc, "GET", &format!("/api/labs/{id}/health"), None).0,
        403
    );
    let topo = svc.get_topology(id, &TopologyQuery::default()).unwrap();
    let TopologyView::Json { topology } = topo else {
        panic!("JSON attendu");
    };
    assert_eq!(
        req(
            &mut svc,
            "PUT",
            &format!("/api/labs/{id}/topology"),
            Some(json!({ "topology": topology }))
        )
        .0,
        409
    );
    assert_eq!(
        req(
            &mut svc,
            "GET",
            &format!("/api/labs/{id}/journey/abc"),
            None
        )
        .0,
        400
    );
    assert_eq!(
        req(
            &mut svc,
            "GET",
            &format!("/api/labs/{id}/trace?limit=x"),
            None
        )
        .0,
        400
    );
}

#[test]
fn the_number_of_labs_is_bounded() {
    let (mut svc, _) = memory_service();
    for _ in 0..MAX_LABS {
        ok(
            &mut svc,
            "POST",
            "/api/labs",
            Some(json!({ "empty": "vide" })),
        );
    }
    let (status, body) = req(
        &mut svc,
        "POST",
        "/api/labs",
        Some(json!({ "empty": "vide" })),
    );
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("trop de labs"));
    assert!(svc.import_lab("network: { name: x }").is_err());
}

#[test]
fn a_challenge_reports_its_detailed_score_and_saves_the_profile_in_storage() {
    let (mut svc, store) = memory_service();
    let now = Arc::new(AtomicU64::new(1_000));
    let clock = now.clone();
    svc.set_clock(Arc::new(move || clock.load(Ordering::SeqCst)));

    let created = ok(
        &mut svc,
        "POST",
        "/api/labs",
        Some(json!({ "challenge": "beginner-01" })),
    );
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(
        created["snapshot"]["challenge"]["score_detail"],
        Value::Null
    );
    exec(&mut svc, &id, "gateway PC1 192.168.1.1");
    exec(&mut svc, &id, "challenge check");
    now.store(1_060, Ordering::SeqCst);
    let r = exec(&mut svc, &id, "answer PC1 wrong-gateway");
    let ch = &r["snapshot"]["challenge"];
    assert_eq!(ch["finished"], true, "{r}");
    let detail = &ch["score_detail"];
    assert_eq!(detail["diagnosis"], 40, "{detail}");
    assert_eq!(detail["fix"], 40, "{detail}");
    assert_eq!(
        detail["elapsed_secs"], 60,
        "l'horloge injectée est utilisée"
    );
    assert_eq!(detail["total"], ch["score"]);

    // Le profil est écrit dans le stockage du service.
    let saved = store.read(Path::new("profils/alice.json")).unwrap();
    assert!(saved.contains("beginner-01"), "{saved}");
    let profile = ok(&mut svc, "GET", "/api/profile", None);
    assert_eq!(profile["completed"], 1);
    let board = ok(&mut svc, "GET", "/api/leaderboard", None);
    assert_eq!(board["entries"][0]["name"], "alice");
}

#[test]
fn profiles_and_labs_can_be_exported_and_imported() {
    let (mut svc, _) = memory_service();
    let created = svc
        .create_lab(&CreateLab {
            builtin: Some("dmz".to_string()),
            ..CreateLab::default()
        })
        .unwrap();
    let text = svc.export_lab(&created.id).unwrap();
    assert!(text.starts_with("format_version: 1"), "{text}");
    let again = svc.import_lab(&text).unwrap();
    assert_eq!(
        again.snapshot["devices"].as_array().unwrap().len(),
        created.snapshot["devices"].as_array().unwrap().len()
    );

    // Un profil venu d'ailleurs (base de données) remplace le profil actuel,
    // y compris dans les labs déjà ouverts.
    let mut other = netiv_education::Profile::new("alice");
    other.mark_lesson_read("arp");
    other.record_attempt("beginner-02");
    svc.import_profile(&other.to_json().unwrap()).unwrap();
    let exported = svc.export_profile().unwrap();
    let back = netiv_education::Profile::from_json(&exported, "test").unwrap();
    assert_eq!(back, other);
    assert!(svc.import_profile("{ cassé").is_err());

    // Sans dossier de profils, il n'y a nulle part où importer.
    let mut volatile = Service::new(ServiceConfig::default(), Box::new(MemoryStorage::new()));
    assert!(volatile.import_profile(&exported).is_err());
    assert_eq!(volatile.leaderboard().entries.len(), 0);
}

#[test]
fn the_library_lists_labs_from_the_storage() {
    let (mut svc, store) = memory_service();
    store
        .write(
            Path::new("labs/mon-lab.netiv"),
            "format_version: 1\nmetadata: { title: Mon lab, author: bob }\ntopology:\n  network: { name: x }\n",
        )
        .unwrap();
    store.write(Path::new("labs/notes.txt"), "x").unwrap();
    let lib = svc.library();
    assert_eq!(lib.labs.len(), 1);
    assert_eq!(lib.labs[0].title, "Mon lab");
    assert_eq!(lib.labs[0].kind, "network");
    let r = ok(
        &mut svc,
        "POST",
        "/api/labs",
        Some(json!({ "lab": "mon-lab" })),
    );
    assert!(r["output"].as_str().unwrap().contains("Mon lab"), "{r}");
    assert_eq!(
        req(
            &mut svc,
            "POST",
            "/api/labs",
            Some(json!({ "lab": "absent" }))
        )
        .0,
        404
    );
}
