//! The HTTP proxy: `http://127.0.0.1:<port>/<group>/<route>/<path>` is
//! forwarded to `<upstream>/<path>` with the route's credential added.

use crate::config::{AuthInjection, RouteConfig, DEFAULT_GROUP};
use crate::state::{Activation, AppState, AuditEntry};
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tokio::net::TcpListener;

const MAX_REQUEST_BODY: usize = 10 * 1024 * 1024;

/// Request headers never forwarded upstream: credentials the client may try
/// to pass, and hop-by-hop headers.
const STRIPPED_REQUEST_HEADERS: &[&str] = &[
    "host",
    "authorization",
    "proxy-authorization",
    "cookie",
    "x-api-key",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
];

const STRIPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "set-cookie",
];

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(list_routes))
        .route("/{group}", get(list_group))
        .route("/{group}/", get(list_group))
        .fallback(forward)
        .layer(middleware::from_fn(local_clients_only))
        .with_state(state)
}

/// Rejects requests sent by a web page and requests whose Host is not the
/// loopback address. Agents (curl, SDKs, scripts) send neither `Origin` nor
/// `Sec-Fetch-Site`, but a browser does: without this check, a malicious
/// site open in the user's browser could call an enabled route, and read the
/// answer whenever the upstream allows any origin. The Host check stops DNS
/// rebinding, where such a site points its own domain at 127.0.0.1.
async fn local_clients_only(req: Request, next: Next) -> Response {
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let hostname = host.rsplit_once(':').map_or(host, |(name, _)| name);
    if !matches!(hostname, "127.0.0.1" | "localhost") {
        let message = "En-tête Host inattendu : le proxy ne répond qu’à 127.0.0.1 et localhost.";
        return (StatusCode::FORBIDDEN, Json(json!({ "error": message }))).into_response();
    }
    let from_page = headers.contains_key(header::ORIGIN)
        || headers
            .get("sec-fetch-site")
            .is_some_and(|site| site.as_bytes() != b"none");
    if from_page {
        let message = "Requête envoyée par une page web refusée : le proxy ne sert que les programmes locaux.";
        return (StatusCode::FORBIDDEN, Json(json!({ "error": message }))).into_response();
    }
    next.run(req).await
}

pub async fn serve(listener: TcpListener, state: Arc<AppState>) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

const USAGE: &str = "Appelle /<groupe>/<route>/<chemin> ; le proxy ajoute le token. Le groupe dit à qui appartient le compte (perso, un client…) : si l’utilisateur ne précise pas de groupe, utilise perso ; n’utilise un autre groupe que s’il le nomme. Une route désactivée doit être activée par l’utilisateur.";

/// `GET /`: lets the AI discover which routes exist and which are enabled.
async fn list_routes(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let routes = sorted_routes(&state);
    let mut groups: Vec<&str> = routes.iter().map(|r| r.group.as_str()).collect();
    groups.dedup();
    Json(json!({
        "usage": USAGE,
        "groups": groups,
        "routes": routes.iter().map(|r| route_json(&state, r)).collect::<Vec<_>>(),
    }))
}

/// `GET /<group>/`: the routes of one group only.
async fn list_group(State(state): State<Arc<AppState>>, Path(group): Path<String>) -> Response {
    let routes: Vec<_> = sorted_routes(&state)
        .into_iter()
        .filter(|r| r.group == group)
        .collect();
    if routes.is_empty() {
        let message = format!("Groupe « {group} » inconnu. GET / liste les groupes et leurs routes.");
        return (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response();
    }
    Json(json!({
        "usage": USAGE,
        "group": group,
        "routes": routes.iter().map(|r| route_json(&state, r)).collect::<Vec<_>>(),
    }))
    .into_response()
}

/// Routes by group, the personal one first, then by name.
fn sorted_routes(state: &AppState) -> Vec<RouteConfig> {
    let mut routes = state.routes();
    routes.sort_by(|a, b| {
        (a.group != DEFAULT_GROUP, &a.group, &a.name).cmp(&(b.group != DEFAULT_GROUP, &b.group, &b.name))
    });
    routes
}

fn route_json(state: &AppState, route: &RouteConfig) -> serde_json::Value {
    let (enabled, expires_in) = match state.activation(&route.id()) {
        Activation::Disabled => (false, None),
        Activation::Enabled { until } => (
            true,
            until.map(|u| u.duration_since(SystemTime::now()).unwrap_or_default().as_secs()),
        ),
    };
    json!({
        "group": route.group,
        "name": route.name,
        "description": route.description,
        "upstream": route.upstream,
        "enabled": enabled,
        "expires_in_seconds": expires_in,
        "base_url": format!("/{}", route.id()),
    })
}

async fn forward(State(state): State<Arc<AppState>>, req: Request) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let raw_path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_owned);

    let (route_id, rest) = split_route(&raw_path);
    let audit = |status: StatusCode, forwarded: bool, note: String| {
        state.record(AuditEntry {
            at: chrono::Local::now(),
            route: route_id.to_string(),
            method: method.to_string(),
            path: rest.to_string(),
            status: status.as_u16(),
            forwarded,
            note,
        });
    };
    let reject = |status: StatusCode, message: String| {
        audit(status, false, message.clone());
        (status, Json(json!({ "error": message, "route": route_id }))).into_response()
    };

    let Some(route) = state.route(route_id) else {
        return reject(
            StatusCode::NOT_FOUND,
            format!(
                "Route « {route_id} » inconnue. Les URL ont la forme /<groupe>/<route>/<chemin> ; GET / liste les routes disponibles."
            ),
        );
    };
    if state.activation(route_id) == Activation::Disabled {
        return reject(
            StatusCode::FORBIDDEN,
            format!("La route « {route_id} » est désactivée. Demande à l’utilisateur de l’activer dans Token Proxy."),
        );
    }
    let url = match upstream_url(&route.upstream, rest, query.as_deref(), &route.auth, &route.token) {
        Ok(url) => url,
        Err(e) => return reject(StatusCode::BAD_REQUEST, e),
    };

    let mut headers = req.headers().clone();
    for name in STRIPPED_REQUEST_HEADERS {
        headers.remove(*name);
    }
    match &route.auth {
        AuthInjection::Bearer => {
            headers.insert(axum::http::header::AUTHORIZATION, secret_header(&format!("Bearer {}", route.token)));
        }
        AuthInjection::Header { name } => {
            // Validated when the route was saved.
            let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
                return reject(StatusCode::INTERNAL_SERVER_ERROR, "Nom d’en-tête invalide dans la route.".into());
            };
            headers.insert(name, secret_header(&route.token));
        }
        AuthInjection::Query { .. } => {}
    }

    let body = match axum::body::to_bytes(req.into_body(), MAX_REQUEST_BODY).await {
        Ok(body) => body,
        Err(_) => return reject(StatusCode::PAYLOAD_TOO_LARGE, "Corps de requête trop volumineux.".into()),
    };

    let upstream = state
        .client
        .request(method.clone(), url)
        .headers(headers)
        .body(body)
        .send()
        .await;
    let upstream = match upstream {
        Ok(resp) => resp,
        Err(e) => {
            // reqwest errors may contain the URL, and so a query-string token.
            let e = e.without_url();
            return reject(StatusCode::BAD_GATEWAY, format!("Échec de la requête vers le service : {e}"));
        }
    };

    let status = upstream.status();
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        // CORS headers would let a web page read the answer.
        if STRIPPED_RESPONSE_HEADERS.contains(&name.as_str()) || name.as_str().starts_with("access-control-") {
            continue;
        }
        if contains_bytes(value.as_bytes(), route.token.as_bytes()) {
            continue;
        }
        response_headers.append(name.clone(), value.clone());
    }
    audit(status, true, format!("{} ms", started.elapsed().as_millis()));

    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}

