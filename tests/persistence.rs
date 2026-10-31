//! Activations must survive a restart, simulated by building a new
//! `AppState` on the same vault.

use std::sync::Arc;
use std::time::Duration;
use token_proxy::config::{AuthInjection, RouteConfig};
use token_proxy::state::{Activation, AppState};
use token_proxy::vault::{self, MemoryStore, SecretStore};

fn route(name: &str) -> RouteConfig {
    RouteConfig {
        name: name.into(),
        upstream: "https://coolify.example.com".into(),
        token: "secret".into(),
        auth: AuthInjection::Bearer,
        description: String::new(),
        group: "perso".into(),
    }
}

fn setup() -> (Arc<MemoryStore>, AppState) {
    let store = Arc::new(MemoryStore::default());
    let state = AppState::new(Box::new(store.clone()), None);
    for name in ["forever", "timed", "off"] {
        state.upsert_route(route(name), None).unwrap();
    }
    (store, state)
}

fn restart(store: &Arc<MemoryStore>) -> AppState {
    AppState::new(Box::new(store.clone()), None)
}

#[test]
fn activations_survive_a_restart() {
    let (store, state) = setup();
    state.enable("perso/forever", None).unwrap();
    state.enable("perso/timed", Some(Duration::from_secs(600))).unwrap();
    drop(state);

    let restarted = restart(&store);
    assert_eq!(restarted.activation("perso/forever"), Activation::Enabled { until: None });
    match restarted.activation("perso/timed") {
        Activation::Enabled { until: Some(until) } => {
            let left = until.duration_since(std::time::SystemTime::now()).unwrap_or_default();
            assert!(left > Duration::from_secs(590) && left <= Duration::from_secs(600), "{left:?}");
        }
        other => panic!("expected a timed activation, got {other:?}"),
    }
    assert_eq!(restarted.activation("perso/off"), Activation::Disabled);
}

#[test]
fn disabling_and_expiry_are_kept_after_a_restart() {
    let (store, state) = setup();
    state.enable("perso/forever", None).unwrap();
    state.enable("perso/timed", Some(Duration::from_millis(20))).unwrap();
    state.disable("perso/forever").unwrap();
    std::thread::sleep(Duration::from_millis(40));
    drop(state);

    let restarted = restart(&store);
    assert_eq!(restarted.activation("perso/forever"), Activation::Disabled);
    assert_eq!(restarted.activation("perso/timed"), Activation::Disabled);
}

#[test]
fn disable_all_and_route_changes_are_saved() {
    let (store, state) = setup();
    state.enable("perso/forever", None).unwrap();
    state.enable("perso/timed", None).unwrap();
    state.enable("perso/off", None).unwrap();
    state.disable_all().unwrap();
    state.enable("perso/forever", None).unwrap();
    state.enable("perso/timed", None).unwrap();
    // Editing a route disables it; deleting one forgets its activation.
    state.upsert_route(route("forever"), Some("perso/forever")).unwrap();
    state.delete_route("perso/timed").unwrap();
    drop(state);

    let restarted = restart(&store);
    assert_eq!(restarted.activation("perso/forever"), Activation::Disabled);
    assert_eq!(restarted.activation("perso/off"), Activation::Disabled);
    assert!(vault::load(store.as_ref()).unwrap().activations.is_empty());
}

/// Fails to load until `fail` is cleared, like a Keychain refusing access at login.
struct FlakyStore {
    inner: Arc<MemoryStore>,
    fail: std::sync::atomic::AtomicBool,
}

impl token_proxy::vault::SecretStore for FlakyStore {
    fn load(&self) -> anyhow::Result<Option<Vec<u8>>> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("Unable to obtain authorization for this operation.");
        }
        self.inner.load()
    }

    fn save(&self, data: &[u8]) -> anyhow::Result<()> {
        self.inner.save(data)
    }
}

#[test]
fn vault_can_be_reloaded_after_a_keychain_refusal() {
    let (store, state) = setup();
    state.enable("perso/forever", None).unwrap();
    drop(state);

    let flaky = Arc::new(FlakyStore { inner: store.clone(), fail: true.into() });
    let restarted = AppState::new(Box::new(flaky.clone()), None);
    assert!(restarted.vault_error().is_some());
    assert!(restarted.routes().is_empty());
    // Nothing is saved while the vault is unreadable, so it is never overwritten.
    assert!(restarted.upsert_route(route("new"), None).is_err());
    assert!(restarted.reload_vault().is_err());

    flaky.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(restarted.reload_vault(), Ok(true));
    assert!(restarted.vault_error().is_none());
    assert_eq!(restarted.routes().len(), 3);
    assert_eq!(restarted.activation("perso/forever"), Activation::Enabled { until: None });
    // Once loaded, a reload leaves the in-memory state alone.
    assert_eq!(restarted.reload_vault(), Ok(false));
}

