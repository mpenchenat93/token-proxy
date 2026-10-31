use std::sync::Arc;
use token_proxy::admin::{self, AdminState};
use token_proxy::approval::LocalAuthentication;
use token_proxy::state::{AppState, ServerStatus, AUDIT_RETENTION};
use token_proxy::{proxy, vault};

const DEFAULT_PROXY_PORT: u16 = 8787;
const DEFAULT_ADMIN_PORT: u16 = 8788;

fn port_from_env(name: &str, default: u16) -> u16 {
    std::env::var(name).ok().and_then(|p| p.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();

    let proxy_port = port_from_env("TOKEN_PROXY_PORT", DEFAULT_PROXY_PORT);
    let admin_port = port_from_env("TOKEN_PROXY_ADMIN_PORT", DEFAULT_ADMIN_PORT);

    let audit_file = std::env::var_os("HOME").map(|home| {
        let dir = std::path::Path::new(&home).join("Library/Logs/token-proxy");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("cannot create {}: {e}", dir.display());
        }
        dir.join("audit.log")
    });

    let state = Arc::new(AppState::new(Box::new(vault::KeychainStore), audit_file));
    if let Some(e) = state.vault_error() {
        tracing::error!("vault not loaded, saving is disabled: {e}");
        // At login the Keychain may refuse access before the desktop can show
        // its prompt: try again a few times, then leave it to the UI button.
        let retry_state = state.clone();
        tokio::spawn(async move {
            for delay in [30, 120, 600] {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                let state = retry_state.clone();
                match tokio::task::spawn_blocking(move || state.reload_vault()).await {
                    Ok(Ok(_)) => {
                        tracing::info!("vault loaded on retry");
                        return;
                    }
                    Ok(Err(e)) => tracing::warn!("vault retry failed: {e}"),
                    Err(e) => tracing::warn!("vault retry task failed: {e}"),
                }
            }
        });
    }

    // Keeps the audit log to its retention period: at startup, then daily.
    let pruning_state = state.clone();
    tokio::spawn(async move {
        let mut daily = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        loop {
            daily.tick().await;
            match pruning_state.prune_audit_file(AUDIT_RETENTION) {
                Ok(0) => {}
                Ok(n) => tracing::info!("audit log: {n} expired entries removed"),
                Err(e) => tracing::warn!("cannot prune the audit log: {e}"),
            }
        }
    });

    // Loopback only: neither server must ever be reachable from the network.
    let proxy_listener = tokio::net::TcpListener::bind(("127.0.0.1", proxy_port)).await;
    let proxy_task = match proxy_listener {
        Ok(listener) => {
            let addr = listener.local_addr()?;
            tracing::info!("proxy listening on http://{addr}");
            *state.server_status.lock() = ServerStatus::Listening(addr);
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = proxy::serve(listener, state.clone()).await {
                    *state.server_status.lock() = ServerStatus::Failed(e.to_string());
                }
            })
        }
        Err(e) => anyhow::bail!("cannot listen on 127.0.0.1:{proxy_port}: {e}"),
    };

    let admin_listener = tokio::net::TcpListener::bind(("127.0.0.1", admin_port))
        .await
        .map_err(|e| anyhow::anyhow!("cannot listen on 127.0.0.1:{admin_port}: {e}"))?;
    tracing::info!("admin UI on http://127.0.0.1:{admin_port}");
    let admin_state = Arc::new(AdminState::new(state, Arc::new(LocalAuthentication), admin_port));
    axum::serve(admin_listener, admin::router(admin_state)).await?;

    proxy_task.abort();
    Ok(())
}