/// `/group/name/rest/of/path` → (`group/name`, `/rest/of/path`);
/// `/group/name` → (`group/name`, `/`).
fn split_route(path: &str) -> (&str, &str) {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let end = trimmed
        .find('/')
        .and_then(|group_end| trimmed[group_end + 1..].find('/').map(|i| group_end + 1 + i));
    match end {
        Some(i) => (&trimmed[..i], &trimmed[i..]),
        None => (trimmed, "/"),
    }
}

fn upstream_url(
    upstream: &str,
    rest: &str,
    query: Option<&str>,
    auth: &AuthInjection,
    token: &str,
) -> Result<reqwest::Url, String> {
    let base = reqwest::Url::parse(upstream).map_err(|e| format!("URL cible invalide : {e}"))?;
    let mut url = reqwest::Url::parse(&format!("{}{}", upstream.trim_end_matches('/'), rest))
        .map_err(|e| format!("Chemin invalide : {e}"))?;
    if url.host() != base.host() || url.port_or_known_default() != base.port_or_known_default() {
        return Err("Le chemin sortirait de l’hôte cible.".into());
    }
    // `..` and `%2e%2e` are resolved while parsing: they must not climb above
    // the route's base path. Not a real boundary (the service may decode
    // paths its own way), only a guard against the obvious escape.
    let base_path = base.path().trim_end_matches('/');
    if url.path() != base_path && !url.path().starts_with(&format!("{base_path}/")) {
        return Err("Le chemin sortirait du chemin de base de la route.".into());
    }
    url.set_query(query);
    if let AuthInjection::Query { param } = auth {
        let kept: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != param)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(kept)
            .append_pair(param, token);
    }
    Ok(url)
}

fn secret_header(value: &str) -> HeaderValue {
    let mut value = HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static(""));
    value.set_sensitive(true);
    value
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        assert_eq!(split_route("/perso/coolify/api/v1/apps"), ("perso/coolify", "/api/v1/apps"));
        assert_eq!(split_route("/perso/coolify"), ("perso/coolify", "/"));
        assert_eq!(split_route("/perso/coolify/"), ("perso/coolify", "/"));
        assert_eq!(split_route("/perso"), ("perso", "/"));
    }

    #[test]
    fn url_building() {
        let url = upstream_url("https://c.example.com/", "/api/v1/apps", Some("a=1"), &AuthInjection::Bearer, "t")
            .unwrap();
        assert_eq!(url.as_str(), "https://c.example.com/api/v1/apps?a=1");

        let auth = AuthInjection::Query { param: "access_token".into() };
        let url = upstream_url("https://d.example.com", "/items", Some("access_token=evil&x=2"), &auth, "real")
            .unwrap();
        assert_eq!(url.as_str(), "https://d.example.com/items?x=2&access_token=real");

        assert!(upstream_url("https://c.example.com", "@evil.com/x", None, &AuthInjection::Bearer, "t").is_err());

        let base = "https://gitlab.example.com/api/v4/projects/42";
        let url = upstream_url(base, "/issues", None, &AuthInjection::Bearer, "t").unwrap();
        assert_eq!(url.path(), "/api/v4/projects/42/issues");
        assert!(upstream_url(base, "/", None, &AuthInjection::Bearer, "t").is_ok());
        for escape in ["/%2e%2e/%2e%2e/user", "/../../user", "/%2E%2E/43"] {
            assert!(upstream_url(base, escape, None, &AuthInjection::Bearer, "t").is_err(), "{escape}");
        }
    }
}
