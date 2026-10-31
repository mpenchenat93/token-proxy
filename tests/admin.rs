use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use token_proxy::admin::{self, AdminState};
use token_proxy::approval::{Approver, FixedApprover};
use token_proxy::state::{Activation, AppState};
use token_proxy::vault::{self, MemoryStore, SecretStore};

struct Harness {
    app: Arc<AppState>,
    base: String,
    client: reqwest::Client,
}

/// Starts the admin server with a vault shared with the caller, so tests can
/// check what was persisted.
async fn harness_with(approve: bool, store: Arc<MemoryStore>) -> Harness {
    harness_with_approver(Arc::new(FixedApprover(approve)), store).await
}

async fn harness_with_approver(approver: Arc<dyn Approver>, store: Arc<MemoryStore>) -> Harness {
    struct Shared(Arc<MemoryStore>);
    impl SecretStore for Shared {
        fn load(&self) -> anyhow::Result<Option<Vec<u8>>> {
            self.0.load()
        }
        fn save(&self, data: &[u8]) -> anyhow::Result<()> {
            self.0.save(data)
        }
    }

    let app = Arc::new(AppState::new(Box::new(Shared(store)), None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(AdminState::new(app.clone(), approver, port));
    tokio::spawn(async move { axum::serve(listener, admin::router(state)).await });
    Harness { app, base: format!("http://127.0.0.1:{port}"), client: reqwest::Client::new() }
}

async fn harness(approve: bool) -> Harness {
    harness_with(approve, Arc::new(MemoryStore::default())).await
}

fn coolify_route(name: &str) -> Value {
    json!({
        "name": name,
        "upstream": "https://coolify.example.com/",
        "token": "secret-token",
        "auth": "Bearer",
        "description": "test"
    })
}

#[tokio::test]
async fn create_route_with_approval_persists_it_without_exposing_the_token() {
    let store = Arc::new(MemoryStore::default());
    let h = harness_with(true, store.clone()).await;

    let resp = h.client.post(format!("{}/api/routes", h.base)).json(&coolify_route("prod")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(!body.contains("secret-token"));

    let saved = vault::load(store.as_ref()).unwrap();
    assert_eq!(saved.routes.len(), 1);
    assert_eq!(saved.routes[0].upstream, "https://coolify.example.com");

    let list = h.client.get(format!("{}/api/routes", h.base)).send().await.unwrap().text().await.unwrap();
    assert!(list.contains("\"prod\""));
    assert!(!list.contains("secret-token"));
    // A new route is disabled.
    assert_eq!(h.app.activation("perso/prod"), Activation::Disabled);
}

#[tokio::test]
async fn refused_approval_changes_nothing() {
    let h = harness(false).await;
    let resp = h.client.post(format!("{}/api/routes", h.base)).json(&coolify_route("prod")).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    assert!(h.app.routes().is_empty());
}

#[tokio::test]
async fn invalid_route_is_rejected_before_asking_for_approval() {
    let h = harness(true).await;
    let mut route = coolify_route("Bad Name");
    route["token"] = json!("");
    let resp = h.client.post(format!("{}/api/routes", h.base)).json(&route).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn enabling_needs_approval_but_disabling_does_not() {
    let approving = harness(true).await;
    approving.client.post(format!("{}/api/routes", approving.base)).json(&coolify_route("prod")).send().await.unwrap();
    let resp = approving
        .client
        .post(format!("{}/api/routes/perso/prod/enable", approving.base))
        .json(&json!({ "minutes": 15 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(matches!(approving.app.activation("perso/prod"), Activation::Enabled { until: Some(_) }));

    // Same vault, approver that refuses: enabling fails, disabling works.
    let store = Arc::new(MemoryStore::default());
    let setup = harness_with(true, store.clone()).await;
    setup.client.post(format!("{}/api/routes", setup.base)).json(&coolify_route("prod")).send().await.unwrap();
    let refusing = harness_with(false, store).await;
    let resp = refusing
        .client
        .post(format!("{}/api/routes/perso/prod/enable", refusing.base))
        .json(&json!({ "minutes": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    assert_eq!(refusing.app.activation("perso/prod"), Activation::Disabled);

    refusing.app.enable("perso/prod", None).unwrap();
    let resp = refusing.client.post(format!("{}/api/routes/perso/prod/disable", refusing.base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(refusing.app.activation("perso/prod"), Activation::Disabled);
}

#[tokio::test]
async fn update_without_token_keeps_the_current_one() {
    let store = Arc::new(MemoryStore::default());
    let h = harness_with(true, store.clone()).await;
    h.client.post(format!("{}/api/routes", h.base)).json(&coolify_route("prod")).send().await.unwrap();

    let mut route = coolify_route("prod-renamed");
    route.as_object_mut().unwrap().remove("token");
    let resp = h.client.put(format!("{}/api/routes/perso/prod", h.base)).json(&route).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let saved = vault::load(store.as_ref()).unwrap();
    assert_eq!(saved.routes.len(), 1);
    assert_eq!(saved.routes[0].name, "prod-renamed");
    assert_eq!(saved.routes[0].token, "secret-token");
}

#[tokio::test]
async fn names_are_unique_within_a_group_only() {
    let h = harness(true).await;
    let create = |route: Value| h.client.post(format!("{}/api/routes", h.base)).json(&route).send();

    // Without a group, a route goes to "perso".
    assert_eq!(create(coolify_route("coolify")).await.unwrap().status(), 200);
    let mut client = coolify_route("coolify");
    client["group"] = json!("client-a");
    assert_eq!(create(client.clone()).await.unwrap().status(), 200);
    assert_eq!(create(client).await.unwrap().status(), 400);
    let mut bad = coolify_route("coolify");
    bad["group"] = json!("Client A");
    assert_eq!(create(bad).await.unwrap().status(), 400);

    let ids: Vec<String> = h.app.routes().iter().map(|r| r.id()).collect();
    assert_eq!(ids, ["perso/coolify", "client-a/coolify"]);

    // Moving a route to another group renames it: it must be enabled again.
    h.app.enable("client-a/coolify", None).unwrap();
    let mut moved = coolify_route("coolify");
    moved["group"] = json!("client-b");
    let resp = h.client.put(format!("{}/api/routes/client-a/coolify", h.base)).json(&moved).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(h.app.route("client-a/coolify").is_none());
    assert_eq!(h.app.activation("client-b/coolify"), Activation::Disabled);
}

#[tokio::test]
async fn foreign_host_and_cross_origin_requests_are_refused() {
    let h = harness(true).await;
    let resp = h
        .client
        .get(format!("{}/api/routes", h.base))
        .header("host", "attacker.example:80")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    let resp = h
        .client
        .post(format!("{}/api/routes", h.base))
        .header("origin", "https://attacker.example")
        .json(&coolify_route("prod"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    assert!(h.app.routes().is_empty());
}

#[tokio::test]
async fn reload_endpoint_reports_an_already_loaded_vault() {
    let h = harness(true).await;
    let resp = h.client.post(format!("{}/api/vault/reload", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.json::<Value>().await.unwrap(), json!({ "reloaded": false }));
}

/// Approves everything and keeps the dialog texts.
#[derive(Default)]
struct RecordingApprover(Mutex<Vec<String>>);

impl Approver for RecordingApprover {
    fn approve(&self, reason: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(reason.to_string());
        Ok(())
    }
}

#[tokio::test]
async fn moving_the_token_elsewhere_needs_the_token_again() {
    let store = Arc::new(MemoryStore::default());
    let approver = Arc::new(RecordingApprover::default());
    let h = harness_with_approver(approver.clone(), store.clone()).await;
    let mut local = coolify_route("local");
    local["upstream"] = json!("http://127.0.0.1:8080");
    for route in [coolify_route("prod"), local] {
        assert_eq!(h.client.post(format!("{}/api/routes", h.base)).json(&route).send().await.unwrap().status(), 200);
    }
    let update = |id: &str, route: Value| h.client.put(format!("{}/api/routes/{id}", h.base)).json(&route).send();
    let without_token = |name: &str, upstream: &str, auth: Value| {
        json!({ "name": name, "upstream": upstream, "auth": auth, "description": "test" })
    };
    let asked = approver.0.lock().unwrap().len();

    // Another host, another port, another way of sending the token: refused
    // without the token, before any dialog.
    for (id, route) in [
        ("perso/prod", without_token("prod", "https://evil.example", json!("Bearer"))),
        ("perso/prod", without_token("prod", "https://coolify.example.com:8443", json!("Bearer"))),
        ("perso/prod", without_token("prod", "https://coolify.example.com", json!({ "Query": { "param": "t" } }))),
        ("perso/local", without_token("local", "http://127.0.0.1:9090", json!("Bearer"))),
    ] {
        let resp = update(id, route).await.unwrap();
        assert_eq!(resp.status(), 400);
        assert!(resp.text().await.unwrap().contains("token"));
    }
    assert_eq!(approver.0.lock().unwrap().len(), asked);
    assert_eq!(vault::load(store.as_ref()).unwrap().routes[0].upstream, "https://coolify.example.com");

    // Same origin, another path: allowed, and the dialog shows before → after.
    let resp = update("perso/prod", without_token("prod", "https://coolify.example.com/api", json!("Bearer"))).await;
    assert_eq!(resp.unwrap().status(), 200);
    let reason = approver.0.lock().unwrap().last().cloned().unwrap();
    assert!(reason.contains("https://coolify.example.com → https://coolify.example.com/api"), "{reason}");

    // With the token, the route can move.
    let mut moved = without_token("prod", "https://other.example", json!({ "Header": { "name": "x-api-key" } }));
    moved["token"] = json!("another-token");
    assert_eq!(update("perso/prod", moved).await.unwrap().status(), 200);
    let reason = approver.0.lock().unwrap().last().cloned().unwrap();
    assert!(reason.contains("https://coolify.example.com/api → https://other.example"), "{reason}");
    assert!(reason.contains("Authorization") && reason.contains("x-api-key"), "{reason}");
}

#[tokio::test]
async fn plain_http_is_only_accepted_on_the_local_network() {
    let h = harness(true).await;
    let create = |name: &str, upstream: &str| {
        let mut route = coolify_route(name);
        route["upstream"] = json!(upstream);
        h.client.post(format!("{}/api/routes", h.base)).json(&route).send()
    };
    for (i, upstream) in ["http://coolify.example.com", "http://8.8.8.8", "http://localhost.example.com"].iter().enumerate() {
        assert_eq!(create(&format!("public{i}"), upstream).await.unwrap().status(), 400, "{upstream}");
    }
    for (i, upstream) in [
        "http://localhost:11434",
        "http://127.0.0.1:8080",
        "http://192.168.1.10",
        "http://10.0.0.2",
        "http://100.101.102.103",
        "http://[::1]:8080",
        "http://nas.local",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(create(&format!("local{i}"), upstream).await.unwrap().status(), 200, "{upstream}");
    }
}
