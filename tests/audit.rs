use chrono::{Duration, Local};
use std::path::PathBuf;
use token_proxy::state::{AppState, AuditEntry, AUDIT_RETENTION};
use token_proxy::vault::MemoryStore;

fn temp_log(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("token-proxy-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("audit.log")
}

fn line(days_ago: i64, path: &str) -> String {
    let at = (Local::now() - Duration::days(days_ago)).to_rfc3339();
    format!("{at} coolify GET {path} 200 12 ms\n")
}

#[test]
fn prune_keeps_only_the_retention_period() {
    let path = temp_log("prune");
    let content = [
        line(45, "/old"),
        line(31, "/expired"),
        "not a dated line\n".to_string(),
        line(29, "/recent"),
        line(0, "/today"),
    ]
    .concat();
    std::fs::write(&path, content).unwrap();

    let state = AppState::new(Box::new(MemoryStore::default()), Some(path.clone()));
    assert_eq!(state.prune_audit_file(AUDIT_RETENTION).unwrap(), 2);

    let kept = std::fs::read_to_string(&path).unwrap();
    assert!(!kept.contains("/old") && !kept.contains("/expired"));
    assert!(kept.contains("/recent") && kept.contains("/today") && kept.contains("not a dated line"));

    // Nothing left to remove; new entries are still appended.
    assert_eq!(state.prune_audit_file(AUDIT_RETENTION).unwrap(), 0);
    state.record(AuditEntry {
        at: Local::now(),
        route: "coolify".into(),
        method: "GET".into(),
        path: "/after-prune".into(),
        status: 200,
        forwarded: true,
        note: "1 ms".into(),
    });
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("/after-prune 200 1 ms\n"));

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn prune_without_file_is_a_no_op() {
    let path = temp_log("missing");
    let state = AppState::new(Box::new(MemoryStore::default()), Some(path.clone()));
    assert_eq!(state.prune_audit_file(AUDIT_RETENTION).unwrap(), 0);
    assert!(!path.exists());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
