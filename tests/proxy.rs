use std::sync::Arc;
use std::time::Duration;
use token_proxy::config::{AuthInjection, RouteConfig};
use token_proxy::proxy;
use token_proxy::state::AppState;
use token_proxy::vault::MemoryStore;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "real-secret-token";

struct Harness {
    state: Arc<AppState>,
    upstream: MockServer,
    base: String,
    client: reqwest::Client,
}

async fn harness() -> Harness {
    let upstream = MockServer::start().await;
    let state = Arc::new(AppState::new(Box::new(MemoryStore::default()), None));

    state
        .upsert_route(
            RouteConfig {
                name: "coolify".into(),
                upstream: upstream.uri(),
                token: TOKEN.into(),
                auth: AuthInjection::Bearer,
                description: String::new(),
                group: "perso".into(),
            },
            None,
        )
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(proxy::serve(listener, state.clone()));

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .unwrap();
    Harness { state, upstream, base, client }
}

async fn hits(upstream: &MockServer) -> usize {
    upstream.received_requests().await.unwrap_or_default().len()
}

#[tokio::test]
async fn disabled_route_is_rejected_without_reaching_upstream() {
    let h = harness().await;
    let resp = h.client.get(format!("{}/perso/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    assert!(resp.text().await.unwrap().contains("disabled"));
    assert_eq!(hits(&h.upstream).await, 0);
}

#[tokio::test]
async fn enabled_route_injects_token_and_drops_client_credentials() {
    let h = harness().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/applications"))
        .and(query_param("tag", "prod"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&h.upstream)
        .await;
    h.state.enable("perso/coolify", None).unwrap();

    let resp = h
        .client
        .get(format!("{}/perso/coolify/api/v1/applications?tag=prod", h.base))
        .header("authorization", "Bearer attacker")
        .header("cookie", "session=x")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "[]");

    let requests = h.upstream.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    assert_eq!(headers.get_all("authorization").iter().count(), 1);
    assert!(headers.get("cookie").is_none());

    let audit = h.state.audit_entries();
    assert_eq!(audit[0].status, 200);
    assert!(audit[0].forwarded);
}

#[tokio::test]
async fn expired_activation_disables_route() {
    let h = harness().await;
    h.state.enable("perso/coolify", Some(Duration::from_millis(50))).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let resp = h.client.get(format!("{}/perso/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn unknown_route_and_route_listing() {
    let h = harness().await;
    let resp = h.client.get(format!("{}/nope/x", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    // The URL must name the group.
    h.state.enable("perso/coolify", None).unwrap();
    let resp = h.client.get(format!("{}/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 404);
    assert_eq!(hits(&h.upstream).await, 0);

    let listing = h.client.get(format!("{}/", h.base)).send().await.unwrap().text().await.unwrap();
    assert!(listing.contains("\"/perso/coolify\""));
    assert!(listing.contains(&h.upstream.uri()));
    assert!(!listing.contains(TOKEN));
}

#[tokio::test]
async fn same_service_in_two_groups_uses_each_group_token() {
    let h = harness().await;
    let mut route = h.state.route("perso/coolify").unwrap();
    route.group = "client-a".into();
    route.token = "client-a-token".into();
    h.state.upsert_route(route, None).unwrap();
    Mock::given(path("/api/v1/applications"))
        .and(header("authorization", "Bearer client-a-token"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&h.upstream)
        .await;
    // Enabling a client's route leaves the personal one disabled.
    h.state.enable("client-a/coolify", None).unwrap();

    let resp = h.client.get(format!("{}/client-a/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let resp = h.client.get(format!("{}/perso/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 403);
    assert_eq!(hits(&h.upstream).await, 1);
}

#[tokio::test]
async fn group_listing_shows_only_that_group() {
    let h = harness().await;
    let mut route = h.state.route("perso/coolify").unwrap();
    route.group = "client-a".into();
    route.name = "directus".into();
    h.state.upsert_route(route, None).unwrap();

    let all: serde_json::Value = h.client.get(format!("{}/", h.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(all["groups"], serde_json::json!(["perso", "client-a"]));

    for url in [format!("{}/client-a", h.base), format!("{}/client-a/", h.base)] {
        let group: serde_json::Value = h.client.get(url).send().await.unwrap().json().await.unwrap();
        let routes = group["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0]["base_url"], "/client-a/directus");
    }
    let resp = h.client.get(format!("{}/client-b/", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn redirects_are_not_followed_and_leaking_headers_are_dropped() {
    let h = harness().await;
    Mock::given(path("/api/v1/servers"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "https://evil.example.com/")
                .insert_header("x-debug", format!("token={TOKEN}").as_str()),
        )
        .mount(&h.upstream)
        .await;
    h.state.enable("perso/coolify", None).unwrap();

    let resp = h.client.get(format!("{}/perso/coolify/api/v1/servers", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 302);
    assert!(resp.headers().get("x-debug").is_none());
    assert_eq!(hits(&h.upstream).await, 1);
}

#[tokio::test]
async fn query_injection_replaces_client_value() {
    let h = harness().await;
    let mut route = h.state.route("perso/coolify").unwrap();
    route.name = "directus".into();
    route.auth = AuthInjection::Query { param: "access_token".into() };
    h.state.upsert_route(route, None).unwrap();
    Mock::given(path("/api/v1/items"))
        .and(query_param("access_token", TOKEN))
        .respond_with(ResponseTemplate::new(200))
        .mount(&h.upstream)
        .await;
    h.state.enable("perso/directus", None).unwrap();

    let resp = h
        .client
        .get(format!("{}/perso/directus/api/v1/items?access_token=attacker", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let url = &h.upstream.received_requests().await.unwrap()[0].url;
    assert_eq!(url.query(), Some(format!("access_token={TOKEN}").as_str()));
}

#[tokio::test]
async fn editing_a_route_disables_it() {
    let h = harness().await;
    h.state.enable("perso/coolify", None).unwrap();
    let route = h.state.route("perso/coolify").unwrap();
    h.state.upsert_route(route, Some("perso/coolify")).unwrap();
    let resp = h.client.get(format!("{}/perso/coolify/api/v1/applications", h.base)).send().await.unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn requests_from_web_pages_are_refused() {
    let h = harness().await;
    Mock::given(path("/user"))
        .respond_with(ResponseTemplate::new(200).insert_header("access-control-allow-origin", "*"))
        .mount(&h.upstream)
        .await;
    h.state.enable("perso/coolify", None).unwrap();
    let url = format!("{}/perso/coolify/user", h.base);

    let resp = h.client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers().get("access-control-allow-origin").is_none());

    let from_page = h.client.get(&url).header("origin", "https://evil.example").send().await.unwrap();
    assert_eq!(from_page.status(), 403);
    let no_cors = h.client.get(&url).header("sec-fetch-site", "cross-site").send().await.unwrap();
    assert_eq!(no_cors.status(), 403);
    // DNS rebinding: a foreign domain resolved to 127.0.0.1.
    let rebound = h.client.get(&url).header("host", "evil.example:8787").send().await.unwrap();
    assert_eq!(rebound.status(), 403);
    let listing = h.client.get(format!("{}/", h.base)).header("origin", "https://evil.example").send().await.unwrap();
    assert_eq!(listing.status(), 403);

    assert_eq!(hits(&h.upstream).await, 1);
}