#[test]
fn routes_saved_before_groups_move_to_perso() {
    let legacy = serde_json::json!({
        "routes": [
            { "name": "coolify-perso", "upstream": "https://c.example.com", "token": "t1", "auth": "Bearer", "description": "" },
            { "name": "coolify-perso-root", "upstream": "https://c.example.com", "token": "t2", "auth": "Bearer", "description": "" },
            { "name": "linkedin-dma", "upstream": "https://l.example.com", "token": "t3", "auth": "Bearer", "description": "" },
            // Would clash with "coolify-perso" once shortened: keeps its name.
            { "name": "coolify", "upstream": "https://c.example.com", "token": "t4", "auth": "Bearer", "description": "" }
        ],
        "activations": { "coolify-perso": null, "linkedin-dma": null, "coolify-perso-root": 1 }
    });
    let store = Arc::new(MemoryStore::default());
    store.save(&serde_json::to_vec(&legacy).unwrap()).unwrap();

    let state = restart(&store);
    let ids: Vec<String> = state.routes().iter().map(|r| r.id()).collect();
    assert_eq!(ids, ["perso/coolify-perso", "perso/coolify-root", "perso/linkedin-dma", "perso/coolify"]);
    // Activations follow their route; expired ones are dropped.
    assert_eq!(state.activation("perso/coolify-perso"), Activation::Enabled { until: None });
    assert_eq!(state.activation("perso/linkedin-dma"), Activation::Enabled { until: None });
    assert_eq!(state.activation("perso/coolify-root"), Activation::Disabled);
    assert_eq!(state.activation("perso/coolify"), Activation::Disabled);

    // Saved with groups on the next change, and loaded unchanged afterwards.
    state.disable("perso/linkedin-dma").unwrap();
    drop(state);
    let saved = vault::load(store.as_ref()).unwrap();
    assert!(saved.routes.iter().all(|r| r.group == "perso"));
    let restarted = restart(&store);
    assert_eq!(restarted.activation("perso/coolify-perso"), Activation::Enabled { until: None });
}

/// Every save waits while the test holds `hold`, like a Keychain prompt.
#[derive(Default)]
struct SlowStore {
    data: MemoryStore,
    hold: std::sync::Mutex<()>,
}

impl SecretStore for SlowStore {
    fn load(&self) -> anyhow::Result<Option<Vec<u8>>> {
        self.data.load()
    }
    fn save(&self, data: &[u8]) -> anyhow::Result<()> {
        drop(self.hold.lock().unwrap());
        self.data.save(data)
    }
}

/// Reads an activation from another thread, failing instead of hanging if
/// it has to wait.
fn activation_now(state: &Arc<AppState>, id: &'static str) -> Activation {
    let (tx, rx) = std::sync::mpsc::channel();
    let state = state.clone();
    std::thread::spawn(move || tx.send(state.activation(id)));
    rx.recv_timeout(Duration::from_secs(1)).expect("reading the state waited for the save")
}

#[test]
fn a_pending_save_blocks_neither_requests_nor_disabling() {
    let store = Arc::new(SlowStore::default());
    let state = Arc::new(AppState::new(Box::new(store.clone()), None));
    for name in ["a", "b"] {
        state.upsert_route(route(name), None).unwrap();
    }
    state.enable("perso/a", None).unwrap();

    let held = store.hold.lock().unwrap();
    let enabling = std::thread::spawn({
        let state = state.clone();
        move || state.enable("perso/b", None)
    });
    std::thread::sleep(Duration::from_millis(50));
    // Requests still read the state while the save waits.
    assert_eq!(activation_now(&state, "perso/a"), Activation::Enabled { until: None });

    let disabling = std::thread::spawn({
        let state = state.clone();
        move || state.disable_all()
    });
    std::thread::sleep(Duration::from_millis(50));
    // Disabling applies at once, before its own save.
    assert_eq!(state.activation("perso/a"), Activation::Disabled);

    drop(held);
    assert!(enabling.join().unwrap().is_err(), "an activation saved during a disable must not apply");
    disabling.join().unwrap().unwrap();

    // Same while a route is being edited.
    state.enable("perso/a", None).unwrap();
    let held = store.hold.lock().unwrap();
    let editing = std::thread::spawn({
        let state = state.clone();
        move || state.upsert_route(route("b"), Some("perso/b"))
    });
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(activation_now(&state, "perso/a"), Activation::Enabled { until: None });
    drop(held);
    editing.join().unwrap().unwrap();
    assert_eq!(state.activation("perso/b"), Activation::Disabled);

    let restarted = AppState::new(Box::new(store.clone()), None);
    assert_eq!(restarted.activation("perso/a"), Activation::Enabled { until: None });
    assert_eq!(restarted.activation("perso/b"), Activation::Disabled);
}
