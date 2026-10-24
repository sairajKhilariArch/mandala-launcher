//! Microkernel-style IPC broker: every message between the host and modules,
//! or between modules, passes through an ACL check before it is queued.
//!
//! NOTE: this is *routing* policy, not memory isolation. Modules are
//! in-process shared libraries; the Ed25519 signature check in
//! [`crate::loader`] is what decides whether code is trusted at all.

use crate::error::{MandalaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Reserved participant id of the host itself.
pub const HOST: &str = "host";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IpcEnvelope {
    pub message_id: String,
    pub sender_module: String,
    pub target_module: String,
    pub payload_action: String,
    pub data_json: String,
}

/// `sender -> set of targets it may address`. The host is always trusted as a
/// sender; any module may always address the host.
#[derive(Clone, Debug, Default)]
pub struct AccessControlList {
    pub allowed_routes: HashMap<String, HashSet<String>>,
}

impl AccessControlList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(&mut self, sender: &str, target: &str) -> &mut Self {
        self.allowed_routes
            .entry(sender.to_string())
            .or_default()
            .insert(target.to_string());
        self
    }

    pub fn permits(&self, sender: &str, target: &str) -> bool {
        sender == HOST
            || target == HOST
            || self
                .allowed_routes
                .get(sender)
                .map(|t| t.contains(target))
                .unwrap_or(false)
    }

    /// Default policy: modules are isolated from each other except for the
    /// explicit routes below.
    pub fn default_policy() -> Self {
        let mut acl = Self::new();
        // Calculator may append results to Notes ("save" action).
        acl.allow("app-calculator", "app-notes");
        acl
    }
}

pub struct MicrokernelIpcBroker {
    acl: AccessControlList,
    tx: mpsc::Sender<IpcEnvelope>,
    next_id: AtomicU64,
}

impl MicrokernelIpcBroker {
    /// Returns the broker and the receiving end consumed by the router.
    pub fn new(buffer_size: usize, acl: AccessControlList) -> (Self, mpsc::Receiver<IpcEnvelope>) {
        let (tx, rx) = mpsc::channel(buffer_size.max(1));
        (Self { acl, tx, next_id: AtomicU64::new(1) }, rx)
    }

    pub fn acl(&self) -> &AccessControlList {
        &self.acl
    }

    pub fn envelope(&self, sender: &str, target: &str, action: &str, data_json: &str) -> IpcEnvelope {
        IpcEnvelope {
            message_id: format!("m{}", self.next_id.fetch_add(1, Ordering::Relaxed)),
            sender_module: sender.to_string(),
            target_module: target.to_string(),
            payload_action: action.to_string(),
            data_json: data_json.to_string(),
        }
    }

    fn check(&self, e: &IpcEnvelope) -> Result<()> {
        if self.acl.permits(&e.sender_module, &e.target_module) {
            Ok(())
        } else {
            Err(MandalaError::AclViolation {
                sender: e.sender_module.clone(),
                target: e.target_module.clone(),
            })
        }
    }

    /// Async dispatch (waits for queue space).
    pub async fn dispatch(&self, envelope: IpcEnvelope) -> Result<()> {
        self.check(&envelope)?;
        self.tx.send(envelope).await.map_err(|_| MandalaError::IpcChannel)
    }

    /// Non-blocking dispatch, safe to call from FFI callbacks and UI threads.
    pub fn try_dispatch(&self, envelope: IpcEnvelope) -> Result<()> {
        self.check(&envelope)?;
        self.tx.try_send(envelope).map_err(|_| MandalaError::IpcChannel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn host_and_allowed_routes_pass() {
        let (b, mut rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
        b.dispatch(b.envelope(HOST, "app-notes", "launch", "")).await.unwrap();
        b.dispatch(b.envelope("app-calculator", "app-notes", "append", "x")).await.unwrap();
        b.dispatch(b.envelope("app-notes", HOST, "reply", "{}")).await.unwrap();
        assert_eq!(rx.recv().await.unwrap().target_module, "app-notes");
        assert_eq!(rx.recv().await.unwrap().sender_module, "app-calculator");
        assert_eq!(rx.recv().await.unwrap().target_module, HOST);
    }

    #[tokio::test]
    async fn unlisted_route_is_denied() {
        let (b, _rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
        let err = b.dispatch(b.envelope("app-notes", "app-calculator", "eval", "1")).await;
        assert!(matches!(err, Err(MandalaError::AclViolation { .. })));
        // Unknown sender is denied too.
        assert!(b.try_dispatch(b.envelope("evil", "app-notes", "append", "x")).is_err());
    }

    #[test]
    fn full_queue_reports_error() {
        let (b, _rx) = MicrokernelIpcBroker::new(1, AccessControlList::new());
        b.try_dispatch(b.envelope(HOST, "a", "x", "")).unwrap();
        assert!(matches!(
            b.try_dispatch(b.envelope(HOST, "a", "x", "")),
            Err(MandalaError::IpcChannel)
        ));
    }

    #[test]
    fn message_ids_are_unique() {
        let (b, _rx) = MicrokernelIpcBroker::new(1, AccessControlList::new());
        assert_ne!(b.envelope(HOST, "a", "x", "").message_id, b.envelope(HOST, "a", "x", "").message_id);
    }
}
