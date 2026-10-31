//! Shared state between the proxy server and the admin window.

use crate::config::RouteConfig;
use crate::vault::{self, SecretStore, VaultData};
use chrono::{DateTime, Local};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const AUDIT_CAPACITY: usize = 500;

/// How long entries are kept in the audit log file.
pub const AUDIT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Debug)]
pub enum ServerStatus {
    Starting,
    Listening(SocketAddr),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    Disabled,
    /// `None` means "until disabled by hand". Wall-clock time, not
    /// `Instant`: on macOS an `Instant` stops while the Mac sleeps, which
    /// would keep a route enabled longer than the user chose.
    Enabled { until: Option<SystemTime> },
}

#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub at: DateTime<Local>,
    /// `<group>/<name>`, as requested (the route may not exist).
    pub route: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub forwarded: bool,
    pub note: String,
}

pub struct AppState {
    store: Box<dyn SecretStore>,
    routes: RwLock<Vec<RouteConfig>>,
    /// Enabled routes, by id (`<group>/<name>`), and when they expire
    /// (`None`: until disabled by hand). Saved in the vault so they survive a restart; keeping them in
    /// the Keychain means no program can enable a route by editing a file.
    activations: Mutex<HashMap<String, Option<SystemTime>>>,
    audit: Mutex<VecDeque<AuditEntry>>,
    audit_file: Option<PathBuf>,
    /// Serializes appends to the audit file with its pruning.
    audit_file_lock: Mutex<()>,
    /// Set when the vault could not be read; saving is then refused so that
    /// an unreadable vault is never overwritten by an empty one. Cleared by
    /// a successful [`AppState::reload_vault`].
    vault_error: Mutex<Option<String>>,
    /// One vault reload at a time (each may show a Keychain prompt).
    reload_lock: Mutex<()>,
    pub server_status: Mutex<ServerStatus>,
    pub client: reqwest::Client,
}

impl AppState {
    pub fn new(store: Box<dyn SecretStore>, audit_file: Option<PathBuf>) -> Self {
        let (routes, activations, vault_error) = match vault::load(store.as_ref()) {
            Ok(data) => {
                let activations = from_stored(data.activations, &data.routes);
                (data.routes, activations, None)
            }
            Err(e) => (Vec::new(), HashMap::new(), Some(format!("{e:#}"))),
        };
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("cannot build the HTTP client");
        Self {
            store,
            routes: RwLock::new(routes),
            activations: Mutex::new(activations),
            audit: Mutex::new(VecDeque::with_capacity(AUDIT_CAPACITY)),
            audit_file,
            audit_file_lock: Mutex::new(()),
            vault_error: Mutex::new(vault_error),
            reload_lock: Mutex::new(()),
            server_status: Mutex::new(ServerStatus::Starting),
            client,
        }
    }

    pub fn vault_error(&self) -> Option<String> {
        self.vault_error.lock().clone()
    }

    /// Reads the vault again after it could not be loaded (for instance when
    /// the Keychain refused access at login). Returns `Ok(false)` when the
    /// vault was already loaded: in-memory state is then left untouched.
    /// Blocks while macOS shows its Keychain prompt.
    pub fn reload_vault(&self) -> Result<bool, String> {
        let Some(_guard) = self.reload_lock.try_lock() else {
            return Err("Une lecture du Trousseau est déjà en cours.".to_string());
        };
        if self.vault_error.lock().is_none() {
            return Ok(false);
        }
        match vault::load(self.store.as_ref()) {
            Ok(data) => {
                let mut routes = self.routes.write();
                let mut activations = self.activations.lock();
                *activations = from_stored(data.activations, &data.routes);
                *routes = data.routes;
                *self.vault_error.lock() = None;
                Ok(true)
            }
            Err(e) => {
                let message = format!("{e:#}");
                *self.vault_error.lock() = Some(message.clone());
                Err(message)
            }
        }
    }

    pub fn routes(&self) -> Vec<RouteConfig> {
        self.routes.read().clone()
    }

    /// Finds a route by id (`<group>/<name>`).
    pub fn route(&self, id: &str) -> Option<RouteConfig> {
        self.routes.read().iter().find(|r| r.id() == id).cloned()
    }

    /// Adds a route, or replaces the one whose id is `previous_id`.
    pub fn upsert_route(&self, route: RouteConfig, previous_id: Option<&str>) -> Result<(), String> {
        let mut routes = self.routes.write();
        crate::config::validate_route(&route, &routes, previous_id)?;
        let mut updated = routes.clone();
        match previous_id.and_then(|id| updated.iter().position(|r| r.id() == id)) {
            Some(i) => updated[i] = route,
            None => updated.push(route),
        }
        let mut activations = self.activations.lock();
        let mut updated_activations = activations.clone();
        // A modified route must be enabled again explicitly.
        if let Some(id) = previous_id {
            updated_activations.remove(id);
        }
        self.persist(&updated, &updated_activations)?;
        *routes = updated;
        *activations = updated_activations;
        Ok(())
    }

    pub fn delete_route(&self, id: &str) -> Result<(), String> {
        let mut routes = self.routes.write();
        let updated: Vec<RouteConfig> = routes.iter().filter(|r| r.id() != id).cloned().collect();
        let mut activations = self.activations.lock();
        let mut updated_activations = activations.clone();
        updated_activations.remove(id);
        self.persist(&updated, &updated_activations)?;
        *routes = updated;
        *activations = updated_activations;
        Ok(())
    }

