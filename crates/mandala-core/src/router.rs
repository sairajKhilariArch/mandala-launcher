//! Message router: consumes the broker queue, runs module calls on the
//! blocking pool, and posts replies back through the broker (so replies are
//! ACL-checked like everything else).

use crate::ipc_bus::{IpcEnvelope, MicrokernelIpcBroker, HOST};
use crate::loader::{describe_error, reply_envelope, ModuleHost};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;

/// Run until the channel closes. `on_host` receives every envelope addressed
/// to the host (module replies, errors, and unsolicited module notices).
pub async fn run_router<F>(mut rx: Receiver<IpcEnvelope>, broker: Arc<MicrokernelIpcBroker>, host: Arc<ModuleHost>, on_host: F)
where
    F: Fn(IpcEnvelope) + Send + Sync + 'static,
{
    let on_host = Arc::new(on_host);
    while let Some(env) = rx.recv().await {
        if env.target_module == HOST {
            on_host(env);
            continue;
        }
        let (broker, host, on_host) = (broker.clone(), host.clone(), on_host.clone());
        tokio::spawn(async move {
            let request = env.clone();
            let outcome = tokio::task::spawn_blocking(move || host.call(&env.target_module, &env.payload_action, &env.data_json)).await;

            // `delivered == false` means a host-side failure (not installed,
            // quarantined, load error). A module answering `ok: false` is
            // still a normal reply carrying its own error text.
            let (ok, json_text) = match outcome {
                Ok(Ok(reply)) => (true, reply.to_json()),
                Ok(Err(e)) => (false, json!({ "ok": false, "text": describe_error(&e) }).to_string()),
                Err(join_err) => (false, json!({ "ok": false, "text": format!("module task failed: {join_err}") }).to_string()),
            };

            // Only the host can receive replies: modules are fire-and-forget.
            if request.sender_module == HOST {
                let reply = reply_envelope(&broker, &request, ok, &json_text);
                if let Err(e) = broker.dispatch(reply).await {
                    log::error!("dropping reply for {}: {e}", request.message_id);
                }
            } else {
                // Forward as a notice so failures of module->module calls
                // are visible instead of silent.
                let notice = broker.envelope(&request.target_module, HOST, if ok { "reply" } else { "error" }, &json_text);
                on_host(notice);
            }
        });
    }
}
