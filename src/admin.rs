//! Admin server: JSON API under `/api` and the embedded web UI.
//!
//! Reading is free. Anything that grants more access (adding, changing,
//! deleting or enabling a route) needs the user's approval through
//! [`Approver`]. Disabling never does: reducing access is always safe.

use crate::approval::Approver;
use crate::config::{same_token_destination, validate_route, AuthInjection, RouteConfig, DEFAULT_GROUP};
use crate::state::{Activation, AppState};
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(rust_embed::RustEmbed)]
#[folder = "web/dist/client"]
#[allow_missing = true]
struct Assets;

pub struct AdminState {
    pub app: Arc<AppState>,
    pub approver: Arc<dyn Approver>,
    /// Only one approval dialog at a time.
    approval_lock: tokio::sync::Mutex<()>,
    /// `host:port` values accepted in the Host header (DNS rebinding guard).
    allowed_hosts: Vec<String>,
    /// Origins accepted for state-changing requests (CSRF guard).
    allowed_origins: Vec<String>,
}

impl AdminState {
    pub fn new(app: Arc<AppState>, approver: Arc<dyn Approver>, port: u16) -> Self {
        let allowed_hosts = vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let allowed_origins = allowed_hosts.iter().map(|h| format!("http://{h}")).collect();
        Self {
            app,
            approver,
            approval_lock: tokio::sync::Mutex::new(()),
            allowed_hosts,
            allowed_origins,
        }
    }
}

pub fn router(state: Arc<AdminState>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/audit", get(audit))
        .route("/routes", get(list_routes).post(create_route))
        .route("/routes/{group}/{name}", put(update_route).delete(delete_route))
        .route("/routes/{group}/{name}/enable", post(enable_route))
        .route("/routes/{group}/{name}/disable", post(disable_route))
        .route("/disable-all", post(disable_all))
        .route("/vault/reload", post(reload_vault))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "Point d’API inconnu.") });
    Router::new()
        .nest("/api", api)
        .fallback(static_asset)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Rejects requests whose Host is not the admin address (DNS rebinding) and
