//! Route definitions: where a route forwards to and which credential it
//! injects. A route is reached at `/<group>/<name>/...`, so a call always
//! says whose account it uses (`perso`, a client…). What a route may do is decided by the token's own permissions on
//! the upstream service.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::IpAddr;

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

/// Whether `a` and `b` send the token to the same place, the same way: same
/// scheme, host and port, same injection. Changing any of them requires the
/// token again, so that whoever edits a route without knowing its token (an
/// agent calling the admin API) cannot send it elsewhere.
pub fn same_token_destination(a: &RouteConfig, b: &RouteConfig) -> bool {
    let origin = |r: &RouteConfig| reqwest::Url::parse(&r.upstream).ok().map(|u| u.origin());
    a.auth == b.auth && origin(a).is_some_and(|o| Some(o) == origin(b))
}

/// Loopback, private network or link-local hosts, where plain http does not
/// carry the token over the internet. Names are only trusted when they cannot
/// be public (`localhost`, mDNS `.local`, `.home.arpa`).
fn is_local_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            // 100.64.0.0/10: carrier-grade NAT, used by Tailscale.
            let shared = ip.octets()[0] == 100 && ip.octets()[1] & 0xc0 == 64;
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || shared
        }
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
        Err(_) => {
            host == "localhost" || [".localhost", ".local", ".home.arpa"].iter().any(|s| host.ends_with(s))
        }
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
        return Err(tr!("Le nom ne peut contenir que a-z, 0-9, « - » et « _ ».", "The name may only contain a-z, 0-9, “-” and “_”."));
    }
    if !is_slug(&route.group) {
        return Err(tr!("Le groupe ne peut contenir que a-z, 0-9, « - » et « _ ».", "The group may only contain a-z, 0-9, “-” and “_”."));
    }
    let id = route.id();
    if existing.iter().any(|r| r.id() == id && Some(r.id().as_str()) != editing) {
        let (name, group) = (&route.name, &route.group);
        return Err(tr!(
            "Une route « {name} » existe déjà dans le groupe « {group} ».",
            "A route “{name}” already exists in the group “{group}”."
        ));
    }
    let url = reqwest::Url::parse(&route.upstream).map_err(|e| tr!("URL cible invalide : {e}", "Invalid service URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(tr!("L’URL cible doit être de la forme http(s)://hôte[…].", "The service URL must look like http(s)://host[…]."));
    }
    if url.scheme() == "http" && !is_local_host(&url) {
        return Err(tr!(
            "http:// n’est accepté que vers une adresse locale (localhost, réseau privé) : ailleurs, le token passerait en clair. Utilise https://.",
            "http:// is only accepted for a local address (localhost, private network): elsewhere the token would travel in clear. Use https://."
        ));
    }
    if url.query().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(tr!("L’URL cible ne doit contenir ni paramètres ni identifiants.", "The service URL must contain neither parameters nor credentials."));
    }
    if route.token.is_empty() {
        return Err(tr!("Le token est vide.", "The token is empty."));
    }
    if route.token.contains(['\r', '\n']) {
        return Err(tr!("Le token doit tenir sur une seule ligne.", "The token must fit on one line."));
    }
    match &route.auth {
        AuthInjection::Header { name } if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() => {
            return Err(tr!("Nom d’en-tête invalide.", "Invalid header name."));
        }
        AuthInjection::Query { param } if param.is_empty() => {
            return Err(tr!("Le nom du paramètre est vide.", "The parameter name is empty."));
        }
        _ => {}
    }
    Ok(())
}