    /// Callers lock `routes` before `activations`, always in that order.
    fn persist(&self, routes: &[RouteConfig], activations: &HashMap<String, Option<SystemTime>>) -> Result<(), String> {
        if let Some(e) = self.vault_error.lock().as_ref() {
            return Err(format!("Le coffre n’a pas pu être chargé, l’enregistrement est désactivé : {e}"));
        }
        let data = VaultData { routes: routes.to_vec(), activations: to_stored(activations) };
        vault::save(self.store.as_ref(), &data).map_err(|e| format!("{e:#}"))
    }

    /// Enables a route only once the activation is saved.
    pub fn enable(&self, id: &str, duration: Option<Duration>) -> Result<(), String> {
        let routes = self.routes.read();
        let mut activations = self.activations.lock();
        let mut updated = activations.clone();
        updated.insert(id.to_string(), duration.map(|d| SystemTime::now() + d));
        self.persist(&routes, &updated)?;
        *activations = updated;
        Ok(())
    }

    /// Disables a route right away; the error only means the change could
    /// not be saved, so the route would be enabled again after a restart.
    pub fn disable(&self, id: &str) -> Result<(), String> {
        let routes = self.routes.read();
        let mut activations = self.activations.lock();
        activations.remove(id);
        self.persist(&routes, &activations)
    }

    /// Same as [`Self::disable`], for every route.
    pub fn disable_all(&self) -> Result<(), String> {
        let routes = self.routes.read();
        let mut activations = self.activations.lock();
        activations.clear();
        self.persist(&routes, &activations)
    }

    pub fn activation(&self, id: &str) -> Activation {
        let mut activations = self.activations.lock();
        match activations.get(id) {
            None => Activation::Disabled,
            Some(Some(until)) if *until <= SystemTime::now() => {
                activations.remove(id);
                Activation::Disabled
            }
            Some(until) => Activation::Enabled { until: *until },
        }
    }

    pub fn record(&self, entry: AuditEntry) {
        if let Some(path) = &self.audit_file {
            let line = format!(
                "{} {} {} {} {} {}\n",
                entry.at.to_rfc3339(),
                entry.route,
                entry.method,
                entry.path,
                entry.status,
                entry.note
            );
            let _guard = self.audit_file_lock.lock();
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(line.as_bytes()));
            if let Err(e) = written {
                tracing::warn!("cannot write audit log {}: {e}", path.display());
            }
        }
        let mut audit = self.audit.lock();
        if audit.len() == AUDIT_CAPACITY {
            audit.pop_front();
        }
        audit.push_back(entry);
    }

    /// Most recent first.
    pub fn audit_entries(&self) -> Vec<AuditEntry> {
        self.audit.lock().iter().rev().cloned().collect()
    }

    /// Removes the audit file entries older than `retention` and returns how
    /// many were removed. Lines without a readable date are kept.
    pub fn prune_audit_file(&self, retention: Duration) -> std::io::Result<usize> {
        let Some(path) = &self.audit_file else { return Ok(0) };
        let _guard = self.audit_file_lock.lock();
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let retention = chrono::Duration::from_std(retention).map_err(std::io::Error::other)?;
        let cutoff = (Local::now() - retention).fixed_offset();
        let mut removed = 0;
        let kept: String = content
            .lines()
            .filter(|line| {
                let at = line.split(' ').next().and_then(|t| DateTime::parse_from_rfc3339(t).ok());
                let keep = at.is_none_or(|at| at >= cutoff);
                removed += usize::from(!keep);
                keep
            })
            .map(|line| format!("{line}\n"))
            .collect();
        if removed > 0 {
            // Write aside then rename, so a crash never leaves a truncated log.
            let tmp = path.with_extension("log.tmp");
            std::fs::write(&tmp, kept)?;
            std::fs::rename(&tmp, path)?;
        }
        Ok(removed)
    }
}

/// Expirations as saved in the vault: milliseconds since the Unix epoch, or
/// `None` for "until disabled". Expired activations are dropped.
fn to_stored(activations: &HashMap<String, Option<SystemTime>>) -> HashMap<String, Option<u64>> {
    let now = SystemTime::now();
    activations
        .iter()
        .filter_map(|(id, until)| match until {
            None => Some((id.clone(), None)),
            Some(until) if *until > now => {
                let at = until.duration_since(UNIX_EPOCH).ok()?;
                Some((id.clone(), Some(at.as_millis() as u64)))
            }
            Some(_) => None,
        })
        .collect()
}

/// Inverse of [`to_stored`], skipping expired activations and unknown routes.
fn from_stored(stored: HashMap<String, Option<u64>>, routes: &[RouteConfig]) -> HashMap<String, Option<SystemTime>> {
    let now = SystemTime::now();
    stored
        .into_iter()
        .filter(|(id, _)| routes.iter().any(|r| &r.id() == id))
        .filter_map(|(id, at)| match at {
            None => Some((id, None)),
            Some(ms) => {
                let at = UNIX_EPOCH + Duration::from_millis(ms);
                (at > now).then_some((id, Some(at)))
            }
        })
        .collect()
}
