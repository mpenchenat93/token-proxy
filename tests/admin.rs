use serde_json::{json, Value};
use std::sync::Arc;
use token_proxy::admin::{self, AdminState};
use token_proxy::approval::FixedApprover;
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
    let state = Arc::new(AdminState::new(app.clone(), Arc::new(FixedApprover(approve)), port));
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
