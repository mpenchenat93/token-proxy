//! User approval for privileged admin actions (enabling or changing a route).
//!
//! The admin API is plain HTTP on loopback, so anything running on the Mac —
//! the AI included — can call it. Privileged actions therefore require the
//! user to authenticate in a system dialog (Touch ID, or the macOS password),
//! which a program cannot complete on the user's behalf.

use std::time::Duration;

pub trait Approver: Send + Sync {
    /// Blocks until the user approves or refuses. `reason` completes the
    /// system sentence "token-proxy is trying to …", in the macOS language.
    fn approve(&self, reason: &str) -> Result<(), String>;
}

/// Touch ID, or the macOS password when Touch ID is unavailable.
pub struct LocalAuthentication;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

impl Approver for LocalAuthentication {
    fn approve(&self, reason: &str) -> Result<(), String> {
        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_foundation::{NSError, NSString};
        use objc2_local_authentication::{LAContext, LAPolicy};
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else if error.is_null() {
                Err(tr!("échec de l’authentification", "authentication failed"))
            } else {
                // SAFETY: a non-null error pointer passed to the reply block is a valid NSError.
                Err(unsafe { &*error }.localizedDescription().to_string())
            };
            let _ = tx.send(outcome);
        });

        // SAFETY: plain Objective-C calls on a freshly created context; the
        // reply block is retained by LocalAuthentication until it is called.
        unsafe {
            let context = LAContext::new();
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &NSString::from_str(reason),
                &reply,
            );
            let outcome = rx
                .recv_timeout(APPROVAL_TIMEOUT)
                .unwrap_or_else(|_| Err(tr!("pas de réponse à temps", "no answer in time")));
            context.invalidate();
            outcome
        }
    }
}

/// Fixed answer, for tests.
pub struct FixedApprover(pub bool);

impl Approver for FixedApprover {
    fn approve(&self, _reason: &str) -> Result<(), String> {
        if self.0 { Ok(()) } else { Err("refused".to_string()) }
    }
}
