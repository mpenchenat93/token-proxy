//! Demo instance, to try the admin UI without touching the Keychain:
//! `cargo run --example demo`, then open http://127.0.0.1:18788.
//!
//! The vault lives in memory and is seeded with fictitious routes; every
//! approval is granted without Touch ID. Never use it with real tokens.

use chrono::Local;
use std::sync::Arc;
use std::time::Duration;
use token_proxy::admin::{self, AdminState};
use token_proxy::approval::FixedApprover;
use token_proxy::config::{AuthInjection, RouteConfig};
use token_proxy::proxy;
use token_proxy::state::{AppState, AuditEntry, ServerStatus};
use token_proxy::vault::MemoryStore;

fn port_from_env(name: &str, default: u16) -> u16 {
    std::env::var(name).ok().and_then(|p| p.parse().ok()).unwrap_or(default)
}

fn route(group: &str, name: &str, upstream: &str, auth: AuthInjection, description: &str) -> RouteConfig {
    RouteConfig {
        name: name.into(),
        upstream: upstream.into(),
        token: "demo-token".into(),
        auth,
        description: description.into(),
        group: group.into(),
    }
}

fn seed(state: &AppState) {
    let routes = [
        route("perso", "github", "https://api.github.com", AuthInjection::Bearer, "Read-only token: repositories and issues"),
        route("perso", "openai", "https://api.openai.com", AuthInjection::Bearer, "API under /v1/"),
        route("perso", "cloudflare", "https://api.cloudflare.com", AuthInjection::Bearer, "DNS edit token"),
        route("acme", "coolify", "https://coolify.acme.example", AuthInjection::Bearer, "Read + deploy, ask before deploying"),
        route(
            "acme",
            "directus",
            "https://cms.acme.example",
            AuthInjection::Query { param: "access_token".into() },
            "Administrator token: ask before any write",
        ),
        route(
            "globex",
            "meilisearch",
            "https://search.globex.example",
            AuthInjection::Header { name: "x-meili-api-key".into() },
            "Search key only",
        ),
    ];
    for r in routes {
        state.upsert_route(r, None).expect("seed route");
    }
    state.enable("perso/github", None).expect("enable");
    state.enable("perso/openai", Some(Duration::from_secs(47 * 60))).expect("enable");
    state.enable("acme/coolify", Some(Duration::from_secs(12 * 60))).expect("enable");

    let now = Local::now();
    let entries = [
        (41, "perso/github", "GET", "/repos/octo/app/issues", 200, true, "214 ms"),
        (33, "acme/directus", "GET", "/items/articles", 403, false, "La route « acme/directus » est désactivée. Demande à l’utilisateur de l’activer dans Token Proxy."),
        (26, "perso/openai", "POST", "/v1/chat/completions", 200, true, "1874 ms"),
        (18, "acme/coolify", "GET", "/api/v1/applications", 200, true, "302 ms"),
        (9, "acme/coolify", "POST", "/api/v1/deploy", 201, true, "655 ms"),
        (2, "perso/github", "GET", "/user/repos", 200, true, "188 ms"),
    ];
    for (minutes_ago, route, method, path, status, forwarded, note) in entries {
        state.record(AuditEntry {
            at: now - chrono::Duration::minutes(minutes_ago),
            route: route.into(),
            method: method.into(),
            path: path.into(),
            status,
            forwarded,
            note: note.into(),
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let proxy_port = port_from_env("TOKEN_PROXY_PORT", 18787);
    let admin_port = port_from_env("TOKEN_PROXY_ADMIN_PORT", 18788);

    let state = Arc::new(AppState::new(Box::new(MemoryStore::default()), None));
    seed(&state);

    let proxy_listener = tokio::net::TcpListener::bind(("127.0.0.1", proxy_port)).await?;
    *state.server_status.lock() = ServerStatus::Listening(proxy_listener.local_addr()?);
    tokio::spawn(proxy::serve(proxy_listener, state.clone()));

    let admin_listener = tokio::net::TcpListener::bind(("127.0.0.1", admin_port)).await?;
    println!("Demo admin UI on http://127.0.0.1:{admin_port} (in-memory vault, no Touch ID)");
    let admin_state = Arc::new(AdminState::new(state, Arc::new(FixedApprover(true)), admin_port));
    axum::serve(admin_listener, admin::router(admin_state)).await?;
    Ok(())
}