/// state-changing requests coming from another web origin (CSRF).
async fn guard(State(state): State<Arc<AdminState>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !state.allowed_hosts.iter().any(|h| h == host) {
        return error(StatusCode::FORBIDDEN, "En-tête Host inattendu.");
    }
    let safe_method = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe_method
        && let Some(origin) = req.headers().get(header::ORIGIN)
        && !state.allowed_origins.iter().any(|o| origin.as_bytes() == o.as_bytes())
    {
        return error(StatusCode::FORBIDDEN, "Requête d’une autre origine refusée.");
    }
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Asks the user to approve `reason`, one dialog at a time.
#[allow(clippy::result_large_err)]
async fn require_approval(state: &AdminState, reason: String) -> Result<(), Response> {
    let Ok(_guard) = state.approval_lock.try_lock() else {
        return Err(error(StatusCode::CONFLICT, "Une autre demande d’approbation est déjà en attente."));
    };
    let approver = state.approver.clone();
    let outcome = tokio::task::spawn_blocking(move || approver.approve(&reason))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    outcome.map_err(|e| error(StatusCode::FORBIDDEN, format!("Non approuvé : {e}")))
}

/// `scheme://host[:port]` of a service URL, for approval dialogs: the scheme
/// and port matter as much as the host.
fn origin_of(upstream: &str) -> String {
    reqwest::Url::parse(upstream)
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_else(|_| upstream.to_string())
}

fn auth_label(auth: &AuthInjection) -> String {
    match auth {
        AuthInjection::Bearer => "en-tête Authorization".to_string(),
        AuthInjection::Header { name } => format!("en-tête {name}"),
        AuthInjection::Query { param } => format!("paramètre d’URL {param}"),
    }
}

/// Describes an edit for the approval dialog, each change that matters for
/// the token shown as before → after.
fn change_reason(previous: &RouteConfig, route: &RouteConfig) -> String {
    let mut changes = Vec::new();
    if previous.id() != route.id() {
        changes.push(format!("renommée « {} »", route.id()));
    }
    if previous.upstream == route.upstream {
        changes.push(format!("vers {}", route.upstream));
    } else {
        changes.push(format!("{} → {}", previous.upstream, route.upstream));
    }
    if previous.auth != route.auth {
        changes.push(format!("token par {} → par {}", auth_label(&previous.auth), auth_label(&route.auth)));
    }
    if previous.token != route.token {
        changes.push("nouveau token".to_string());
    }
    format!("modifier la route « {} » : {}", previous.id(), changes.join(" ; "))
}

#[derive(Serialize)]
struct RouteView {
    name: String,
    upstream: String,
    auth: AuthInjection,
    description: String,
    group: String,
    enabled: bool,
    /// `None` while disabled or enabled until disabled by hand.
    expires_in_seconds: Option<u64>,
}

fn route_view(app: &AppState, route: RouteConfig) -> RouteView {
    let (enabled, expires_in_seconds) = match app.activation(&route.id()) {
        Activation::Disabled => (false, None),
        Activation::Enabled { until } => (
            true,
            until.map(|u| u.duration_since(SystemTime::now()).unwrap_or_default().as_secs()),
        ),
    };
    RouteView {
        name: route.name,
        upstream: route.upstream,
        auth: route.auth,
        description: route.description,
        group: route.group,
        enabled,
        expires_in_seconds,
    }
}

async fn status(State(state): State<Arc<AdminState>>) -> Json<serde_json::Value> {
    let proxy = match &*state.app.server_status.lock() {
        crate::state::ServerStatus::Starting => json!({ "state": "starting" }),
        crate::state::ServerStatus::Listening(addr) => json!({ "state": "listening", "url": format!("http://{addr}") }),
        crate::state::ServerStatus::Failed(e) => json!({ "state": "failed", "error": e }),
    };
    Json(json!({
        "proxy": proxy,
        "vault_error": state.app.vault_error(),
        "approval_pending": state.approval_lock.try_lock().is_err(),
    }))
}

async fn audit(State(state): State<Arc<AdminState>>) -> Json<serde_json::Value> {
    let entries: Vec<_> = state
        .app
        .audit_entries()
        .into_iter()
        .take(200)
        .map(|e| {
            json!({
                "at": e.at.to_rfc3339(),
                "route": e.route,
                "method": e.method,
                "path": e.path,
                "status": e.status,
                "forwarded": e.forwarded,
                "note": e.note,
            })
        })
        .collect();
    Json(json!({ "entries": entries }))
}

async fn list_routes(State(state): State<Arc<AdminState>>) -> Json<Vec<RouteView>> {
    let app = &state.app;
    Json(app.routes().into_iter().map(|r| route_view(app, r)).collect())
}

#[derive(Deserialize)]
struct RouteInput {
    name: String,
    upstream: String,
    /// Required on creation; omitted or empty on update keeps the current token.
    #[serde(default)]
    token: Option<String>,
    auth: AuthInjection,
    #[serde(default)]
    description: String,
    /// Empty means [`DEFAULT_GROUP`].
    #[serde(default)]
    group: String,
}

impl RouteInput {
    fn into_route(self, current_token: Option<String>) -> RouteConfig {
        let token = match self.token.map(|t| t.trim().to_string()) {
            Some(t) if !t.is_empty() => t,
            _ => current_token.unwrap_or_default(),
        };
        RouteConfig {
            name: self.name.trim().to_string(),
            upstream: self.upstream.trim().trim_end_matches('/').to_string(),
            token,
            auth: self.auth,
            description: self.description.trim().to_string(),
            group: match self.group.trim() {
                "" => DEFAULT_GROUP.to_string(),
                group => group.to_string(),
            },
        }
    }
}

async fn create_route(State(state): State<Arc<AdminState>>, Json(input): Json<RouteInput>) -> Response {
    let route = input.into_route(None);
    save_route(&state, route, None).await
}

/// Finds the route at `/routes/{group}/{name}`, or the 404 response.
#[allow(clippy::result_large_err)]
fn find_route(state: &AdminState, (group, name): &(String, String)) -> Result<RouteConfig, Response> {
    let id = format!("{group}/{name}");
    state
        .app
        .route(&id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, format!("Route « {id} » inconnue.")))
}

async fn update_route(
    State(state): State<Arc<AdminState>>,
    Path(path): Path<(String, String)>,
    Json(input): Json<RouteInput>,
) -> Response {
    let current = match find_route(&state, &path) {
        Ok(route) => route,
        Err(response) => return response,
    };
    let route = input.into_route(Some(current.token.clone()));
    save_route(&state, route, Some(current)).await
}

