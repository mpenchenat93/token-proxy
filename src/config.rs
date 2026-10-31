//! Route definitions: where a route forwards to and which credential it
//! injects. A route is reached at `/<group>/<name>/...`, so a call always
//! says whose account it uses (`perso`, a client…). What a route may do is decided by the token's own permissions on
//! the upstream service.

use serde::{Deserialize, Serialize};
use std::fmt;

/// How the credential is added to the upstream request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthInjection {
    /// `Authorization: Bearer <token>`
    Bearer,
    /// `<name>: <token>` (e.g. `x-api-key`)
    Header { name: String },
    /// `?<param>=<token>`
    Query { param: String },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConfig {
    /// URL segment the AI uses: `http://127.0.0.1:8787/<group>/<name>/...`
    pub name: String,
    /// Base URL of the upstream service, e.g. `https://coolify.example.com`
    pub upstream: String,
    pub token: String,
    pub auth: AuthInjection,
    /// Free text shown to the AI when it lists routes.
    pub description: String,
    /// Who the route belongs to (`perso`, a client…). Empty only in vaults
    /// saved before groups existed, until [`crate::vault::load`] migrates them.
    #[serde(default)]
    pub group: String,
}

/// Group of the routes saved before groups existed, and of new routes by default.
pub const DEFAULT_GROUP: &str = "perso";

impl RouteConfig {
    /// `<group>/<name>`: identifies the route, unique across the vault.
    pub fn id(&self) -> String {
        format!("{}/{}", self.group, self.name)
    }
}

fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// Never print the token, even in debug logs.
impl fmt::Debug for RouteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteConfig")
            .field("name", &self.name)
            .field("upstream", &self.upstream)
            .field("token", &"<redacted>")
            .field("auth", &self.auth)
            .field("group", &self.group)
            .finish()
    }
}

/// Checks a route before it is saved. Returns a human-readable error.
/// `editing` is the id of the route being replaced, if any.
pub fn validate_route(route: &RouteConfig, existing: &[RouteConfig], editing: Option<&str>) -> Result<(), String> {
    if !is_slug(&route.name) {
        return Err("Le nom ne peut contenir que a-z, 0-9, « - » et « _ ».".into());
    }
    if !is_slug(&route.group) {
        return Err("Le groupe ne peut contenir que a-z, 0-9, « - » et « _ ».".into());
    }
    let id = route.id();
    if existing.iter().any(|r| r.id() == id && Some(r.id().as_str()) != editing) {
        return Err(format!("Une route « {} » existe déjà dans le groupe « {} ».", route.name, route.group));
    }
    let url = reqwest::Url::parse(&route.upstream).map_err(|e| format!("URL cible invalide : {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("L’URL cible doit être de la forme http(s)://hôte[…].".into());
    }
    if url.query().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("L’URL cible ne doit contenir ni paramètres ni identifiants.".into());
    }
    if route.token.is_empty() {
        return Err("Le token est vide.".into());
    }
    if route.token.contains(['\r', '\n']) {
        return Err("Le token doit tenir sur une seule ligne.".into());
    }
    match &route.auth {
        AuthInjection::Header { name } if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() => {
            return Err("Nom d’en-tête invalide.".into());
        }
        AuthInjection::Query { param } if param.is_empty() => {
            return Err("Le nom du paramètre est vide.".into());
        }
        _ => {}
    }
    Ok(())
}
