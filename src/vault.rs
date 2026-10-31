//! Persistent storage of the routes, tokens included.
//!
//! Everything (upstream URLs too) lives in a single macOS Keychain item, so
//! nothing that runs as the user can read a token or point a route at another
//! host by editing a file: the Keychain asks the user before letting any other
//! program read or change the item.

use crate::config::{RouteConfig, DEFAULT_GROUP};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Serialize, Deserialize)]
pub struct VaultData {
    pub routes: Vec<RouteConfig>,
    /// Enabled routes, by id (`<group>/<name>`): expiry in milliseconds
    /// since the Unix epoch, or `None` for "until disabled".
    #[serde(default)]
    pub activations: HashMap<String, Option<u64>>,
}

pub trait SecretStore: Send + Sync {
    /// Returns `None` when nothing has been stored yet.
    fn load(&self) -> Result<Option<Vec<u8>>>;
    fn save(&self, data: &[u8]) -> Result<()>;
}

pub fn load(store: &dyn SecretStore) -> Result<VaultData> {
    match store.load()? {
        None => Ok(VaultData::default()),
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(migrate)
            .with_context(|| tr!("le contenu du coffre n’est pas du JSON valide", "the vault content is not valid JSON")),
    }
}

/// Moves the routes saved before groups existed to [`DEFAULT_GROUP`]. Their
/// `-perso` suffix is dropped, the group now saying it (`coolify-perso` is
/// reached at `/perso/coolify`), unless that would clash with another route.
/// Activations, then keyed by name, follow their route. Changes nothing on a
/// vault saved with groups; the result is saved with the next change.
fn migrate(mut data: VaultData) -> VaultData {
    let mut renamed: HashMap<String, String> = HashMap::new();
    for i in 0..data.routes.len() {
        if !data.routes[i].group.is_empty() {
            continue;
        }
        let old_name = data.routes[i].name.clone();
        let short = match old_name.strip_suffix("-perso") {
            Some(short) => short.to_string(),
            None => old_name.replacen("-perso-", "-", 1),
        };
        let taken = |name: &str| {
            data.routes
                .iter()
                .enumerate()
                .any(|(j, r)| j != i && (r.group.is_empty() || r.group == DEFAULT_GROUP) && r.name == name)
        };
        let name = if short.is_empty() || taken(&short) { old_name.clone() } else { short };
        let route = &mut data.routes[i];
        route.group = DEFAULT_GROUP.to_string();
        route.name = name;
        renamed.insert(old_name, route.id());
    }
    if !renamed.is_empty() {
        data.activations = data
            .activations
            .into_iter()
            .filter_map(|(key, until)| match renamed.get(&key) {
                Some(id) => Some((id.clone(), until)),
                None if key.contains('/') => Some((key, until)),
                None => None,
            })
            .collect();
    }
    data
}

pub fn save(store: &dyn SecretStore, data: &VaultData) -> Result<()> {
    store.save(&serde_json::to_vec(data)?)
}

const SERVICE: &str = "token-proxy";
const ACCOUNT: &str = "vault";
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct KeychainStore;

impl SecretStore for KeychainStore {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(anyhow::anyhow!(tr!("lecture du Trousseau impossible : {e}", "cannot read the Keychain: {e}"))),
        }
    }

    fn save(&self, data: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, data)
            .map_err(|e| anyhow::anyhow!(tr!("écriture dans le Trousseau impossible : {e}", "cannot write to the Keychain: {e}")))
    }
}

impl<T: SecretStore + ?Sized> SecretStore for Arc<T> {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        (**self).load()
    }

    fn save(&self, data: &[u8]) -> Result<()> {
        (**self).save(data)
    }
}

/// In-memory store, for tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<Option<Vec<u8>>>);

impl SecretStore for MemoryStore {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.0.lock().clone())
    }

    fn save(&self, data: &[u8]) -> Result<()> {
        *self.0.lock() = Some(data.to_vec());
        Ok(())
    }
}