/// `previous` is the route being replaced, if any.
async fn save_route(state: &AdminState, route: RouteConfig, previous: Option<RouteConfig>) -> Response {
    let previous_id = previous.as_ref().map(RouteConfig::id);
    if let Err(e) = validate_route(&route, &state.app.routes(), previous_id.as_deref()) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    // Keeping the token is only allowed while it goes to the same place.
    if let Some(previous) = &previous
        && route.token == previous.token
        && !same_token_destination(previous, &route)
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Colle le token à nouveau : l’URL du service (schéma, hôte ou port) ou la transmission du token a changé.",
        );
    }
    let id = route.id();
    let reason = match &previous {
        None => format!("ajouter la route « {id} » vers {} (token par {})", route.upstream, auth_label(&route.auth)),
        Some(previous) => change_reason(previous, &route),
    };
    if let Err(response) = require_approval(state, reason).await {
        return response;
    }
    match state.app.upsert_route(route, previous_id.as_deref()) {
        Ok(()) => {
            let view = state.app.route(&id).map(|r| route_view(&state.app, r));
            Json(view).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

async fn delete_route(State(state): State<Arc<AdminState>>, Path(path): Path<(String, String)>) -> Response {
    let route = match find_route(&state, &path) {
        Ok(route) => route,
        Err(response) => return response,
    };
    let id = route.id();
    let reason = format!("supprimer la route « {id} » ({})", origin_of(&route.upstream));
    if let Err(response) = require_approval(&state, reason).await {
        return response;
    }
    match state.app.delete_route(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
struct EnableInput {
    /// `None` keeps the route enabled until it is disabled by hand.
    minutes: Option<u64>,
}

async fn enable_route(
    State(state): State<Arc<AdminState>>,
    Path(path): Path<(String, String)>,
    Json(input): Json<EnableInput>,
) -> Response {
    let route = match find_route(&state, &path) {
        Ok(route) => route,
        Err(response) => return response,
    };
    let id = route.id();
    if input.minutes == Some(0) || input.minutes.is_some_and(|m| m > 24 * 60) {
        return error(StatusCode::BAD_REQUEST, "La durée doit être comprise entre 1 minute et 24 heures.");
    }
    let duration = match input.minutes {
        Some(m) => format!("pendant {m} min"),
        None => "jusqu’à désactivation".to_string(),
    };
    let reason = format!("activer la route « {id} » ({}) {duration}", origin_of(&route.upstream));
    if let Err(response) = require_approval(&state, reason).await {
        return response;
    }
    if let Err(e) = state.app.enable(&id, input.minutes.map(|m| Duration::from_secs(m * 60))) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("Route non activée : {e}"));
    }
    Json(route_view(&state.app, route)).into_response()
}

const NOT_SAVED: &str = "Désactivée, mais l’état n’a pas pu être enregistré : elle serait réactivée au prochain démarrage.";

async fn disable_route(State(state): State<Arc<AdminState>>, Path(path): Path<(String, String)>) -> Response {
    let route = match find_route(&state, &path) {
        Ok(route) => route,
        Err(response) => return response,
    };
    if let Err(e) = state.app.disable(&route.id()) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{NOT_SAVED} ({e})"));
    }
    Json(route_view(&state.app, route)).into_response()
}

async fn disable_all(State(state): State<Arc<AdminState>>) -> Response {
    match state.app.disable_all() {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{NOT_SAVED} ({e})")),
    }
}

/// Reads the Keychain vault again after a failure at startup. Needs no
/// approval: it only restores what was saved, and macOS asks the user before
/// letting the proxy read the Keychain.
async fn reload_vault(State(state): State<Arc<AdminState>>) -> Response {
    let app = state.app.clone();
    match tokio::task::spawn_blocking(move || app.reload_vault()).await {
        Ok(Ok(reloaded)) => Json(json!({ "reloaded": reloaded })).into_response(),
        Ok(Err(e)) => error(StatusCode::SERVICE_UNAVAILABLE, format!("Coffre toujours illisible : {e}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Serves the web UI; unknown paths fall back to the SPA shell.
async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, path) = match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(file) => (file, path),
        None => match Assets::get("_shell.html") {
            Some(file) => (file, "_shell.html"),
            None => return error(StatusCode::NOT_FOUND, "Interface web non construite. Lance scripts/install.sh."),
        },
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .header(header::CONTENT_TYPE, file.metadata.mimetype())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.data.into_owned()))
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Impossible de construire la réponse."))
}
