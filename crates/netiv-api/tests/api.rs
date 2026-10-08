#![allow(clippy::unwrap_used, clippy::expect_used)]

//! L'API HTTP, requête par requête (sans ouvrir de port).

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use netiv_api::{ApiConfig, MAX_LABS, router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn labs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../labs")
}

fn app() -> Router {
    router(ApiConfig {
        labs_dir: labs_dir(),
        ..ApiConfig::default()
    })
}

struct Reply {
    status: StatusCode,
    json: Value,
    headers: HeaderMap,
    text: String,
}

async fn send(app: &Router, method: Method, uri: &str, body: Option<Value>) -> Reply {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    raw(app, b.body(body).unwrap()).await
}

async fn raw(app: &Router, req: Request<Body>) -> Reply {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    Reply {
        status,
        json,
        headers,
        text,
    }
}

async fn get(app: &Router, uri: &str) -> Reply {
    send(app, Method::GET, uri, None).await
}

async fn post(app: &Router, uri: &str, body: Value) -> Reply {
    send(app, Method::POST, uri, Some(body)).await
}

async fn create(app: &Router, body: Value) -> String {
    let r = post(app, "/api/labs", body).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json["id"].as_str().unwrap().to_string()
}

async fn exec(app: &Router, id: &str, line: &str) -> Value {
    let r = post(
        app,
        &format!("/api/labs/{id}/exec"),
        json!({ "line": line }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.json
}

fn out(v: &Value) -> &str {
    v["output"].as_str().unwrap()
}

// ------------------------------------------------------------------ généralités

#[tokio::test]
async fn info_and_library_describe_what_is_available() {
    let app = app();
    let info = get(&app, "/api/info").await;
    assert_eq!(info.status, StatusCode::OK);
    assert_eq!(info.json["name"], "NETIV");
    assert_eq!(info.json["sandbox"], true);

    let lib = get(&app, "/api/library").await.json;
    assert_eq!(lib["challenges"].as_array().unwrap().len(), 15);
    assert_eq!(lib["ranges"].as_array().unwrap().len(), 3);
    assert!(
        lib["topologies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "office")
    );
    assert_eq!(lib["paths"][0]["id"], "fundamentals");
    let labs = lib["labs"].as_array().unwrap();
    assert!(
        labs.iter()
            .any(|l| l["name"] == "defi-boucle" && l["kind"] == "challenge"),
        "{labs:?}"
    );
    assert!(
        lib["generator"]["types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "campus")
    );
}

#[tokio::test]
async fn static_files_are_served_with_the_right_types() {
    let app = app();
    for (uri, kind) in [
        ("/", "text/html"),
        ("/app.js", "text/javascript"),
        ("/style.css", "text/css"),
        ("/favicon.svg", "image/svg+xml"),
    ] {
        let r = get(&app, uri).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}");
        assert!(
            r.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with(kind),
            "{uri}"
        );
        assert!(!r.text.is_empty());
    }
    assert_eq!(get(&app, "/nope").await.status, StatusCode::NOT_FOUND);
}

// ------------------------------------------------------------------ cycle de vie d'un lab

#[tokio::test]
async fn a_lab_is_created_inspected_and_deleted() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;

    let snap = get(&app, &format!("/api/labs/{id}")).await.json;
    assert_eq!(snap["name"], "office");
    assert_eq!(snap["devices"].as_array().unwrap().len(), 6);
    assert_eq!(snap["links"].as_array().unwrap().len(), 5);
    let pc1 = snap["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "PC1")
        .unwrap();
    assert_eq!(pc1["ifaces"][0]["ip"], "192.168.1.10/24");
    assert_eq!(pc1["ifaces"][0]["peer"], "SW1:port1");
    assert_eq!(pc1["role"], "user");
    assert!(snap["challenge"].is_null());

    let list = get(&app, "/api/labs").await.json;
    assert_eq!(list["labs"][0]["id"], id.as_str());

    let del = send(&app, Method::DELETE, &format!("/api/labs/{id}"), None).await;
    assert_eq!(del.status, StatusCode::OK);
    assert_eq!(
        get(&app, &format!("/api/labs/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, Method::DELETE, &format!("/api/labs/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn exactly_one_source_is_required_and_bad_sources_are_refused() {
    let app = app();
    for body in [
        json!({}),
        json!({ "builtin": "office", "challenge": "beginner-01" }),
    ] {
        let r = post(&app, "/api/labs", body).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert!(
            r.json["error"]
                .as_str()
                .unwrap()
                .contains("exactement une source")
        );
    }
    assert!(
        post(&app, "/api/labs", json!({ "builtin": "nope" }))
            .await
            .json["error"]
            .as_str()
            .unwrap()
            .contains("réseau inconnu")
    );
    assert_eq!(
        post(&app, "/api/labs", json!({ "challenge": "zzz" }))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(&app, "/api/labs", json!({ "range": "zzz" }))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(&app, "/api/labs", json!({ "lab": "zzz" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&app, "/api/labs", json!({ "yaml": "pas: du yaml valide" }))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    // Aucun lab fantôme n'a été créé par ces échecs.
    assert_eq!(
        get(&app, "/api/labs").await.json["labs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn identifiers_cannot_smuggle_commands() {
    let app = app();
    for evil in [
        "beginner-01 ; load /etc/passwd",
        "x\nload /etc/passwd",
        "../../etc/passwd",
        "a b",
        "",
    ] {
        for field in ["challenge", "range", "lab"] {
            let r = post(&app, "/api/labs", json!({ field: evil })).await;
            assert!(
                r.status == StatusCode::BAD_REQUEST || r.status == StatusCode::NOT_FOUND,
                "{field}={evil:?} → {}",
                r.status
            );
        }
    }
    let r = post(
        &app,
        "/api/labs",
        json!({ "generate": { "type": "lan ; load x", "hosts": 3 } }),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn labs_can_come_from_yaml_a_generator_or_the_library() {
    let app = app();
    let yaml = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/mvp.yaml"),
    )
    .unwrap();
    let a = create(&app, json!({ "yaml": yaml })).await;
    assert_eq!(
        get(&app, &format!("/api/labs/{a}")).await.json["name"],
        "mvp"
    );

    let b = create(
        &app,
        json!({ "generate": { "type": "campus", "routers": 4, "hosts": 8, "seed": 2 } }),
    )
    .await;
    let snap = get(&app, &format!("/api/labs/{b}")).await.json;
    assert_eq!(snap["name"], "campus-4r-8h");

    let c = create(&app, json!({ "lab": "bureau" })).await;
    assert_eq!(
        get(&app, &format!("/api/labs/{c}")).await.json["devices"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    // Un lab de la bibliothèque qui porte un challenge démarre ce challenge.
    let d = create(&app, json!({ "lab": "defi-boucle" })).await;
    let snap = get(&app, &format!("/api/labs/{d}")).await.json;
    assert_eq!(snap["challenge"]["id"], "advanced-02");
}

#[tokio::test]
async fn the_number_of_open_labs_is_bounded() {
    let app = app();
    for _ in 0..MAX_LABS {
        create(&app, json!({ "builtin": "triangle" })).await;
    }
    let r = post(&app, "/api/labs", json!({ "builtin": "triangle" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json["error"].as_str().unwrap().contains("trop de labs"));
}

// ------------------------------------------------------------------ simulation

#[tokio::test]
async fn pinging_through_the_api_produces_a_trace_a_transaction_and_a_journey() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let r = exec(&app, &id, "ping PC1 192.168.2.10").await;
    assert!(out(&r).contains("0 % de perte"), "{r}");
    assert!(r["error"].is_null());
    assert_eq!(r["new_events_from"], 0);
    assert!(r["snapshot"]["trace_len"].as_u64().unwrap() > 50);

    let trace = get(&app, &format!("/api/labs/{id}/trace?since=0&limit=10"))
        .await
        .json;
    assert_eq!(trace["events"].as_array().unwrap().len(), 10);
    assert!(trace["len"].as_u64().unwrap() > 50);
    // Pagination : la suite commence où la page s'arrête.
    let page2 = get(&app, &format!("/api/labs/{id}/trace?since=10&limit=3"))
        .await
        .json;
    assert_eq!(page2["events"][0]["seq"], 10);

    let tx = get(&app, &format!("/api/labs/{id}/transactions"))
        .await
        .json;
    let first = &tx["transactions"][0];
    assert_eq!(first["kind"], "ping");
    assert_eq!(first["outcome"], "ok");
    assert_eq!(first["from"], "PC1");
    let tid = first["id"].as_u64().unwrap();

    let j = get(&app, &format!("/api/labs/{id}/journey/{tid}"))
        .await
        .json;
    let steps = j["steps"].as_array().unwrap();
    assert!(steps.len() > 20);
    // Les trames émises portent le câble emprunté, pour l'animation.
    let edges: Vec<&Value> = steps
        .iter()
        .filter(|s| s["kind"] == "frame_sent")
        .map(|s| &s["edge"])
        .collect();
    assert!(
        edges
            .iter()
            .all(|e| e["from"].is_string() && e["to"].is_string()),
        "{edges:?}"
    );
    assert!(edges.iter().any(|e| e["from"] == "PC1" && e["to"] == "SW1"));
    assert!(j["text"].as_str().unwrap().contains("PARCOURS"));

    assert_eq!(
        get(&app, &format!("/api/labs/{id}/journey/99999"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn command_errors_are_reported_in_the_body_not_as_http_errors() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let r = exec(&app, &id, "frobnicate").await;
    assert!(r["error"].as_str().unwrap().contains("commande inconnue"));
    assert_eq!(out(&r), "");
    let r = exec(&app, &id, "ping NOPE 1.1.1.1").await;
    assert!(r["error"].as_str().unwrap().contains("équipement inconnu"));
    let r = exec(&app, &id, "quit").await;
    assert_eq!(r["quit"], true);
    // Le lab survit à « quit » : c'est l'interface qui décide quoi en faire.
    assert_eq!(
        get(&app, &format!("/api/labs/{id}")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn device_tables_depend_on_the_kind_of_device() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    exec(&app, &id, "ping PC1 192.168.2.10").await;
    let titles = |v: &Value| -> Vec<String> {
        v["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap().to_string())
            .collect()
    };
    let r1 = get(&app, &format!("/api/labs/{id}/device/R1")).await.json;
    assert_eq!(
        titles(&r1),
        ["Interfaces", "Routes", "Cache ARP", "Services", "Filtrage"]
    );
    assert!(
        r1["sections"][1]["text"]
            .as_str()
            .unwrap()
            .contains("192.168.2.0/24")
    );
    let sw = get(&app, &format!("/api/labs/{id}/device/SW1")).await.json;
    assert_eq!(titles(&sw), ["Interfaces", "VLAN et ports", "Table MAC"]);
    assert!(
        sw["sections"][2]["text"]
            .as_str()
            .unwrap()
            .contains("02:4E:49")
    );
    assert_eq!(
        get(&app, &format!("/api/labs/{id}/device/NOPE"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn analysis_endpoints_work_and_never_change_the_lab() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let before = get(&app, &format!("/api/labs/{id}")).await.json;

    let h = get(&app, &format!("/api/labs/{id}/health")).await.json;
    assert_eq!(h["categories"].as_array().unwrap().len(), 6);
    assert!(h["global"].as_u64().unwrap() <= 100);
    assert!(h["text"].as_str().unwrap().contains("GLOBAL SCORE"));

    let s = get(&app, &format!("/api/labs/{id}/security")).await.json;
    assert!(
        s["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == "SEC-011")
    );
    assert_eq!(s["counts"]["high"], 0);

    let d = get(&app, &format!("/api/labs/{id}/docs?format=json"))
        .await
        .json;
    assert!(
        serde_json::from_str::<Value>(d["text"].as_str().unwrap()).unwrap()["devices"].is_array()
    );
    assert!(
        get(&app, &format!("/api/labs/{id}/docs")).await.json["text"]
            .as_str()
            .unwrap()
            .starts_with("# Documentation réseau")
    );
    assert_eq!(
        get(&app, &format!("/api/labs/{id}/docs?format=pdf"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    let w = post(
        &app,
        &format!("/api/labs/{id}/what-if"),
        json!({ "scenario": "block tcp/80" }),
    )
    .await
    .json;
    assert_eq!(w["affected_services"], json!(["tcp/80 sur SERVER"]));
    assert_eq!(w["harmless"], false);
    let bad = post(
        &app,
        &format!("/api/labs/{id}/what-if"),
        json!({ "scenario": "explode" }),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    assert_eq!(get(&app, &format!("/api/labs/{id}")).await.json, before);
}

// ------------------------------------------------------------------ apprentissage

#[tokio::test]
async fn a_challenge_can_be_played_end_to_end_through_the_api() {
    let app = app();
    let id = create(&app, json!({ "challenge": "beginner-01" })).await;
    let snap = get(&app, &format!("/api/labs/{id}")).await.json;
    assert_eq!(snap["challenge"]["id"], "beginner-01");
    assert_eq!(snap["challenge"]["finished"], false);
    assert!(
        snap["challenge"]["description"]
            .as_str()
            .unwrap()
            .contains("PC1")
    );

    // Les analyses qui trahissent la panne sont refusées pendant l'épreuve.
    assert_eq!(
        get(&app, &format!("/api/labs/{id}/health")).await.status,
        StatusCode::FORBIDDEN
    );
    assert!(
        exec(&app, &id, "diagnose PC1 192.168.2.10").await["error"]
            .as_str()
            .unwrap()
            .contains("indisponible")
    );

    let hint = exec(&app, &id, "hint").await;
    assert!(out(&hint).contains("Indice 1"));
    assert_eq!(hint["snapshot"]["challenge"]["hints_used"], 1);
    exec(&app, &id, "gateway PC1 192.168.1.1").await;
    assert!(out(&exec(&app, &id, "challenge check").await).contains("Objectif atteint"));
    let end = exec(&app, &id, "answer PC1 wrong-gateway").await;
    assert!(out(&end).contains("Score : 90/100"), "{}", out(&end));
    assert_eq!(end["snapshot"]["challenge"]["finished"], true);
    assert_eq!(end["snapshot"]["challenge"]["score"], 90);

    // Terminé : les analyses sont de nouveau permises.
    assert_eq!(
        get(&app, &format!("/api/labs/{id}/health")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn generated_challenges_and_cyber_ranges_start_from_the_api() {
    let app = app();
    let g = create(
        &app,
        json!({ "generate_challenge": { "topic": "routing", "difficulty": "hard", "seed": 3 } }),
    )
    .await;
    let snap = get(&app, &format!("/api/labs/{g}")).await.json;
    assert!(
        snap["challenge"]["id"]
            .as_str()
            .unwrap()
            .starts_with("gen-routing-hard-")
    );

    let r = create(&app, json!({ "range": "range-01" })).await;
    let snap = get(&app, &format!("/api/labs/{r}")).await.json;
    assert_eq!(snap["challenge"]["kind"], "range");
    assert!(
        snap["trace_len"].as_u64().unwrap() > 100,
        "l'incident est déjà dans la trace"
    );
    // La forensique voit le coupable.
    assert!(out(&exec(&app, &r, "forensic analyze live").await).contains("WEB01"));
}

// ------------------------------------------------------------------ progression

#[tokio::test]
async fn progress_is_saved_ranked_and_shared_between_labs() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(ApiConfig {
        profile_name: "alice".into(),
        profile_dir: Some(dir.path().to_path_buf()),
        labs_dir: labs_dir(),
        ..ApiConfig::default()
    });
    // Un autre profil existe déjà sur le disque.
    std::fs::write(dir.path().join("bob.json"), r#"{"name":"bob","completed":{"beginner-01":{"score":100,"xp":100,"hints":0,"elapsed_secs":5,"topic":"addressing","cause":"wrong-gateway","times":1}},"attempts":{},"lessons_read":[],"badges":["network-beginner"]}"#).unwrap();
    std::fs::write(dir.path().join("zz.json"), "{ cassé").unwrap();

    assert_eq!(get(&app, "/api/profile").await.json["xp"], 0);
    let a = create(&app, json!({ "challenge": "beginner-01" })).await;
    let b = create(&app, json!({ "challenge": "beginner-02" })).await; // deux labs, un seul profil
    exec(&app, &a, "gateway PC1 192.168.1.1").await;
    exec(&app, &a, "challenge check").await;
    exec(&app, &a, "answer PC1 wrong-gateway").await;
    exec(&app, &b, "cable R1 eth1 restore").await;
    exec(&app, &b, "challenge check").await;
    exec(&app, &b, "answer R1 link-down").await;

    let p = get(&app, "/api/profile").await.json;
    assert_eq!(
        p["xp"], 200,
        "les deux résultats s'additionnent malgré deux labs"
    );
    assert_eq!(p["level_name"], "Technicien");
    assert_eq!(p["completed"], 2);
    assert!(
        p["dashboard"]
            .as_str()
            .unwrap()
            .contains("TABLEAU DE BORD — alice")
    );

    let board = get(&app, "/api/leaderboard").await.json;
    let names: Vec<&str> = board["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alice", "bob"]);
    assert_eq!(board["skipped"], 1);
}

#[tokio::test]
async fn without_a_profile_directory_progress_stays_in_memory() {
    let app = app();
    assert_eq!(
        get(&app, "/api/leaderboard").await.json["entries"],
        json!([])
    );
    assert_eq!(get(&app, "/api/info").await.json["persistent"], false);
}

// ------------------------------------------------------------------ sécurité

#[tokio::test]
async fn sessions_cannot_read_or_write_the_users_files() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("pwned");
    for line in [
        "load /etc/passwd".to_string(),
        format!("capture save {}", target.display()),
        format!("docs md {}", target.display()),
        format!("lab export {}", target.display()),
        "forensic analyze /etc/passwd".to_string(),
        format!(
            "topology generate --type lan --hosts 3 --save {}",
            target.display()
        ),
        "challenge start /etc/passwd".to_string(),
        "lab list".to_string(),
    ] {
        let r = exec(&app, &id, &line).await;
        let e = r["error"].as_str().unwrap_or_default();
        assert!(
            e.contains("indisponible depuis l'interface web") || e.contains("challenge inconnu"),
            "« {line} » devrait être refusé : {r}"
        );
    }
    assert!(!target.exists());
}

#[tokio::test]
async fn the_host_header_is_checked_against_dns_rebinding() {
    let app = router(ApiConfig {
        allowed_hosts: ApiConfig::allowed_hosts_for("127.0.0.1:8787".parse().unwrap()),
        labs_dir: labs_dir(),
        ..ApiConfig::default()
    });
    let with_host = |host: Option<&str>| {
        let mut b = Request::builder().uri("/api/info");
        if let Some(h) = host {
            b = b.header(header::HOST, h);
        }
        b.body(Body::empty()).unwrap()
    };
    for ok in [
        "127.0.0.1:8787",
        "localhost:8787",
        "LOCALHOST:8787",
        "[::1]:8787",
    ] {
        assert_eq!(
            raw(&app, with_host(Some(ok))).await.status,
            StatusCode::OK,
            "{ok}"
        );
    }
    for bad in [
        "evil.example.com",
        "evil.example.com:8787",
        "127.0.0.1:9999",
        "localhost",
    ] {
        assert_eq!(
            raw(&app, with_host(Some(bad))).await.status,
            StatusCode::FORBIDDEN,
            "{bad}"
        );
    }
    assert_eq!(
        raw(&app, with_host(None)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn posts_must_be_json_and_well_formed() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let uri = format!("/api/labs/{id}/exec");
    let post_raw = |ct: Option<&str>, body: &str| {
        let mut b = Request::builder().method(Method::POST).uri(uri.clone());
        if let Some(ct) = ct {
            b = b.header(header::CONTENT_TYPE, ct);
        }
        b.body(Body::from(body.to_string())).unwrap()
    };
    // Un formulaire d'une page tierce ne peut pas envoyer du JSON sans contrôle préalable.
    assert_eq!(
        raw(&app, post_raw(Some("text/plain"), r#"{"line":"devices"}"#))
            .await
            .status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        raw(
            &app,
            post_raw(Some("application/x-www-form-urlencoded"), "line=devices")
        )
        .await
        .status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        raw(&app, post_raw(None, r#"{"line":"devices"}"#))
            .await
            .status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    // JSON invalide, champ inconnu ou manquant.
    assert!(
        raw(&app, post_raw(Some("application/json"), "{ pas du json"))
            .await
            .status
            .is_client_error()
    );
    assert!(
        raw(
            &app,
            post_raw(Some("application/json"), r#"{"line":"devices","extra":1}"#)
        )
        .await
        .status
        .is_client_error()
    );
    assert!(
        raw(&app, post_raw(Some("application/json"), "{}"))
            .await
            .status
            .is_client_error()
    );
}

#[tokio::test]
async fn oversized_requests_are_refused() {
    let app = app();
    let id = create(&app, json!({ "builtin": "office" })).await;
    let long = "x".repeat(netiv_api::MAX_LINE + 1);
    let r = post(
        &app,
        &format!("/api/labs/{id}/exec"),
        json!({ "line": long }),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json["error"].as_str().unwrap().contains("trop longue"));
    let huge = "y".repeat(netiv_api::MAX_BODY + 10);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/labs")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "yaml": huge }).to_string()))
        .unwrap();
    assert_eq!(raw(&app, req).await.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn security_headers_are_present_and_the_api_is_never_cached() {
    let app = app();
    let r = get(&app, "/api/info").await;
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    assert_eq!(r.headers["x-frame-options"], "DENY");
    assert_eq!(r.headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    let csp = r.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(
        csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    // Aucun en-tête CORS : une autre origine ne peut pas lire les réponses.
    assert!(r.headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    // La page elle-même reçoit aussi les en-têtes de sécurité.
    assert_eq!(
        get(&app, "/").await.headers["x-content-type-options"],
        "nosniff"
    );
}

#[tokio::test]
async fn labs_are_independent_of_each_other() {
    let app = app();
    let a = create(&app, json!({ "builtin": "office" })).await;
    let b = create(&app, json!({ "builtin": "office" })).await;
    assert_ne!(a, b);
    exec(&app, &a, "cable R1 eth1 cut").await;
    assert!(out(&exec(&app, &a, "ping PC1 192.168.2.10").await).contains("100 % de perte"));
    assert!(out(&exec(&app, &b, "ping PC1 192.168.2.10").await).contains("0 % de perte"));
}

#[tokio::test]
async fn parallel_requests_do_not_corrupt_the_state() {
    let app = app();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let id = create(&app, json!({ "builtin": "triangle" })).await;
            let r = exec(&app, &id, "ping PC1 10.3.0.10").await;
            assert!(out(&r).contains("0 % de perte"));
            id
        }));
    }
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "identifiants uniques");
    assert_eq!(
        get(&app, "/api/labs").await.json["labs"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

#[tokio::test]
async fn the_api_is_deterministic() {
    let play = || async {
        let app = app();
        let id = create(&app, json!({ "range": "range-03" })).await;
        let mut all = String::new();
        for line in ["forensic analyze live", "security analyze", "show acl FW1"] {
            all += out(&exec(&app, &id, line).await);
        }
        all + &get(&app, &format!("/api/labs/{id}/trace?since=0&limit=2000"))
            .await
            .text
    };
    assert_eq!(play().await, play().await);
}
